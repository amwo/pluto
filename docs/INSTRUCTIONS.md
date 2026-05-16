# Claude Code 指示書: Self-Learning Agent (Memory + Dreaming) の実装

あなたはこのリポジトリ内で「自己学習エージェント」のローカル再現実装を行う Claude Code です。
Anthropic の Managed Agents API における **Memory** と **Dreaming** の挙動を、ローカルのファイルシステム上で忠実に再現してください。

---

## 1. ゴール

長時間・複数並行で動くエージェント群が、セッションをまたいで継続的に自己学習・自己改善できる仕組みを構築する。具体的には次の2層を実装する。

1. **Memory 層** — エージェントが「タスク実行中にリアルタイムに読み書き」する短期/作業メモリ。
2. **Dreaming 層** — セッション終了後に「複数エージェントのトランスクリプトを横断的に分析」して、メモリを検証・整理・強化するバッチ非同期プロセス。

最終目標: 「昨日のエージェント群の経験を踏まえて、今日のエージェントが自動的に賢くなる」状態。

---

## 2. ディレクトリ構成

```
.
├── memory_stores/
│   ├── org_knowledge/         # 読み取り専用(runbook, SLO, owner)
│   │   ├── README.md
│   │   └── runbooks/*.md
│   ├── team_sre/              # 読み書き(SRE エージェント用 作業メモリ)
│   │   └── notes/*.md
│   └── codebase/              # コードベース固有の学習
│       └── *.md
├── sessions/
│   └── <session_id>/
│       ├── transcript.jsonl   # 各エージェントの会話・ツール呼び出し
│       └── meta.json          # agent_id, started_at, ended_at, store_refs
├── dreaming/
│   ├── jobs/<job_id>/
│   │   ├── input_sessions.json
│   │   ├── diff.patch         # 提案された memory 更新
│   │   └── report.md
│   └── cron.yaml
└── audit/
    └── memory_history.jsonl   # 全 memory 変更の監査ログ
```

---

## 3. Memory 層の実装要件

### 3.1 ファイルシステムとしてのメモリ
- メモリは「特定スキーマの DB」ではなく、**Markdown ファイル群とディレクトリ階層** としてモデル化する。
- エージェントは `bash` と `grep`(GP) 系の通常ツールでメモリを読み書きする。専用 API を増やさない。
- Opus 4.7 想定: 「何を残すか」「どんなファイル粒度に分割するか」をモデル自身に判断させる。

### 3.2 メモリストアと権限スコープ
各メモリストアは `permissions.yaml` を持つ:
```yaml
store_id: org_knowledge
mode: read_only            # read_only | read_write
owners: [platform-team]
```
1 つのエージェントは複数ストアを **mix & match** できる(例: `org_knowledge` は read-only、`team_sre` は read-write)。

### 3.3 並行性 (Optimistic Concurrency)
- 書き込み時には対象ファイルの **content hash (precondition hash)** を取得 → 編集 → 書き込み直前に再ハッシュして比較。
- 不一致なら衝突として中断し、最新内容を再読込してから差分を再生成する。

### 3.4 バージョン履歴と属性メタデータ
すべての書き込みは `audit/memory_history.jsonl` に追記:
```json
{"ts":"...", "store":"team_sre", "path":"notes/dispatch.md",
 "agent_id":"sre-agent-7", "session_id":"...", "before_hash":"...", "after_hash":"...", "diff":"..."}
```
- 過去状態への巻き戻し用 CLI (`memctl history`, `memctl checkout <hash>`) を提供する。

### 3.5 スタンドアロン API
メモリは特定の harness にロックインせず、外部スクリプト(PII スキャン、クリーンアップ、外部 KB へのクローン等)から扱えるよう **CLI / Python SDK** を切り出す:
```
memctl read  <store> <path>
memctl write <store> <path> --if-hash <hash>
memctl list-versions <store> <path>
```

---

## 4. Dreaming 層の実装要件

### 4.1 性質
- **Out-of-band**: 個別エージェントのホットパスには遅延を一切加えない、バッチ非同期処理。
- **Multi-session 横断**: 単一エージェント視点では気付けない「複数エージェントに共通する失敗パターンや成功戦略」を発見するのが目的。
- 起動方式は (a) cron 定期実行、(b) エージェント終了フックからの呼び出し、の両方をサポート。

