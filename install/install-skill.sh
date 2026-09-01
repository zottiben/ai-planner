#!/usr/bin/env sh
# Install the ai-planner agent skill so a harness knows the build plan lives in the
# database rather than in markdown files.
#
#   User-wide (default - every repo and every worktree inherits it):
#     curl -fsSL https://zottiben.github.io/ai-planner/install-skill.sh | sh
#
#   Into the current repo only:
#     curl -fsSL https://zottiben.github.io/ai-planner/install-skill.sh | sh -s -- --project
set -eu

SKILL_NAME="ai-planner"
RAW_SKILL="https://raw.githubusercontent.com/zottiben/ai-planner/main/skill/SKILL.md"

say() { printf '\033[1;34m==>\033[0m %s\n' "$1"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$1" >&2; exit 1; }

# User scope is the default here, unlike file-sql: a build plan is not a property of
# one repo, and the point is that no repo has to be set up for this to work.
scope="user"
for arg in "$@"; do
  case "$arg" in
    --project) scope="project" ;;
    --user) scope="user" ;;
    -h|--help) echo "usage: install-skill.sh [--project|--user]"; exit 0 ;;
    *) die "unknown option: $arg" ;;
  esac
done

src=""
for cand in "skill/SKILL.md" "$(dirname "$0")/../skill/SKILL.md"; do
  if [ -f "$cand" ]; then src="$cand"; break; fi
done
if [ -z "$src" ]; then
  command -v curl >/dev/null 2>&1 || die "curl is required to download the skill."
fi

# Claude Code reads .claude/skills; Codex, OpenCode and Pi read .agents/skills.
if [ "$scope" = "user" ]; then
  bases="$HOME/.claude/skills $HOME/.agents/skills"
else
  bases=".claude/skills .agents/skills"
fi

for base in $bases; do
  dest="$base/$SKILL_NAME"
  mkdir -p "$dest"
  if [ -n "$src" ]; then
    cp "$src" "$dest/SKILL.md"
  else
    curl -fsSL "$RAW_SKILL" -o "$dest/SKILL.md"
  fi
  say "Installed skill -> $dest/SKILL.md"
done

say "Done. Restart your agent so it picks up the '$SKILL_NAME' skill."
