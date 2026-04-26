#!/usr/bin/env bash
# grimoire installer — download the latest GitHub release tarball and drop the
# binary into ~/.local/bin. No Rust toolchain or C compiler needed.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/strycore/grimoire/main/install.sh | bash
#
# Pin a version:
#   curl -fsSL https://raw.githubusercontent.com/strycore/grimoire/main/install.sh | VERSION=v0.1.0 bash
#
# Different install dir:
#   curl -fsSL https://raw.githubusercontent.com/strycore/grimoire/main/install.sh | GRIMOIRE_INSTALL_DIR=$HOME/bin bash

set -euo pipefail

REPO="${GRIMOIRE_REPO:-strycore/grimoire}"
INSTALL_DIR="${GRIMOIRE_INSTALL_DIR:-$HOME/.local/bin}"
BIN_NAME="grimoire"

err()  { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
info() { printf '\033[32m›\033[0m %s\n' "$*"; }

# Detect platform.
arch="$(uname -m)"
os="$(uname -s)"
case "$os $arch" in
  "Linux x86_64"|"Linux amd64")  target="linux-x86_64" ;;
  "Linux aarch64"|"Linux arm64") target="linux-aarch64" ;;
  *) err "no prebuilt binary for $os $arch — build from source: https://github.com/${REPO}" ;;
esac

# Required tools.
for cmd in curl tar sed; do
  command -v "$cmd" >/dev/null 2>&1 || err "missing required command: $cmd"
done

# Resolve version.
if [ -z "${VERSION:-}" ]; then
  info "looking up latest release..."
  VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name":[[:space:]]*"\(v[^"]*\)".*/\1/p' \
    | head -n 1)
  [ -n "$VERSION" ] || err "couldn't resolve latest release for ${REPO}"
fi
version_no_v="${VERSION#v}"

archive="${BIN_NAME}-${version_no_v}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/${VERSION}/${archive}"
sha_url="${url}.sha256"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

info "downloading ${archive}..."
curl -fL --progress-bar -o "${tmp}/${archive}" "$url" \
  || err "download failed: $url"

if curl -fsSLI "$sha_url" >/dev/null 2>&1; then
  info "verifying SHA-256 checksum..."
  curl -fsSL -o "${tmp}/${archive}.sha256" "$sha_url"
  (cd "$tmp" && sha256sum -c --quiet "${archive}.sha256") \
    || err "checksum mismatch — refusing to install a tampered tarball"
fi

mkdir -p "$INSTALL_DIR"
tar -xzf "${tmp}/${archive}" -C "$tmp"
mv "${tmp}/${BIN_NAME}" "${INSTALL_DIR}/${BIN_NAME}"
chmod +x "${INSTALL_DIR}/${BIN_NAME}"

info "installed ${BIN_NAME} ${version_no_v} → ${INSTALL_DIR}/${BIN_NAME}"

# PATH check — warn but don't fix automatically.
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    printf '\n'
    printf 'Note: %s is not in $PATH.\n' "$INSTALL_DIR"
    printf '  Add to your shell rc:\n'
    printf '    export PATH="%s:$PATH"\n' "$INSTALL_DIR"
    ;;
esac
