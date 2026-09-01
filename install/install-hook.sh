#!/usr/bin/env sh
# Install the ai-planner session-start hook, so every new session is told which build
# plan its worktree is on without being asked.
#
# This is the piece that makes the tool seamless: a skill only fires once the agent
# already suspects it needs one, but the hook arrives unprompted.
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
cmd = os.environ["HOOKCMD"]

try:
    with open(path) as f:
        data = json.load(f)
except (json.JSONDecodeError, FileNotFoundError):
    print(f"error: {path} is not valid JSON - not touching it.", file=sys.stderr)
    sys.exit(1)

hooks = data.setdefault("hooks", {})
starts = hooks.setdefault("SessionStart", [])

already = any(
    h.get("command") == cmd
    for group in starts
    if isinstance(group, dict)
    for h in group.get("hooks", [])
    if isinstance(h, dict)
)
if already:
    print("hook already registered")
    sys.exit(0)

starts.append({"hooks": [{"type": "command", "command": cmd}]})
with open(path, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
print("registered")
PY

say "Wired into $settings"
say "Codex and Pi: run \`aip hook\` from your own session-start hook - it prints the same JSON."
