[English](README.md) | 日本語

# agent-adjutant

<img alt="agent-adjutant-logo" src="docs/images/agent-adjutant-logo.png" />

> 英語版が正本 ([README.md](README.md))

コーディングエージェントのためのタスク hub を、1つのバイナリで。

![agent-adjutant デモ](docs/images/demo.ja.gif)

`adjutant` はリポジトリの「副官」として、タスクを worker に割り振り、その報告を受け取ります。hub がタスクを選定して worktree を作成し、指示書を用意して別タブで worker を起動します。worker が作業中に関係のないバグを見つけた場合は、自ら修正や Issue 起票を行わずに hub へ差し戻します。これら一連のワークフローは手順書（プロンプト）としてバイナリに同梱されています。

## なぜプロンプトを配信するバイナリなのか

従来、エージェントへの手順書は各エージェントのコマンドディレクトリに Markdown ファイルとして配置していました。しかし、ツールのアップデートに伴いファイルが乖離し、環境ごとに古い手順書が残り続ける問題がありました。MCP 経由でバイナリから配信することで、エージェントは常に単一の最新手順を参照できます。

また、hub 名の解決や設定の読み込み、タブ操作、メッセージの受け渡しといった機械的な処理も、以前は複数の手順書に重複して記載されていました。これらを CLI コマンドとして切り出し、手順書からはそのコマンドを呼び出す構成に整理しています。

## インストール

Homebrew の場合:

```bash
brew install syarihu/tap/agent-adjutant # `adjutant` と短縮版 `adj` の両方が入ります
adjutant install-mcp                   # Claude Code に MCP サーバーを登録（user スコープ）
adjutant install-mcp --target agy       # Antigravity (agy) に MCP サーバーを登録
adjutant install-mcp --target json      # 他のクライアント向けに設定用 JSON を出力
```

Cargo の場合:

```bash
cargo install --git https://github.com/syarihu/agent-adjutant # `adjutant` と短縮版 `adj` の両方が入ります
# またはローカルチェックアウトから:
#   cargo install --path .
# または cargo install を使わない場合:
#   cargo build --release && cp target/release/adjutant target/release/adj ~/bin/
```

`install-mcp` は `--target claude-code`（既定）で `claude mcp add`、`--target agy` で `agy mcp add` を直接実行して登録します。他のクライアントを使う場合は `--target json` で設定 JSON を出力して手動登録できます。

**登録前にバイナリへ PATH を通してください。** PATH が通っていない状態で `install-mcp` を実行するとビルドディレクトリの絶対パスで登録されるため、`cargo clean` などでバイナリが消えるとサーバーが動かなくなります。

**`adj` は `adjutant` の短縮名です。** どちらを実行しても同じように動作します。`adj work` から起動された worker タブも `adj worker` として立ち上がります。

## 2つのモード

シェル側（エージェント起動前のランチャーやフック、手順書内の `Bash` ステップ向け。すべて `adj` でも実行可能）：

