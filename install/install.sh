#!/usr/bin/env sh
# Install ai-planner: the `aip` binary, the agent skill, and the session-start hook.
#
#   curl -fsSL https://zottiben.github.io/ai-planner/install.sh | sh
#
# Then, in each repo:  aip init
set -eu

REPO="https://github.com/zottiben/ai-planner"

say() { printf '\033[1;34m==>\033[0m %s\n' "$1"; }
ok()  { printf '\033[32m✓\033[0m %s\n' "$1"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$1" >&2; exit 1; }

command -v cargo >/dev/null 2>&1 || die "cargo is required - install Rust from https://rustup.rs"

here=$(CDPATH= cd -- "$(dirname -- "$0")/.." 2>/dev/null && pwd || true)

say "Installing the aip binary"
if [ -n "$here" ] && [ -f "$here/Cargo.toml" ]; then
  cargo install --path "$here/crates/ai-planner" --locked
else
  # Behind a TLS-intercepting proxy, tell cargo to use the git CLI so it trusts the
  # system cert store.
  cargo install --git "$REPO" ai-planner --locked
fi
ok "aip installed"

if [ -n "$here" ] && [ -f "$here/install/install-skill.sh" ]; then
  sh "$here/install/install-skill.sh"
  sh "$here/install/install-hook.sh"
else
  curl -fsSL "https://zottiben.github.io/ai-planner/install-skill.sh" | sh
  curl -fsSL "https://zottiben.github.io/ai-planner/install-hook.sh" | sh
fi

cat <<'EOF'

Done. Next:
  cd <your repo> && aip init         # register it (once, from any worktree)
  aip import --scan <worktree root>  # bring existing BUILD_PLAN / HANDOFF files in
  aip status                         # where you are
  aip doctor                         # check the setup

The database is one file for every repo: ~/.ai-planner/planner.db
Open it in TablePlus with `aip db open`.
EOF
