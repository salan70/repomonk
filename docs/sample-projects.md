# 初回体験用サンプル

状態: contract revision 1

## リポジトリ

| 言語 | リポジトリ | 推奨モード |
| --- | --- | --- |
| TypeScript | [`salan70/repomonk-sample-typescript`](https://github.com/salan70/repomonk-sample-typescript) | Flow |
| Python | [`salan70/repomonk-sample-python`](https://github.com/salan70/repomonk-sample-python) | Flow |
| Java | [`salan70/repomonk-sample-java`](https://github.com/salan70/repomonk-sample-java) | Manual |

## 共通仕様

自動販売機は、Water（A1、120円、在庫2）、Tea（B1、150円、在庫1）、
Coffee（C1、180円、在庫0）を持ちます。CashBoxの初期硬貨は100円4枚、
50円1枚、10円10枚、500円0枚です。対応硬貨は500 / 100 / 50 / 10円です。

購入は`<product-code> <coin>...`で指定します。引数なしの場合は、B1へ100円を
2枚投入する購入を2回行い、1回目を成功、2回目を売り切れとして表示します。
失敗した購入は在庫とCashBoxを変更しません。

釣銭は500→100→50→10円の有限枚数貪欲法で計算します。商品価格は10円単位で、
対応額面はすべて下位額面で割り切れるため、動的計画法や組合せ探索は使いません。

## ゴールデン出力

3言語の引数なし実行は、次の出力を一致させます。

```text
Vending Machine Demo
Purchased: Tea (B1)
Price: 150 yen
Paid: 200 yen
Change: 50 yen [50 x 1]
Remaining stock: 0

Second purchase: sold out (B1)
```

通常の購入拒否は終了コード1、CLI形式不正は終了コード2とします。

## Git運用

3リポジトリはそれぞれ独立した`main`を持ち、PR必須、CI必須、squash mergeのみ、
force-push禁止で運用します。共通仕様を変更するときはrepomonk側の1 Issueから
3本のPRを起こし、3版のCI成功コミットを確認してから同じ`v1.0.0`タグを付けます。
clean cloneとrepomonkでの走査確認後にHomeカタログを公開します。
