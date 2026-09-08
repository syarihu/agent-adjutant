[English](README.md) | 日本語

# agent-adjutant

<img alt="agent-adjutant-logo" src="docs/images/agent-adjutant-logo.png" />

> 英語版が正本 ([README.md](README.md))

コーディングエージェントのためのタスク hub を、1つのバイナリで。

`adjutant` はリポジトリの副官です。仕事を worker に配り、その報告を受け取ります。hub はタスクを選び、worktree を切り、指示書を書いて、新しいタブで worker を起動します。worker が自分のタスクと無関係なバグを踏んだときは、自分で直したり自分で issue を立てたりせず、hub に差し戻します。この両方が手順書として定められており、その手順書はバイナリの中に同梱されています。

## なぜプロンプトを配信するバイナリなのか

手順書は以前、各エージェントのコマンドディレクトリにコピーする markdown ファイルでした。コピーはずれていきます。ツールを更新してもコピーは古いまま取り残され、設定ディレクトリごとに1つずつ、どれも微妙に違うものが残ります。MCP 経由で配信すれば、手順書は実体ではなくポインタになります。1回の更新がすべての呼び出し元に反映されます。

機械的な処理（hub 名の導出、設定の解決、タブを開く、メッセージを運ぶ）も、以前は3つの手順書ファイルに同じ文章として書き写されていました。これは1つのルールに3つのバージョンがあるのと同じです。今はそれらがコマンドになっており、手順書はそのコマンドを呼び出します。

## インストール

```bash
cargo install --git https://github.com/syarihu/agent-adjutant # `adjutant` と短い `adj` の両方が入ります
# またはローカルチェックアウトから:
#   cargo install --path .
# または cargo install を使わない場合:
#   cargo build --release && cp target/release/adjutant target/release/adj ~/bin/
adjutant install-mcp            # MCP サーバーを Claude Code に登録（user スコープ）
adjutant install-mcp --target json   # 他のクライアント向けに JSON を出力する場合
```

`install-mcp` は、誰かが手で追う手順を表示するのではなく、登録そのものを実行します。中で `claude mcp add` を実行します。設定ファイルの持ち主であるツールが、そのファイルを書くべきだからです。`json` ターゲットは、このツールが知らないクライアント向けの逃げ道であり、貼り付け作業をお願いする唯一の経路です。

**登録の前にインストールしてください。** 登録処理は、実行したバイナリが PATH ですでに解決できる場合は `adjutant` と記録し、そうでない場合はその絶対パスを記録します。つまりビルドディレクトリの中から `install-mcp` を実行すると、そのビルドが恒久的に固定されてしまい、`cargo clean` した時点でサーバーが壊れます。

**`adj` は同じプログラムの短い名前です。** 両方のバイナリがインストールされ、以下の例はどちらの名前でも動きます。`adj work` は worker のタブを `adj worker` として起動します。自分自身の名前を書き出すコマンドは、呼ばれたときの名前をそのまま使うためです。

## 2つのモード

シェル側。エージェントが存在する*前*に動く場所のためのものです。ランチャー、フック、手順書自身の `Bash` ステップなど（好みに応じて、すべて `adj` に置き換えられます）：

| | |
| --- | --- |
| `adjutant hub` | このリポジトリの hub を、メインチェックアウトで、1つだけ起動する |
| `adjutant hub-name [--json]` | hub のセッション名 — 報告の宛先になるアドレス |
| `adjutant config` | このリポジトリ向けに解決された設定を JSON で出力する |
| `adjutant pending [--json\|--read N\|--ack N\|--path]` | hub 宛に待っているもの |
| `adjutant send --subject … --body …` | hub にメッセージを渡す（本文は stdin からでもよい） |
| `adjutant work --worktree … --title …` | タブを開いて、そこで worker を起動する |
| `adjutant worker --worktree …` | 自分が worker になる（`work` がタブを開いて実行するもの） |
| `adjutant tell --worktree … --subject …` | その worktree の worker にメッセージを置く |
| `adjutant outbox [--clear]` | hub がここの worker に残したもの |
| `adjutant spawn --cwd … -- cmd …` | タブを開いて、その中で何かを実行する |
| `adjutant focus` | 動いている hub のタブを前面に出す。なければ exit 1 |
| `adjutant ide --worktree …` | worktree を設定されたエディタで開く |
| `adjutant title --title …` | このプロセスがいるタブに名前を付ける（hub は自分で自分に付ける） |
| `adjutant notify --message …` | 何かが起きたことを人間に伝える |
| `adjutant worktree-path --name …` | タスクの worktree のブランチ・パス・ベース |
| `adjutant hub-stop` | このリポジトリの hub の記録を消す |

