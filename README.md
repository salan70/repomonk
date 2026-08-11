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

# 管理データの削除（確認あり。CI では --yes）
cargo run -- --purge
```

操作の目安:

- Tree: `j`/`k` 移動、Enter でファイルを開いて写経、Space で折りたたみ、`q`/Esc で終了
- Typing: 正しいキーのみ受理、Backspace、Esc で中断
- Result: Enter/Esc で Tree へ戻る

## ドキュメント

- [プロダクト要件](docs/product-requirements.md)
- [アーキテクチャ](docs/architecture.md)
- [実装計画](docs/implementation-plan.md)
- [開発ガイド](docs/development.md)
- [意思決定記録](docs/decisions.md)
- [GitHub公開前チェックリスト](docs/public-release-checklist.md)

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

MVP の縦切り体験（取得 → ツリー → 写経 → 結果 → 進捗保存）が動作します。
tree-sitter ラベル、dependency モード、Home/Search、演出などは MVP 後です。

## ライセンス

[MIT License](LICENSE)
