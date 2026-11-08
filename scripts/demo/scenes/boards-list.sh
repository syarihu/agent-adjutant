#!/usr/bin/env bash
# Seeds a board with tasks on every stage so the list view reads well in a short clip.
source "$(dirname "$0")/common.sh"

# Workers playing their scripted scenes.
ISSUE_URL=https://github.com/acme/shop-app/issues/61 start_worker shipping-fee \
  "カートの合計に送料が入らない" start \
  "カート画面で合計金額を計算する際に送料が加算されない不具合を修正します。" > /dev/null

ISSUE_URL=https://github.com/acme/shop-app/issues/58 start_worker order-mail \
  "注文確認メールに配送予定日を入れる" start \
  "注文確認メールの本文にお届け予定日を記載するようにします。" > /dev/null

ISSUE_URL=https://github.com/acme/shop-app/issues/64 start_worker price-filter \
  "商品検索に価格の絞り込みを足す" start \
  "商品一覧の検索条件に価格帯での絞り込み機能を追加します。" > /dev/null

ISSUE_URL=https://github.com/acme/shop-app/issues/60 start_worker low-stock \
  "在庫が少ない商品を管理画面で知らせる" start \
  "在庫数が設定した閾値を下回った商品を管理画面のトップにアラート表示します。" > /dev/null

# Tasks with no worker.
add_task "ポイントの有効期限をマイページに表示する" 55 pr \
  https://github.com/acme/shop-app/pull/66 \
  "マイページで保有ポイントと有効期限を確認できるようにします。" > /dev/null

add_task "注文履歴を CSV で書き出す" 52 pr \
  https://github.com/acme/shop-app/pull/68 \
  "注文履歴一覧を CSV 形式でダウンロードできるようにします。" > /dev/null

# No word the scripted hub plays a scene for (クーポン): a queued task is handed to the hub.
add_task "ギフト包装を選べるようにする" 70 queued "" \
  "注文時にギフト包装の有無と包装紙の柄を選べるようにします。" > /dev/null

add_task "商品画像を遅延読み込みにする" 49 done "" \
  "商品一覧ページで画像の遅延読み込みを行い初期表示を高速化します。" > /dev/null

# The server reads a card's pull request on its own schedule; read them now, so the approved one is
# in マージ待ち and the one waiting on mika in コードレビュー待ち when the clip starts.
adj task refresh > /dev/null