エージェント側（`adjutant mcp`）。同じ仕組みを、7つのツールと3つのプロンプトとして提供します：

- **プロンプト** — `adj-hub`（hub を動かす）、`adj-worker`（指示書から引き渡しまでタスクを進める）、`adj-report`（見つけたバグを hub に渡す）。Claude Code ではこれらが `/mcp__adjutant__adj-hub` のように公開されます。

- **ツール** — `adjutant_config`、`adjutant_hub_status`、`adjutant_send`、`adjutant_pending`、`adjutant_tell`、`adjutant_outbox`、`adjutant_skill`。最後のものは、プロンプトと同じ手順書のテキストを返します。MCP のプロンプト対応はエージェントによってばらつきがあり、誰も取得できない手順書は誰も従わない手順書だからです。

名前は、人間が打つところでは短く、何かが読み返すところでは長くしています。コマンドラインでは `adj`、プロンプトは `adj-…`、対して MCP サーバーは `adjutant`、ツールは `adjutant_…` です。`adjutant` と書かれた登録内容や解決済みの設定は、1年後に見ても何のものか分かります。

リポジトリについて答えるコマンドはすべて `--repo owner/name` を受け取ります（`outbox` / `mcp` / `install-mcp` はリポジトリの話ではないので取りません）。指定しない場合は、いま自分が立っているチェックアウト（worktree も含む）の origin リモートからリポジトリを読み取ります。

`instructions` は5行です。1500行ある手順書が意味を持つのは hub か worker が動いている間だけで、どちらも必要になった時点で自分から取得します。常時コンテキストに載せる価値があるのは1点だけ、worker は直しに来たわけではないバグを報告してよいということです。

## 権限

MCP 経由で配信される手順書は、ツールの許可リストを持てません。スラッシュコマンドのファイルなら frontmatter の `allowed-tools:` で持てました。手順書をバイナリに取り込んで失ったのは、この1点だけです。

そのため、**hub も worker も、既定では承認を求めずに起動します**。worker が止まってはいけないのは、終わらせるべきビルドがあるからです。hub が止まってはいけないのは、承認を待っている hub は受信箱を読んでいない hub であり、しかもそのタブは誰も見ていないからです。誰も見ていないことがこの仕組みの前提そのものです。これは重要な問いかけを消すものではありません。手順書自身が持つ `AskUserQuestion` のチェックポイント（この issue を立てるか？ 着手するか？）はそのままです。なくなるのは、`gh issue view` を実行してよいかを尋ねられることです。

hub に尋ねさせたい場合は、このモードを設定から外します。

```jsonc
"hubRunner": "claude -n {name} {prompt}"
```

そのうえで、手順書が使うものを `~/.claude/settings.json` で事前に許可しておきます。ただしこの許可リストはある時点のスナップショットです。手順書が新しいものを使い始めた瞬間にずれ、その症状は hub が黙り込むという形で現れます。

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
  "mcp__adjutant__adjutant_skill"
]}
```

hub はメインチェックアウトで動くため、承認を求めずに動く hub はそのワーキングツリーに触れられます。手順書はそこで何かを実装することを禁じており、作業はすべて自分の worktree を持つ worker に出されますが、それは文章で書かれたルールであって、サンドボックスではありません。

## 設定

`~/.config/adjutant/config.json`（`$XDG_CONFIG_HOME` と `ADJUTANT_CONFIG` はどちらも尊重されます）。**`config.example.json`** を参照してください。その `//` で始まるキーがスキーマのドキュメントで、セッションに渡る前にすべて取り除かれます。

より具体的な指定が勝ちます。リポジトリ単位のエントリ、次に `defaults`、次にトップレベル、最後に組み込みの既定値の順です。設定が壊れていても解決処理は失敗しません。`warnings` を返し、どこがおかしいかは hub が説明します。

マシン固有のことは何一つハードコードされていません。以下はどれもコマンドテンプレートで、プレースホルダは**シェルのクォートを済ませた状態で**展開されます。したがって、プレースホルダを引用符で囲まないでください：

