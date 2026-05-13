# Solana Copy Bot 要件書

## 目的

Solana上で、短期モメンタム型の勝ちwalletを追従するコピーbotを作る。初期の追跡対象walletは以下。

```text
2FJiJVTHrLjqnReWVhNmSv5mPwd5a9bUTkJXcpaE7xrb
```

狙うのは「大口の単発裁量トレード」ではなく、少額で機械的に繰り返されている取引パターンのコピー。

## 追跡対象walletの観察結果

直近30日分析。

| 指標 | 値 |
|---|---:|
| 総トレード数 | 422 |
| Buy | 178 |
| Sell | 244 |
| Buyしたmint数 | 98 |
| Close済みmint数 | 95 |
| Buy総量 | 68.800644 SOL |
| Sell総量 | 122.910067 SOL |
| Cashflow PnL | 54.109423 SOL |
| Close済みmint PnL | 10.592615 SOL |
| 平均Buy size | 0.386520 SOL |
| Median Buy size | 0.322128 SOL |
| P90 Buy size | 0.602114 SOL |
| 稼働日数 | 23日 |
| 平均トレード数/日 | 18.35 |
| 最大トレード数/日 | 40 |

トークンとルートの傾向。

| 項目 | 観察結果 |
|---|---|
| 主対象 | pump系トークン |
| pump suffix Buy数 | 161 / 178 |
| pump suffix Buy量 | 62.589489 / 68.800644 SOL |
| 主ルート | Jupiter -> PumpSwap |
| 初回BuyでJupiterあり | 91 / 98 sampled mints |
| 初回BuyでPumpSwapあり | 94 / 98 sampled mints |
| Raydium | サンプル上はほぼなし |
| Orca | まれ |
| pump.fun作成直後snipe | 観察されず |
| `jitodontfront` marker | 一部の初回Buyであり |

時間軸とexit傾向。

| 指標 | 観察結果 |
|---|---|
| 初回Sellまでのmedian | 約14分 |
| 初回SellまでのP75 | 約29分 |
| lifecycle median | 約40分 |
| 初回Sell倍率 median | 0.914x |
| 初回Sell倍率 P75 | 1.071x |
| 初回Sell倍率 P90 | 1.218x |
| 典型挙動 | 高倍率TPではなく、短期回転・損切り寄り |

解釈。

- 作成直後の秒単位snipe専用botではない。
- pump/PumpSwap系トークンを、一定の勢いや流動性が出た後に選んでいる可能性が高い。
- entryがmint直後数秒ではないため、コピー可能性はある。
- edgeは「銘柄選定 + 早い損切り + route品質」であり、単純な高TPホールドではない。

## 配置リージョン

AWS `eu-west-2` を使う。

`t3.nano` で各AZを実測。各12サンプルの中央値。

| Target | eu-west-2a | eu-west-2b | eu-west-2c | 最速 |
|---|---:|---:|---:|---|
| ChainStack `getSlot` | 48.0ms | 38.2ms | 40.5ms | 2b |
| ChainStack `getLatestBlockhash` | 48.2ms | 39.3ms | 37.8ms | 2c |
| Jupiter quote RTT | 26.0ms | 28.2ms | 28.2ms | 2a |
| Jito mainnet | 12.4ms | 14.4ms | 15.6ms | 2a |
| Jito Amsterdam | 31.7ms | 34.7ms | 33.3ms | 2a |
| Jito Frankfurt | 48.4ms | 51.2ms | 50.0ms | 2a |
| Jito NY | 206.9ms | 206.9ms | 207.1ms | 2a/2b同等 |
| Jito Tokyo | 634.9ms | 638.6ms | 635.9ms | 2a |

配置判断。

```text
Primary region: eu-west-2
Primary AZ: eu-west-2a
理由: JupiterとJitoが最速。copy botではquote/send経路が重要。
```

RPC単体では `2b/2c` が少し良いが、blockhashを常時prefetchすれば差は吸収しやすい。総合では `eu-west-2a` を第一候補にする。

## インフラ要件

初期構成。

```text
Region: eu-west-2
AZ: eu-west-2a
Instance: c7i.large または c7a.large から検証開始
OS: Amazon Linux 2023相当
Network: public egressあり
Secrets: AWS SSM Parameter Store
```

使う外部サービス。

```text
ChainStack HTTPS RPC
Jupiter Lite/Swap API または Ultra API
Jito block engine endpoints
AWS SSM
```

役割分担。

```text
RPC read: ChainStack HTTPS
Quote/build: Jupiter
Protected send: Jupiter Ultra / Jito anti-front-run
Jito direct fallback: mainnet + Amsterdam
```

