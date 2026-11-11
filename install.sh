#!/usr/bin/env bash
# ───────────────────────────────────────────────────────────────────────────
#  Keyhold — installer
#
#  Builds the interface bundle, compiles the binary with it embedded, puts it
#  on your PATH, and adds a desktop shortcut that opens Keyhold in your
#  browser.
#
#  Options:
#      --with=set-host[=NAME]  Also give Keyhold a name in /etc/hosts
#                              (keyhold.local by default). Asks for sudo.
#
#  Environment:
#      SKIP_BUILD    "1" to skip the frontend build (web/dist must exist).
#      SKIP_DESKTOP  "1" to skip the desktop shortcut.
#      BIN_DIR       Where to put the binary (default: ~/.local/bin).
#
#  Re-running upgrades in place.
# ───────────────────────────────────────────────────────────────────────────
set -Eeuo pipefail

APP_BIN="keyhold"
APP_DISPLAY_NAME="Keyhold"
APP_GENERIC_NAME="Password Vault"
APP_COMMENT="A password vault that lives entirely on this machine"
APP_CATEGORIES="System;Security;"
APP_KEYWORDS="password;vault;secrets;keyhold;"

SOURCE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

SET_HOST=""
for arg in "$@"; do
  case "$arg" in
    --with=set-host) SET_HOST="keyhold.local" ;;
    --with=set-host=*) SET_HOST="${arg#--with=set-host=}" ;;
    -h|--help)
      sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) printf "unknown option: %s (try --help)\n" "$arg" >&2; exit 2 ;;
  esac
done
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"

if [ -t 1 ]; then
  RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'
  CYAN=$'\033[36m'; BOLD=$'\033[1m'; DIM=$'\033[2m'; RESET=$'\033[0m'
else
  RED=""; GREEN=""; YELLOW=""; CYAN=""; BOLD=""; DIM=""; RESET=""
fi

info() { printf "  %s●%s %s\n" "$CYAN" "$RESET" "$*"; }
ok()   { printf "  %s●%s %s\n" "$GREEN" "$RESET" "$*"; }
warn() { printf "  %s●%s %s\n" "$YELLOW" "$RESET" "$*" >&2; }
fail() { printf "  %s●%s %s\n" "$RED" "$RESET" "$*" >&2; exit 1; }

has() { command -v "$1" >/dev/null 2>&1; }

header() {
  printf "\n  %sKeyhold — installer%s\n" "$BOLD" "$RESET"
  printf "  %s───────────────────%s\n\n" "$DIM" "$RESET"
}

check_toolchain() {
  info "Checking the toolchain..."

  has cargo || fail "cargo not found. Install Rust: https://rustup.rs"

  if [ "${SKIP_BUILD:-0}" != "1" ]; then
    has node || fail "Node.js is required to build the interface bundle."
    has npm  || fail "npm is required. It usually ships with Node.js."
  fi

  ok "cargo $(cargo --version | cut -d' ' -f2)$( [ "${SKIP_BUILD:-0}" != "1" ] && printf ", node %s" "$(node --version)")"
}

build_frontend() {
  if [ "${SKIP_BUILD:-0}" = "1" ]; then
    warn "SKIP_BUILD=1 — using the bundle already in web/dist"
    [ -f "$SOURCE_DIR/web/dist/index.html" ] || fail "web/dist is empty; run without SKIP_BUILD."
    return
  fi

  info "Building the interface..."
  (cd "$SOURCE_DIR/web" && npm install --silent --no-audit --no-fund && npm run build >/dev/null)
  ok "interface built"
}

build_binary() {
  # The bundle is compiled in, so this has to happen after the frontend build.
  info "Compiling Keyhold (this takes a minute the first time)..."
  (cd "$SOURCE_DIR" && cargo build --release --quiet)

  mkdir -p "$BIN_DIR"
  install -m 755 "$SOURCE_DIR/target/release/$APP_BIN" "$BIN_DIR/$APP_BIN"
  ok "installed $BIN_DIR/$APP_BIN"
}

install_icons() {
  local source="$SOURCE_DIR/assets/icon.png"
  [ -f "$source" ] || { warn "no assets/icon.png — the shortcut will use a generic icon"; return 1; }

  # Real sizes rather than one file in every directory: a theme trusts the
  # directory's name, and a 1024px image served as a 48px icon is what makes a
  # launcher look blurry.
  local sizes="48 64 128 256 512"
  local installed=0

  for size in $sizes; do
    local dir="$HOME/.local/share/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$dir"

    if has convert; then
      convert "$source" -resize "${size}x${size}" "$dir/$APP_BIN.png" 2>/dev/null && installed=1
    elif has magick; then
      magick "$source" -resize "${size}x${size}" "$dir/$APP_BIN.png" 2>/dev/null && installed=1
    fi
  done

  # The original, at its own size.
  local full="$HOME/.local/share/icons/hicolor/1024x1024/apps"
  mkdir -p "$full"
  cp "$source" "$full/$APP_BIN.png"
  installed=1

  if [ "$installed" = "1" ] && ! has convert && ! has magick; then
    warn "ImageMagick not found — only the full-size icon was installed"
  fi

  return 0
}

