# 開発ガイド

## 1. 前提

実装開始時に正確なRustバージョンを`rust-toolchain.toml`で固定します。外部コマンド
として`git`が必要です。SQLiteは`rusqlite`のbundled featureを使い、利用者に
SQLiteの別途インストールを要求しません。

## 2. 標準コマンド

Cargoプロジェクト作成後は、次を標準コマンドとします。

```sh
cargo run -- <ローカルパスまたはGitHub URL>
cargo test --all-features
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
```

開発用のキャッシュ、設定、DBの場所を本番利用データから分離できるよう、テストや
手動確認では一時ディレクトリを注入可能にします。テストが実際の
`~/.cache/repomonk`や`~/.local/share/repomonk`へ書き込んではいけません。

## 3. テスト方針

### 単体テスト

- 打鍵: 正解、誤入力、Backspace、Enter、自動インデント、Esc、完走
- 計測: 時刻を注入し、実時間に依存しない速度・正確率計算
- 抽出: タブ、行末空白、非ASCII、長いファイルでも分割せず1単位、hash一致
- 進捗: todo / done / skippedとディレクトリの行数集計
- 対象判定: 除外ディレクトリ、生成物、lock、binary、巨大行・巨大ファイル

### 統合テスト

- 一時fixtureリポジトリの走査結果
- SQLiteへの保存、再接続、ハッシュ照合
- system `git`呼び出しの引数と失敗処理
- CLI引数、終了コード、エラー本文
- 完走と中断時のトランザクション差分
- ファイルEnterで直接Typingへ入ること（チャンク一覧なし）

### E2Eと手動確認

TUIの正しさはスナップショットだけに頼りません。PTYまたは実端末で、
`docs/implementation-plan.md`の受け入れ条件を確認します。テスト用Gitリモートは
ネットワークへ依存せず、ローカルfixtureから作れる構成にします。

## 4. テストfixture

`tests/fixtures/`には次を含む小さなリポジトリを用意します。

- 複数行の対象ファイル（分割されないことを検証）
- 非ASCII行、コメント、空行、タブ、行末空白
- 自動対象外になるlock、生成物、binary、巨大行
- ネストしたディレクトリ
- 正規化後の本文は同じで、前後に空行だけが増えた更新版

巨大ファイルをそのままコミットせず、テスト内で生成するか、判定境界を注入可能に
します。

## 5. エラー設計

- ライブラリ境界では型付きエラーを使う。
- 実行入口でのみ文脈を付け、ユーザー向けメッセージと終了コードへ変換する。
- gitのstderrは秘密情報を含む可能性があるため、DBへ永続化しない。
- panic時にも端末復元を試みるが、panicを期待するテスト設計にはしない。

## 6. AIエージェント作業規約

- 担当開始前に`AGENTS.md`、プロダクト要件、アーキテクチャ、実装計画を読む。
- 統合担当が共有契約と所有ファイルを決めてから並列作業を開始する。
- 担当外の公開API変更が必要なら、直接広範囲を編集せず統合担当へ理由を伝える。
- 新規依存は、用途、代替、ライセンス、ビルド影響を確認して統合担当が追加する。
- 各担当は実装とテストを同じ作業単位で完成させる。
- formatterによる共有ファイルの機械的変更は統合時に行う。
- 既存の無関係な変更を戻さない。

## 7. 完了時の報告

担当エージェントは次を短く報告します。

- 実装した契約と挙動
- 変更ファイル
- 実行したテストと結果
- 未解決事項または統合担当が行う作業

統合担当は、受け入れ条件、品質ゲート、手動スモークの結果をまとめてMVP完成を
判断します。

## 8. 配布とリリース

### 8.1 仕組み

配布はGitHub Releasesのビルド済みバイナリと`install.sh`だけで行い、
crates.io、npm、Homebrewへは公開しません。ワークフローは2つです。

- `.github/workflows/ci.yml` — `main`へのpushとPRで品質ゲートを回す通知装置。
  `main`にbranch protectionはかけないため、これは赤くても作業を止めません。
- `.github/workflows/release.yml` — `vX.Y.Z`タグのpushで動きます。
  `verify`ジョブがタグ名と`Cargo.toml`の`version`の一致を確認し、品質ゲートを
  再実行します。ここを通らない限りバイナリは外へ出ません。その後
  3ターゲットをビルドし、`SHA256SUMS`を添えてReleaseを作ります。

ビルド対象は次の3つです。Linuxはglibc互換性のため意図的に`ubuntu-22.04`
（glibc 2.35）でビルドします。`ubuntu-latest`だとより新しいglibcを要求します。

| ランナー | ターゲット | 形式 |
| --- | --- | --- |
| `macos-latest` | `aarch64-apple-darwin` | tar.gz |
| `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | tar.gz |
| `windows-latest` | `x86_64-pc-windows-msvc` | zip |

### 8.2 リリース手順

1. `Cargo.toml`の`version`を上げ、`cargo build`で`Cargo.lock`も更新する。
   0.xの間は、ユーザーから見える挙動の破壊的変更でminor、それ以外でpatch。
2. 依存クレートを増減した場合は`THIRD-PARTY-LICENSES.md`を再生成する（8.3）。
3. 品質ゲート3コマンドをローカルで通す。
4. `main`へコミットしてpushし、CIが緑になることを確認する。
5. タグを打つ。**ユーザーの明示指示なしにこの操作をしない。**

   ```sh
   git tag v0.1.1
   git push origin v0.1.1
   ```

6. Actionsの成功と、Releaseに資材4点（tar.gz×2、zip×1、`SHA256SUMS`）が
   揃っていることを確認する。
7. 一度pushしたタグは打ち直さず、削除もしない。誤った場合は次のパッチ版を出す。

タグを打つ前に動作確認したい場合は、`release.yml`を`workflow_dispatch`で
実行します。`tag`入力を空にするとビルドまで走らせてReleaseを作りません。
タグ名を入れると`verify`のバージョン照合だけ試せます。

### 8.3 サードパーティライセンスの再生成

配布アーカイブに`THIRD-PARTY-LICENSES.md`を同梱しています。依存クレートを
増減したら再生成してください。CIには入れていないので手作業です。

```sh
cargo install cargo-about --locked   # 初回のみ
cargo about generate --all-features -o THIRD-PARTY-LICENSES.md about.hbs
```

新しいライセンスが増えて生成が失敗した場合は、内容を確認したうえで
`about.toml`の`accepted`へ追加します。GPL系が混ざった場合は、追加せずに
依存の採用そのものを見直してください。

### 8.4 手元での確認

インストール経路を試すときは、既存のインストールと本番のDBを壊さないよう
分離します。

```sh
REPOMONK_INSTALL_DIR=/tmp/rmtest sh install.sh
REPOMONK_CACHE_DIR=/tmp/rmcache REPOMONK_DATA_DIR=/tmp/rmdata \
  /tmp/rmtest/repomonk salan70/repomonk-sample-typescript
```
