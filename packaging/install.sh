#!/usr/bin/env bash
# Builds a release binary and installs DTerminal for the current user
# (no sudo needed) — the lightweight replacement for electron-builder's
# AppImage/deb targets: a stripped release binary plus a .desktop entry
# so it shows up in the application menu.
set -euo pipefail

if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi

cd "$(dirname "$0")/.."

echo "Building release binary..."
cargo build --release

mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"
install -m 755 target/release/dterminal "$HOME/.local/bin/dterminal"
sed "s|__HOME__|$HOME|" packaging/dterminal.desktop > "$HOME/.local/share/applications/dterminal.desktop"

echo "Installed $HOME/.local/bin/dterminal"
echo "Installed $HOME/.local/share/applications/dterminal.desktop"
echo "Make sure \$HOME/.local/bin is on your PATH, then look for \"DTerminal\" in your application menu."
