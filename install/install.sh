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

# Semantic search is opt-in: it pulls in an ONNX runtime and downloads a model on
# first use, and lexical search answers most questions without either.
features=""
for arg in "$@"; do
  case "$arg" in
    --with-model) features="--features model-embeddings" ;;
    -h|--help) echo "usage: install.sh [--with-model]"; exit 0 ;;
  esac
done

say "Installing the aip binary${features:+ (with the local embedding model)}"
if [ -n "$here" ] && [ -f "$here/Cargo.toml" ]; then
  # shellcheck disable=SC2086
  cargo install --path "$here/crates/ai-planner" --locked $features
else
  # Behind a TLS-intercepting proxy, tell cargo to use the git CLI so it trusts the
  # system cert store:  export CARGO_NET_GIT_FETCH_WITH_CLI=true
  # shellcheck disable=SC2086
  cargo install --git "$REPO" ai-planner --locked $features
fi
ok "aip installed"

if [ -n "$here" ] && [ -f "$here/install/install-skill.sh" ]; then
  sh "$here/install/install-skill.sh"
  sh "$here/install/install-hook.sh"
  sh "$here/install/install-mcp.sh"
  say "Adding the always-on rules to your global charter"
  aip rules install 2>/dev/null || "$HOME/.cargo/bin/aip" rules install
else
  curl -fsSL "https://zottiben.github.io/ai-planner/install-skill.sh" | sh
  curl -fsSL "https://zottiben.github.io/ai-planner/install-hook.sh" | sh
  curl -fsSL "https://zottiben.github.io/ai-planner/install-mcp.sh" | sh
  say "Adding the always-on rules to your global charter"
  aip rules install 2>/dev/null || "$HOME/.cargo/bin/aip" rules install
fi

cat <<'EOF'

Done. Next:
  cd <your repo> && aip init         # register it (once, from any worktree)
  aip import --scan <worktree root>  # bring existing BUILD_PLAN / HANDOFF files in
  aip status                         # where you are
  aip doctor                         # check the setup

Installed with --with-model? Build the semantic index once:
  aip embed

The database is one file for every repo: ~/.ai-planner/planner.db
Open it in TablePlus with `aip db open`.
EOF
