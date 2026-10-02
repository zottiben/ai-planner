#!/bin/sh
# Embedded in ai-planner-update. Unlike the public first-install script this has
# explicit destinations, never falls back to Cargo and never prompts for sudo.
set -eu
version=${1:?release version required}
cli=${2-}
app=${3-}
cli_version=${4-}
old_app_version=${5-}
[ -n "$cli$app" ] || { echo 'no installation target selected' >&2; exit 1; }

fail() { echo "error: $*" >&2; exit 1; }
case $(uname -s) in
  Darwin) platform=macos-universal ;;
  Linux)
    [ -z "$app" ] || fail 'desktop updates on Linux use the release download'
    case $(uname -m) in
      x86_64) platform=linux-x86_64 ;;
      aarch64|arm64) platform=linux-aarch64 ;;
      *) fail 'unsupported architecture' ;;
    esac ;;
  *) fail 'install this platform from the releases page' ;;
esac

scratch=$(mktemp -d)
cli_stage=''
app_stage=''
cli_replaced=no
cli_lock=''
app_lock=''
provenance_lock=''
committed=no
provenance_changed=no
keep_scratch=no
cleanup() {
  set +e
  if [ "$committed" != yes ]; then
    # Do not delete the recovery copy if restoring it fails.
    if [ "$cli_replaced" = yes ]; then
      mv -f "$cli_stage/previous" "$cli" || { echo "restore CLI from $cli_stage/previous" >&2; cli_stage=''; }
    fi
    if [ "$provenance_changed" = yes ]; then
      for name in install-method install-path; do
        if [ -f "$scratch/old-$name" ]; then
          cp -p "$scratch/old-$name" "$HOME/.ai-planner/$name" || keep_scratch=yes
        else
          rm -f "$HOME/.ai-planner/$name" || keep_scratch=yes
        fi
      done
    fi
    if [ -n "$app_stage" ] && [ -d "$app_stage/previous.app" ]; then
      if ! { rm -rf "$app" && mv "$app_stage/previous.app" "$app"; }; then
        echo "restore app from $app_stage/previous.app" >&2
        app_stage=''
      fi
    fi
  fi
  [ -z "$cli_stage" ] || rm -rf "$cli_stage"
  [ -z "$app_stage" ] || rm -rf "$app_stage"
  [ -z "$cli_lock" ] || rmdir "$cli_lock"
  [ -z "$app_lock" ] || rmdir "$app_lock"
  [ -z "$provenance_lock" ] || rmdir "$provenance_lock"
  if [ "$keep_scratch" = yes ]; then
    echo "restore installation provenance from $scratch/old-*" >&2
  else
    rm -rf "$scratch"
  fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
if [ -n "$cli" ]; then
  mkdir "$cli.update-lock" || fail "could not lock $cli: another update holds the lock or its directory is not writable"
  cli_lock="$cli.update-lock"
fi
if [ -n "$app" ]; then
  mkdir "$app.update-lock" || fail "could not lock $app: another update holds the lock or its directory is not writable"
  app_lock="$app.update-lock"
fi

check_targets() {
  if [ -n "$cli" ]; then
    [ -n "$cli_version" ] && [ "$("$cli" --version)" = "aip $cli_version" ] || fail 'CLI changed since checking; retry the update'
  fi
  if [ -n "$app" ]; then
    [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")" = 'dev.zottiben.ai-planner' ] || fail 'target is not an ai-planner app'
    [ -n "$old_app_version" ] && [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")" = "$old_app_version" ] || fail 'desktop changed since checking; retry the update'
  fi
}
check_targets
if [ -n "$cli" ]; then
  # Two different CLI copies can share these records. Serialize their publication
  # too, so a failed update cannot restore metadata over another successful one.
  mkdir -p "$HOME/.ai-planner"
  mkdir "$HOME/.ai-planner/install.update-lock" || fail 'another CLI update holds the installation records, or they are not writable'
  provenance_lock="$HOME/.ai-planner/install.update-lock"
fi

base="https://github.com/zottiben/ai-planner/releases/download/v${version}"
file="ai-planner-v${version}-${platform}.tar.gz"
download() { curl -fsSL --connect-timeout 15 --max-time 180 --retry 2 "$1" -o "$2"; }
download "$base/$file" "$scratch/$file"
download "$base/checksums.txt" "$scratch/checksums.txt"
expected=$(awk -v file="$file" '$2 == file { print $1 }' "$scratch/checksums.txt")
[ -n "$expected" ] || fail "missing checksum for $file"
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$scratch/$file" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
  actual=$(shasum -a 256 "$scratch/$file" | awk '{print $1}')
else
  fail 'a SHA-256 tool is required; nothing was installed'
fi
[ "$actual" = "$expected" ] || fail "checksum mismatch for $file"
tar xzf "$scratch/$file" -C "$scratch"
[ -x "$scratch/aip" ] || fail 'release archive has no aip binary'
[ "$("$scratch/aip" --version)" = "aip $version" ] || fail 'archive CLI version does not match the release'

# Stage on each destination filesystem before touching a working installation.
if [ -n "$cli" ]; then
  [ -f "$cli" ] || fail "CLI target does not exist: $cli"
  cli_stage=$(mktemp -d "$(dirname "$cli")/.aip-update.XXXXXX")
  install -m 0755 "$scratch/aip" "$cli_stage/next"
  cp -p "$cli" "$cli_stage/previous"
fi
if [ -n "$app" ]; then
  [ -d "$app" ] || fail "app target does not exist: $app"
  [ -x "$scratch/ai-planner.app/Contents/MacOS/ai-planner" ] || fail 'release archive has no desktop app'
  app_version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$scratch/ai-planner.app/Contents/Info.plist")
  [ "$app_version" = "$version" ] || fail 'archive desktop version does not match the release'
  [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$scratch/ai-planner.app/Contents/Info.plist")" = 'dev.zottiben.ai-planner' ] || fail 'archive does not contain an ai-planner app'
  app_stage=$(mktemp -d "$(dirname "$app")/.ai-planner-update.XXXXXX")
  cp -R "$scratch/ai-planner.app" "$app_stage/next.app"
fi
if [ -n "$cli" ]; then
  # Prepare provenance before publishing, so a permissions failure cannot result in
  # an updated binary whose next update is sent down the wrong route again.
  mkdir -p "$HOME/.ai-planner"
  for name in install-method install-path; do
    if [ -f "$HOME/.ai-planner/$name" ]; then
      cp -p "$HOME/.ai-planner/$name" "$scratch/old-$name"
    fi
  done
  printf 'release\n' > "$scratch/install-method"
  printf '%s\n' "$cli" > "$scratch/install-path"
fi
check_targets
if [ -n "$app" ]; then
  mv "$app" "$app_stage/previous.app"
  mv "$app_stage/next.app" "$app"
fi
if [ -n "$cli" ]; then
  cli_replaced=yes
  mv -f "$cli_stage/next" "$cli"
  # Keep the existing marker until the replacement has succeeded.
  provenance_changed=yes
  cp "$scratch/install-path" "$HOME/.ai-planner/install-path"
  cp "$scratch/install-method" "$HOME/.ai-planner/install-method"
fi
committed=yes
[ -z "$cli" ] || printf 'Installed aip %s at %s\n' "$version" "$cli"
[ -z "$app" ] || printf 'Installed desktop %s at %s (restart the app to use it)\n' "$version" "$app"
