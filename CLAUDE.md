## 実装

- **公式サイトが存在する場合は、必ず公式サイトで仕様を確認し、正しいやり方を理解してから着手**
- コーディング時は常にcodexにレビュワーとして常駐してもらい、すべてレビューに通す
- 設計は複雑にしない。誰が見てもわかるような命名規則、設計を優先する
- 実行速度は常に最速を狙う
- 実装の結果、状況が変わったら`./docs/spec.md` を更新する

## Chat

- チャットで指摘されたミスは二度と起こさないようにrules.mdに記載して毎回読めるようにする

## メモリ / 自己学習 (Memory + Dreaming)

`memory/` は自己学習エージェント基盤。トランスクリプト記録・Dreaming・メモリ反映は
`.claude/settings.json` のフックで全自動。

- **着手前(手動)**: 関連ストア(既定 `team_sre`)の `notes/` を grep/読み取りで確認し、過去の知見を再利用する。同じ調査を繰り返さない
- **作業中(手動)**: 有効な戦略・恒久的な知見を `memctl write`(`--if-hash` で楽観ロック)で記録する。`org_knowledge` は read_only
- **自動**: SessionStart / UserPromptSubmit / PostToolUse フックがセッションとトランスクリプトを `memory/sessions/` に記録、SessionEnd フックが `dreamer --apply` を実行してメモリへ反映する
- 全書込みは `audit/memory_history.jsonl` に残り、`memctl checkout` で巻き戻せる
- 解析はヒューリスティック。自動で効くのは反復ツール失敗の検出・重複ノート統合・検証ノート追記まで。深い戦略抽出は LLM 解析器が必要(未実装)
- ビルド: `cd memory && cargo build --release`(バイナリは `memory/target/release/`)