| コマンド | 説明 |
| --- | --- |
| `adjutant hub [--tab] [--resume\|--new] [--no-dashboard\|--dashboard]` | このリポジトリの hub をメインチェックアウトで1つ起動。前回のセッションが `hubAutoResumeHours` 以内に終了していれば再開する（`--tab` は今のタブが hub になるのではなく、新しいタブを開いてそこで起動。`--resume` は終了からの時間に関係なく前回のセッションを再開し、`--new` は時間内でも新しく起動する。`--no-dashboard` は起動時の一覧収集を省略し、`--dashboard` は逆に収集させる。どちらも `startupDashboard` より優先） |
| `adjutant hub-name [--json]` | hub のセッション名（報告先のアドレス）を出力 |
| `adjutant config` | このリポジトリ向けに解決された設定を JSON で出力 |
| `adjutant pending [--json\|--read N\|--ack N\|--path]` | hub 宛ての未処理メッセージを一覧・確認 |
| `adjutant send --subject … --body …` | hub にメッセージを送信（本文は stdin 可） |
| `adjutant work --worktree … (--title … \| --task <id> \| --resume)` | 新しいタブを開いて worker を起動（`--task` はタスクレコードのタイトルでタブを名乗る。Issue 由来のタイトルをコマンド行にクォートして書かずに済む。`--resume` はその worktree に保存されたセッションを再開）。`maxWorkers` の数だけ worker が動いていると、何も起動せずに終了コード 3 で返る |
| `adjutant worker --worktree … [--resume]` | 自身を worker として起動（`work` のタブ内で実行されるコマンド。worktree の中で `--resume` を付けると保存されたセッションを再開） |
| `adjutant tell --worktree … --subject …` | 指定 worktree の worker にメッセージを送信 |
| `adjutant outbox [--clear]` | hub から現在の worker 宛てに届いたメッセージを確認 |
| `adjutant spawn --cwd … -- cmd …` | 新しいタブを開いてコマンドを実行 |
| `adjutant focus [--worktree …]` | 実行中の hub タブ（`--worktree` ならその worktree の worker のタブ）をアクティブにする（なければ exit 1） |
| `adjutant phase [--set …]` | worker が今どの工程にいるかを書く（`plan` / `implement` / `self-review` / `verify` / `pr` / `review` / `report`）。`--set` 無しなら今の工程を表示 |
| `adjutant close --worktree …` | 指定 worktree の worker が座っているタブを閉じる（閉じられなければ exit 1） |
| `adjutant ide --worktree …` | worktree を設定されたエディタで開く |
| `adjutant title --title …` | 現在のタブの名前を設定（hub 自身も使用） |
| `adjutant notify --message …` | 人間にデスクトップ通知を送る |
| `adjutant worktree-path --name …` | タスク用 worktree のブランチ名・パスと、作成コマンドを打つメインチェックアウトを出力 |
| `adjutant jules start\|show\|findings\|relay` | タスクの承認済みの計画を Jules に渡す。渡した session の状態を確認する。レビュー指摘を Jules に回す（[Jules に実装を渡す](#jules-に実装を渡す)を参照） |
| `adjutant hub-stop` | このリポジトリの hub 実行記録をクリア |

エージェント側（`adjutant mcp`）：9つのツールと3つのプロンプトを提供します。

- **プロンプト**: `adj-hub`（hub 実行）、`adj-worker`（タスクの着手から完了引き渡しまで）、`adj-report`（作業中に発見したバグを hub に報告）。Claude Code では `/mcp__adjutant__adj-hub` のように呼び出せます。
- **ツール**: `adjutant_config`、`adjutant_hub_status`、`adjutant_send`、`adjutant_pending`、`adjutant_tell`、`adjutant_outbox`、`adjutant_gate_open`、`adjutant_refresh`、`adjutant_skill`。`adjutant_skill` は、プロンプト機能に未対応のエージェントでも同じ手順書を取得できるように用意されています。エージェントに応じた形式（Claude Code の `AskUserQuestion` や Antigravity の `ask_question` など）に自動調整されます（`--agent` または `agent` 引数で指定も可能）。

名前の使い分けとして、人間が入力する CLI コマンドやプロンプトは短く（`adj`, `adj-…`）、システムが参照する MCP サーバー名やツール名は長めに（`adjutant`, `adjutant_…`）揃えています。

リポジトリ固有の情報を扱うコマンドは `--repo owner/name` を受け取ります。省略した場合は、カレントディレクトリ（worktree 含む）の git origin リモートからリポジトリを自動判定します。

1つのリポジトリに hub を複数立てられます。`--hub <id>` はそのどれを指すかを表します。動くのは宛先（セッション名・受信箱・レコード）だけで、設定は変わりません。設定は引き続き `owner/name` で引かれるので、登録済みリポジトリの2つめの hub でも taskSources / issueKeys / verify はそのまま使えます。`--hub` を付けなければ、これまでと同じアドレスのリポジトリ自身の hub になります。ただし `adj work` が開いた worktree の中でだけは、hub を**指す**コマンドがその worktree のレコードから識別子を読みます。何かを**起こす**側（`hub` / `work` / `worker`）は読みません。worktree の中から立てた hub も、タブが開かれた worktree に登録する worker も、他人のレコードを読むことになるからです。

識別子を毎回書き直す必要はありません。`adj hub --hub <id>` はエージェントを起動するコマンドラインに `ADJUTANT_HUB` を載せるので、そのエージェントが叩く `adj` も MCP ツールも自分自身の hub を指します。`adj work` は開いた worktree に識別子を書き込むので、worker は宛先を書かずに送っても自分を出した hub に届きます。

`adj worker` も、登録した識別子をエージェントのコマンドラインに載せます。その前に、引き継いだ `ADJUTANT_HUB` は環境から外します。tmux のように環境を引き継ぐ terminal テンプレートでは、タブを開いた hub の識別子がエージェントに渡り、worktree のレコードより優先されてしまうからです。

`agentEnv` には `ADJUTANT_HUB` を書けます。これは既定値の扱いで、`--hub` も環境変数も無いときに `hub` / `work` / `worker` がこの値を使い、リポジトリ自身の hub ではなくその hub を立てます。起動したコマンドとエージェントが同じ hub を指すようにするためです。`--hub` や引き継いだ `ADJUTANT_HUB` があればそちらが優先され、エージェントのコマンドラインでも設定の値を置き換えます。このキーを足す前に、そのリポジトリで動いている hub は止めてください。足したあとは、素の `adj hub` が設定の hub を探して立て、`adj work` も新しい worker をその hub の下に登録します。hub を指すコマンド（`send` / `pending` / `hub-stop` など）は `agentEnv` を読まないので、hub 自身のシェル以外から打つときは `--hub` か `ADJUTANT_HUB` で指定してください。

`--no-dashboard` / `--dashboard` も同じ経路を通ります。これらは同じコマンドラインに `ADJUTANT_STARTUP_DASHBOARD` として載り、`adjutant config` が解決の時点で織り込むため、`settings.startupDashboard` を読む手順書には設定ファイルの値ではなく**その hub が起動したときのフラグ**が見えます。ただし `--tab` のときはこの変数が出てきません。ターミナルに渡せるのはコマンドラインだけなので、フラグは新しいタブで走る `adjutant hub` にそのまま転送され、**環境を組み立てるのはそちらの `adjutant hub`** になります。最終的な結果は同じで、1プロセス遅れるだけです（2つの経路の dry run の出力が違って見えるのはこのためです）。

### 再起動後の再開

エージェントのアップデートやクラッシュで hub や worker が終了することがあります。hub は終了から `hubAutoResumeHours`（既定は3時間）以内なら、`adj hub` を打つだけで前回の会話に戻ります。それを過ぎていれば空の会話で起動するので、朝の1枚目はまっさらになります。時間に関係なく再開したいときは `--resume` を、時間内でも新しく立てたいときは `--new` を付けます。

```bash
adj hub --resume                  # このリポジトリの hub（リポジトリ内のどこからでも）
adj hub --resume --hub ALPHA-233  # 親タスクの hub（識別子は推測しないので明示する）
adj worker --resume               # worktree の中で実行すると、そこで作業していた worker
```

`{sessionId}` を含むランナー（既定のランナーは `--session-id {sessionId}` として含んでいます）で新しく起動するときは、セッション ID を作ってエージェントに渡し、保存します。再開するときは新しく作らず、保存済みの ID を使います。`{sessionId}` を含まないランナーで新しく起動したときは ID を作らず、前の起動が保存した ID を消します。これで2つ前の起動の会話が開かれることはありません。ID はレコードとは別の場所に保存します。hub の分は state ディレクトリの `sessions/` に、worker の分は worktree の `.claude/adjutant-session.json` に置きます。`hub-stop` や `close` はレコードを消しますが、この ID は残ります。`--resume` はその ID を `hubResumeRunner` / `agentResumeRunner`（既定は Claude Code の `--resume`）で開き直します。二重起動の防止は通常の起動と同じ仕組みで行い、再開したエージェントには止まっていた間に届いた受信箱・outbox を確認するよう伝えます。

hub の終了時刻は、hub の下で動く MCP サーバーが記録します。`adj hub` はセッションを記録する起動（`{sessionId}` を含むランナーでの起動と、再開）のときだけ、`exec` するコマンドラインに `ADJUTANT_HUB_SESSION` を載せます。エージェントが起動する `adjutant mcp` がそれを引き継ぎます。`{sessionId}` を含まないランナーで新しく起動した hub にはこの変数が付かないので、MCP サーバーが動いていても終了時刻は記録されません。MCP サーバーは1分ごとと、エージェントがパイプを閉じたときに、セッションが生きていたことを `sessions/<slug>.alive` に書きます。保存したセッションとは別のファイルにしているのは、古い hub の最後の書き込みが新しい hub の保存を上書きしないようにするためです。MCP サーバーはマシン上のすべてのセッションで動きますが、書き込むのはこの変数を持つものだけです。`adj worker` はエージェントを起動する前にこの変数を外します。hub の下に MCP サーバーが無い場合は終了時刻が分からないので、推測せずに新しく起動します。`hubRunner` を独自に設定していて `hubResumeRunner` を設定していない場合も同じです。組み込みの再開コマンドで開くと独自の runner で足した指定が抜けるので、`--resume` を付けたときだけ再開します。

worker は `--resume` を付けたときだけ再開します。`adj work` は hub が新しい指示書を渡す経路なので、そこで古い会話に戻ると指示書が埋もれてしまいます。

再開した worker は、保存しておいた「自分を出した hub」の下に戻ります。`--resume` を打ったタブが別の hub の `ADJUTANT_HUB` を引き継いでいても、保存された値を優先します。保存されたセッションが無い hub を再開しようとすると、そのリポジトリで再開できる hub の一覧を表示します。`{sessionId}` を含まないランナーで起動したセッションは再開できません。ただしエラーになるのは `--resume` を付けたときだけです。

MCP の `instructions` は約5行の最小限に抑えています。1500行を超える詳細な手順書は、hub や worker が必要になったタイミングでオンデマンドに取得するため、常時コンテキストを圧迫しません。

## 権限

MCP 経由で配信される手順書からは、個別ツールの許可リスト（`allowed-tools`）を指定できません。

そのため、**hub も worker も既定では自動実行モード（unattended）で起動します**。worker のビルドや hub の受信箱監視が確認ダイアログで止まるのを防ぐためです。ただし、Issue の起票確認やタスクの着手確認など、人による判断が必要なチェックポイント（`AskUserQuestion`）は手順書側で維持されます。スキップされるのは `gh issue view` などの日常的なコマンド実行確認です。

都度確認を挟みたい場合は、設定で `hubRunner` と `hubResumeRunner` を変更してください：

```jsonc
"hubRunner": "claude -n {name} --session-id {sessionId} {prompt}",
"hubResumeRunner": "claude -n {name} --resume {sessionId} {prompt}"
```

その場合、hub が停止しないよう `~/.claude/settings.json` で必要なツールを事前に許可しておく必要があります：

```jsonc
"permissions": { "allow": [
  "Bash(adj:*)", "Bash(adjutant:*)",
  "Bash(git:*)", "Bash(gh:*)",
  "Bash(cat:*)", "Bash(ls:*)", "Bash(mkdir:*)", "Bash(mv:*)", "Bash(cp:*)",
  "Bash(sed:*)", "Bash(awk:*)", "Bash(printf:*)", "Bash(date:*)", "Bash(ps:*)",
  "Bash(basename:*)", "Bash(open:*)", "Bash(which:*)",
  "Bash(proctor:*)", "Bash(lk:*)", "Bash(codex:*)",
  "mcp__adjutant__adjutant_config", "mcp__adjutant__adjutant_hub_status",
  "mcp__adjutant__adjutant_send", "mcp__adjutant__adjutant_pending",
  "mcp__adjutant__adjutant_tell", "mcp__adjutant__adjutant_outbox",
  "mcp__adjutant__adjutant_gate_open", "mcp__adjutant__adjutant_refresh",
  "mcp__adjutant__adjutant_skill"
]}
```

hub はメインチェックアウトで動作します。手順書によってメイン側での直接実装は禁止され、作業は必ず worktree 上の worker に委任されますが、これは手順書による制約であり、OS レベルのサンドボックスではない点に留意してください。

## 設定

設定ファイルは `~/.config/adjutant/config.json` です（`$XDG_CONFIG_HOME` および `ADJUTANT_CONFIG` に対応）。詳細は **`config.example.json`** を参照してください。`//` で始まるキーはスキーマ説明用で、実行時に自動で除去されます。

設定は「リポジトリ固有エントリ > `defaults` > トップレベル > 組み込みの既定値」の順に優先されます。設定内容に不備があってもエラー終了はせず、`warnings` を含んだ上で hub が警告を通知します。

各設定項目はコマンドテンプレートになっており、プレースホルダは**シェルクォートされた状態で**展開されます。テンプレート側でプレースホルダを引用符で囲まないでください：

| キー | プレースホルダ | 既定値 |
| --- | --- | --- |
| `terminal.spawn` | `{cwd}` `{title}` `{command}` | iTerm2 |
| `terminal.focus` | `{pid}` `{tty}` `{title}` | iTerm2 |
| `terminal.close` | `{pid}` `{tty}` `{title}` | iTerm2（`false` でタブを一切閉じない。その場合 `adjutant close` は何もせず exit 1） |
| `terminal.title` | `{title}` | tty への OSC エスケープシーケンス（`spawn` が開く全タブにも適用） |
| `wake` | `{pid}` `{tty}` `{subject}` `{line}` | iTerm2 の `write text` で対象セッションに入力 |
| `hubWake` / `workerWake` | 同上 | `wake` を方向別に上書き |
| `agentRunner` | `{sessionId}` `{prompt}` `{worktree}` `{title}` | `claude --session-id {sessionId} --permission-mode auto {prompt}` |
| `hubRunner` | `{name}` `{sessionId}` `{prompt}` | `claude -n {name} --session-id {sessionId} --permission-mode auto {prompt}` |
| `agentResumeRunner` | `agentRunner` と同じ | `claude --resume {sessionId} --permission-mode auto {prompt}` |
| `hubResumeRunner` | `hubRunner` と同じ | `claude -n {name} --resume {sessionId} --permission-mode auto {prompt}` |
| `notification` | `{title}` `{message}` `{nwo}` | `terminal-notifier`（未インストールなら `osascript`） |
| `ide` | `{worktree}` | なし（手順書内でユーザーに確認） |
| `worktreePattern` | `{repo}` `{branch}` `{name}` | `.claude/worktrees/{name}` |
| `hubAutoResumeHours` | なし（数値。`0` で無効） | `3`（この時間以内に終了した hub は `adj hub` で自動的に再開する） |
| `stuckAfterMinutes` | なし（数値。`0` で無効） | `120`（worker が同じ工程にこの分数とどまると板のカードを赤くする。worker が止まっているカードはこの値に関係なく赤くなる） |
| `julesKey` | なし（Jules の API キーを出力するコマンド） | macOS のキーチェーン項目 `jules-api`（`false` にすると Jules に渡せなくなる） |
| `maxWorkers` | なし（1以上の整数） | 制限なし（チェックアウトごとに数える。gate で待っている worker と起動中の worker は枠を使い、止まった worker は使わない） |
| `startupDashboard` | なし（`true` / `false`） | `true`（`false` にすると hub が起動時に一覧を集めなくなる。人が「一覧」と言ったときの収集は止まらない） |
| `hubServe` | なし（`true` / `false`） | `true`（hub の MCP サーバーがその hub の板を hub と同じ寿命で立てる。`127.0.0.1:4577` が空いていればそこ、埋まっていれば空いている port。URL は `adjutant_config` の `board` に入る。手で立てた板が既に動いていればそのままにする。`false` にすると板は `adj serve` で手で立てる） |

キーを省略した場合は既定値が使われ、`false` を指定した場合はその機能が無効化されます。ただしコマンドではない2つの設定は別の値を取ります。`startupDashboard` と `hubServe` は `true` / `false` で、`hubAutoResumeHours` は数値です。`hubAutoResumeHours` を無効にするには `0` を指定します。`false` を指定すると `warnings` に報告され、既定値が使われます。`maxWorkers` は整数で、それ以外の値は `warnings` に報告されて制限なしになります。`terminal` や `wake` 系はキー単位でマージされるため、必要な項目だけを上書きできます。

`{pid}` と `{tty}` は OS 側から見たセッションの名前（プロセスIDと、そのセッションが載っている端末デバイス `ttys004`）であって、**ターミナル自身の pane / window の id ではありません**。そのため `focus` / `close` / `wake` のテンプレートは、動く前にその id を自分で引き当てる必要があります。`{pid}` を pane id を期待する引数（`--pane-id` など）に渡すと別の番号空間を指すことになり、その番号を持っていた無関係な pane に対して動作します。id 解決を行うラッパースクリプトを指定してください。`close` を「実行できたら成功」とみなさないのも同じ理由です。テンプレートは終了コードだけで判断されるため、adjutant は close 後に**その worker が実際に居なくなったこと**を確認してから記録を消し、居たままなら exit 1 を返します。

### 通知の詳細設定
組み込みの通知は `terminal-notifier` があればそれを使い、無ければ `osascript` にフォールバックします。この優先順位には理由があります。コマンドラインの `osascript` が出した通知は macOS が**スクリプトエディタ**からのものとして扱うため、通知バナーの送り主が意図しないアプリになり、クリックしても空のスクリプトエディタが起動するだけで、呼び出し元のセッションには戻れません。[`terminal-notifier`](https://github.com/julienXX/terminal-notifier)（`brew install terminal-notifier`）が入っている場合、組み込みの通知は次のコマンドになります：

```jsonc
"terminal-notifier -title {title} -message {message} -sound Glass -activate com.googlecode.iterm2"
```

これでクリック時にターミナルが前面に出ます。さらに `{nwo}` を使うと、タブ単位まで狙えます。`{nwo}` はそのメッセージが属するリポジトリで、`--repo` が受け取るのと同じ値です（`owner/name`、ただし使える remote が無いチェックアウトではそのディレクトリ名になります）。クリック時に `adj focus --repo {nwo}` を実行する通知コマンドにすれば、**そのリポジトリの hub タブそのもの**が前面に出ます。ただしテンプレート内にクォートで囲んだコマンドを入れ子にするのは避け、スクリプトを用意してそれを指定してください（理由は後述の wake の注意点と同じで、置換される値が自身のクォートを伴って展開されるためです）。なお `adjutant notify` をリポジトリ外で実行した場合 `{nwo}` は空になり、その旨が標準エラー出力に出ます。

macOS 以外には組み込みの通知手段がなく、通知できないことはエラーではなく無音として扱われます。そのため Linux で hub を動かす場合はこのキーの設定が必要です（例: `"notify-send {title} {message}"`）。逆に proctor のサイドバーや tmux のステータス行など、セッションを監視する仕組みが別にある場合は、バナーを二重に出さず `false` で無効化してください。

テンプレートが実際にどのコマンドへ展開されるかは `adjutant notify --message … --dry-run` で確認できます（実際の送信は行われません）。

### wake の詳細設定
`wake`（セッションへの入力通知）は、「ターミナルにどう入力するか」と「エージェントに何を伝えるか」に分かれています。そのため、`hubWake` / `workerWake` ではオブジェクト形式で個別に上書きできます：

```jsonc
"wake": "wake-tab {tty} {line}",                  // ターミナル側の操作
"workerWake": { "line": "check `adj outbox`" }     // エージェント側の入力文言
```

- **複数コマンドの実行**: `sh -c '…'` で囲むとクォートの二重展開で壊れる可能性があるため、シェルスクリプトを用意してそれを呼び出してください。
- **カレントディレクトリ**: `spawn` テンプレートに `{cwd}` が含まれている場合はそのコマンド自身でディレクトリ移動を行うものとみなし、含まれていない場合は先頭に `cd` が付与されます。
- **タブ名**: 新規タブの名前はターミナル API ではなく、タブ内で実行されるシェルが `adjutant title` を呼ぶことで設定されます。
- **環境変数**: `agentEnv` で、hub および worker の起動時に渡す環境変数を設定できます。別プロファイルでエージェントを動かしたい場合に便利です。

## Jules に実装を渡す

タスクの実装を worker ではなく [Jules](https://jules.google) に任せることもできます。その場合も計画は worker が立てます。強いモデルが必要なのは設計のほうなので、plan gate で承認を受けるところまでは worker が進め、承認された設計を Jules の session に渡して終わります。実装、パッチのセルフレビュー、PR の作成は Jules が行います。

どちらに実装させるかはタスクレコードに持たせます。`adjutant task add --executor jules`（または `task update --executor jules`）で指定します。Jules に渡すのは worker が実行する次のコマンドです。

```bash
adj jules start --id <task> --prompt-file design.md --base main
```

このリポジトリを対象に、ファイルの中身を prompt として session を作ります。PR は自動で作らせ、計画は確認なしで承認させます（人がすでに承認しているため）。作った session の id はタスクの `julesSession` に書き込みます。`adj jules show --id <task>` で、session の状態、jules.google.com のページ、PR ができていればその URL を確認できます。1つのタスクを渡せるのは1回だけで、`julesSession` を消すまで2つ目の session は作れません。

API キーはエージェントから読めない場所に置きます。`adj config` はすべての設定を出力し、エージェントはそれを読むので、`julesKey` にはキーそのものではなく、キーを出力するコマンドを書きます。組み込みの既定値は macOS のキーチェーン項目 `jules-api` を読みます。次のコマンドで一度だけ登録してください。キーはコマンド行に書かず、表示されるプロンプトで入力します。

```bash
security add-generic-password -s jules-api -a "$USER" -w
```

キーは `curl` に stdin で渡します（引数にすると `ps` で読めてしまうため）。エラーを含め、出力する文字列からはキーを取り除きます。macOS 以外では、キーを保管している場所から読み出して出力するコマンドを `julesKey` に指定してください。

板では、このカードに worker の代わりに session の状態（待機中・作業中・完了・失敗）が表示され、session のページへのリンクになります。worker は Jules に渡したところで終わるので、worker がいなくても赤くしません。session が失敗したときだけ赤くします。状態は板のページが開いている間だけ、進行中とレビュー中のカードについて、1つの session あたり最大45秒に1回問い合わせます。問い合わせはページの応答とは別のスレッドで行うため、API の応答が遅くても板は遅くならず、表示が少し古くなるだけです。PR ができた session を初めて見たとき、板はその PR をタスクに書き込み、カードをレビュー中に移し、hub に `jules-pr` メッセージ（タスクと PR）を送ります。Jules はコメントに対応するたびに完了し直しますが、そのころにはタスクに PR が入っているので、送るのは1回だけです。新規タスクのフォームの「実装」で、どちらに実装させるかを選べます。

その先は手順書が進めます。指示書に「実装」の行があり、`jules` なら `adj-worker` は計画の承認後に Jules 向けの設計書を書きます。変えるファイルをすべて挙げ、それぞれ何をどう変えるか、触らないもの、足すテストまで書きます。受け取る側のモデルが弱いので、要約ではなく判断を済ませた設計書にします。worker はそのファイルで `adj jules start` を実行し、hub に片付けを頼んで終わります。worktree に残すものはありません。`jules-pr` メッセージが届くと、`adj-hub` は PR の説明の書き直しを軽いモデルのサブエージェントに任せます。材料は設計書（`adj jules show --json` の `prompt`）、Jules が書いた本文、変更ファイルの一覧で、リポジトリの書き方に合わせて書き直します。CodeRabbit の要約ブロックと、Jules の session へのリンク行はそのまま残します。

レビュー指摘は、人が選んで本人の名前で Jules に回します。Jules は起動した本人のコメントには対応しますが、ほかの bot のスレッドには入らないため、レビュー bot の指摘はそのままでは届きません。レビュー中の Jules タスクのサイドシートにある「レビュー指摘を Jules に回す」で、Jules と本人以外（レビュー bot、Copilot、ほかの人）が書いた各スレッドの最初のコメントを一覧し、チェックしたものを1つのコメントにまとめて `gh` で PR に投稿します。`gh` は本人としてログインしているので、本人のコメントになります。Jules は自分の PR へのコメントをメンションなしで読むので、メンションは付けません。`reviewBots` は worker が待つレビューを指定する設定なので、ここでは使いません。各指摘には、コメントにエージェント向けのプロンプトがあれば太字の見出しとそのプロンプトを、無ければ隠しコメントと折りたたみ部分を除いた本文を載せます。プロンプトに毎回入る定型の段落（bot の CLI を実行するよう促す段落など）は除きます。回したコメントの id はタスクの `relayed` に残し、同じコメントを2回回さないようにします。Jules はほかのアカウントのコメントを無視するので、`gh` は session を起動したアカウント（`adj jules start` がタスクの `julesBy` に記録する）でログインしている必要があります。違うアカウントからの転送は断ります。シェルからは `adj jules findings --id <task>` と `adj jules relay --id <task> --comment <id>` で同じことができます。一覧に出るのは行へのコメントだけで、レビュー本文に書かれた指摘は対象外です。

レビューは hub が仕分けて、人は承認するだけにできます。板は、開いている間、レビュー中の Jules タスクの PR も見ます。Jules と本人以外のコメントが届いていて Jules が作業中でなければ、hub に `jules-review` メッセージでそのコメントを知らせます。hub はサブエージェントに、各コメントを PR のコード（ブランチはチェックアウトしない）と照らして、まだ当てはまるか、本当に直す場所はどこかを判断させます。レビュー bot は差分のある行にしかコメントできないので、指摘の場所と直す場所がずれることがあるためです。そのうえで、回す指摘と各指摘への補足、外した指摘とその理由を `relay` の gate に出します。承認すると `adj jules relay --plan-file` で、補足を各指摘の下に付けて投稿します。`changes` なら hub がコメントのとおりに直してから回し、`reject` なら回しません。板がこれを行うのは1つの PR につき2回まで（タスクの `relayRounds`）で、それを超えたらサイドシートの手動の転送を使います。

事前に、対象リポジトリへ Jules の GitHub App を入れておく必要があります。Jules から見えないリポジトリは API が受け付けません。

## 両者はどうやって連絡を取り合うか

### アドレス体系（hub 名）
宛先アドレスとなる hub 名は、両セッションともに `adjutant hub-name` で自動算出します。リポジトリの `owner/name` を正規化した文字列と、小文字化ハッシュの組み合わせで構成され、リポジトリ間での受信箱の衝突を防ぎます。手動で組み立てず、必ずコマンドから取得してください。

### メッセージ配送の仕組み
- **worker → hub**:
  - `adjutant send`（または `adjutant_send` ツール）が `~/.local/state/adjutant/inbox/<slug>/` にファイルを書き込み、`adjutant pending` が読み出します。
  - hub が停止中でもメッセージは保持されます。
  - hub の生存確認は、記録された PID への `kill -0` およびプロセス引数の照合で行われます。
  - すべてのメッセージに `worktree:` ヘッダが付きます。送信元が実際に居た worktree の絶対パスを（本文への手書きではなく）導出したもので、hub が `done` 依頼などを処理するときの宛先になります。git が worktree を特定できない場合はヘッダごと省略され、このヘッダが無い既存のメッセージもそのまま読めます。
- **hub → worker**:
  - 宛先はセッションではなく worktree です。`adjutant tell` が `{worktree}/.claude/adjutant-outbox.md` に追記し、`adjutant outbox` が読み出します。
  - worker 起動時に `adjutant worker` ランチャーが自身の PID を記録し、そのプロセス上でエージェントを `exec` することで、`workerWake` による入力通知を可能にしています。

### wake（起こす処理）と通知
ファイル書き込みだけではエージェントが気づかないため、相手が起動中の場合は **`hubWake`** / **`workerWake`**（既定では iTerm2 へのキー入力）を実行して受信箱の確認を促します。
- `send` は常にデスクトップ通知（`notification`）を発火します。
- `tell` は worker を wake できなかった場合のみデスクトップ通知を発火します（wake できた場合は worker が自律して読むため）。
- wake 処理はベストエフォートであり、失敗してもメッセージ送信自体は成功します。

これらをテンプレート経由で抽象化しているため、ターミナルやエージェントの種類を問わず柔軟に連携できます。

## レイヤ構成

```
repo  config  template  prompts        末端（標準ライブラリと自身の入力のみ）
terminal  runner  notify  ide  messaging   下位層のみ参照可能（横の参照は不可）
cmd/  mcp                                  各モジュールを結合する最上位層
```

モジュール間の依存方向は `scripts/check-layering.sh` で強制され、CI でも検証されます。

## 開発

```bash
cargo test                    # 単体テスト + CLI テスト
./scripts/check-layering.sh   # レイヤリング検証
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```

テストはすべて環境から隔離されています。`ADJUTANT_CONFIG` と `ADJUTANT_STATE_DIR` が一時ディレクトリに向けられるため、ローカルの設定や実行中の hub に影響を与えることはありません。外部プロセスを起動する処理も `--dry-run` 下で実行されます。

## サブエージェントの利用

worker はタスク実行中、環境内に特化サブエージェントが定義されていればそれを活用します：

- **調査**: コードベースの探索や Issue・ドキュメントの読み取りに特化したエージェント（`task-researcher` や、説明に調査・探索・research を含むもの）。見当たらない場合は、組み込みの `Explore` または読み取り専用の指示を付与した `general-purpose` へ自動でフォールバックします。
- **レビュートリアージ**: PR のレビューコメント取得と分類に特化したエージェント（`review-triage` やトリアージ用）。見当たらない場合は `general-purpose` へフォールバックします。
- **セルフレビュー**: 差分の独立検証に特化したエージェント（`self-reviewer` やレビュー用）。見当たらない場合は `general-purpose`（または設定された codex）へフォールバックします。

カスタムサブエージェントはエージェントクライアント側（Claude Code の場合は `~/.claude/agents/*.md` など）で定義されます。これらが定義されていないまっさらな環境でも、すべて `general-purpose` でそのまま動作します。

## 任意の連携ツール

[`proctor`](https://github.com/syarihu/agent-proctor)（worktree 規約、セッション台帳、タブ着色）や [`lk`](https://github.com/syarihu/local-knowledge-cli)（ローカルナレッジベース）が PATH 上にあれば自動で連携し、なければスキップします。

どちらも必須ではありません。worktree の配置や命名規約は `adjutant worktree-path --name`（ブランチ: `{user}/{name}`、パス: `<main>/.claude/worktrees/{name}`）が同等の形式を標準で提供します。また、リポジトリごとの追加ファイル配置は、本ツールの設定にある `postCreate` フックで対応できます。

3つ目は [`terminal-notifier`](https://github.com/julienXX/terminal-notifier) で、これは組み込みの処理自体が参照する唯一のツールです。インストールされていればクリック先を指定できる通知コマンドを使い、無ければ `osascript`（＝スクリプトエディタ名義の通知）にフォールバックします。

## ライセンス

MIT — [LICENSE](LICENSE) を参照してください。
