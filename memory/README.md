# memory — Self-Learning Agent (Memory + Dreaming)

Anthropic Managed Agents の **Memory** と **Dreaming** をローカルファイルシステム上で
再現した実装。長時間・複数並行で動くエージェント群が、セッションをまたいで継続的に
自己学習・自己改善できる仕組みを提供する。

`pluto`(Rust トレーディングボット)とは独立したクレートで、`memory/` 配下に閉じる。
親パッケージのビルド・CI・デプロイには一切影響しない。

## アーキテクチャ

```
   ┌──────────────┐   read: 直接 fs / grep                ┌───────────────┐
   │   Agents     │   write: memctl(楽観ロック+監査)      │   Memory 層    │
   │  (sessions)  │ ───────────────────────────────────▶ │ memory_stores/ │
   │              │ ◀─────────────────────────────────── │ permissions.yaml│
   └──────┬───────┘        mount したストア群              └───────┬───────┘
          │ transcript.jsonl                                      │ snapshot + history
          ▼                                                        ▼
   ┌──────────────┐   out-of-band バッチ解析(横断)       ┌───────────────┐
   │ Dreaming 層   │ ───────────────────────────────────▶ │  Knowledge    │
   │  (dreamer)   │   diff.patch / report.md / proposal   │  audit/ +objects│
   └──────────────┘   適用は memctl write 経由(監査付き)  └───────────────┘
```

- **Memory 層**: タスク実行中にリアルタイムで読み書きする作業メモリ。Markdown 群。
- **Dreaming 層**: セッション終了後に複数トランスクリプトを横断解析し、メモリを
  検証・統合・整理するバッチ非同期処理。ホットパスには遅延を加えない。

## ディレクトリ

```
memory/
├── memory_stores/<store>/   permissions.yaml + Markdown 群
├── sessions/<id>/           transcript.jsonl + meta.json
├── dreaming/{jobs,cron.yaml} Dreaming ジョブ出力
├── audit/                   memory_history.jsonl + objects/(内容アドレス)
├── scripts/                 agent-session.sh / session-end-hook.sh
└── src/                     memctl / dreamer / ライブラリ
```

## ビルド

```sh
cd memory
cargo build --release
```

`target/release/{memctl,dreamer}` が生成される。

## 使い方

### 初期化

```sh
memctl --root memory init
```

`org_knowledge`(read_only)、`team_sre`(read_write)、`codebase`(read_write)を生成。

### メモリ操作(memctl)

```sh
memctl read  team_sre notes/dispatch.md
memctl hash  team_sre notes/dispatch.md
echo "..." | memctl write team_sre notes/dispatch.md --if-hash <hash>
memctl list  team_sre
memctl list-versions team_sre notes/dispatch.md
memctl checkout      team_sre notes/dispatch.md <hash>
```

- 書き込みは `--if-hash` で **楽観的並行制御**。precondition と実ハッシュが
  不一致なら衝突として中断する。
- 全書き込みは `audit/memory_history.jsonl` に追記され、各バージョンの内容は
  `audit/objects/<hash>` に内容アドレスで保存される(`checkout` で巻き戻し可能)。
- `read_only` ストアへの書き込みは拒否される。

### エージェント harness

```sh
eval "$(scripts/agent-session.sh sre-agent-a org_knowledge,team_sre)"
# 以降 memctl は MEMORY_AGENT_ID / MEMORY_SESSION_ID で属性付けされる
```

read は通常の `bash` / `grep` で `memory_stores/` を直接参照、write のみ `memctl`
を通す。メモリ操作専用の DSL は持たない。

### Dreaming

```sh
dreamer --root memory --store team_sre            # diff を生成(未適用)
memctl apply <job_id>                             # レビュー後に適用
dreamer --store team_sre --apply                  # 生成と同時に適用
```

Dreaming は対象ストアに触れたセッションのトランスクリプトを横断し、

1. 複数セッション共通の観測パターン
2. 複数セッションで反復する tool 失敗
3. 同一内容の重複ノート → 1 件に統合
4. トランスクリプトで否定された stale ノート → 削除
5. 生存ノートへの検証ノート追記

を抽出し、`dreaming/jobs/<job_id>/` に `proposal.json` / `diff.patch` / `report.md`
を出力する。適用は通常の memory write を通るため、audit log に `agent_id: "dreamer"`
として記録される。

### トリガ

- cron: `dreaming/cron.yaml`(`schedule` を crontab に登録)
- セッション終了フック: `scripts/session-end-hook.sh`

## テスト

```sh
cargo test
```

- `tests/sre_scenario.rs` — SRE デモシナリオの end-to-end(エージェント A→B の
  メモリ共有 → Dreaming → 翌日の効率化、トークン削減ベンチを含む)
- `tests/memory_ops.rs` — 楽観ロック / 権限スコープ / 履歴 checkout
