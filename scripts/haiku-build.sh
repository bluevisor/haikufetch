#!/bin/sh
# Builds haikufetch natively on a Haiku machine over ssh (Haiku ships rustc + cargo as packages,
# cross-compiling would need a Haiku sysroot for libbe). Usage: scripts/haiku-build.sh [ssh-host]
set -eu
host="${1:-haiku}"
cd "$(dirname "$0")/.."

# --no-mac-metadata keeps macOS' xattrs out, Haiku's tar warns on every one of them
tar --no-mac-metadata --exclude=./target --exclude=./.git -czf - . \
    | ssh "$host" 'mkdir -p ~/haikufetch && tar -C ~/haikufetch -xzf -'
ssh "$host" 'cd ~/haikufetch && cargo build --release && ls -l target/release/haikufetch'
