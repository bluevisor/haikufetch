#!/bin/sh
# Installs the latest haikufetch release for this machine.
#   curl -fsSL https://raw.githubusercontent.com/bluevisor/haikufetch/main/install.sh | sh
# Installs to /usr/local/bin when writable, otherwise ~/.local/bin. Set HAIKUFETCH_DIR to choose.
set -eu

repo="bluevisor/haikufetch"

case "$(uname -s)" in
    Linux) os="unknown-linux-musl" ;;
    Darwin) os="apple-darwin" ;;
    Haiku)
        echo "On Haiku, build it: pkgman install rust_bin && cargo install --git https://github.com/$repo" >&2
        exit 1
        ;;
    *) echo "haikufetch has no build for $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) arch="x86_64" ;;
    aarch64 | arm64) arch="aarch64" ;;
    *) echo "haikufetch has no build for $(uname -m)" >&2; exit 1 ;;
esac

target="$arch-$os"
url="https://github.com/$repo/releases/latest/download/haikufetch-$target.tar.gz"

if [ -n "${HAIKUFETCH_DIR:-}" ]; then
    dir="$HAIKUFETCH_DIR"
elif [ -w /usr/local/bin ]; then
    dir="/usr/local/bin"
else
    dir="$HOME/.local/bin"
fi
mkdir -p "$dir"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "Downloading haikufetch for $target"
if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$work/haikufetch.tar.gz"
else
    wget -qO "$work/haikufetch.tar.gz" "$url"
fi
tar -xzf "$work/haikufetch.tar.gz" -C "$work"
install -m 755 "$work/haikufetch" "$dir/haikufetch"

echo "Installed $dir/haikufetch"
case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "Add $dir to your PATH to run it by name." ;;
esac
"$dir/haikufetch" || true