runtime workspaceは `/home/am/Projects/edge` または専用deployディレクトリに置く。`/home/am/Projects/zeus` は実行環境として使わない。

## 基本フロー

1. 対象walletのsignatureを監視する。
2. 新規txを取得してdecodeする。
3. 対象walletのBuy/Sellを判定する。
4. Buyならentry filterを通す。
5. 通過したら縮小sizeでcopy orderを作る。
6. protected routeで送信する。
7. copy positionを管理する。
8. 対象walletのSell、stop loss、take profit、trailing stop、max holdでexitする。
9. すべての判断、order、fill、PnLを記録する。

## Watcher要件

gRPCは前提にしない。ChainStack HTTPS RPCで監視する。

```text
method: getSignaturesForAddress(target_wallet)
初期間隔: 1.0s - 1.5s
backoff: 429 / 5xx / timeout / latency spikeでadaptive
concurrency: 低め。target wallet監視を最優先。
```

保持する状態。

```text
last_seen_signature
seen_signature_lru
slot_lag_estimate
rpc_latency_stats
rate_limit_state
```

live copy loopでは全チェーンscanをしない。既知のrole model walletだけを追う。

## Tx Decoder要件

対象walletのtxごとに以下をdecodeする。

```text
signature
slot
block_time
対象walletのSOL delta
対象walletのtoken balance delta
mint
side: buy / sell / unknown
quote amount in SOL
base amount
route/program IDs
Jupiter有無
PumpSwap有無
jitodontfront marker有無
priority fee / compute budget instruction
```

判定ルール。

- Buy: 対象walletがSOLまたはSOL相当quoteを減らし、token balanceを増やしたtx。
- Sell: 対象walletがtoken balanceを減らし、SOLまたはSOL相当quoteを増やしたtx。
- deltaが不明瞭なtxは無理にcopyしない。

重要program。

```text
Jupiter v6: JUP6...
PumpSwap: pAMM...
Jito anti-front-run marker: jitodontfront11111111111JustUseJupiterU1tra
ComputeBudget program
```

## Entry Filter

初期entry条件。

```text
target side = buy
target buy size <= 1.0 SOL
target buy size >= 0.05 SOL
mintがpump系、またはrouteにPumpSwapあり
routeにJupiterまたはPumpSwapあり
target tx block_timeからlocal検知まで <= 90秒
同mintのopen copied positionがない
daily kill switchが発動していない
```

推奨レンジ。

```text
target buy size: 0.10 - 0.75 SOL
copy size: target buy sizeの10% - 30%
初期max copy size: 0.05 - 0.15 SOL
mint age: わかる場合は30分以上、24時間以内を優先
作成直後snipeは初期では無効
```

paper modeで追加検証したいfilter。

```text
target txがJupiter + PumpSwap
target txにjitodontfront markerあり
直近volume増加
最低流動性以上
Jupiter quoteのprice impactが閾値以下
spread/slippageが閾値以下
```

## Exit Rule

最優先exit。

```text
対象walletがcopy済みmintをSellしたら即Sell。
```

保護exit。

```text
Stop loss: -10% - -15%
Take profit: +20%で50%売却
Trailing stop: +15%以降またはTP後、高値から-15%
Max hold: 初期45分
Hard max hold: 60分
```

理由。

- 対象walletは初回Sellがbreak-even以下のことも多い。
- 2x固定TPを待つ戦略ではない。
- 大きく勝つより、間違ったentryを早く切ることが重要。

## Position Sizing

初期式。

```text
copy_size_sol = min(target_buy_sol * 0.20, 0.10 SOL)
absolute min copy = 0.01 SOL
absolute max copy = 0.15 SOL
```

risk limit。

```text
max open positions: 3
max exposure per mint: 0.15 SOL
max total open exposure: 0.45 SOL
max daily realized loss: 0.5 SOL
max failed sends per 10 minutes: 5
max same-mint reentries: 1
```

最初は必ずpaper mode。live化はcopy候補、slippage、exit再現性を確認してから。

## Send Path要件

優先send path。

```text
Jupiter Ultra または protected Jupiter route
anti-front-run routeが使える場合は使う
priority feeを使う
blockhashは事前prefetchしたものを使う
```

Jito direct fallback。

```text
Primary: mainnet.block-engine.jito.wtf
Secondary: amsterdam.mainnet.block-engine.jito.wtf
Optional: frankfurt.mainnet.block-engine.jito.wtf
NY/Tokyoはeu-west-2からは遅いため通常使わない
```

