#!/usr/bin/env sh
# Install the ai-planner harness hooks, so a session is told about its build plan
# without being asked, and is told when the plan has fallen out of step with the work.
#
# Three events, because one is not enough:
#   SessionStart      - which plan this worktree is on, once at the start.
#   UserPromptSubmit  - the same one-liner on every turn. This is the one that stops
#                       the plan being forgotten between tasks: a new task arrives as
#                       a new prompt, and SessionStart is long out of context by then.
#   Stop              - at the end of a turn, only when the plan is demonstrably out
#                       of step. Deduplicated per state, so it cannot become noise.
#
# PreCompact and SessionEnd are deliberately not used: neither can inject context, so
# a hook there could only block, which is worse than saying nothing.
#
#   curl -fsSL https://zottiben.github.io/ai-planner/install-hook.sh | sh
#
# User scope by default (~/.claude/settings.json). Pass --project to wire it into the
# current repo's .claude/settings.json instead.
set -eu

say() { printf '\033[1;34m==>\033[0m %s\n' "$1"; }
warn() { printf '\033[1;33mnote:\033[0m %s\n' "$1"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$1" >&2; exit 1; }

scope="user"
for arg in "$@"; do
  case "$arg" in
    --project) scope="project" ;;
    --user) scope="user" ;;
    -h|--help) echo "usage: install-hook.sh [--project|--user]"; exit 0 ;;
    *) die "unknown option: $arg" ;;
  esac
done

command -v python3 >/dev/null 2>&1 || die "python3 is required to merge the settings file."
command -v aip >/dev/null 2>&1 || warn "aip is not on PATH yet - the hook stays silent until it is."

if [ "$scope" = "user" ]; then
  settings="$HOME/.claude/settings.json"
  hookdir="$HOME/.ai-planner/hooks"
else
  settings=".claude/settings.json"
  hookdir=".agents/hooks"
fi

# Ship the wrapper script alongside the settings entry so the command is stable.
mkdir -p "$hookdir"
src=""
for cand in "install/hooks/ai-planner-session.sh" "$(dirname "$0")/hooks/ai-planner-session.sh"; do
  if [ -f "$cand" ]; then src="$cand"; break; fi
done
if [ -n "$src" ]; then
  cp "$src" "$hookdir/ai-planner-session.sh"
else
  curl -fsSL \
    "https://raw.githubusercontent.com/zottiben/ai-planner/main/install/hooks/ai-planner-session.sh" \
    -o "$hookdir/ai-planner-session.sh"
fi
chmod +x "$hookdir/ai-planner-session.sh"
say "Installed hook -> $hookdir/ai-planner-session.sh"

mkdir -p "$(dirname "$settings")"
[ -f "$settings" ] || printf '{}\n' > "$settings"

# Merge rather than overwrite: these settings files already carry other hooks.
SETTINGS="$settings" HOOKCMD="$hookdir/ai-planner-session.sh" python3 <<'PY'
import json, os, sys

path = os.environ["SETTINGS"]
script = os.environ["HOOKCMD"]

try:
    with open(path) as f:
        data = json.load(f)
except (json.JSONDecodeError, FileNotFoundError):
    print(f"error: {path} is not valid JSON - not touching it.", file=sys.stderr)
    sys.exit(1)

events = {
    "SessionStart": f"{script} session-start",
    "UserPromptSubmit": f"{script} user-prompt-submit",
    "Stop": f"{script} stop",
}

hooks = data.setdefault("hooks", {})
added, kept = [], []
for event, cmd in events.items():
    groups = hooks.setdefault(event, [])
    # Match on the script, not the full command, so an older single-event install is
    # upgraded in place rather than left behind as a duplicate.
    existing = [
        h
        for group in groups
        if isinstance(group, dict)
        for h in group.get("hooks", [])
        if isinstance(h, dict) and script in str(h.get("command", ""))
    ]
    if existing:
        for h in existing:
            h["command"] = cmd
        kept.append(event)
        continue
    groups.append({"hooks": [{"type": "command", "command": cmd}]})
    added.append(event)

with open(path, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")

if added:
    print("registered: " + ", ".join(added))
if kept:
    print("already present (refreshed): " + ", ".join(kept))
PY

say "Wired into $settings"
say "Codex and Pi: run \`aip hook --event <session-start|user-prompt-submit|stop>\` from"
say "your own hooks - it prints the same JSON."
