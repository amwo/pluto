# pluto Spec — ブラッシュアップ版

要件書 (`solana_copy_bot_requirements.md`) を起点に、外部サービス公式情報・OSS 実装・実観察データ・ミルストーン構造の 4 角度でレビューした統合仕様。

## 1. プロジェクト概要

Solana 上で短期モメンタム型の勝ち wallet `2FJiJVTHrLjqnReWVhNmSv5mPwd5a9bUTkJXcpaE7xrb` を追従するコピーボット。狙いは「銘柄選定 + 早い損切り + route 品質」。**「2x TP を待つ戦略ではない」**。

### 1.1 観察データ要約 (直近 30 日)

| 指標 | 値 |
|---|---:|
| 平均 trade/日 | 18.35 |
| Buy/Sell | 178 / 244 |
| Buy total | 68.80 SOL |
| 平均 Buy size | 0.387 SOL |
| Median Buy size | 0.322 SOL |
| P90 Buy size | 0.602 SOL |
| 初回 Sell median | 0.914x (-8.6%) |
| 初回 Sell P75 | 1.071x (+7.1%) |
| 初回 Sell P90 | 1.218x (+21.8%) |
| Lifecycle median | 約 40 分 |
| 初回 Sell median | 約 14 分 |
| pump 系 Buy 比率 | 161/178 (90%) |
| 主 route | Jupiter → PumpSwap |

### 1.2 現状 (実装済み)

- gRPC subscriber (Chainstack Yellowstone, FRA region)
- Session DB (PostgreSQL, port 5435)
- Mode 型 (Observe / Paper / Live, default paper)
- adapter 層: `Grpc`, `Http`, `Db`
- domain 層: `Slot`, `Signature`, `Pubkey`, `Commitment`, `DetectedTx`, `StreamEvent`, `Subscription`, `Session`, `Mode`, `SkipReason`

### 1.3 設計の北極星

- **シンプル優先** — 不要な抽象避ける、状態なしは自由関数、状態あり struct
- **adapter は外部型を漏らさない** — proto/tonic/sqlx/reqwest を main/domain に見せない
- **domain は依存ゼロに近い** — thiserror のみ
- **観測 first** — paper/live 評価のため Monitoring を early に
- **safety は early に薄く、live 直前に厚く** — entry 抑止系は paper でも入れる

## 2. インフラ構成

### 2.1 配置

```
Region: AWS eu-west-2
AZ:     eu-west-2a
Instance: c7i.large または c7a.large から検証
OS:     Amazon Linux 2023
Secrets: AWS SSM Parameter Store (現状 sops/age key、AWS deploy 時に切替)
```

### 2.2 外部サービス

| 用途 | サービス | プラン |
|---|---|---|
| Stream (read) | **Chainstack Yellowstone gRPC** (FRA) | $49 1-stream |
| Quote / Build | **Jupiter Swap V2 `/order`** `mode=ultra` | Developer tier $25/mo, 10 RPS |
| Send (primary) | **Jito Block Engine `amsterdam`** `bundleOnly=true` | Free, 1 req/s/IP/region |
| Send (secondary) | **Jito `frankfurt`** | Free |
| Send (fallback) | **Jito `mainnet`** (LB) | Free |
| Tip floor | `bundles.jito.wtf/api/v1/bundles/tip_floor` | Free |
| Token metadata | **Jupiter Tokens V2** + DexScreener fallback | Free / paid |

### 2.3 重要な現状からの修正

- **要件書の `mainnet` primary → `amsterdam` primary に変更**: eu-west-2a から amsterdam が物理 8-12ms、frankfurt 15-18ms、mainnet (LB) は経由が増えて遅い
- 要件書 `Jupiter Ultra` 単独 API は **deprecated** (2025-10 Ultra V3 で Swap V2 に統合)。`Swap V2 mode=ultra` を使う

## 3. Send Path 詳細

```
[gRPC stream] target tx 検知
   ↓
decode (proto → ObservedTrade)
   ↓
filter (entry 9 条件)
   ↓
quote: Jupiter Swap V2 /order mode=ultra
   ↓
sign + jitodontfront marker (read-only AccountMeta 1個追加)
   ↓
send: Jito amsterdam /api/v1/transactions?bundleOnly=true
       + 1000-p75 lamports tip
       + priority fee 70%
   ↓ (300-500ms 内に着地確認なければ)
send fallback: frankfurt → mainnet (LB)
   ↓
confirm: getSignatureStatuses + Yellowstone slot stream の突き合わせ
        3 slot 以内に finalized なければ retry/abort
```