| キー | プレースホルダ | 既定値 |
| --- | --- | --- |
| `terminal.spawn` | `{cwd}` `{title}` `{command}` | iTerm2 |
| `terminal.focus` | `{pid}` `{tty}` `{title}` | iTerm2 |
| `terminal.title` | `{title}` | このプロセスの tty に書き込む OSC エスケープ |
| | | *`spawn` が開くすべてのタブにも名前を付ける* |
| `wake` | `{pid}` `{tty}` `{subject}` `{line}` | iTerm2 の `write text` でそのセッションに送る |
| `hubWake` / `workerWake` | 同じもの | 片方向だけ `wake` を上書きする |
| `agentRunner` | `{prompt}` `{worktree}` `{title}` | `claude --permission-mode auto {prompt}` |
| `hubRunner` | `{name}` `{prompt}` | `claude -n {name} --permission-mode auto {prompt}` |
| | | *`{name}` を外すと、人が読むどの一覧でもそのセッションが無名になる* |
| `notification` | `{title}` `{message}` | 音付きの macOS 通知 |
| `ide` | `{worktree}` | なし — 手順書は推測せずに尋ねる |
| `worktreePattern` | `{repo}` `{branch}` `{name}` | `.claude/worktrees/{name}` |

キーを省略すると組み込みの既定値になります。`false` を設定するとその挙動が無効になり、これは省略とは別の答えです。`terminal` と `wake` 系はキー単位でマージされるので、リポジトリごとに片方だけ変えても、もう片方を書き直す必要はありません。型の違う設定は無視されますが、同時に `warnings` に出ます — 何かが黙って効かないときは `adj config` を見てください。

`wake` は、設定の他の項目にはない切れ目で分かれています。セッションを**どうつつくか**はターミナルの性質で、つついた後に**何と言うか**はエージェントの性質です。そのため `hubWake` / `workerWake` は、どちらの半分だけでも上書きできる長い形式を受け取ります。

```jsonc
"wake": "tmux send-keys -t {tty} {line} Enter",   // マシン側
"workerWake": { "line": "check `adj outbox`" }     // このエージェントは MCP を持たない
```

これは、hub が片方のエージェント、worker が別のエージェントというリポジトリに必要な形です。組み込みの文言は MCP ツール（`adjutant_pending`、`adjutant_outbox`）を名指ししており、方向ごとに受信箱と送信箱をそれぞれ指しています。1コマンドを超えるものを書きたいときは、`sh -c '…'` で包むのではなくスクリプトを指してください。展開された値は自分のクォートを伴って届くため、包んでいる側の引用符付き文字列を途中で終わらせてしまいます。`{cwd}` を含む `spawn` テンプレートは、自分でディレクトリを移動するものとして信頼されます。含まないものには `cd` が先頭に付けられます。新しいタブの名前は、ターミナル自身の API ではなく、そのタブの中のシェルが `adjutant title` を呼ぶことで付けられます。`set name of session` は唯一一般化できない仕組みで、タイトル書式をユーザー変数で駆動しているプロファイルではこれが無視され、タブは間違った名前のまま黙って残ります。`agentEnv` は、hub とその worker の両方を起動するときに渡す環境変数のオブジェクトで、別のエージェントプロファイルの下で動かすリポジトリのためのものです。

## 両者はどうやって連絡を取り合うか

宛先は hub 名です。両者はそれぞれルールを書き写すのではなく、同じ方法（`adjutant hub-name`）で導出します。中身は `owner/name` を読める形に潰したものと、小文字化した元の名前のダイジェストです。小文字化するのは、設定の引き当てが大文字小文字を区別しないためで、宛先だけ区別すると1つのリポジトリが2つに割れます。潰した側だけでは一意になりません（`acme/foo-bar` と `acme/foo_bar` と `acme-foo/bar` は同じ形に潰れます）し、宛先を共有する2つのリポジトリは受信箱を共有してしまいます。**名前を手で組み立てないでください** — 1文字違えば別の箱で、ダイジェストは当てられません。

**worker → hub** はファイルです。`adjutant send`（または `adjutant_send` ツール）が `~/.local/state/adjutant/inbox/<slug>/` に書き込み、`adjutant pending` がそれを読みます。受け手がいないせいで配送が失敗することはありません。hub が動いていなければメッセージはそのまま待つだけで、そのどちらが起きたのかは応答が伝えます。`adjutant hub-stop` と、記録された PID に対する `kill -0` およびコマンドラインの照合が、終了済みの hub が動いているように見えるのを防いでいます。

