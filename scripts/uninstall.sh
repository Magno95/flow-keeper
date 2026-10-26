#!/bin/bash
# Uninstalls Flow Keeper. Removes only what the installation added and is
# still recognizable as its own; data stays, unless --purge.
#
#   ./scripts/uninstall.sh [--purge]
#
#   --purge   also deletes collected conversations, settings and the API key
#             stored in the keychain

set -euo pipefail

APP_NAME="Flow Keeper"
BUNDLE_ID="io.github.magno95.flowkeeper"
DATA_DIR="$HOME/Library/Application Support/Flow Keeper"
MANIFEST="$DATA_DIR/install-manifest.json"
LOCAL_PLUGIN="flow-keeper@flow-keeper-app"
LOCAL_MARKETPLACE="flow-keeper-app"
# App name up to 0.4: whatever is left of it is removed too.
OLD_APP_NAME="AI Tracker"
OLD_BUNDLE_ID="com.davidemagnoni.aitracker"
OLD_DATA_DIR="$HOME/Library/Application Support/AI Tracker"
OLD_LOCAL_PLUGIN="ai-tracker@ai-tracker-app"
OLD_LOCAL_MARKETPLACE="ai-tracker-app"

purge=0
[ "${1:-}" = "--purge" ] && purge=1

say() { printf '\033[1m›\033[0m %s\n' "$*"; }
warn() { printf '\033[33m!\033[0m %s\n' "$*" >&2; }

json_field() { [ -f "$MANIFEST" ] && sed -n "s/.*\"$1\": *\"\{0,1\}\([^\",]*\)\"\{0,1\}.*/\1/p" "$MANIFEST" | head -1; }

app_path="$(json_field appPath)"
[ -n "$app_path" ] || app_path="$HOME/Applications/$APP_NAME.app"

for pair in "flow-keeper:$APP_NAME" "ai-tracker:$OLD_APP_NAME"; do
  proc="${pair%%:*}" name="${pair#*:}"
  if pgrep -x "$proc" >/dev/null; then
    say "Quitting $name…"
    osascript -e "tell application \"$name\" to quit" >/dev/null 2>&1 || true
    sleep 1
    pkill -x "$proc" 2>/dev/null || true
  fi
done

# Launch at login: only the agent that points to our app.
for name in "$APP_NAME" "$OLD_APP_NAME"; do
  for plist in "$HOME/Library/LaunchAgents/$name.plist" "$HOME/Library/LaunchAgents/$BUNDLE_ID.plist" "$HOME/Library/LaunchAgents/$OLD_BUNDLE_ID.plist"; do
    if [ -f "$plist" ] && grep -q "$name.app" "$plist"; then
      launchctl unload "$plist" >/dev/null 2>&1 || true
      rm -f "$plist"
      say "Removed launch at login."
    fi
  done
done

for path in "$app_path" "$(dirname "$app_path")/$OLD_APP_NAME.app"; do
  id="$(defaults read "$path/Contents/Info" CFBundleIdentifier 2>/dev/null || true)"
  if [ -d "$path" ] && { [ "$id" = "$BUNDLE_ID" ] || [ "$id" = "$OLD_BUNDLE_ID" ]; }; then
    rm -rf "$path"
    say "Removed the app from $path"
  elif [ -d "$path" ]; then
    warn "$path doesn't look like Flow Keeper: left as is."
  fi
done

# Only what the app added: local plugins and marketplaces. A manual
# installation from the GitHub marketplace, if any, is left to the user.
if command -v claude >/dev/null; then
  for pair in "$LOCAL_PLUGIN:$LOCAL_MARKETPLACE" "$OLD_LOCAL_PLUGIN:$OLD_LOCAL_MARKETPLACE"; do
    plugin="${pair%%:*}" marketplace="${pair#*:}"
    if claude plugin list --json 2>/dev/null | grep -q "\"$plugin\""; then
      claude plugin uninstall "$plugin" >/dev/null && say "Disconnected Claude Code."
    fi
    if claude plugin marketplace list --json 2>/dev/null | grep -q "\"$marketplace\""; then
      claude plugin marketplace remove "$marketplace" >/dev/null || true
    fi
  done
  if claude plugin list --json 2>/dev/null | grep -q '"flow-keeper@flow-keeper"'; then
    warn "The plugin installed by hand from GitHub is still present: remove it with claude plugin uninstall flow-keeper@flow-keeper"
  fi
fi

if command -v codex >/dev/null; then
  for pair in "$LOCAL_PLUGIN:$LOCAL_MARKETPLACE" "$OLD_LOCAL_PLUGIN:$OLD_LOCAL_MARKETPLACE"; do
    plugin="${pair%%:*}" marketplace="${pair#*:}"
    if codex plugin list --json 2>/dev/null | grep -q "\"$plugin\""; then
      FLOW_KEEPER_INTERNAL=1 AI_TRACKER_INTERNAL=1 codex plugin remove "$plugin" >/dev/null && say "Disconnected Codex."
    fi
    if codex plugin marketplace list --json 2>/dev/null | grep -q "\"$marketplace\""; then
      FLOW_KEEPER_INTERNAL=1 AI_TRACKER_INTERNAL=1 codex plugin marketplace remove "$marketplace" >/dev/null || true
    fi
  done
fi

if [ "$purge" = 1 ]; then
  for name in "$APP_NAME" "$OLD_APP_NAME"; do
    security delete-generic-password -s "$name" -a anthropic-api-key >/dev/null 2>&1 && say "Removed the API key from the keychain." || true
  done
  rm -rf "$DATA_DIR" "$OLD_DATA_DIR"
  say "Deleted the collected data."
else
  rm -f "$MANIFEST"
  # The old path is just a symlink to the new one.
  [ -L "$OLD_DATA_DIR" ] && rm -f "$OLD_DATA_DIR"
  say "Data kept in \"$DATA_DIR\" (use --purge to delete it)."
fi
