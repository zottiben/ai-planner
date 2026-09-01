#!/usr/bin/env sh
# Register the ai-planner MCP server with the harnesses on this machine.
#
#   curl -fsSL https://zottiben.github.io/ai-planner/install-mcp.sh | sh
#
# User scope by default, so every repo and every worktree inherits it. Pass --project
# to write into the current repo instead.
#
# The CLI works on its own - the MCP server is for harnesses that prefer structured
# tool calls over shelling out.
set -eu

say()  { printf '\033[1;34m==>\033[0m %s\n' "$1"; }
warn() { printf '\033[1;33mnote:\033[0m %s\n' "$1"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$1" >&2; exit 1; }

scope="user"
for arg in "$@"; do
  case "$arg" in
    --project) scope="project" ;;
    --user) scope="user" ;;
    -h|--help) echo "usage: install-mcp.sh [--project|--user]"; exit 0 ;;
    *) die "unknown option: $arg" ;;
  esac
done

command -v python3 >/dev/null 2>&1 || die "python3 is required to merge the config files."
command -v aip >/dev/null 2>&1 || warn "aip is not on PATH yet - install it first."

# Claude Code keeps its own registry; add there when the CLI is available.
if command -v claude >/dev/null 2>&1; then
  if [ "$scope" = "user" ]; then
    claude mcp add ai-planner --scope user -- aip serve >/dev/null 2>&1 \
      && say "Registered with Claude Code (user scope)" \
      || warn "claude mcp add failed - add it by hand: claude mcp add ai-planner -- aip serve"
  else
    claude mcp add ai-planner -- aip serve >/dev/null 2>&1 \
      && say "Registered with Claude Code (this project)" \
      || warn "claude mcp add failed - add it by hand: claude mcp add ai-planner -- aip serve"
  fi
else
  warn "claude CLI not found - skipping Claude Code"
fi

# Codex reads TOML; Pi and the .mcp.json convention read JSON.
if [ "$scope" = "user" ]; then
  codex_cfg="$HOME/.codex/config.toml"
  pi_cfg="$HOME/.pi/mcp.json"
  mcp_json=""
else
  codex_cfg=".codex/config.toml"
  pi_cfg=".pi/mcp.json"
  mcp_json=".mcp.json"
fi

mkdir -p "$(dirname "$codex_cfg")"
[ -f "$codex_cfg" ] || : > "$codex_cfg"
if grep -q '^\[mcp_servers.ai-planner\]' "$codex_cfg" 2>/dev/null; then
  say "Codex already has it ($codex_cfg)"
else
  # Appended rather than rewritten: this file usually holds other servers.
  printf '\n[mcp_servers.ai-planner]\ncommand = "aip"\nargs = ["serve"]\n' >> "$codex_cfg"
  say "Registered with Codex ($codex_cfg)"
fi

for target in "$pi_cfg" $mcp_json; do
  mkdir -p "$(dirname "$target")"
  [ -f "$target" ] || printf '{}\n' > "$target"
  TARGET="$target" python3 <<'PY'
import json, os, sys

path = os.environ["TARGET"]
try:
    with open(path) as f:
        data = json.load(f)
except (json.JSONDecodeError, FileNotFoundError):
    print(f"error: {path} is not valid JSON - not touching it.", file=sys.stderr)
    sys.exit(1)

servers = data.setdefault("mcpServers", {})
if "ai-planner" in servers:
    print(f"already registered in {path}")
    sys.exit(0)

servers["ai-planner"] = {
    "command": "aip",
    "args": ["serve"],
    "transport": "stdio",
    "lifecycle": "eager",
}
with open(path, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
print(f"registered in {path}")
PY
done

say "Done. Restart your agent so it picks the server up."
