#!/bin/bash
# Installs Flow Keeper on macOS straight from GitHub.
#
#   gh api -H "Accept: application/vnd.github.raw" \
#     repos/Magno95/flow-keeper/contents/scripts/install.sh | bash
#
# or from a clone: ./scripts/install.sh [options]
#
# Why a signed app isn't needed: macOS blocks apps downloaded from a browser
# because the download adds the com.apple.quarantine attribute. `gh` and `git`
# don't add it, and the app is signed locally (ad hoc) after installation,
# as happens with any locally built program.
#
# Options:
#   --from-source    build from source (requires Rust and Node.js)
#   --version vX.Y.Z install a specific release (default: the latest)
#   --no-open        don't launch the app when done
#   --bundle PATH    install an already downloaded and verified bundle (used
#                    by the app to update itself)
#   --wait-pid PID   first wait for process PID to exit

set -euo pipefail

REPO="${FLOW_KEEPER_REPO:-Magno95/flow-keeper}"
APP_NAME="Flow Keeper"
DEST_DIR="${FLOW_KEEPER_APP_DIR:-$HOME/Applications}"
DATA_DIR="$HOME/Library/Application Support/Flow Keeper"
MANIFEST="$DATA_DIR/install-manifest.json"
# App name up to 0.4: its bundle and data folder get replaced.
OLD_APP_NAME="AI Tracker"
OLD_BUNDLE_ID="com.davidemagnoni.aitracker"
OLD_DATA_DIR="$HOME/Library/Application Support/AI Tracker"

mode="auto"
version=""
open_app=1
bundle=""
wait_pid=""
while [ $# -gt 0 ]; do
  case "$1" in
    --from-source) mode="source" ;;
    --version) version="$2"; mode="release"; shift ;;
    --no-open) open_app=0 ;;
    --bundle) bundle="$2"; mode="bundle"; shift ;;
    --wait-pid) wait_pid="$2"; shift ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

