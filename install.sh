#!/bin/sh
set -e
cd "$(dirname "$0")"
cargo build --release
install -Dm755 target/release/files "$HOME/.local/bin/files"
install -Dm644 assets/icons/files.svg "$HOME/.local/share/icons/hicolor/scalable/apps/files.svg"
install -Dm644 files.desktop "$HOME/.local/share/applications/files.desktop"
gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
