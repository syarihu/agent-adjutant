[English](jules.md) | 日本語

# Jules へのタスク委譲

タスクの実装をローカルの worker ではなく [Jules](https://jules.google) に任せることができます。ローカルの worker セッションは立ち上げず、hub が読み取り専用の detached な worktree を作ってサブエージェントに詳細な設計書を書かせます。人が計画を承認した段階で Jules のセッションへ引き渡します。

---

## 事前準備

1. **Jules GitHub App**: 対象リポジトリに Jules の GitHub App をインストールしてください。
2. **API キーの設定**:
   設定の `julesKey` には、API キーを出力するコマンドを指定します。macOS の場合はキーチェーンの `jules-api` にキーを登録します。

   ```bash
   security add-generic-password -s jules-api -a "$USER" -w
   ```

   キーは標準入力経由で `curl` に渡され、コマンド出力やログからは自動でマスクされます。Linux の場合は、キーを出力するコマンドを `julesKey` に設定してください。

---

## 実行フロー

### 1. タスクの担当者を Jules に設定する

タスクの作成時または更新時に `--executor jules` を指定します。

```bash
adjutant task add --executor jules
# または
adjutant task update --id <task-id> --executor jules
```

### 2. サブエージェントによる設計書の作成

Jules に渡すタスクでは、`adj-hub` 手順が計画用サブエージェントに詳細な設計書（`design.md`）を書かせます。
- 変更対象のファイルをすべて列挙し、変更内容を具体的に指定
- 変更してはいけないファイルを明記
- 必要なテストと動作確認手順を整理

hub は `adj gate open --body-file design.md --openedBy hub` で plan gate を開きます。ボード上には作成された設計書がそのまま表示され、人が確認します。

### 3. セッションの開始

計画が承認されると、hub が Jules にタスクを引き渡し、一時 worktree を削除します。

```bash
adj jules start --id <task-id> --prompt-file design.md
```

- `--base`（またはタスクに記録された base）を起点ブランチとしてセッションを開始
- 作業完了時に自動で PR を作成するよう指定
- 生成されたセッション ID をタスクの `julesSession` に記録
- 進行状況は `adj jules show --id <task-id>` で確認可能

---

## ボードでの進捗管理と PR 反映

ボード上では、worker の代わりに Jules セッションの状態（待機中・作業中・完了・失敗）が表示され、`jules.google.com` の該当ページへのリンクになります。
- worker セッションが存在しなくても停止扱い（赤色警告）にはなりません。
- Jules が PR を作成すると、ボードが自動で PR を検知してタスクに紐付け、カードをレビュー中へ移動します。
- hub の手順が、リポジトリのスタイルに合わせて PR の説明文を自動で整形します。

---

## レビュー指摘の転送（Relay）

Jules はセッションを起動した本人のコメントにのみ反応し、外部の bot や他のレビュー担当者のスレッドには自発的に介入しません。

### 手動による指摘の転送
タスクパネルの「レビュー指摘を Jules に回す」から実行できます。
1. レビュアーや Copilot、各種 review bot のスレッドの先頭コメントを一覧表示
2. Jules に対応させたい指摘にチェックを入れる
3. `gh` がログインしているアカウントの名義で PR にまとめて投稿。タスクに Jules セッションの起動者が記録されている場合は、そのアカウントと一致しないと転送は拒否されるので、`gh auth switch` で切り替えてからやり直す
4. CLI からの実行例:
   ```bash
   adj jules findings --id <task-id>
   adj jules relay --id <task-id> --comment <comment-id>
   ```

### hub による自動仕分け
ボードが稼働している間は、以下の自動処理も利用できます。
1. Jules が待機中のときに第三者からのレビューコメントが届くと、ボードが hub に通知
2. hub のサブエージェントが、ブランチをチェックアウトせずに各コメントをコードと照合
3. まだ有効な指摘か、修正箇所のずれはないかを判断し、`relay` gate を開いて人に提示
4. 人が承認すると、`adj jules relay --plan-file` で各指摘に補足を添えて自動投稿
5. この自動仕分けは 1 つの PR につき最大 2 回まで実行されます（`relayRounds`）。