install_desktop_shortcut() {
  if [ "${SKIP_DESKTOP:-0}" = "1" ]; then
    warn "SKIP_DESKTOP=1 — skipping the desktop shortcut"
    return
  fi

  info "Installing the desktop shortcut..."

  local apps_dir="$HOME/.local/share/applications"
  mkdir -p "$apps_dir" "$BIN_DIR"

  install_icons || true

  # A wrapper rather than calling the binary directly: a desktop launcher has no
  # terminal, so Keyhold's output — including the URL it opens — would go
  # nowhere. This keeps it in a log you can read when something did not start.
  local launcher="$BIN_DIR/$APP_BIN-launch"
  cat >"$launcher" <<LAUNCHER
#!/usr/bin/env bash
# Desktop launcher for $APP_DISPLAY_NAME (installed by install.sh).
# Runs Keyhold detached from any terminal and keeps its output in a log.
set -e

# The log carries the URL, and the URL carries the session token. A default umask
# leaves it group- and world-readable, which is a live credential in a file
# anybody on the machine can open. Nothing here is anybody else's business.
umask 077

STATE_DIR="\${XDG_STATE_HOME:-\$HOME/.local/state}/$APP_BIN"
mkdir -p "\$STATE_DIR"
chmod 700 "\$STATE_DIR" 2>/dev/null || true

# One generation, then start clean: the log answers "why did nothing happen just
# now", and every run it keeps is another dead token sitting on the disk.
LOG="\$STATE_DIR/app.log"
[ -f "\$LOG" ] && mv -f "\$LOG" "\$LOG.1"

BIN="\$(command -v $APP_BIN 2>/dev/null || true)"
if [ -z "\$BIN" ] && [ -x "$BIN_DIR/$APP_BIN" ]; then
  BIN="$BIN_DIR/$APP_BIN"
fi

if [ -z "\$BIN" ]; then
  notify-send "$APP_DISPLAY_NAME" "Binary '$APP_BIN' not found on PATH." 2>/dev/null || true
  echo "$APP_BIN not found on PATH" >&2
  exit 127
fi

exec "\$BIN" "\$@" >"\$LOG" 2>&1
LAUNCHER
  chmod +x "$launcher"

  local desktop_file="$apps_dir/$APP_BIN.desktop"
  cat >"$desktop_file" <<DESKTOP
[Desktop Entry]
Type=Application
Version=1.0
Name=$APP_DISPLAY_NAME
GenericName=$APP_GENERIC_NAME
Comment=$APP_COMMENT
Exec=$launcher
Icon=$APP_BIN
Terminal=false
Categories=$APP_CATEGORIES
StartupNotify=true
Keywords=$APP_KEYWORDS
Actions=present;replace;

# Right-clicking the icon skips the question a plain click asks. The two answers
# are the two things somebody can mean by clicking it a second time.
[Desktop Action present]
Name=Open and continue
Name[pt_BR]=Abrir e continuar
Exec=$launcher --present

[Desktop Action replace]
Name=Close it and start over
Name[pt_BR]=Encerrar e começar de novo
Exec=$launcher --replace
DESKTOP
  chmod 644 "$desktop_file"

  # keyhold:// opens Keyhold, so a tab whose program was closed can offer "Abrir o
  # Keyhold" as a button. It only ever runs the launcher with --present (start, or
  # bring the running one forward): any page could link to it, and that is all a
  # page gets. The address is never passed on.
  local url_file="$apps_dir/$APP_BIN-url.desktop"
  cat >"$url_file" <<URLDESKTOP
[Desktop Entry]
Type=Application
Name=$APP_DISPLAY_NAME
Exec=$launcher --present
Icon=$APP_BIN
Terminal=false
NoDisplay=true
MimeType=x-scheme-handler/$APP_BIN;
URLDESKTOP
  chmod 644 "$url_file"
  has xdg-mime && xdg-mime default "$APP_BIN-url.desktop" "x-scheme-handler/$APP_BIN" >/dev/null 2>&1 || true

  # Logs written by an earlier version inherited the umask, and they hold the
  # session token of every run they saw.
  local state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/$APP_BIN"
  [ -d "$state_dir" ] && chmod 700 "$state_dir" 2>/dev/null || true
  for stale in "$state_dir/app.log" "$state_dir/app.log.1"; do
    [ -f "$stale" ] && chmod 600 "$stale" 2>/dev/null || true
  done

  has update-desktop-database && update-desktop-database "$apps_dir" >/dev/null 2>&1 || true
  has gtk-update-icon-cache &&
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true

  ok "shortcut installed — look for \"$APP_DISPLAY_NAME\" in your launcher"
}

check_path() {
  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
      warn "$BIN_DIR is not on your PATH"
      warn "add it with: echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc"
      ;;
  esac
}

# Only with --with=set-host. The name goes into /etc/hosts, which is root's,
# so this one step runs under sudo — and says so before the password prompt.
set_host() {
  [ -z "$SET_HOST" ] && return 0

  info "giving Keyhold the name $SET_HOST in /etc/hosts (sudo asks for your password)"

  if sudo "$BIN_DIR/$APP_BIN" set-host "$SET_HOST"; then
    ok "$SET_HOST reaches Keyhold from its next start (the address is printed above)"
  else
    warn "the name was not set; run later: sudo $BIN_DIR/$APP_BIN set-host $SET_HOST"
  fi
}

farewell() {
  printf "\n  %sDone.%s Run %s%s%s, or open %s%s%s from your launcher.\n" \
    "$BOLD" "$RESET" "$CYAN" "$APP_BIN" "$RESET" "$BOLD" "$APP_DISPLAY_NAME" "$RESET"
  printf "  %sLogs from the shortcut: ~/.local/state/%s/app.log%s\n\n" "$DIM" "$APP_BIN" "$RESET"
}

header
check_toolchain
build_frontend
build_binary
install_desktop_shortcut
check_path
set_host
farewell