### 3.1 jitodontfront marker

- `jitodontfront` で始まる任意の有効 pubkey を tx の **任意の instruction の account に read-only で 1 個追加** するだけ
- Jito 経由で送信時、その tx を含む bundle は **tx が index 0 でない限り Block Engine に reject される** = sandwich front-run が物理的に不可
- 注意: (a) Jito 経由でないと無効、(b) back-run は防がない、(c) keypair は固定の `jitodontfront11111111111111111111111111111` 系を使い回し OK

### 3.2 Bundle vs single tx

- **通常**: `sendTransaction?bundleOnly=true` (revert protection 付き single tx、tip 不要だが入れた方が ranking 良い)
- **Bundle 化が必要なケース**: SL+TP atomic 同梱、CU 1.4M 超過、arb/sandwich 防御の追加 leg

## 4. Decoder 設計

### 4.1 採用方針: balance-delta-first

OSS 実装の de-facto は 2 通り:

1. **instruction discriminator + account list 固定 index** (sol-trade-sdk 流) — DEX 非互換、メンテ負荷高
2. **`meta.pre_/post_balances` + `pre_/post_token_balances` から balance delta 計算** (cutupdev 流) — DEX 非依存、`solana-transaction-status` 1 crate で 90% カバー

**pluto は (2) を主軸、program_id は補助情報として記録のみ**。

### 4.2 gRPC subscription を拡張

現状の `proto::decode_tx` は signature しか取らない。Chainstack Yellowstone は `SubscribeUpdateTransaction.transaction.meta` を流すので、それを `StreamEvent::Tx` に積めば **追加 RPC 0 回で decode できる**:

```
meta.pre_balances           // SOL delta 計算
meta.post_balances
meta.pre_token_balances     // token delta 計算
meta.post_token_balances
meta.inner_instructions     // route / program ID 抽出
meta.loaded_addresses       // ATL 解決
transaction.message         // signature, accountKeys
```

### 4.3 識別する program ID 集合

`domain/dex_registry.rs` (新設) に集約:

| DEX | Program ID |
|---|---|
| PumpFun (bonding curve) | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` |
| PumpSwap (AMM) | `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` |
| Raydium AMM v4 | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` |
| Raydium CPMM | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` |
| Raydium CLMM | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` |
| Bonk LaunchLab | `LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj` |
| Jupiter v6 | `JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4` |
| Jito anti-front-run | `jitodontfront11111111111JustUseJupiterU1tra` |

### 4.4 ObservedTrade 型 (domain)

```rust
pub enum Side { Buy, Sell, Unknown }

pub enum DexKind {
    PumpFun, PumpSwap, RaydiumAmmV4, RaydiumCpmm, RaydiumClmm,
    Bonk, Jupiter, Other,
}

pub struct ObservedTrade {
    pub signature: Signature,
    pub slot: Slot,
    pub block_time: Option<i64>,
    pub side: Side,
    pub mint: Option<Pubkey>,
    pub sol_delta_lamports: i64,           // wallet の SOL 増減
    pub token_delta: i128,                  // wallet の token 増減 (raw)
    pub route: Vec<DexKind>,                // 経由した DEX 列
    pub jupiter: bool,
    pub pump_swap: bool,
    pub jito_marker: bool,
    pub priority_fee_lamports: u64,
    pub compute_unit_limit: Option<u32>,
}
```

判定ルール:
- **Buy**: SOL decreased, token increased
- **Sell**: token decreased, SOL increased
- **Unknown**: それ以外 → `SkipReason::DecodeUncertain`

### 4.5 OSS 参考実装

