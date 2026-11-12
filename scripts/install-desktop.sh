#!/usr/bin/env bash
# Builds Halcyon and installs it for the current user only (no sudo):
#   ~/.local/bin/halcyon-desktop
#   ~/.local/share/applications/halcyon.desktop   (app launcher entry)
#   ~/.local/share/icons/hicolor/*/apps/halcyon.png
#
#   scripts/install-desktop.sh            build and install
#   scripts/install-desktop.sh --remove   remove the launcher, icons and binary
#
# Mail data and passwords are not touched by either.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor"
DESKTOP="$APP_DIR/halcyon.desktop"

refresh() {
  command -v update-desktop-database >/dev/null && update-desktop-database "$APP_DIR" || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$ICON_DIR" || true
}

if [[ "${1:-}" == "--remove" ]]; then
  rm -f "$DESKTOP" "$BIN_DIR/halcyon-desktop"
  for size in 32 64 128 256 512; do
    rm -f "$ICON_DIR/${size}x${size}/apps/halcyon.png"
  done
  refresh
  echo "Halcyon-Starter entfernt. Mails und Konten sind unverändert."
  exit 0
fi

cd "$ROOT/apps/desktop"
pnpm install --frozen-lockfile
pnpm tauri build --no-bundle

install -Dm755 "$ROOT/target/release/halcyon-desktop" "$BIN_DIR/halcyon-desktop"

ICONS="$ROOT/apps/desktop/src-tauri/icons"
install -Dm644 "$ICONS/32x32.png" "$ICON_DIR/32x32/apps/halcyon.png"
install -Dm644 "$ICONS/64x64.png" "$ICON_DIR/64x64/apps/halcyon.png"
install -Dm644 "$ICONS/128x128.png" "$ICON_DIR/128x128/apps/halcyon.png"
install -Dm644 "$ICONS/128x128@2x.png" "$ICON_DIR/256x256/apps/halcyon.png"
install -Dm644 "$ICONS/icon.png" "$ICON_DIR/512x512/apps/halcyon.png"

mkdir -p "$APP_DIR"
cat > "$DESKTOP" <<DESKTOP_ENTRY
[Desktop Entry]
Type=Application
Name=Halcyon
GenericName=E-Mail
Comment=E-Mail mit Komfort
Exec=$BIN_DIR/halcyon-desktop
Icon=halcyon
Terminal=false
Categories=Network;Email;Office;
Keywords=Mail;E-Mail;IMAP;
StartupWMClass=halcyon-desktop
DESKTOP_ENTRY

refresh
echo "Halcyon ist im App-Menü. Entfernen mit: scripts/install-desktop.sh --remove"
