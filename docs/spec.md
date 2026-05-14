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
- Mode 型 (Observe / Dry / Live, default dry)
- adapter 層: `Grpc`, `Http`, `Db`
- domain 層: `Slot`, `Signature`, `Pubkey`, `Commitment`, `DetectedTx`, `StreamEvent`, `Subscription`, `Session`, `Mode`, `SkipReason`

### 1.3 設計の北極星

- **シンプル優先** — 不要な抽象避ける、状態なしは自由関数、状態あり struct
- **adapter は外部型を漏らさない** — proto/tonic/sqlx/reqwest を main/domain に見せない
- **domain は依存ゼロに近い** — thiserror のみ
- **観測 first** — dry/live 評価のため Monitoring を early に
- **safety は early に薄く、live 直前に厚く** — entry 抑止系は dry でも入れる

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
| **5** | Dry Send Path | Jupiter quote/swap を dry simulate、quote latency / price impact / route failure を記録 | 3, 4 |
| **6** | Position + Exit Simulation | dry position、target sell follow、SL/trailing/max hold (TP は spec 5.1 で撤廃)、PnL 集計、**🔴 SELL Telegram 通知 (PnL 付き)** | 5 |
| **7** | Safety Gate v1 | daily kill switch、risk limit、max exposure、exit > entry priority、stale 検知 | 6 |
| **8** | Live Send Path | 明示 config 時のみ実 order。Jupiter Swap V2 mode=ultra + jitodontfront + Jito amsterdam → frankfurt → mainnet | 7 |
| **9** | Monitoring v2 + Live Readiness | daily report 安定、100 candidates、slippage、route failure、exit 再現性 | 8 |
| **10** | AWS eu-west-2a deploy | c7i/c7a、SSM secrets、運用ログ、再起動手順 | 9 |

### 8.0 Carry-over items (各 milestone で defer したもの)

