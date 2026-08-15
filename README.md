# repomonk

`repomonk` は、実在するリポジトリのソースコードをターミナル上で写経する
Rust 製 TUI アプリです。ファイル単位で写経し、完了した行数を積み上げて
リポジトリのファイルツリーを少しずつ塗りつぶしていく体験を目指します。

## 使い方

```sh
# ローカルリポジトリまたは単一ファイル
cargo run -- /path/to/repo

# GitHub（system git で shallow clone、~/.cache/repomonk にキャッシュ）
cargo run -- https://github.com/<owner>/<repo>
cargo run -- <owner>/<repo>

# 初回体験用サンプル（Homeの「サンプル」からも選択可能）
cargo run -- salan70/repomonk-sample-typescript
cargo run -- salan70/repomonk-sample-python
cargo run -- salan70/repomonk-sample-java

# 管理データの削除（確認あり。CI では --yes）
cargo run -- --purge
```

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

## 開発

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## 技術

- Rust / `ratatui` / `crossterm`
- bundled SQLite（`rusqlite`）
- リポジトリ取得はシステムの `git`（GitHub API は使わない）

## 現在の状態

MVP の縦切り体験（取得 → ツリー → 写経 → 結果 → 進捗保存）と、
Homeから開けるTypeScript / Python / Javaの自動販売機サンプルが動作します。

## ライセンス

[MIT License](LICENSE)