注意。

- `jitodontfront` はanti-front-run/protected routing marker。
- それ自体は「手組みJito bundle」の証拠ではない。
- Jito bundleはblock-engine submission pathで、単一txだけでは判定できないことがある。

## Latency要件

HTTP connectionはkeep-alive必須。

```text
HTTP client: keep-alive
RPC blockhash prefetch: 400ms - 1000msごと
target wallet polling: 1.0s - 1.5s
quote request timeout: 800ms以下
RPC live path timeout: 1000ms以下
send timeout: 1000ms以下でretry/fallback
```

実行path。

```text
target tx検知
decode
filter
quote/build swap
sign
protected send
confirmは非同期
```

final confirmation待ちで次の検知ループを止めない。

## Rate Limit / Semaphore要件

別々のsemaphoreを持つ。

```text
rpc_read
rpc_heavy
jupiter_quote
jupiter_swap
jito_send
```

初期値。

```text
rpc_read concurrency: 2
rpc_heavy concurrency: 1
jupiter_quote concurrency: 2
jupiter_swap concurrency: 1
jito_send concurrency: 2
```

backoff trigger。

```text
HTTP 429
HTTP 5xx
timeout
median latency > baselineの2倍が30秒継続
decode failure連続
```

backoff動作。

```text
polling intervalを広げる
concurrencyを下げる
非必須metadata lookupを止める
position exitは可能な限り止めない
```

## DB要件

初期はSQLiteでよい。

```text
/db/edge/solana_copy_bot.sqlite
```

必要テーブル。

```text
target_transactions
decoded_trades
copy_decisions
orders
positions
fills
exits
latency_samples
rpc_errors
daily_pnl
```

copyしなかったtradeには理由を必ず残す。

```text
too_large
too_small
unknown_route
stale_detection
existing_position
high_price_impact
risk_limit
decode_uncertain
rate_limited
```

## Mode

必須mode。

```text
observe: decodeとlogのみ
paper: copy entry/exitをsimulate
live: 実order。ただし厳格capあり
```

defaultは必ず `paper`。

live modeは明示config必須。

```text
LIVE_TRADING=true
MAX_COPY_SIZE_SOL set
MAX_DAILY_LOSS_SOL set
WALLET_KEYはSSMまたは安全なlocal secret
```

## Monitoring要件

常時出すもの。

```text
target tx detected
buy/sell classification
copy decision
decision reason
detection delay
quote latency
send latency
open positions
realized PnL
unrealized PnL
daily loss
RPC/Jupiter/Jito error counts
```

日次report。

```text
date
target trades observed
copy candidates
copied trades
skipped trades by reason
win/loss count
realized PnL
fees
average detection delay
average quote latency
average send latency
largest loss
largest win
```

## Safety要件

hard stop条件。

```text
daily loss limit到達
RPC stale > 10秒
blockhash prefetch stale > 5秒
wallet balanceがreserve未満
exit失敗が3連続
想定外のtoken account ownership
```

新規entryよりexitを常に優先する。

live trading開始条件。

```text
paper modeでcopy candidate 100件以上
simulated slippageを測定済み
route failure rateが許容範囲
target sell追従exitが検証済み
daily reportが安定
```

## 初期Config

```text
TARGET_WALLET=2FJiJVTHrLjqnReWVhNmSv5mPwd5a9bUTkJXcpaE7xrb
AWS_REGION=eu-west-2
AWS_AZ=eu-west-2a
MODE=paper
POLL_INTERVAL_MS=1200
MAX_TARGET_BUY_SOL=1.0
MIN_TARGET_BUY_SOL=0.05
COPY_RATIO=0.20
MAX_COPY_SIZE_SOL=0.10
MAX_OPEN_POSITIONS=3
MAX_TOTAL_EXPOSURE_SOL=0.45
STOP_LOSS_PCT=12
TAKE_PROFIT_PCT=20
TAKE_PROFIT_SELL_FRACTION=0.50
TRAILING_STOP_PCT=15
MAX_HOLD_SECONDS=2700
HARD_MAX_HOLD_SECONDS=3600
PRIMARY_JITO_ENDPOINT=mainnet
SECONDARY_JITO_ENDPOINT=amsterdam
```

## 未決事項

- Jupiter Ultraの認証/API limitが使えるか。
- live executionをJupiter Swap APIで組むか、Ultra order flowにするか。
- protected Jupiter routeだけで十分か、Jito direct bundleまで必要か。
- role model walletを追加するか。
- token metadataをRPCだけで取るか、DexScreenerなど無料APIを併用するか。