### 4.2 入力
- 対象メモリストアに触れた直近 N セッション (例: 過去7日) のトランスクリプト一式。
- 現在のメモリストアのスナップショット + バージョン履歴。

### 4.3 処理
Dreaming ジョブはサブエージェントを起動し、各トランスクリプトを走査して以下を抽出する:
1. **共通の失敗パターン**(例: 同じ tool call が複数エージェントで失敗している)。
2. **有効だった戦略**(例: 60秒後に必ず再 spike が来るので short-circuit できる、等)。
3. **重複エントリの統合**(同内容のメモが5件あれば1件に集約)。
4. **stale エントリの削除**(トランスクリプトで否定された古い知識)。
5. **検証ノート**: 「このメモは本日のセッション X, Y で再検証済み」と追記。

### 4.4 出力
- `dreaming/jobs/<job_id>/diff.patch` に「メモリストアへの提案 diff」を出力。
- ユーザは手動レビュー、自動適用、または PII チェック等のパイプライン経由での適用を選択可能。
- 適用時も通常の memory write を通すので、audit log に Dreaming 由来であることが記録される(`agent_id: "dreamer"`)。

### 4.5 設計上の意図
- メモリ品質を「タスク完了」とは **別の独立した最適化目標** として切り出す。
- test-time compute と同じ発想で、「メモリの整備に余分なトークン/計算を投資すれば、下流の全エージェントの性能が上がる」というスケーリング特性を活用する。
- 大規模メモリを “検索インデックス” のように事前構築 → 多数の下流エージェントで amortize する。

---

## 5. デモシナリオ (SRE 例) を再現せよ

1. P1 アラートを受け、SRE エージェント A が起動。CPU 使用率と最近の PR を調査し、結果を `team_sre/notes/dispatch.md` に記録する。
2. 数分後、同じアラートで SRE エージェント B が別セッションで起動。最初に `team_sre` を読み、A が残した short-circuit ノートを発見 → 同じ調査を繰り返さずトークンと時間を節約する。
3. 1日の終わりに Dreaming ジョブを実行:
   - 「dispatch アラートは上流 CPU spike の **約60秒後** に再発する」というパターンを発見し、再試行ロジック由来の可能性をメモに追記する。
   - 重複5件を1件に統合、古い1件を削除、検証ノートを追加。
4. 翌日のエージェントは更新後メモリを読み、最初から効率よく triage を開始する。

このシナリオが end-to-end で動くテストを `tests/test_sre_scenario.py` として用意すること。

---

## 6. 実装タスク (Claude Code 用 TODO)

1. ディレクトリ構成と空ストアを生成するブートストラップスクリプト。
2. `memctl` CLI (read / write / list-versions / checkout / hash) の実装。
3. Optimistic concurrency と audit log。
4. Permission スコープ(read_only / read_write)の強制。
5. エージェント用の薄いラッパー: 「ストアを mount → bash/grep で操作」できる harness。
6. Dreaming ランナー: サブエージェント分割、トランスクリプト解析、diff 生成、レポート出力。
7. Cron / セッション終了フックからのトリガ。
8. SRE シナリオの統合テストとベンチマーク(タスク完了率・トークン消費量の前後比較)。
9. README にアーキテクチャ図(Memory ←→ Dreaming ←→ Knowledge Base)と使い方を記載。

---

## 7. 守るべき原則

- **モデルを邪魔しない**: メモリ操作のためだけの専用 DSL を作らず、bash / grep / file edit に委ねる。
- **観測可能性**: いつ・誰が・どのセッションで・何を変えたか、必ず audit log に残す。
- **可搬性**: メモリは外部ツールから読み書きできる単純なファイル群であり続けること。
- **目的の分離**: 「タスク完了」と「メモリ品質」を別々の最適化対象として扱う。
- **マルチエージェント前提**: 1エージェント視点では見えないパターンを Dreaming が補う。

実装に着手する前に、上記の構成・API 形・テスト方針をユーザに提示して合意を取ること。
