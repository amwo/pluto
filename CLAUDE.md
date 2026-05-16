## 実装

- **公式サイトが存在する場合は、必ず公式サイトで仕様を確認し、正しいやり方を理解してから着手**
- コーディング時は常にcodexにレビュワーとして常駐してもらい、すべてレビューに通す
- 設計は複雑にしない。誰が見てもわかるような命名規則、設計を優先する
- 実行速度は常に最速を狙う
- 実装の結果、状況が変わったら`./docs/spec.md` を更新する

## Chat

- チャットで指摘されたミスは二度と起こさないようにrules.mdに記載して毎回読めるようにする

## メモリ / 自己学習 (Memory + Dreaming)

`memory/` は自己学習エージェント基盤。セッションごとに以下のルーティンで進める。

- **着手前**: 関連ストア(既定 `team_sre`)の `notes/` を grep/読み取りで確認し、過去セッションの知見を再利用する。同じ調査を繰り返さない
- **作業中**: 有効な戦略・調査結果・失敗パターンを `memctl write`(`--if-hash` で楽観ロック)で作業メモリに記録する。`org_knowledge` は read_only、書込み不可
- **終了時**: `dreamer` を起動 → `dreaming/jobs/<id>/diff.patch` をレビュー → `memctl apply` でメモリへ反映する
- セッションは `scripts/agent-session.sh` で登録し、書込みを agent_id / session_id で属性付けする
- 全書込みは `audit/memory_history.jsonl` に残る。専用 DSL は使わず bash / grep / memctl で操作する
- ビルド: `cd memory && cargo build --release`(バイナリは `memory/target/release/`)
