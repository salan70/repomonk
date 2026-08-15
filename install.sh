#!/bin/sh
# repomonk installer
#
#   curl -fsSL https://raw.githubusercontent.com/salan70/repomonk/main/install.sh | sh
#
# 環境変数:
#   REPOMONK_VERSION      入れたいバージョン（例: 0.1.0）。既定は最新リリース。
#   REPOMONK_INSTALL_DIR  インストール先。既定は $HOME/.local/bin。
#
# GitHub API は使わず、releases のリダイレクトだけで取得する。
# レート制限を避けられ、repomonk 本体が GitHub API を使わない方針とも揃う。

set -eu

REPO="salan70/repomonk"
BASE_URL="https://github.com/${REPO}/releases"
INSTALL_DIR="${REPOMONK_INSTALL_DIR:-${HOME}/.local/bin}"

TMPDIR_INSTALL=""
cleanup() {
	if [ -n "$TMPDIR_INSTALL" ] && [ -d "$TMPDIR_INSTALL" ]; then
		rm -rf "$TMPDIR_INSTALL"
	fi
}
trap cleanup EXIT INT TERM

info() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
	printf 'error: %s\n' "$*" >&2
	exit 1
}

# --- ターゲット判定 -----------------------------------------------------------

os="$(uname -s)"
arch="$(uname -m)"

case "${os}/${arch}" in
Darwin/arm64 | Darwin/aarch64)
	target="aarch64-apple-darwin"
	ext="tar.gz"
	;;
Linux/x86_64 | Linux/amd64)
	target="x86_64-unknown-linux-gnu"
	ext="tar.gz"
	;;
*)
	printf 'error: %s は自動インストールに対応していません（%s %s）\n' "$REPO" "$os" "$arch" >&2
	printf '\n配布しているのは次の 3 つです。\n' >&2
	printf '  - macOS Apple Silicon  aarch64-apple-darwin\n' >&2
	printf '  - Linux x86_64         x86_64-unknown-linux-gnu\n' >&2
	printf '  - Windows x86_64       x86_64-pc-windows-msvc（手動 zip のみ）\n' >&2
	printf '\n手動での入手先: %s\n' "$BASE_URL" >&2
	printf 'Rust があれば次でも入ります:\n' >&2
	printf '  cargo install --git https://github.com/%s --locked\n' "$REPO" >&2
	exit 1
	;;
esac

asset="repomonk-${target}.${ext}"

if [ -n "${REPOMONK_VERSION:-}" ]; then
	version_tag="v${REPOMONK_VERSION#v}"
	download_base="${BASE_URL}/download/${version_tag}"
	info "repomonk ${version_tag} (${target}) を入れます。"
else
	download_base="${BASE_URL}/latest/download"
	info "repomonk の最新版 (${target}) を入れます。"
fi

# --- 取得ツール ---------------------------------------------------------------

if command -v curl >/dev/null 2>&1; then
	fetch() { curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
	fetch() { wget -qO "$2" "$1"; }
else
	die "curl か wget が必要です。"
fi

if command -v sha256sum >/dev/null 2>&1; then
	sha256_of() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
	sha256_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
	die "sha256sum か shasum が必要です。検証なしでは入れません。"
fi

# --- ダウンロードと検証 -------------------------------------------------------

TMPDIR_INSTALL="$(mktemp -d)"

info "取得中: ${download_base}/${asset}"
fetch "${download_base}/${asset}" "${TMPDIR_INSTALL}/${asset}" ||
	die "ダウンロードに失敗しました。バージョン名と ${BASE_URL} を確認してください。"

fetch "${download_base}/SHA256SUMS" "${TMPDIR_INSTALL}/SHA256SUMS" ||
	die "SHA256SUMS を取得できませんでした。"

expected="$(grep " ${asset}\$" "${TMPDIR_INSTALL}/SHA256SUMS" | cut -d' ' -f1 || true)"
[ -n "$expected" ] || die "SHA256SUMS に ${asset} の記載がありません。"

actual="$(sha256_of "${TMPDIR_INSTALL}/${asset}")"
if [ "$expected" != "$actual" ]; then
	printf 'error: チェックサムが一致しません。中止します。\n' >&2
	printf '  expected: %s\n' "$expected" >&2
	printf '  actual:   %s\n' "$actual" >&2
	exit 1
fi
info "チェックサム OK"

# --- 展開と配置 ---------------------------------------------------------------

tar -xzf "${TMPDIR_INSTALL}/${asset}" -C "$TMPDIR_INSTALL" ||
	die "アーカイブの展開に失敗しました。"
[ -f "${TMPDIR_INSTALL}/repomonk" ] || die "アーカイブに repomonk が入っていません。"

mkdir -p "$INSTALL_DIR" || die "${INSTALL_DIR} を作成できませんでした。"
cp "${TMPDIR_INSTALL}/repomonk" "${INSTALL_DIR}/repomonk" ||
	die "${INSTALL_DIR} へ書き込めませんでした。REPOMONK_INSTALL_DIR で別の場所を指定できます。"
chmod +x "${INSTALL_DIR}/repomonk"

installed_version="$("${INSTALL_DIR}/repomonk" --version 2>/dev/null || echo 'repomonk')"
info ""
info "インストールしました: ${INSTALL_DIR}/repomonk (${installed_version})"

# --- 事後チェック -------------------------------------------------------------

if ! command -v git >/dev/null 2>&1; then
	info ""
	warn "git が見つかりません。GitHub のリポジトリを取得するには git が必要です。"
	warn "ローカルのパスを指定する使い方だけなら git なしでも動きます。"
fi

case ":${PATH}:" in
*":${INSTALL_DIR}:"*) ;;
*)
	info ""
	warn "${INSTALL_DIR} が PATH に入っていません。シェルの設定へ次を足してください。"
	# shellcheck disable=SC2016 # $PATH はそのまま出力したい
	printf '\n  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
	;;
esac

info ""
info "はじめの一歩:"
info "  repomonk salan70/repomonk-sample-typescript"
info ""
info "管理データを消すとき:"
info "  repomonk --purge"
