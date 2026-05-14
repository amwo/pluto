# pluto terraform (minimum)

最低限のリソース:
- Default VPC + AZ subnet (新規 VPC 作らない)
- Security Group (egress only — SSH なし、SSM Session Manager で入る)
- IAM Role + Instance Profile (SSM Managed + `/pluto/*` SecureString 読み取り)
- SSM Parameter Store (secrets)
- EC2 instance (root volume のみ、EBS 別追加なし)

無し: EIP / Route53 / EBS / CloudWatch / VPC

## 使い方

```bash
cd deploy/terraform

cat > terraform.tfvars <<'EOF'
secrets = {
  CHAINSTACK_GRPC_ENDPOINT  = "yellowstone-solana-mainnet.core.chainstack.com"
  CHAINSTACK_HTTPS_ENDPOINT = "https://solana-mainnet.core.chainstack.com"
  CHAINSTACK_USERNAME       = "..."
  CHAINSTACK_PASSWORD       = "..."
  SOLANA_WALLET_ADDRESS     = "..."
  TARGET_WALLET             = "2FJiJVTHrLjqnReWVhNmSv5mPwd5a9bUTkJXcpaE7xrb"
  PLUTO_MODE                = "dry"
  TELEGRAM_BOT_TOKEN        = "..."
  TELEGRAM_CHAT_ID          = "..."
  DATABASE_URL              = "postgres://pluto@localhost/pluto"
  JITO_BLOCK_ENGINE_URLS    = "https://amsterdam.mainnet.block-engine.jito.wtf,..."
}
EOF

terraform init
terraform apply
```

`terraform output ssm_session_command` で出る `aws ssm start-session` を実行して shell に入る。

## binary push

EIP 無し / SSH 無しなので、binary は SSM Run Command 経由で取得:

```bash
# dev box でビルド + S3 にアップ
cargo build --release
aws s3 cp target/release/pluto s3://your-bucket/pluto

# EC2 に取得 (instance ID は terraform output)
INSTANCE_ID=$(terraform output -raw instance_id)
aws ssm send-command --instance-ids $INSTANCE_ID --document-name AWS-RunShellScript \
  --parameters 'commands=["aws s3 cp s3://your-bucket/pluto /opt/pluto/bin/pluto && chown pluto:pluto /opt/pluto/bin/pluto && chmod 750 /opt/pluto/bin/pluto"]'
```

S3 を使わないなら `aws ssm start-session` で入って `wget` か `scp -o ProxyCommand="aws ssm start-session ..."` 経由 (面倒)。

## secrets を fetch するスクリプト

`/opt/pluto/secrets/pluto.env` を生成 (起動前に systemd ExecStartPre で叩く):

```bash
#!/bin/bash
aws ssm get-parameters-by-path --path /pluto/ --with-decryption --recursive \
  --query "Parameters[*].[Name,Value]" --output text \
  | awk '{ sub(/^\/pluto\//, "", $1); print $1 "=" $2 }' > /opt/pluto/secrets/pluto.env
chmod 600 /opt/pluto/secrets/pluto.env
chown pluto:pluto /opt/pluto/secrets/pluto.env
```

## destroy

```bash
terraform destroy
```

EBS 別途無いので state も小さい。SSM Parameter は SecureString なので state file に value は載らないが `tfvars` には入る — `tfvars` は git に commit しない。
