#!/usr/bin/env sh
# Install ai-planner: the `aip` binary, the desktop board on macOS, the agent skill,
# and the session-start hook.
#
#   curl -fsSL https://zottiben.github.io/ai-planner/install.sh | sh
#
# Then, in each repo:  aip init
set -eu

REPO="zottiben/ai-planner"
REPO_URL="https://github.com/${REPO}"

say()  { printf '\033[1;34m==>\033[0m %s\n' "$1"; }
ok()   { printf '\033[32m✓\033[0m %s\n' "$1"; }
warn() { printf '\033[33m!\033[0m %s\n' "$1" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$1" >&2; exit 1; }

# A script in a clone can use its sibling files. A script piped to `sh` cannot:
# there `$0` is just "sh", and treating the current directory as its source tree can
# make an unrelated Cargo.toml win by accident.
here=""
# shellcheck disable=SC1007 # CDPATH is intentionally empty for this one command.
case "$0" in
  */*) here=$(CDPATH= cd -- "$(dirname -- "$0")/.." 2>/dev/null && pwd || true) ;;
esac

# Semantic search is opt-in: it pulls in an ONNX runtime and downloads a model on
# first use, and lexical search answers most questions without either. It is the one
# thing a prebuilt binary cannot give you, because the feature is compiled in - so
# asking for it means building from source.
features=""
from_source=no
for arg in "$@"; do
  case "$arg" in
    --with-model) features="--features model-embeddings"; from_source=yes ;;
    --from-source) from_source=yes ;;
    -h|--help)
      echo "usage: install.sh [--with-model] [--from-source]"
      echo "  --with-model   compile in local semantic search (implies --from-source)"
      echo "  --from-source  build with cargo instead of downloading a release"
      exit 0 ;;
    *) die "unknown argument: $arg" ;;
  esac
done

# Pick a binary directory already on PATH, without sudo when possible.
if echo "$PATH" | tr ':' '\n' | grep -qx "$HOME/.local/bin"; then
  BIN_DIR="$HOME/.local/bin"
elif echo "$PATH" | tr ':' '\n' | grep -qx "$HOME/.cargo/bin"; then
  BIN_DIR="$HOME/.cargo/bin"
else
  BIN_DIR="/usr/local/bin"
fi

# --- prebuilt release -----------------------------------------------------------
#
# Preferred, because it needs no Rust toolchain and takes seconds. The board is
# compiled into the binary either way, so a downloaded `aip` has the full UI.
install_release() {
  command -v curl >/dev/null 2>&1 || return 1
  command -v tar >/dev/null 2>&1 || return 1

  os=$(uname -s | tr '[:upper:]' '[:lower:]')
  arch=$(uname -m)
  case "$os" in darwin|linux) ;; *) return 1 ;; esac

  version=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
    | grep '"tag_name"' | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
  [ -n "$version" ] || return 1
  num="${version#v}"
  base="${REPO_URL}/releases/download/${version}"

  if [ "$os" = "darwin" ]; then
    file="ai-planner-v${num}-macos-universal.tar.gz"
  else
    case "$arch" in
      x86_64|amd64)  larch=x86_64 ;;
      arm64|aarch64) larch=aarch64 ;;
      *) return 1 ;;
    esac
    file="ai-planner-v${num}-linux-${larch}.tar.gz"
  fi

  tmp=$(mktemp -d) || return 1
  trap 'rm -rf "$tmp"' EXIT HUP INT TERM

  say "Downloading ai-planner ${version}"
  curl -fsSL "${base}/${file}" -o "${tmp}/${file}" || return 1

  # Best effort: only when checksums are published and a hasher exists.
  if curl -fsSL "${base}/checksums.txt" -o "${tmp}/checksums.txt" 2>/dev/null; then
    expected=$(grep " ${file}\$" "${tmp}/checksums.txt" | awk '{print $1}')
    if [ -n "$expected" ]; then
      if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "${tmp}/${file}" | awk '{print $1}')
      elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "${tmp}/${file}" | awk '{print $1}')
      else
        actual=""
      fi
      [ -z "$actual" ] || [ "$actual" = "$expected" ] \
        || die "checksum mismatch for ${file}"
    fi
  fi

  tar xzf "${tmp}/${file}" -C "$tmp" || return 1

  mkdir -p "$BIN_DIR" 2>/dev/null || true
  if [ -w "$BIN_DIR" ]; then
    install -m 0755 "${tmp}/aip" "${BIN_DIR}/aip"
  else
    sudo install -m 0755 "${tmp}/aip" "${BIN_DIR}/aip"
  fi
  ok "aip installed to ${BIN_DIR}/aip"
  INSTALLED_AIP="${BIN_DIR}/aip"

  # This marker deliberately outranks stale ~/.cargo install metadata. Without it,
  # replacing a cargo-installed binary with a release could make `aip update` rebuild
  # an old clone and silently downgrade the user.
  mkdir -p "$HOME/.ai-planner"
  printf 'release\n' > "$HOME/.ai-planner/install-method"

  # The desktop board, when the archive carries one. `aip ui` works regardless; this
  # is for people who would rather have it in the Dock.
  if [ -d "${tmp}/ai-planner.app" ]; then
    rm -rf "/Applications/ai-planner.app" 2>/dev/null || true
    if cp -R "${tmp}/ai-planner.app" /Applications/ 2>/dev/null; then
      ok "ai-planner.app installed to /Applications"
    else
      warn "could not write /Applications - run 'aip ui' in a browser instead"
    fi
  fi
  return 0
}

install_from_source() {
  command -v cargo >/dev/null 2>&1 \
    || die "no release for this platform and cargo is not installed - get Rust from https://rustup.rs"

  say "Building aip${features:+ (with the local embedding model)}"
  if [ -n "$here" ] && [ -f "$here/Cargo.toml" ]; then
    # shellcheck disable=SC2086
    cargo install --path "$here/crates/ai-planner" --locked $features
  else
    # Behind a TLS-intercepting proxy, tell cargo to use the git CLI so it trusts the
    # system cert store:  export CARGO_NET_GIT_FETCH_WITH_CLI=true
    # shellcheck disable=SC2086
    cargo install --git "$REPO_URL" ai-planner --locked $features
  fi
  ok "aip installed"
  INSTALLED_AIP=$(command -v aip 2>/dev/null || printf '%s' "$HOME/.cargo/bin/aip")

  # A source install supersedes release provenance. Leaving the marker behind would
  # make `aip update` replace this feature-selected build with a stock release.
  rm -f "$HOME/.ai-planner/install-method"
}

if [ "$from_source" = yes ]; then
  install_from_source
elif install_release; then
  :
else
  warn "no prebuilt release for this platform - building from source"
  install_from_source
fi

# The binary carries the skill, the rules and the hook script, so it installs its own
# setup. That is the same code path `aip update` runs, which is what keeps the two from
# drifting apart.
AIP=${INSTALLED_AIP:-$(command -v aip 2>/dev/null || printf '%s' "$HOME/.cargo/bin/aip")}
say "Installing the skill, the always-on rules and the harness hooks"
"$AIP" setup --force

# MCP registration edits a TOML file and talks to the claude CLI, so it stays in shell.
if [ -n "$here" ] && [ -f "$here/install/install-mcp.sh" ]; then
  sh "$here/install/install-mcp.sh"
else
  curl -fsSL "https://zottiben.github.io/ai-planner/install-mcp.sh" | sh
fi

cat <<'EOF'

Done. Next:
  cd <your repo> && aip init         # register it (once, from any worktree)
  aip import --scan <worktree root>  # bring existing BUILD_PLAN / HANDOFF files in
  aip status                         # where you are
  aip doctor                         # check the setup

Later:
  aip update                         # update by the same route it was installed, then
                                     # refresh the skill, rules and hooks

See it as a board:
  aip ui                             # opens a browser; no dev server, nothing to start

Installed with --with-model? Build the semantic index once:
  aip embed

The database is one file for every repo: ~/.ai-planner/planner.db
Open it in TablePlus with `aip db open`.
EOF
