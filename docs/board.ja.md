[English](board.md) | 日本語

# Web ボードと常駐サーバー

agent-adjutant は、複数リポジトリのタスク監視、セッション管理、差分確認、エージェントからの判断待ち（gate）への回答を一画面で行う Web ボードを提供します。

---

## 概要と起動モード

ボードの配信方法には次の2種類があります。

1. **常駐サーバー（`adj server start`）［推奨］**:
   state ディレクトリごとに1つのバックグラウンドプロセスを立ち上げ、全リポジトリのボードを `/b/<slug>/` で配り、`/` で横断ダッシュボードを提供します。
2. **リポジトリ hub の個別ボード（`adjutant serve` または自動起動 `hubServe`）**:
   対象リポジトリの hub MCP サーバーが、その hub の生存期間中に限って直接配信します（既定ポートは 4577、埋まっていれば空きポート）。そのリポジトリのタスクと worker だけを表示します。

### 常駐サーバーの操作コマンド

```bash
adj server start [--port <port>] [--foreground] [--no-open]
adj server status [--json]
adj server stop
adj server restart [--port <port>] [--open]
```

- **ポートの割り当て**: 既定では `127.0.0.1:4577` を使い、埋まっている場合は空いているポートを自動で選びます。
- **リポジトリの切り替え**: `/` の左サイドバーに全リポジトリと親タスク hub の一覧が並びます。「すべて」を選ぶと全ボードのタスクを一括表示できます。
- **自動再接続**: `adj server restart` の実行前後でポートが変わらなければ、ブラウザの各タブは自動で再接続します。
- **セキュリティとトークン**: `127.0.0.1` のループバックアドレスにのみバインドします。状態を変更する操作には `~/.local/state/adjutant/dashboard-token` に保存されたトークンが必要です（URL クエリまたは `Origin` ヘッダーで検証）。

### launchd による常駐化（macOS）

ログイン時に常駐サーバーを自動起動するには、`~/Library/LaunchAgents/adj.server.plist` を作成します。

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>adj.server</string>
  <key>ProgramArguments</key>
  <array>
    <string>/path/to/adj</string>
    <string>server</string>
    <string>start</string>
    <string>--foreground</string>
    <string>--no-open</string>
  </array>
  <key>KeepAlive</key><true/>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin</string>
  </dict>
</dict>
</plist>
```

- 登録: `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/adj.server.plist`
- 停止: `launchctl bootout gui/$(id -u)/adj.server`
- 再起動: `launchctl kickstart -k gui/$(id -u)/adj.server`（`KeepAlive` の配下では `adj server restart` を使わないでください。launchd の自動再起動と競合します）

---

## 画面構成と主なビュー

サイドバーの切り替えで、**ボード**と**いまの仕事**の2つのビューを利用できます。

### 1. いまの仕事（`?view=work`）

全リポジトリの進行中の作業を3列で一括管理するメインビューです。
- **左列**: アクティブなセッション、hub、gate、レビュー依頼の一覧。
- **中央列**: 選択中のセッションまたは hub の内蔵 tmux ターミナル。
- **右列**: タスク詳細パネル、副官ペイン、または AI 下読み画面。

#### グループ化の切り替え
タスク一覧の見出しメニューからグループ化を変更できます。
- **親 Issue［既定］**: オーナー → リポジトリ → 親 Issue → 子タスク の順で階層表示します。親のないタスクや hub は最上位に表示されます。
- **状態**: 「新着」「実行中」「そのほか」の3つの枠で整理します。

#### 新着
人の対応が必要な項目が集まります。
- 新しく開いた gate（判断待ち）
- 権限確認待ち（`waiting`）や質問待ち（`AskUserQuestion`）で止まっているセッション
- gate を開かずにターンを終えた worker
- 失敗したセッション
- あなたの対応が必要な PR（変更依頼、CI 失敗、承認済み、マージされずにクローズ）

gate に回答する、ターミナルで入力する、または「既読（`done` / `done_all`）」を押すと新着から外れます。

#### 離れていた間に
タスクパネルの先頭に、前回の確認以降に発生した出来事（工程の遷移、gate の発生、権限待ち、ターンの完了など）を時系列で要約表示します。

#### タスクを置く（Parking）
PdM やデザイナーの確認待ち、外部レビュー待ち、マージ待ちなど、一時的に作業を保留したいときに使います。
- CLI: `adj task park --id <id> --reason pdm|design|review|merge-timing|other [--text <text>]`
- CLI: `adj task unpark --id <id>`
- UI: タスクサマリの「置く」ボタンから実行
置いたタスクは新着に入らず、デスクトップ通知も抑制されます。

### 2. AI 下読み（PR レビュー支援）

他者から自分宛てに届いた GitHub PR のレビュー依頼を一覧表示します。
- PR のメタ情報: 規模（S / M / L）、差分概要、CI 状態、レビュアーの順番。
- 下読みの構成: 読む順、注目ポイント、観点別の分類（仕様・ロジック、UI、ドキュメント、テスト、リソース）。
- コメント下書き: ボード上で各行に下書きコメントを記入し、「N 件を pending review に積む」で GitHub の pending review へ一括反映できます。
- 巨大な PR: 1ファイルで1,500行を超える変更がある PR には「分割案」ボタンが表示され、レビューしやすい単位への分割案を worker に検討させることができます。

#### コードレビュー hub と読み手
- レビュー hub の起動: `adj hub --review`（または画面上の「hub を起動」）
- 下読みの依頼: `adj review request --pr <URL|owner/repo#N>`（または PR 行のボタン）
- 読み手プロセス: 同時に最大3つの独立した一時ディレクトリで動作し、コードベースを変更せずに差分を読み込みます。

