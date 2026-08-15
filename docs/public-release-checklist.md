# GitHub公開前チェックリスト

確認日: 2026-08-11

## 現在の判定

初回公開に必要なライセンスとコミット作者情報は設定済みです。GitHub上のpublic
リポジトリを作成できる状態です。実装コードの公開後は、本書の「実装開始後に必須」
も満たしてからリリースします。

## 確認済み

- `main`を初期ブランチとしてGitを初期化済み。
- 現在の追跡候補ファイルに、APIキー、トークン、秘密鍵、パスワード、個人用絶対
  パス、元仕様書のダウンロード先を検出していない。
- ローカルDB、環境変数ファイル、ビルド成果物、ログを`.gitignore`へ追加済み。
- 原典仕様書そのものはリポジトリへコピーせず、実装用に整理した文書だけを置いて
  いる。
- MIT Licenseを採用し、`LICENSE`とREADMEへ明記済み。
- このリポジトリのコミット作者メールをGitHub提供のnoreply形式に設定済み。
- GitHub、crates.io、npmで`repomonk`の完全一致名を確認し、2026-08-11時点で
  明確な既存プロジェクトまたはパッケージを検出していない。

名称の空きは商標権や将来の登録可能性を保証するものではありません。公開・配布の
直前にも再確認します。

## 公開前に決定済み

### ライセンスを選ぶ

ライセンスがない公開リポジトリは、閲覧やforkが可能でも、第三者に利用・改変・
再配布の一般的な許可を与えません。OSSとして利用や貢献を許可するなら、
`LICENSE`を追加します。

候補:

- MIT: 短く、利用条件が緩い。著作権表示とライセンス表示の維持を求める。
- Apache-2.0: 明示的な特許ライセンスと特許条項を含む。
- 非OSSとして公開: ライセンスを付けず、READMEに閲覧目的であることを明記する。

本プロジェクトではMIT Licenseを採用しました。

### コミット作者情報を決める

このリポジトリだけにGitHubのID付きnoreplyアドレスを設定します。グローバルGit
設定は変更しません。

## 実装開始後に必須

- [x] Rustツールチェーンを固定し、アプリケーションとして`Cargo.lock`をコミットする。
  `rust-toolchain.toml`で`stable`と`rustfmt`/`clippy`を指定し、`Cargo.lock`は
  コミット済み。
- [x] `cargo fmt`、`cargo clippy`、`cargo test`を実行するGitHub Actionsを追加する。
  `.github/workflows/ci.yml`（`main`へのpushとPR）と
  `.github/workflows/release.yml`の`verify`ジョブ（タグ時）で実行する。
  `main`へのbranch protectionはかけず、リリース経路の側でゲートする（D-034）。
- [x] 依存クレートのライセンスと配布条件を確認する。詳細は後述の
  「依存クレートのライセンス確認」を参照。
- [x] fixtureには第三者リポジトリのコードを無断転載せず、自作の最小コードを使う。
  `tests/fixtures/`は自作の最小コードのみで構成している。
- [x] リリースバイナリを配布する場合、対応OS、検証方法、同梱ライセンスを明記する。
  対応OSはREADMEの「対応環境」、検証方法はReleasesの`SHA256SUMS`（`install.sh`が
  自動照合）、同梱ライセンスは`THIRD-PARTY-LICENSES.md`をアーカイブへ同梱。
- [x] READMEにインストール方法、対応環境、開発状態、データ保存先、`--purge`を記載する。
- [x] 脆弱性報告を受け付ける場合は`SECURITY.md`と連絡方法を追加する。
  `SECURITY.md`を追加済み。GitHub設定でPrivate vulnerability reportingを
  有効化する作業が残っている。

## 依存クレートのライセンス確認

2026-08-15時点、`cargo metadata --all-features`で解決される195クレートを確認した。

- GPL、AGPL、CDDL、EPLは0件。すべてpermissiveなライセンス。
- 内訳の大半は`MIT OR Apache-2.0`（118）と`MIT`（37）。
- `option-ext 0.2.0`（`dirs`経由）のみMPL-2.0。ファイル単位のcopyleftであり、
  当該クレートを改変せずリンクする使い方では自プロジェクトの開示義務は生じない。
  表示義務は残るため同梱一覧に含める。
- `r-efi 6.0.0`はLGPL-2.1-or-laterも選択肢に含むが、`MIT OR Apache-2.0`を
  選択できる。加えてUEFIターゲット用であり、配布する3ターゲットでは
  実際にはリンクされない。
- `(MIT OR Apache-2.0) AND Unicode-3.0`が1件あり、Unicodeライセンスの表示が要る。

同梱物として特に確認したもの:

- `libsqlite3-sys`はMITで、一覧に含まれる。同梱されるSQLite本体のCソースは
  public domainであり、追加の表示義務はない。
- `syntect`が同梱するシンタックス定義は、`syntect`クレート自体のライセンス表示に
  含まれる形で扱う。

表示は`cargo about`が生成する`THIRD-PARTY-LICENSES.md`を正とし、依存を増減した
ときに再生成する。手順は`docs/development.md`§8.3。生成対象は配布する3ターゲット
の実行時依存に絞っており、2026-08-15時点で162クレート（MIT 158、Apache-2.0 1、
MPL-2.0 1、Unicode-3.0 1、Zlib 1）。デュアルライセンスのクレートは、repomonk自身の
ライセンスに合わせてMITへ解決している。

## 実装時のセキュリティ条件

- Git URLを厳格に検証し、ユーザー入力をシェル文字列として実行しない。
- `git`は`std::process::Command`の個別引数で実行し、オプション終端を考慮する。
- 走査ではシンボリックリンクを既定で辿らない。
- clone先とpurge対象がアプリ管理ディレクトリ内にあることを正規化後に検証する。
- raw modeとalternate screenを正常終了・中断・エラー・panic時に復元する。
- gitのstderrやリポジトリ内容をテレメトリ、DB、ログへ不用意に複製しない。
- MVPでは生の打鍵列を永続化せず、時間、入力数、ミス数など必要最小限の集計だけを
  保存する。
- SQLite更新をトランザクション化し、破損時には安全に失敗させる。

## GitHubリポジトリ作成時

- visibilityが`public`であることを作成直前に再確認する。
- 説明文とtopicsに、TUI、Rust、typing-practiceなど内容に即した語を使う。
- 初期状態ではbranch protectionより先にCIを用意し、CI完成後に必須チェックを設定
  する。
- IssuesやDiscussionsは、実際に対応する予定がある場合だけ有効にする。
- 公開後、GitHub上のファイル一覧とコミット作者情報をもう一度確認する。
