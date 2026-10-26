#!/bin/sh
# Flow Keeper - Claude Code and Codex CLI hook.
# Records the event in the local spool and exits right away. It must never block
# or fail the agent: every error ends with exit 0 and no output.
#
#   capture.sh <Event> [claude|codex]

drain() { cat >/dev/null 2>&1; exit 0; }

# The tracker's internal calls (e.g. recaps with `claude -p`) must not be tracked.
[ -n "$FLOW_KEEPER_INTERNAL" ] && drain

event="${1:-unknown}"
case "$event" in *[!A-Za-z]*) drain ;; esac
tool="${2:-claude}"
case "$tool" in claude|codex) ;; *) drain ;; esac

spool="${FLOW_KEEPER_SPOOL:-${FLOW_KEEPER_HOME:-$HOME/Library/Application Support/Flow Keeper}/spool}"
umask 077
mkdir -p "$spool/.tmp" 2>/dev/null || drain

name="$(date +%s)-$$-$tool-$event"
tmp="$spool/.tmp/$name"

host=""
if [ "$tool" = codex ]; then
  # Codex doesn't expose its own PID: walk up the ancestor processes to the
  # first `codex`. If it's an app-server we flag it: the TUI can use one too;
  # the normalizer tells CLI, app and subagents apart from the rollout.
  pid=""
  p="$PPID"
  i=0
  while [ "$i" -lt 4 ] && [ -n "$p" ] && [ "$p" -gt 1 ] 2>/dev/null; do
    line=$(ps -o ppid=,comm= -p "$p" 2>/dev/null) || break
    read -r pp comm <<EOF
$line
EOF
    if [ "${comm##*/}" = codex ]; then
      pid="$p"
      case "$(ps -o args= -p "$p" 2>/dev/null)" in *" app-server"*) host="app-server" ;; esac
      break
    fi
    p="$pp"
    i=$((i + 1))
  done
else
  pid="${CLAUDE_PID:-$PPID}"
fi

# The terminal is needed to bring the session back to the front: we read it
# only on events that may indicate a new instance.
tty=""
if [ -n "$pid" ]; then
  case "$event" in
    SessionStart|UserPromptSubmit) tty=$(ps -o tty= -p "$pid" 2>/dev/null | tr -d ' ') ;;
  esac
fi

{
  printf 'flow-keeper-spool 1\n'
  printf 'TOOL=%s\n' "$tool"
  printf 'EVENT=%s\n' "$event"
  [ -n "$pid" ] && printf 'PID=%s\n' "$pid"
  [ -n "$tty" ] && printf 'TTY=%s\n' "$tty"
  [ -n "$host" ] && printf 'CODEX_HOST=%s\n' "$host"
  for var in CLAUDE_CODE_ENTRYPOINT CLAUDE_CONFIG_DIR CODEX_HOME TERM_PROGRAM TERM_PROGRAM_VERSION \
             __CFBundleIdentifier ITERM_SESSION_ID TERM_SESSION_ID TMUX TMUX_PANE FLOW_KEEPER_PROBE; do
    eval "val=\${$var:-}"
    [ -n "$val" ] && printf '%s=%s\n' "$var" "$(printf '%s' "$val" | tr -d '\n\r')"
  done
  printf '\n'
  # Only the start of the payload: it holds identity and useful fields, while
  # tool output (potentially large or sensitive) stays out.
  head -c 8192
  cat >/dev/null
} > "$tmp" 2>/dev/null && mv -f "$tmp" "$spool/$name.evt" 2>/dev/null

rm -f "$tmp" 2>/dev/null
exit 0
