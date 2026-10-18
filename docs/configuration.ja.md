[English](configuration.md) | 日本語

# 設定とランタイム詳細

agent-adjutant の高度な設定項目、コマンドテンプレート、環境変数、ターミナルバックエンド、セッションのライフサイクル管理について説明します。

コメント付きの完全な JSON スキーマについては [`config.example.json`](../config.example.json) を参照してください。

---

## 設定ファイルの配置と優先順位

設定ファイルの配置場所: `~/.config/adjutant/config.json`（`$XDG_CONFIG_HOME` および `$ADJUTANT_CONFIG` に対応）。

設定の解決順序（より具体的な指定が優先されます）:
1. リポジトリ個別設定（`repos["owner/name"]`）
2. リポジトリの既定設定（`defaults`）
3. `config.json` のトップレベル設定
4. 組み込みの既定値

未知のキーや不正な形式があっても設定の読み込みでエラー終了はせず、`adjutant config` で確認できる警告（`warnings`）を出力したうえで安全にフォールバックします。

---

## コマンドテンプレートとプレースホルダ

`config.json` に記述するコマンド群はテンプレートとして展開されます。プレースホルダは**あらかじめシェルクォートされた状態で**挿入されるため、テンプレート側で引用符を付ける必要はありません。

```jsonc
// プレースホルダをダブルクォートで囲まないでください
"terminal": {
  "spawn": "my-terminal --title {title} -- {command}"
}
```

### 長いコマンドラインの退避（ステージング）
展開後のコマンドラインが900バイトを超える場合（`agentEnv` で長い `PATH` を渡す場合など）、adjutant は内容をシステムの一時ディレクトリ（`TMPDIR`、未設定ならプラットフォームの既定）の `adjutant-spawn-*.sh` に書き出し、`sh <そのスクリプト>` を実行します。ターミナルに長い行を入力させると途中が欠けて別のコマンドとして実行されることがあるため、短い行で済ませてこれを防ぎます。なお `--dry-run` 時は中身を確認できるよう退避を行いません。

---

## 環境変数

- `ADJUTANT_CONFIG`: 独自の設定ファイルパスを指定します。`adj hub --tab` や `adj work` が開くタブへ自動で引き継がれます。
- `ADJUTANT_STATE_DIR`: 独自の状態保存ディレクトリを指定します（既定: `~/.local/state/adjutant`）。相対パスで渡された場合はメインチェックアウトを基準とした絶対パスに変換して渡されます。
- `ADJUTANT_HUB`: 対象の hub を指定します。`adj work` は起動した worktree に hub の識別子を書き込むため、worker からの報告は正しい hub へ確実に届きます。

---

## ターミナルバックエンド

### 1. 既定（macOS 上の iTerm2）
AppleScript を使ってタブの作成、タイトルの設定、テキスト入力を行います。テンプレートに渡される `{pid}` と `{tty}` は OS の識別子（`ttys004` など）であり、ターミナルのペイン番号ではありません。

### 2. 組み込み tmux バックエンド（`terminal.preset: "tmux"`）
設定で `"terminal": { "preset": "tmux" }` を指定すると、tmux を標準バックエンドとして利用できます。
- **バックグラウンドウィンドウ起動**: worker をデタッチウィンドウ（`tmux new-window -d`）として起動するため、作業中のフォーカスが奪われません。起動後の約3秒間プロセスを監視し、即座に終了した場合はスロットを解放してエラーを出力します。
- **ペインへの自動解決**: PID、TTY、プロセス階層を自動探索して tmux ペインを特定するため、専用のラッパースクリプトが不要です。
- **画面確認を伴う wake（入力通知）**: tmux ペインに入力する前に画面（`tmux capture-pane`）を確認します。エージェントが空の入力欄で待機しているときだけキーを送信します。質問、許可確認、メニュー、入力途中の文字がある場合は入力を保留し、デスクトップ通知へ切り替えます。
- **フォーカスと終了**: `adj focus` で対象のウィンドウとペインを選択し、`adj close` でウィンドウを終了します（`kill-window`）。
- **接続（アタッチ）**: `tmux attach -t adjutant` や iTerm2 経由の `tmux -CC attach -t adjutant` でいつでもセッションに接続できます。
- **CLI サブコマンド**: `adj tmux`（`pane`, `spawn`, `wake`, `focus`, `close`）から tmux セッションを直接確認・操作できます。
- **環境変数**: `$ADJUTANT_TMUX_SESSION`（既定の `"adjutant"` を上書き）および `$ADJUTANT_TMUX_SOCKET`（`tmux -L <socket>` に対応）に対応しています。

---

## ランナーテンプレートとセッション再開

### 各種ランナー
- `agentRunner`: タスク worker を起動するコマンドテンプレート。
  - 既定値: `claude --session-id {sessionId} --permission-mode auto {settings} {prompt}`
- `hubRunner`: リポジトリ hub を起動するコマンドテンプレート。
  - 既定値: `claude -n {name} --session-id {sessionId} --permission-mode auto {settings} {prompt}`
- `reviewerRunner`: PR の読み手（AI 下読み）を起動するテンプレート。
  - 設定ファイルと MCP の読み込みを無効化し（`--setting-sources '' --strict-mcp-config --permission-mode dontAsk`）、読み取り専用の安全な環境で動作させます。
- `agentResumeRunner` / `hubResumeRunner`: `--resume` 指定時に既存の会話を再開するためのテンプレート。

### セッションのライフサイクルと自動再開
- **自動再開の有効期間（`hubAutoResumeHours`）**: hub の終了からこの時間（既定: 3時間）以内であれば、素の `adj hub` を実行するだけで前回の会話を自動的に再開します。有効期間を過ぎている場合は空の会話で新しく立ち上がります。
- **セッション ID の管理**: ランナーに `{sessionId}` を含めると、ID を `~/.local/state/adjutant/sessions/`（hub）または `.claude/adjutant-session.json`（worker）に保持し、次回以降の確実な再開に利用します。
- **生存監視**: 稼働中の hub は1分ごとおよび終了時にハートビートファイル（`sessions/<slug>.alive`）を更新します。これにより、予期せず終了したプロセスが稼働中と誤認されるのを防ぎます。

---

## デスクトップ通知

`notification` テンプレートで使えるプレースホルダ: `{title}`, `{message}`, `{nwo}`（リポジトリ名 `owner/name`）。

- **macOS での動作**: `terminal-notifier`（クリック時にターミナルをアクティブ化可能）を優先し、未インストールの場合は `osascript` にフォールバックします。
- **Linux での動作**: `"notification": "notify-send {title} {message}"` を設定してください。
- **発火条件**:
  - `adjutant send` / `adjutant tell` による直接メッセージ
  - 数秒以上、権限プロンプトや質問への回答待ちになっているセッション
  - 新しく開いた gate
  - なお、ブラウザで Web ボードが開かれてポーリング中の場合は、画面側のデスクトップ通知が優先され、OS コマンド側は抑制されます。