### 3. 副官（Aide）

タスクを選択していないときに右列に表示される対話型アシスタントです。
- 起動とリセット: 「副官に聞く」または `POST /api/aide/open`
- `<state>/aide` ディレクトリで動作し、自身の Claude Code 設定をそのまま利用します。読み取り系ツールと画面表示用の `adjutant_show` に加え、hub への依頼送信用ツール `adjutant_request`、gate 回答用ツール `adjutant_gate_answer` が事前許可されています。
- **副官の表示領域**: `adjutant_show` を介して、gate の詳細、判断待ち一覧、PR 下読み、差分の抜粋、hub への依頼状況（`request`）などを隣接する表示領域に出力できます。
- **副官経由での gate 回答**: 判断待ちの gate で「副官に答えを伝える」や「この判断」を押すと、プロンプトに `[gate <board>/<id>] ` が挿入されます。指示（「B で」など）を入力すると、副官が送信内容の確認を提示し、承認後にボタン操作と同様の経路で回答します。

---

## タスクパネルと Gate

カードをクリックすると、右側に詳細パネルが開きます。

### パネルのタブ構成
1. **タスクサマリ**: Issue / PR リンク、現在の gate、進捗フェーズ、課題、目標、worktree 情報。
2. **ターミナル**: worker セッションの内蔵ターミナル（`terminal.preset: "tmux"` 時）。
3. **コードレビュー**: セルフレビューのラウンド数、指摘事項、差分、検証記録。
4. **動作確認**: 検証コマンド、手動確認チェックリスト、実行結果。
5. **経過**: 過去の gate、回答、工程遷移の履歴。

### Gate（判断待ち）の仕組み

gate は、エージェントが作業を止め、人間の判断を仰ぐためのチェックポイントです。
- **3つの情報構成**: 何を見るべきか、何が決定済みか、エージェントが迷っている点はどこか。
- **主な gate の種類**:
  - `plan`: 実装計画、課題、目標の承認。
  - `diff`: コードレビュー結果と指摘事項の確認。
  - `verify`: 検証コマンドの成否と手動確認事項。
  - `dispatch`: worker 起動前のタスク仕様確認。
- **回答方法**: 選択肢（承認、修正など）のクリック、コメント入力、またはセッションのターミナルで直接入力して解決（`adj gate close --terminal`）。
- **サーバー稼働確認**: `adj gate open` はボードサーバーが動いているかを判定します（`server: up` / `server: down`）。サーバーが停止している場合は、エージェント自身のターミナルで直接質問するフォールバックを行います。

---

## カードの自動配置と GitHub 同期

ボードの列は、worker の進捗フェーズと GitHub PR の状態に応じて自動的に移動します。

| PR の状態 | 誰の番か | カードの配置列 |
| --- | --- | --- |
| 下書き（Draft） | worker | 進行中（エージェント） |
| オープン・他者のレビュー待ち | 他のレビュアー | エージェント列（レビュー待ち（他の人）） |
| review bot や CI の完了待ち | bot / CI | エージェント列（bot・CI 待ち） |
| 変更依頼（Changes requested） | あなた | PR レビュー（修正の依頼あり） |
| 承認済み（Approved） | あなた（マージ担当） | PR レビュー（マージ待ち） |
| CI 失敗 | あなた | PR レビュー（CI 失敗） |
| マージ完了 | なし | 完了（Done） |
| マージされずにクローズ | あなた（判断待ち） | PR レビュー（閉じられた） |

常駐サーバーは GitHub の通知イベント（`GET /notifications`）をポーリングし、更新のあった PR を GraphQL で一括取得します。通知が届かない操作もあるため、5分ごとの定期再確認も行われます。手動更新は `adjutant task refresh` または「PR確認」ボタンから実行できます。

---

## セッションの管理

### タスクなしのセッション
事前に Issue やタスクを作らず、一時的な作業用の worker セッションを立ち上げることができます。
- UI: hub パネルの「タスクなしのセッションを始める…」
- API: `POST /api/sessions`（`instruction`, `worktreeName` などを指定）
- 後から `POST /api/sessions/<id>/link` で既存または新規のタスクに紐付け可能です。

### セッションの操作
ターミナル上部のバーまたは API から以下の操作が可能です。
- **再開（`POST /api/sessions/<id>/resume`）**: 停止した worker を既存の worktree で開き直します。
- **再起動（`POST /api/sessions/<id>/restart`）**: 会話履歴を保持したまま、worker のウィンドウを安全に再起動します（新しいバージョンを反映させたい場合などに利用）。
- **外部端末で開く（`POST /api/sessions/<id>/open`）**: 現在のセッションを手元のターミナル（iTerm2 の `tmux -CC attach` など）で直接開きます。
- **片付ける（`POST /api/sessions/<id>/cleanup`）**: 変更がすべてコミット・push されていれば、worktree とローカルブランチを削除して片付けます。未コミットの変更がある場合は確認（ディレクトリ名の再入力）を要求します。