| 出元 | 項目 | 内容 | 状態 |
|---|---|---|---|
| 4 → 5 | detection delay metric | slot-based 近似 (起動時 `getSlot` で reference、`(slot - ref) * 400ms` で block_time 推定)、`observed_trades.detection_delay_ms` 列 | ✅ done (milestone 5) |
| 4 → 5 | quote latency | `dry_trades.quote_latency_ms` 列 (旧 `paper_trades`, milestone 6 phase 6c で rename) | ✅ done (milestone 5) |
| 4 → 8 | send / confirm latency (`latency_samples` テーブル) | 送信パス実装後に時系列 metric として蓄積 | 引き取り先: 8 (Live Send Path) |
| 4 → 7 | 追加 entry filter 条件 (spec 5.3) | cold streak (target_wallet 直近1h PnL)、同 mint loss history、priority fee 異常、mint creation < 30min、target size > P95 | 引き取り先: 7 (Safety Gate) |
| 4 → 7 | mint_blocklist テーブル | 2連敗 mint の 24h block | 引き取り先: 7 |
| 5 → 6 | 🔴 SELL Telegram 通知 | target SELL に追従して dry exit、PnL/差分/route を含む完全フォーマット (cost basis + realized PnL は position が前提) | ✅ done (milestone 6 phase 6a) |
| 6 | SL / Trailing / MaxHold exit | `domain::exit::should_exit` 純粋関数、30秒 tick で各 open position に Jupiter quote → peak 更新 → 判定、`out_amount=0` ガード | ✅ done (milestone 6 phase 6b) |
| 6 → 8 | `check_exits` の main loop ブロック解消 | `Db`/`Http`/`Jupiter`/`Telegram` を `Arc` 化、`exit_tick` 内で `Semaphore::try_acquire_owned` ガードしつつ `tokio::spawn` | ✅ done (milestone 8 phase 8-a) |
| 4 → 8 | `latency_samples` テーブル | migration 013、`LatencyKind { JupiterQuote, JupiterSwap, JitoSend, JitoConfirm }`、`Db::latency_samples().insert(session, kind, ms, success, detail)`。Jupiter quote 3 callsite + executor の各 step で記録 | ✅ done (milestone 8 phase 8-a) |
| 7 phase 7a | stale + max_open entry filter | `detection_delay_ms > 5000` で `StaleDetection`、`open_positions >= 4` で `MaxOpenPositions`。`FilterContext` 経由で domain pure 維持。Observe は `open_positions = 0` 固定 | ✅ done (milestone 7 phase 7a) |
| 7 phase 7b | mint_blocklist | `mint_blocklist` テーブル新設、close 時に `realized_pnl < 0` なら `record_loss`、`>= 0` なら `clear`。`loss_count >= 2` かつ `last_loss_at > NOW() - 24h` で `MintBlocked` skip | ✅ done (milestone 7 phase 7b) |
| 7 phase 7c | daily kill switch | `positions.realized_pnl_today()` を `FilterContext.daily_realized_pnl_lamports` に詰めて `decide` に渡す。`<= -900_000_000` lamports で `DailyLossLimit` skip。集計範囲は **UTC calendar day** (rolling 24h ではない) | ✅ done (milestone 7 phase 7c) |
| 7 → 9 | safety-gate DB adapter integration test | `mint_blocklist` UPSERT / `realized_pnl_today` 集計の adapter テスト。本番前に postgres harness で 1 回実行 | 引き取り先: 9 (Live Readiness) |
| 7 → 9 | blocklist 更新失敗時のアラート | `update_mint_blocklist` のエラーは現在 warn ログのみ。monitoring v2 で metric 化 | 引き取り先: 9 |
| 8 phase 8-a | Live Send Path skeleton | `adapters/signer` (ed25519-dalek), `adapters/jupiter::{quote_raw, build_swap}` (`/swap/v2/order` w/ taker), `adapters/jito` (`sendTransaction?bundleOnly=true`, amsterdam→frankfurt→mainnet fallback), `adapters/tx::sign_versioned_tx_b64` (shortvec-aware), `adapters/executor::LiveExecutor`。Live mode 起動時に `SOLANA_SIGNER_SECRET` + `JITO_BLOCK_ENGINE_URLS` 必須、欠落で bail。`handle_buy` / `handle_sell` / `check_exits` の close path に live ブランチ追加 (dry path は無変更) | ✅ done (milestone 8 phase 8-a) |
| 9 phase 9-a | Monitoring v2 daily report 拡張 | `DailyReport` に detection_delay P50/P95、`LatencyStats { kind, samples, success_count, p50, p95 }` リスト、closed positions / wins / losses / realized PnL 追加。`reports.rs` で `percentile_disc(...) WITHIN GROUP` 集計 | ✅ done (milestone 9 phase 9-a) |
| 8 phase 8-b | Jupiter Swap API self-managed flow | 公式 docs (developers.jup.ag/llms.txt) 確認結果: V2 `/order` + `/execute` は managed landing 専用、self-managed (Jito 直送) は **V1 `/quote` + `/swap`** か V2 `/build` (router path)。pluto は V1 を採用: `quote_v1` (GET `/swap/v1/quote`)、`build_swap_v1` (POST `/swap/v1/swap` w/ `quoteResponse + userPublicKey + wrapAndUnwrapSol`) → `{swapTransaction, lastValidBlockHeight}` | ✅ done (milestone 8 phase 8-b #2) |
| 8 phase 8-c | live exit > entry priority | `Arc<AtomicU32>` カウンタを `ExitInFlightGuard` (RAII) で increment/decrement、`execute_live_sell` と `check_exits` Live branch が send 中保持。`handle_buy` は Live mode 時 counter > 0 で `SkipReason::ExitInProgress` 記録して早期return | ✅ done (milestone 8 phase 8-c) |
| 4 → 7 phase 7d | spec 5.3 priority fee filter | `ObservedTrade.priority_fee_lamports > max_priority_fee_lamports (5_000_000)` で `SkipReason::PriorityFeeAnomaly` | ✅ done (milestone 7 phase 7d) |
| 4 → 7 phase 7d | spec 5.3 cold streak filter | `ObservedTrades::target_recent_pnl_lamports(target, 1h)` で target の直近1h SOL delta 集計、`< -300_000_000` で `SkipReason::TargetColdStreak`。`mint_blocklist` は同 mint 抑制、cold streak は wallet 全体の調子悪検知 | ✅ done (milestone 7 phase 7d) |
| - | crash recovery for stuck `closing` positions | 起動時 `Positions::mark_closing_as_crashed()` で前回 send mid-flight crash 由来の closing → crashed に強制遷移、件数を WARN ログ。Operator は対応 signature を on-chain で確認して fund 状態判定 | ✅ done |
| 9 phase 9-b | route failure rate + price_impact | `DailyReport` に `dry_trades_count / dry_trades_failed`, `price_impact_p50/95_bps`, `route_labels_top` (top 5 DEX) を追加。Latency section の per-kind success_rate と合わせて route 品質を観測 | ✅ done (milestone 9 phase 9-b) |
| 10 | NixOS module (pluto.service) | `deploy/nixos/module.nix`: services.pluto NixOS option、postgres + user + systemd unit (Type=exec, KillSignal=SIGINT 30s, ProtectSystem=strict)、sops-nix で `/run/secrets/pluto.env` 生成。`deploy/nixos/host.nix`: amazon-image import + ssh + sops age key (`/etc/ssh/ssh_host_ed25519_key`) | ✅ done (milestone 10 NixOS) |
| 10 | flake.nix deploy-rs 統合 | `packages.default = pluto Rust binary` (rust-bin nightly + makeRustPlatform)、`nixosConfigurations.pluto` (amazon-image + sops-nix + module)、`deploy.nodes.pluto.profiles.system` (deploy-rs target、root SSH)、`checks` (deploy-rs static) | ✅ done (milestone 10 flake) |
| 10 | Terraform 最低限 | `deploy/terraform/main.tf`: NixOS AMI (eu-west-2、24.11)、Default VPC / Default subnet (eu-west-2a)、Security Group (SSH ingress + egress all)、EC2 (root volume のみ)。EIP / Route53 / CloudWatch / EBS 別ボリューム / VPC 新規作成 / IAM role / SSM は **不要として明示的に除外** | ✅ done (milestone 10 IaC) |
| 10 | GitHub Actions (2026 best practices) | `.github/workflows/ci.yml` (PR/push: nix build + cargo test + clippy + flake check)、`.github/workflows/deploy.yml` (main push: 2-job build→deploy、`DeterminateSystems/nix-installer-action@v17` + `flakehub-cache-action@v2` で warm cache、`deploy-rs --magic-rollback --auto-rollback` で diff 転送 + 自動 rollback)。実測 cold ~8min / warm ~2min build + ~30s deploy | ✅ done (milestone 10 CI/CD) |
| 8 phase 8-b | live tx confirmation latency | `Http::get_signature_status(sig) -> SignatureStatus { Pending, Processed, Confirmed, Finalized, Failed(err) }`。`LiveExecutor.wait_for_confirmation`: 400ms poll, 30s timeout、確定で `JitoConfirm` 成功記録、err / timeout で失敗記録 | ✅ done (milestone 8 phase 8-b #4) |
| 8 phase 8-b [CRITICAL] | live double-sell race | `PositionStatus::Closing` 追加、`Positions::try_claim_for_close(id)` (atomic open→closing) と `release_claim(id)` (closing→open リリース) を新設。`execute_live_sell` と `check_exits` Live branch は send 前に claim、失敗で skip。`close()` の WHERE を `IN ('open','closing')` に緩和。`find_open_by_mint` / `list_open` / `count_open` は `status='open'` のままなので claim 中の position は次の検査から消える | ✅ done (milestone 8 phase 8-b #1) |
| 8 → 8-b [CRITICAL] | Jupiter swap order → Jito 直送の整合性 | 現在 `/swap/v2/order` を Jito `sendTransaction?bundleOnly=true` に直送している。Jupiter Z/RFQ 系の order は managed `/swap/v2/execute` 経由で landing する前提なので、self-managed 送信したいなら `/swap/v2/build` 系または `/swap/v1/swap` (legacy) に切り替える必要あり。Codex review 2026-05-13 指摘 | 引き取り先: 8 phase 8-b (live 解放前 必須) |
| 8 phase 8-b [HIGH] | live exit quote latency 二重取得 | check_exits Live ブランチで `executed_quote_latency_ms` を `outcome.quote_latency_ms` (executor 内部 quote) に切替、`dry_trades.quote_latency_ms` に正しい実送信 latency を記録 | ✅ done (milestone 8 phase 8-b #3) |

### 8.1 Smallest complete loop

**Milestone 1+2+3** = observe mode で full pipeline (subscribe → decode → filter → DB log) が回って `observed_trades` + `copy_decisions` に観察結果が積まれる状態。Send/Position/Exit はまだ無い。これが **dry 移行可能な最小単位**。

### 8.2 Live 開始条件 (要件書通り)

- dry mode で copy candidate 100 件以上
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
| Jupiter Ultra の認証 / API limit | Ultra 単独 API は deprecated。**Swap V2 `mode=ultra` を Developer tier ($25/月, 10 RPS) で利用**。Free tier (1 RPS) で dry/observe |
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
