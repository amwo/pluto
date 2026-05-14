# pluto deploy (AWS eu-west-2a)

## Target

| 項目 | 値 |
|---|---|
| Region | eu-west-2a (London) |
| Instance | c7i.large or c7a.large |
| OS | Amazon Linux 2023 |
| Postgres | 18 (managed via systemd) |
| User | `pluto` (system) |
| Working dir | `/opt/pluto` |
| Data dir | `/var/lib/pluto` |
| Secrets | `/opt/pluto/secrets/pluto.env` (sops or SSM) |

## One-shot bootstrap

```bash
sudo useradd -r -m -d /var/lib/pluto -s /usr/sbin/nologin pluto
sudo mkdir -p /opt/pluto/{bin,secrets} /var/lib/pluto
sudo chown -R pluto:pluto /opt/pluto /var/lib/pluto
```

## Build + ship

```bash
# on dev box
cargo build --release
scp target/release/pluto ec2:/tmp/pluto
ssh ec2 'sudo install -o pluto -g pluto -m 750 /tmp/pluto /opt/pluto/bin/pluto'
```

## Secrets (`/opt/pluto/secrets/pluto.env`)

Plain `KEY=value` per line, mode 0600, owner `pluto`. AWS SSM Parameter
Store fetch is preferred — see `fetch-secrets.sh` (TODO).

```
CHAINSTACK_GRPC_ENDPOINT=...
CHAINSTACK_HTTPS_ENDPOINT=...
CHAINSTACK_USERNAME=...
CHAINSTACK_PASSWORD=...
SOLANA_WALLET_ADDRESS=...
TARGET_WALLET=2FJiJVTHrLjqnReWVhNmSv5mPwd5a9bUTkJXcpaE7xrb
PLUTO_MODE=dry            # observe / dry / live
DATABASE_URL=postgres://pluto@/pluto?host=/var/run/postgresql
TELEGRAM_BOT_TOKEN=...
TELEGRAM_CHAT_ID=...
SOLANA_SIGNER_SECRET=...   # base58 64-byte ed25519 keypair (live only)
JITO_BLOCK_ENGINE_URLS=https://amsterdam.mainnet.block-engine.jito.wtf,https://frankfurt.mainnet.block-engine.jito.wtf,https://mainnet.block-engine.jito.wtf
```

## systemd

```bash
sudo install -m 644 deploy/pluto.service /etc/systemd/system/pluto.service
sudo systemctl daemon-reload
sudo systemctl enable --now pluto.service
journalctl -u pluto -f
```

## Operations

```bash
# graceful restart (rotates session)
sudo systemctl restart pluto

# stop with confirmation (waits for in-flight live exits up to 30s)
sudo systemctl stop pluto

# health check via daily report
sudo -u pluto /opt/pluto/bin/pluto report

# tail Telegram-relevant errors
journalctl -u pluto -p warning -n 200 --no-pager
```

## Rollback

`pluto run` always calls `mark_running_as_crashed()` (sessions) and
`mark_closing_as_crashed()` (positions) on startup. Stuck `closing`
positions left from a crash are surfaced as a startup WARN — operator
must manually inspect the corresponding signature on chain (Solscan)
to decide whether to re-credit, claim a refund, or treat as final.

## Live transition checklist (spec 8.2)

- [ ] dry mode で copy candidate >= 100
- [ ] simulated slippage 測定済み (daily report price impact P50/P95)
- [ ] route failure rate < 5% (daily report Latency section per kind)
- [ ] target sell follow exit が 24h 検証済み
- [ ] daily report 安定 (3 日連続クラッシュなし)
- [ ] `SOLANA_SIGNER_SECRET` を SSM Parameter Store にローテーション
- [ ] Jito endpoint への connectivity 確認 (curl amsterdam endpoint)
- [ ] PLUTO_MODE=live に切替 → systemctl restart
