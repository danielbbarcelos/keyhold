#!/usr/bin/env bash
# Builds Keyhold.app around an already-compiled universal `keyhold` binary.
#
#   packaging/macos/make-app.sh <path-to-keyhold> <version> <out-dir>
#
# The app carries the program inside it: opening it starts Keyhold (or brings the running one
# forward) and the browser opens on it. It also owns keyhold:// links. It is signed ad hoc only, so
# the first opening needs "Abrir mesmo assim" in Ajustes do Sistema > Privacidade e Segurança.
set -euo pipefail

BIN="$1"
VERSION="$2"
OUT="$3"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
APP="$OUT/Keyhold.app"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m 755 "$BIN" "$APP/Contents/Resources/keyhold"

cat >"$APP/Contents/MacOS/Keyhold" <<'LAUNCH'
#!/usr/bin/env bash
# Start Keyhold, or bring the running one forward, keeping its output in a log only this user
# can read: the log carries the session address.
umask 077
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/keyhold"
mkdir -p "$STATE"
chmod 700 "$STATE" 2>/dev/null || true
LOG="$STATE/app.log"
[ -f "$LOG" ] && mv -f "$LOG" "$LOG.1"
exec "$(cd "$(dirname "$0")/../Resources" && pwd)/keyhold" --present >"$LOG" 2>&1
LAUNCH
chmod +x "$APP/Contents/MacOS/Keyhold"

SET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$SET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$ROOT/assets/icon.png" --out "$SET/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$ROOT/assets/icon.png" --out "$SET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o "$APP/Contents/Resources/AppIcon.icns"

cat >"$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Keyhold</string>
  <key>CFBundleDisplayName</key><string>Keyhold</string>
  <key>CFBundleIdentifier</key><string>io.github.keyhold</string>
  <key>CFBundleExecutable</key><string>Keyhold</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSUIElement</key><true/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>Keyhold</string>
      <key>CFBundleURLSchemes</key><array><string>keyhold</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP"
echo "$APP"
