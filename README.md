# repomonk

`repomonk` は、実在するリポジトリのソースコードをターミナル上で写経する
Rust 製 TUI アプリです。ファイル単位で写経し、完了した行数を積み上げて
リポジトリのファイルツリーを少しずつ塗りつぶしていく体験を目指します。

## インストール

### 前提

- リモートリポジトリを取得するにはシステムの `git` が必要です。
  ローカルのパスを指定する使い方だけなら `git` なしでも動きます。
- ソースからビルドする場合のみ、C コンパイラも必要です
  （`rusqlite` の bundled SQLite と tree-sitter の grammar が使います）。

### macOS (Apple Silicon) / Linux (x86_64)

```sh
curl -fsSL https://raw.githubusercontent.com/salan70/repomonk/main/install.sh | sh
```

既定で `~/.local/bin/repomonk` へ入ります。変更したい場合:

```sh
curl -fsSL https://raw.githubusercontent.com/salan70/repomonk/main/install.sh | REPOMONK_INSTALL_DIR=/usr/local/bin sh
```

バージョンを固定したい場合は `REPOMONK_VERSION=0.1.0` を渡してください。
スクリプトは [Releases](https://github.com/salan70/repomonk/releases) の
アーカイブを取得し、`SHA256SUMS` と照合してから配置します。

### 手動でインストールする

[Releases](https://github.com/salan70/repomonk/releases) から自分の環境の
アーカイブと `SHA256SUMS` を取得し、検証してから PATH の通った場所へ置きます。

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing   # Linux では sha256sum -c
tar -xzf repomonk-aarch64-apple-darwin.tar.gz
```

配布バイナリにはコード署名・公証をしていません。ブラウザからダウンロードした
場合は macOS の Gatekeeper に隔離されるため、次で解除してください
（`curl` 経由の上記スクリプトでは隔離属性が付かないので不要です）。

```sh
xattr -d com.apple.quarantine ./repomonk
```

### Windows (x86_64)

Releases の `repomonk-x86_64-pc-windows-msvc.zip` を展開して配置してください。
バイナリは提供していますが、**動作は未検証**です。ターミナルによっては表示が
崩れる可能性があります。

### Rust ツールチェーンがある場合

```sh
cargo install --git https://github.com/salan70/repomonk --locked
```

### 対応環境

| 環境 | 状態 |
| --- | --- |
| macOS (Apple Silicon) | 対応 |
| Linux x86_64 (glibc 2.35 以降) | 対応 |
| Windows x86_64 | バイナリのみ提供、動作未検証 |
| その他 | `cargo install` でのビルドをお試しください |

### アンインストール

```sh
repomonk --purge          # 管理しているデータをすべて削除
rm ~/.local/bin/repomonk  # バイナリを削除
```

## 使い方

```sh
# ローカルリポジトリまたは単一ファイル
repomonk /path/to/repo

# GitHub（system git で shallow clone、~/.cache/repomonk にキャッシュ）
repomonk https://github.com/<owner>/<repo>
repomonk <owner>/<repo>

# 初回体験用サンプル（Homeの「サンプル」からも選択可能）
repomonk salan70/repomonk-sample-typescript
repomonk salan70/repomonk-sample-python
repomonk salan70/repomonk-sample-java

# 管理データの削除（確認あり。CI では --yes）
repomonk --purge
```

### データの保存先

repomonk が作るものはすべてローカルに置かれ、外部へは送信されません。

| 用途 | 場所 |
| --- | --- |
| 設定 | `~/.config/repomonk/config.toml`（無ければ既定値） |
| clone キャッシュ | `~/.cache/repomonk/repos/`（OS の cache ディレクトリ） |
| 進捗・統計 DB | `~/.local/share/repomonk/repomonk.db`（OS の data ディレクトリ） |

`repomonk --purge` はこれらをまとめて削除します。

操作の目安:

- 共通: Esc/`q` で戻る（Home の `q` だけ終了）、`?` ヘルプ、`,` 設定、`S` 実績
- Home: `j`/`k` 選択、Enter で開く、`/` 検索
- Tree: `j`/`k` 移動、Enter で写経、`Tab` でおすすめ、`/` 絞り込み、`x`/`X` で対象の切替
- Typing: 正しいキーのみ受理、Backspace、Esc でポーズ（もう一度 Esc で Tree へ戻る）
- Result: Enter で次へ、`r` でもう一度、Esc で Tree へ

## ドキュメント

- [プロダクト要件](docs/product-requirements.md)
- [アーキテクチャ](docs/architecture.md)
- [実装計画](docs/implementation-plan.md)
- [開発ガイド](docs/development.md)
- [意思決定記録](docs/decisions.md)
- [GitHub公開前チェックリスト](docs/public-release-checklist.md)
- [初回体験用サンプル仕様](docs/sample-projects.md)
- [セキュリティポリシー](SECURITY.md)

## 開発

```sh
# ソースから直接動かす
cargo run -- /path/to/repo
cargo run -- salan70/repomonk-sample-typescript

# 品質ゲート
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

リリース手順は [開発ガイド](docs/development.md) を参照してください。

## 技術

- Rust / `ratatui` / `crossterm`
- bundled SQLite（`rusqlite`）
- リポジトリ取得はシステムの `git`（GitHub API は使わない）

## 現在の状態

MVP の縦切り体験（取得 → ツリー → 写経 → 結果 → 進捗保存）と、
Homeから開けるTypeScript / Python / Javaの自動販売機サンプルが動作します。

## ライセンス

[MIT License](LICENSE)

依存クレートのライセンス表示は [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md)
にまとめてあり、配布アーカイブにも同梱しています。

脆弱性の報告方法は [SECURITY.md](SECURITY.md) を参照してください。