| Repo | 用途 | 参考点 |
|---|---|---|
| [0xfnzero/sol-trade-sdk](https://github.com/0xfnzero/sol-trade-sdk) (302★) | DEX trade SDK | スタック上 borsh エンコード、program ID 集約方法 |
| [0xfnzero/sol-parser-sdk](https://github.com/0xfnzero/sol-parser-sdk) | gRPC イベント → DexEvent | TradeEvent 構造体の設計 |
| [cutupdev/Solana-Copytrading-bot](https://github.com/cutupdev/Solana-Copytrading-bot) (269★) | コピボット参考 | balance-delta decode, BondingCurveAccount |
| [keidev-sol/Solana-Copy-Trading-Bot-Rust](https://github.com/keidev-sol/Solana-Copy-Trading-Bot-Rust) | AutoSell + zero-block send | TP/SL/trailing actor task の構造 |

## 5. パラメータ最終決定

要件書のデフォルトを観察データに照らして補正。

### 5.1 変更推奨パラメータ表

| Parameter | 要件書 | 改訂後 | 理由 |
|---|---:|---:|---|
| `STOP_LOSS_PCT` | 12 | **18** | target follow exit が主軸、bot 単独 SL は暴落保護のみ |
| `TAKE_PROFIT_PCT` | 20 | **撤廃 or 50** | observed median 0.914x, P90 1.218x で +20% は発火しないデッドコード |
| `TAKE_PROFIT_SELL_FRACTION` | 0.50 | **0.40** | 残ホールドで trailing に委ねる |
| `TRAILING_STOP_PCT` | 15 | **8 (armed at +8%)** | +7% 帯で止まる挙動に合わせ早期 lock-in |
| `MAX_HOLD_SECONDS` | 2700 (45m) | **3600 (60m)** | lifecycle median 40min なので余裕持つ |
| `HARD_MAX_HOLD_SECONDS` | 3600 (60m) | **5400 (90m)** | P75 lifecycle に対応 |
| `MAX_COPY_SIZE_SOL` | 0.10 | **0.15** | fee/slippage 3-5% を edge で吸収 |
| `COPY_RATIO` | 0.20 | **0.25** | target median 0.322 → copy 0.08 SOL |
| `MAX_OPEN_POSITIONS` | 3 | **4** | target 同時並行 3-5 想定 |
| `MAX_DAILY_LOSS_SOL` | 0.5 | **0.9** | 2-3 連敗許容、過敏 trigger 回避 |
| Detection delay 上限 | 90s | **5s** | gRPC で 100ms 検知、stale 防御に厳格化 |
| `MAX_TARGET_BUY_SOL` | 1.0 | **0.7** | observed P90 = 0.602 SOL、大口裁量除外 |
| `POLL_INTERVAL_MS` | 1200 | **撤廃** | gRPC stream 化により無用 |

### 5.2 変更しないパラメータ

| Parameter | 値 | 理由 |
|---|---:|---|
| `MIN_TARGET_BUY_SOL` | 0.05 | observed min 帯 |
| `MAX_FAILED_SENDS_PER_10MIN` | 5 | route quality brake として妥当 |
| `MAX_SAME_MINT_REENTRIES` | 1 | pump 系は 1 回限り、target 再 buy は新シグナル |

### 5.3 追加すべき条件

Entry filter に追加:

1. **target wallet 直近 1h realized PnL < -0.3 SOL なら新規 entry suspend** (cold streak protection)
2. **同 mint で過去 fail / loss あり → skip** (再エントリ抑制)
3. **priority fee 異常高 (> 0.005 SOL) tx は MEV/insider 疑いで skip**
4. **mint creation < 30min かつ liquidity 不明な作成直後 snipe を明示 skip** (要件書 line 222 の文言を filter 化)
5. **target buy size > P95 (≈ 0.7 SOL) → 大口裁量疑いで skip**

## 6. 監視 metric (追加)

要件書記載 (target tx detected, decision reason, latency, PnL 等) に加えて:

1. **`target_wallet_recent_pnl_1h`** — 直近 1h realized PnL、cold streak detection 入力
2. **`grpc_stream_health`** — heartbeat interval, reconnect_count, slot_gap_detected。15 秒以上 silence で hard stop
3. **`roundtrip_cost_estimate_per_position`** — fee + tip + slippage の実測、breakeven 必要倍率を逆算
4. **`mint_blocklist`** — 同一 mint で 2 連敗 or route failure 発生 → 24h block
5. **`target_to_copy_fill_delta`** — target fill price と pluto fill price の乖離、route 品質 dashboard

## 7. DB 設計

### 7.1 採用: PostgreSQL (要件書は SQLite だが PG 維持)

理由: zeus と運用揃い、bigserial / uuid v7 / timestamptz / index が PG の方が運用しやすい。

### 7.2 必須テーブル

```
sessions             既存
observed_trades      Decoder 出力、全 tx を保存 (skip 含む)
copy_decisions       Filter 出力、Copy / Skip(reason) を必ず記録
orders               発注予定
positions            建玉
fills                約定
exits                exit イベント
latency_samples      gRPC / quote / send の latency
rpc_errors           外部 API エラー
daily_pnl            日次集計
mint_blocklist       追加: 同 mint NG リスト
```

### 7.3 マイグレーション順序

```
003_observed_trades.sql
004_copy_decisions.sql
005_positions_orders_fills_exits.sql
006_latency_rpc_errors.sql
007_daily_pnl_mint_blocklist.sql
```

## 8. 改訂版ミルストーン

要件書の 9 ステップを Codex review で 10 段階に再編。**DB 追加は Decoder/Filter と同時 (3 ではなく 1+2)**、**Monitoring を 4 番目に early 配置**、**Safety は段階的に薄→厚**。

| # | タイトル | 出来上がるもの | 依存 |
|---:|---|---|---|
| **1** | Decoder + observed_trades DB | proto → `ObservedTrade` 完全 decode、全 tx を `observed_trades` に保存 | gRPC, HTTP RPC (既存) |
| **2** | Entry Filter + copy_decisions DB | `ObservedTrade` → `CopyDecision { Copy{size} \| Skip(SkipReason) }`、両方 DB に保存 | 1 |
| **3** | **Observe 完全ループ** | observe mode で subscribe → decode → filter → DB log が止まらず回る。送信なし | 1, 2 |
| **4** | Monitoring v1 | 主要 metric (tx, decision, reason, detection delay, skip 集計, 日次 report 最小版) | 3 |
| **5** | Paper Send Path | Jupiter quote/swap を paper simulate、quote latency / price impact / route failure を記録 | 3, 4 |
| **6** | Position + Exit Simulation | paper position、target sell follow、SL/TP/trailing/max hold、PnL 集計 | 5 |
| **7** | Safety Gate v1 | daily kill switch、risk limit、max exposure、exit > entry priority、stale 検知 | 6 |
| **8** | Live Send Path | 明示 config 時のみ実 order。Jupiter Swap V2 mode=ultra + jitodontfront + Jito amsterdam → frankfurt → mainnet | 7 |
| **9** | Monitoring v2 + Live Readiness | daily report 安定、100 candidates、slippage、route failure、exit 再現性 | 8 |
| **10** | AWS eu-west-2a deploy | c7i/c7a、SSM secrets、運用ログ、再起動手順 | 9 |

### 8.0 Carry-over items (各 milestone で defer したもの)

| 出元 | 項目 | 内容 | 引き取り先 |
|---|---|---|---|
| 4 | **detection delay metric** | chain block_time vs pluto 検知時刻の差。slot-based 近似 (起動時に `getSlot` で reference 取得、`(slot - ref) * 0.4s` で推定) または `getBlockTime` 追加 RPC で取得して `observed_trades.block_time_estimate` 列を追加 | 5 (paper send 実装と同時、latency 関連) |
| 4 | **latency_samples テーブル** | quote / send / confirm の latency を時系列で蓄積 | 5 (送信パス実装で値が出る) |
| 4 | **追加 entry filter 条件** (spec 5.3) | cold streak (target_wallet 直近1h PnL)、同 mint loss history、priority fee 異常、mint creation < 30min、target size > P95 | 7 (Safety Gate と同時) |
| 4 | **mint_blocklist テーブル** | 2連敗 mint の 24h block | 7 |

### 8.1 Smallest complete loop

**Milestone 1+2+3** = observe mode で full pipeline (subscribe → decode → filter → DB log) が回って `observed_trades` + `copy_decisions` に観察結果が積まれる状態。Send/Position/Exit はまだ無い。これが **paper 移行可能な最小単位**。

### 8.2 Live 開始条件 (要件書通り)

- paper mode で copy candidate 100 件以上
- simulated slippage を測定済み
- route failure rate が許容範囲
- target sell 追従 exit が検証済み
- daily report が安定

## 9. アーキテクチャ追加 module

```
src/
├── domain/
│   ├── trade.rs            新: Side, ObservedTrade, DexKind
│   ├── decision.rs         新: CopyDecision (現 SkipReason をここへ移動)
│   ├── position.rs         新: Position, ExitReason
│   ├── dex_registry.rs     新: program ID 定数 + discriminator
│   ├── decoder.rs          既存: DetectedTx, raw tx → ObservedTrade 純粋関数
│   └── ...
├── adapters/
│   ├── grpc/               既存
│   ├── http/               既存 (拡張: get_balance に加えて get_account_info, get_transaction)
│   ├── db/                 既存 (sessions に加えて observed_trades, copy_decisions, positions, ...)
│   ├── jupiter/            新: Quote / Swap V2 client
│   └── jito/               新: bundleOnly send + tip floor watch
└── ...

migrations/
├── 001_sessions.sql                 既存
├── 002_session_mode.sql             既存
├── 003_observed_trades.sql          新
├── 004_copy_decisions.sql           新
├── 005_positions_orders_fills_exits.sql  新
├── 006_latency_rpc_errors.sql       新
└── 007_daily_pnl_mint_blocklist.sql 新
```

## 10. 未決事項への推奨判断

要件書 line 521-527 に対する明確な決定:

| 未決事項 | 推奨判断 |
|---|---|
| Jupiter Ultra の認証 / API limit | Ultra 単独 API は deprecated。**Swap V2 `mode=ultra` を Developer tier ($25/月, 10 RPS) で利用**。Free tier (1 RPS) で paper/observe |
| Live を Swap API か Ultra order flow か | **Swap V2 `/order` (managed)** 採用。`/build` (custom) は CPI 必要時まで不要 |
| Protected route だけで十分 / Jito direct bundle 必要か | **両方併用**。経路は Jupiter `mode=ultra` (Beam)、送信は自前で Jito `sendTransaction?bundleOnly=true` + jitodontfront。bundle (`/bundles`) は SL/TP atomic か CU 超過時のみ |
| Role-model wallet 追加 | **MVP は単一 wallet 固定**。第二 wallet は live 安定後に config 配列化で追加 |
| Token metadata 取得経路 | **Jupiter Tokens V2** 一次、**DexScreener** fallback、`getAccountInfo` Metaplex は最終手段 |

## 11. 修正履歴

- v1: 要件書ベース (`solana_copy_bot_requirements.md`)
- v2 (本ドキュメント): 公式情報 (Jupiter Swap V2 / Jito 9 region)、OSS de-facto pattern、観察データ実証、Codex review を統合

## 12. 参考文献

### Jupiter
- [Jupiter Developer Platform llms.txt index](https://developers.jup.ag/docs/llms.txt)
- [API Rate Limiting](https://developers.jup.ag/portal/rate-limit)
- [Migration to Developer Platform](https://developers.jup.ag/docs/portal/migration)
- [Ultra V3 blog](https://dev.jup.ag/docs/blog/ultra-v3)

### Jito
- [Low Latency Transaction Send](https://docs.jito.wtf/lowlatencytxnsend/)
- [Mainnet Block Engine Addresses](https://jito-labs.gitbook.io/mev/searcher-resources/block-engine/mainnet-addresses)
- [Solana Cookbook MEV Protection](https://solana.com/developers/cookbook/transactions/mev-protection)
- [QuickNode Jito Bundles Guide](https://www.quicknode.com/guides/solana-development/transactions/jito-bundles)
- [Tip Floor API](https://bundles.jito.wtf/api/v1/bundles/tip_floor)

### RPC providers
- [Chainstack Yellowstone gRPC](https://chainstack.com/marketplace/yellowstone-grpc-geyser-plugin/)
- [Helius LaserStream](https://www.helius.dev/laserstream)
- [Chainstack Best Solana RPC Providers 2026](https://chainstack.com/best-solana-rpc-providers-in-2026/)
- [RPCFast Low-Latency HFT Playbook](https://docs.rpcfast.com/solana/low-latency-solana-playbook-for-hft)

### OSS reference
- [0xfnzero/sol-trade-sdk](https://github.com/0xfnzero/sol-trade-sdk)
- [0xfnzero/sol-parser-sdk](https://github.com/0xfnzero/sol-parser-sdk)
- [cutupdev/Solana-Copytrading-bot](https://github.com/cutupdev/Solana-Copytrading-bot)
- [keidev-sol/Solana-Copy-Trading-Bot-Rust](https://github.com/keidev-sol/Solana-Copy-Trading-Bot-Rust)