say() { printf '\033[1m›\033[0m %s\n' "$*"; }
warn() { printf '\033[33m!\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = "Darwin" ] || die "Flow Keeper currently supports macOS only."

work="$(mktemp -d)"
target=""
# Update from the app: if something goes wrong, reopen whichever version is left.
cleanup() {
  local rc=$?
  rm -rf "$work"
  if [ "$rc" -ne 0 ] && [ -n "$wait_pid" ]; then
    if [ -n "$target" ] && [ -d "$target" ]; then
      open "$target" || true
    elif [ -n "$bundle" ] && [ -d "$bundle" ]; then
      open "$bundle" || true
    fi
  fi
}
trap cleanup EXIT

script_dir=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi
repo_root=""
if [ -n "$script_dir" ] && [ -f "$script_dir/../app/src-tauri/tauri.conf.json" ]; then
  repo_root="$(cd "$script_dir/.." && pwd)"
fi

need_gh() {
  command -v gh >/dev/null || die "GitHub CLI is required: brew install gh && gh auth login"
  gh auth status >/dev/null 2>&1 || die "Sign in to GitHub with: gh auth login"
}

# ---- 1. Get the app ---------------------------------------------------------

app_src=""

download_release() {
  need_gh
  local tag_args=()
  [ -n "$version" ] && tag_args=("$version")
  say "Downloading the ${version:-latest} release from ${REPO}…"
  if ! gh release download ${tag_args[@]+"${tag_args[@]}"} --repo "$REPO" --pattern "*.app.tar.gz" --dir "$work" 2>"$work/gh.err"; then
    [ "$mode" = "release" ] && die "Download failed: $(cat "$work/gh.err")"
    warn "No release available: building from source."
    return 1
  fi
  tar -xzf "$work"/*.app.tar.gz -C "$work"
  # Transition releases contain the bundle under the old name, because 0.4
  # can only update that way.
  app_src="$work/$APP_NAME.app"
  [ -d "$app_src" ] || app_src="$work/$OLD_APP_NAME.app"
  [ -d "$app_src" ] || die "Invalid release archive."
}

build_from_source() {
  command -v cargo >/dev/null || die "Building requires Rust: https://rustup.rs"
  command -v npm >/dev/null || die "Building requires Node.js: brew install node"
  local root="$repo_root"
  if [ -z "$root" ]; then
    need_gh
    say "Cloning ${REPO}…"
    gh repo clone "$REPO" "$work/src" -- --depth 1 >/dev/null
    root="$work/src"
  fi
  say "Building the app (a few minutes the first time)…"
  (cd "$root/app" && npm ci --silent && npx tauri build --bundles app) >"$work/build.log" 2>&1 || {
    tail -30 "$work/build.log" >&2
    die "Build failed."
  }
  app_src="$root/target/release/bundle/macos/$APP_NAME.app"
  [ -d "$app_src" ] || die "Bundle not found after the build."
}

case "$mode" in
  source) build_from_source ;;
  release) download_release ;;
  bundle) app_src="$bundle"; [ -d "$app_src" ] || die "Bundle not found: $app_src" ;;
  auto) download_release || build_from_source ;;
esac

# ---- 2. Install the app -------------------------------------------------------

mkdir -p "$DEST_DIR"
target="$DEST_DIR/$APP_NAME.app"

if [ -n "$wait_pid" ]; then
  for _ in $(seq 1 100); do
    kill -0 "$wait_pid" 2>/dev/null || break
    sleep 0.2
  done
fi

for pair in "flow-keeper:$APP_NAME" "ai-tracker:$OLD_APP_NAME"; do
  proc="${pair%%:*}" name="${pair#*:}"
  if pgrep -x "$proc" >/dev/null; then
    say "Quitting the running version…"
    osascript -e "tell application \"$name\" to quit" >/dev/null 2>&1 || true
    sleep 1
    pkill -x "$proc" 2>/dev/null || true
  fi
done

# Copy alongside, then swap: if the copy fails, the installed version stays.
staged="$DEST_DIR/.$APP_NAME.app.new"
rm -rf "$staged"
ditto "$app_src" "$staged"
rm -rf "$target"
mv "$staged" "$target"
# Defensive: no quarantine attribute, valid ad hoc signature on this Mac.
xattr -dr com.apple.quarantine "$target" 2>/dev/null || true
codesign --force --deep --sign - "$target" >/dev/null 2>&1 || warn "Ad hoc signing failed."
say "App installed in $target"

# The app under the old name (including the one the update was started from).
old_bundle="$DEST_DIR/$OLD_APP_NAME.app"
if [ -d "$old_bundle" ]; then
  old_id="$(defaults read "$old_bundle/Contents/Info" CFBundleIdentifier 2>/dev/null || true)"
  if [ "$old_id" = "$OLD_BUNDLE_ID" ] || [ "$old_id" = "$(defaults read "$target/Contents/Info" CFBundleIdentifier 2>/dev/null)" ]; then
    rm -rf "$old_bundle"
    say "Removed the previous version ($OLD_APP_NAME)."
  fi
fi

# ---- 3. Claude Code and Codex -----------------------------------------------
# The app does the connecting (“Connect your agents”): it registers the plugin
# shipped in the bundle with Claude Code and Codex, from a local folder. No
# GitHub access.

# ---- 4. Manifest for a clean uninstall --------------------------------------

# AI Tracker's data folder: move it here as the app would, leaving a symlink
# for the hooks of sessions already open.
if [ -d "$OLD_DATA_DIR" ] && [ ! -L "$OLD_DATA_DIR" ] && [ ! -e "$DATA_DIR" ]; then
  mv "$OLD_DATA_DIR" "$DATA_DIR" && ln -s "$DATA_DIR" "$OLD_DATA_DIR"
fi
mkdir -p "$DATA_DIR"
chmod 700 "$DATA_DIR"
app_version="$(defaults read "$target/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo unknown)"
cat >"$MANIFEST" <<EOF
{
  "version": "$app_version",
  "installedAt": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "appPath": "$target"
}
EOF
chmod 600 "$MANIFEST"

if [ "$open_app" = 1 ]; then
  open "$target"
fi
say "Done. FloK appears on your desktop: with “Connect your agents” the app connects to Claude Code and Codex on its own."
