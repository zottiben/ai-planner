#!/usr/bin/env bash
# SessionStart: tell the session which build plan this worktree is on.
#
# Cross-harness: resolves the project dir from the payload's `cwd` (Codex) or
# ${CLAUDE_PROJECT_DIR} (Claude Code), falling back to the current directory.
# Silent whenever there is nothing useful to say, so it never adds noise to a
# session that has no plan - and it never fails the session.
set -uo pipefail

HOOK_JSON=$(cat 2>/dev/null || true)

proj=""
if command -v jq >/dev/null 2>&1; then
  proj=$(printf '%s' "$HOOK_JSON" | jq -r '.cwd // empty' 2>/dev/null)
elif command -v python3 >/dev/null 2>&1; then
  proj=$(printf '%s' "$HOOK_JSON" | python3 -c 'import sys,json
try: print(json.load(sys.stdin).get("cwd",""))
except Exception: pass' 2>/dev/null)
fi
: "${proj:=${CLAUDE_PROJECT_DIR:-.}}"

command -v aip >/dev/null 2>&1 || exit 0
aip -C "$proj" hook 2>/dev/null || true
exit 0
