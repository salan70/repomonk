# CLAUDE.md

Claude Code 向けの入口です。作業ルールの正は [AGENTS.md](AGENTS.md) とし、
ここには重複して書かず、参照先と最低限の手順のみを置きます。

## 最初に読むもの

- [AGENTS.md](AGENTS.md) — 文書の優先順位、実装ルール、検証ルール、Git運用ルール
- [docs/product-requirements.md](docs/product-requirements.md) — ユーザーから見える挙動の正
- [docs/architecture.md](docs/architecture.md) — モジュール境界と依存方向
- [docs/decisions.md](docs/decisions.md) — 決定事項の記録

## 標準コマンド

```sh
cargo run -- <ローカルパスまたはGitHub URL>
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

タスクは fmt、clippy、test がすべて通った時点で完了とします。

## Git

`main` に直接コミットし、コミット指示があればそのまま `origin/main` へ
プッシュします。詳細は [AGENTS.md](AGENTS.md) の「Git運用ルール」を参照。
