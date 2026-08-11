# repomonk

`repomonk` は、実在するリポジトリのソースコードをターミナル上で写経する
Rust 製 TUI アプリです。完了したチャンクを積み上げ、リポジトリのファイル
ツリーを少しずつ塗りつぶしていく体験を目指します。

現在は、実装に先立って仕様と設計を整備している段階です。細かな段階リリースは
設けず、AIエージェントで作業を分担し、最初から一連の体験が使えるMVPを完成
させます。

## ドキュメント

- [プロダクト要件](docs/product-requirements.md)
- [アーキテクチャ](docs/architecture.md)
- [実装計画](docs/implementation-plan.md)
- [開発ガイド](docs/development.md)
- [意思決定記録](docs/decisions.md)
- [GitHub公開前チェックリスト](docs/public-release-checklist.md)

ユーザーから見える挙動については、プロダクト要件を正とします。設計や計画を
変更する際、ユーザー体験にも影響がある場合は、プロダクト要件と意思決定記録も
同じ変更で更新します。

## 採用予定の技術

- Rust
- `ratatui` / `crossterm`
- `tree-sitter`
- bundled SQLite を有効にした `rusqlite`
- リポジトリ取得にはシステムの `git`

## 現在の状態

実行可能なコードはまだありません。MVPの範囲と完了条件は
[実装計画](docs/implementation-plan.md)に記載します。

## ライセンス

[MIT License](LICENSE)
