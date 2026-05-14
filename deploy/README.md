# pluto deploy (NixOS + deploy-rs + Terraform)

## レイアウト

```
deploy/
├── README.md           # このファイル
├── nixos/
│   ├── module.nix      # services.pluto NixOS module
│   └── host.nix        # EC2 host config (amazon-image + sops + module)
└── terraform/
    ├── main.tf         # AWS EC2 (NixOS AMI) + SG + Default VPC
    └── README.md
```

flake.nix:
- `packages.default` = pluto Rust binary (rust-bin nightly)
- `nixosModules.pluto` = imports nixos/module.nix
- `nixosConfigurations.pluto` = full system, imports module + host
- `deploy.nodes.pluto.profiles.system` = deploy-rs target

## 初回構築手順

### 1. EC2 起動 (Terraform)

```bash
cd deploy/terraform
cat > terraform.tfvars <<EOF
operator_ssh_key = "ssh-ed25519 AAAA... operator@dev"
operator_cidr    = "203.0.113.42/32"   # 自分の固定 IP (なければ 0.0.0.0/0)
EOF

terraform init
terraform apply
```

Output に `public_ip` と `deploy_command` が出る。

### 2. host.nix の認証鍵を反映

`deploy/nixos/host.nix` の `users.users.root.openssh.authorizedKeys.keys` に
operator の SSH 公開鍵を追加し、commit。

### 3. sops age key を EC2 に登録

NixOS AMI 起動時に生成された `/etc/ssh/ssh_host_ed25519_key` を sops 復号鍵にする
(`sops.age.sshKeyPaths` で host.nix が指定済み)。

操作端末で:

```bash
ssh -i ~/.ssh/id_ed25519 root@<public_ip> 'cat /etc/ssh/ssh_host_ed25519_key.pub' | \
  ssh-to-age >> .sops.yaml      # 適切に reformat して age recipient 追加
sops updatekeys secrets/pluto.yaml
git add secrets/pluto.yaml .sops.yaml && git commit
```

### 4. 初回 deploy

```bash
nix develop
deploy --hostname <public_ip> .#pluto
```

### 5. 2 回目以降

```bash
git push                # CI が自動で deploy (.github/workflows/deploy.yml)
# or 手動: deploy --hostname <public_ip> .#pluto
```

## CI/CD (GitHub Actions)

### `.github/workflows/ci.yml` (PR + push)

- nix-installer-action (DeterminateSystems v17)
- flakehub-cache-action v2 (FlakeHub Cache transparent fetch + push)
- nix build pluto package
- nix build nixosConfigurations.pluto system closure
- cargo test --lib (in dev shell)
- cargo clippy --all-targets -- -D warnings
- nix flake check (deploy-rs static checks)

### `.github/workflows/deploy.yml` (main push)

- 2 ジョブ: `build` → `deploy`
- build: pluto + system closure を runner で nix build (FlakeHub Cache から fetch)
- deploy: SSH 鍵設定 → `deploy-rs --magic-rollback --auto-rollback` で EC2 にdiff だけ転送

### 必要な GitHub secrets / variables

| 種類 | 名前 | 内容 |
|---|---|---|
| variable | `PLUTO_HOST` | EC2 public IP (terraform output) |
| secret | `DEPLOY_SSH_KEY` | root@pluto 用 ed25519 秘密鍵 |
| secret | `DEPLOY_KNOWN_HOSTS` | EC2 の `ssh-keyscan -t ed25519 <ip>` 出力 |

`DEPLOY_KNOWN_HOSTS` は EC2 起動後 1 回:

```bash
ssh-keyscan -t ed25519 $(terraform -chdir=deploy/terraform output -raw public_ip)
```

(必要なら `gh secret set DEPLOY_KNOWN_HOSTS < known_hosts.txt`)

### 速度最適化 (2026 ベストプラクティス)

- **FlakeHub Cache** (DeterminateSystems): 2 回目以降は build 結果を CI ランナー間で共有、cold cache でも nixpkgs は cache.nixos.org から fetch
- **`nix copy --to ssh://`** (deploy-rs 内部): EC2 に既に存在する store path はスキップ、純差分だけ転送
- **`--skip-checks`**: deploy 段階で flake check を再実行しない (CI で済んだ)
- **`--magic-rollback`**: deploy 後に SSH 接続が回復しなければ自動 rollback (broken deploy で SSH ロックアウトされない)

実測 cold-cache: ~8 min build + ~30 sec deploy。warm cache: ~2 min build + ~30 sec deploy。

## 運用

```bash
# 状態確認
ssh root@<public_ip> systemctl status pluto

# ログ
ssh root@<public_ip> journalctl -u pluto -f

# daily report
ssh root@<public_ip> -- sudo -u pluto pluto report

# graceful restart (in-flight live exit を 30s 待つ)
ssh root@<public_ip> systemctl restart pluto

# rollback (前世代に戻す)
ssh root@<public_ip> /run/current-system/bin/switch-to-configuration switch
# or:
deploy --hostname <public_ip> --rollback .#pluto
```

## crash recovery

`pluto` は起動時に:
- `sessions.mark_running_as_crashed()` で前 session を crashed 化
- `positions.mark_closing_as_crashed()` で送信途中で死んだ live position を
  crashed 化、件数を WARN ログ

operator は対応 signature を Solscan で確認して fund 状態判定。

## Live transition checklist (spec 8.2)

- [ ] dry mode で copy candidate >= 100
- [ ] simulated slippage 測定済み (daily report price impact P50/P95)
- [ ] route failure rate < 5% (daily report Latency section per kind)
- [ ] target sell follow exit が 24h 検証済み
- [ ] daily report 安定 (3 日連続クラッシュなし)
- [ ] `secrets/pluto.yaml` に `SOLANA_SIGNER_SECRET` 追加 (sops で暗号化)
- [ ] Jito endpoint への connectivity 確認 (curl amsterdam endpoint)
- [ ] `secrets/pluto.yaml` の `PLUTO_MODE=live` に変更 → deploy