**hub → worker** もファイルですが、宛先はセッションではなく worktree です。`adjutant tell` が `{worktree}/.claude/adjutant-outbox.md` に `##` のセクションを追記し、`adjutant outbox` がそれを読みます。worker 自身の記録はその隣に置かれ、書くのは `adjutant worker` です。これはタブが実際に実行するランチャーで、自分の PID を記録したうえで、自分自身の上にエージェントを `exec` します。`adjutant hub` が hub に対して行うのと全く同じです。この記録があるおかげで `workerWake` が成立します。PID がなければ、つつく相手がいません。

worker のコマンドラインには特徴がありません（設定が名指ししたエージェントそのものです）。そのため記録はプロセスの開始時刻に紐づけられています。`exec` は開始時刻を保つので、ランチャーが書いた記録が、自分と入れ替わったエージェントを今も特定できます。

ディレクトリにファイルが現れても誰にも通知は届かないため、配送には両方向とも後半があります。相手が起動していれば、`send` は **`hubWake`** を、`tell` は **`workerWake`** を実行します。既定ではその tty のセッションに対する iTerm2 の `write text` で、対話中のエージェントには人間が打ち込んだのと全く同じように届きます。打ち込まれる1行は報告の内容を繰り返さず受信箱を指すだけなので、本文の置き場所は1箇所だけになります。`send` はどちらの場合でも `notification` を発火します。`tell` が発火するのは worker を起こせなかったときだけです。起こされた worker は誰の手も借りずにメッセージを読むからです。どちらの応答も、`present` / `woken` のどちらが起きたのかを伝えます。起こす処理は仕組みからしてベストエフォートです。フックが動く前にメッセージはすでに配送されているので、つつくのに失敗しても send が失敗することはありません。そもそも hub は決まった3箇所で受信箱を読み直します。

これらすべてをテンプレート経由にしている理由は、そうすればこの通信路がどのターミナルのどのエージェントからでも機能するからです。以前のバージョンは、あるコーディングエージェント固有のセッション一覧とセッション間メッセージの上に作られており、それ以外の場所では動きませんでした。

## レイヤ構成

```
repo  config  template  prompts        葉 — stdlib と自分への入力だけ、他には触らない
terminal  runner  notify  ide  messaging   下には手を伸ばせる、横には伸ばせない
cmd/  mcp                                  それらを繋ぐ唯一の層
```

`scripts/check-layering.sh` がこの矢印を強制し、CI がそれを実行します。これはスタックではなく扇形なので、Cargo のワークスペースに分割すると、実体のある2つの層の間に何もしない中継層が生えてしまいます。チェックが通っている限り、後から分割することは機械的な作業のままです。

## 開発

```bash
cargo test                    # ユニットテスト + エンドツーエンドテスト
./scripts/check-layering.sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```

テストは外部から隔離されています。`ADJUTANT_CONFIG` と `ADJUTANT_STATE_DIR` は一時ディレクトリを指すため、どのテストもあなたの設定を読むことはなく、本当に動いている hub にテスト用の報告を投げ込むこともありません。ウィンドウを開く・エージェントを起動する・人間に通知する処理は、すべて `--dry-run` の下で動きます。

## 任意の隣接ツール

`proctor`（worktree の規約、セッション台帳、タブの色）と `lk`（ローカルナレッジベース）は、PATH 上にあれば使い、なければ飛ばします。

どちらも必須ではありません。規約ツールは worktree を*作る*わけではありません。worktree をどこに置き、どのブランチに乗せるかを答えるだけで、`git worktree add` はどちらにしても自分で打ちます。つまり規約ツールがないときに欠けるのは規約そのもので、それは `adjutant worktree-path --name` が供給します。ブランチは `{user}/{name}`、パスは `<main>/.claude/worktrees/{name}` で、対抗する2つ目の規約を作るのではなく、規約ツールが使うのと意図的に同じ形にしてあります。唯一代わりがないのは、リポジトリごとの `copyFiles` です。それはこの設定の `postCreate` に書くもので、`postCreate` はどちらの場合でも実行されます。

## ライセンス

MIT — [LICENSE](LICENSE) を参照してください。
