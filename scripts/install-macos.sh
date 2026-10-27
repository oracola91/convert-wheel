#!/bin/sh
# Builds Convert Wheel and installs it in /Applications, replacing an older copy,
# then starts it. Run again after pulling changes to update.
set -eu
cd "$(dirname "$0")/.."

# A local build is never signed with the update key: only releases are. It is signed
# with Convert Wheel's own certificate when this Mac has it (scripts/macos-signing-cert.sh),
# so macOS keeps the permissions it gave; otherwise ad hoc, as tauri.conf.json says.
if security find-certificate -c "Convert Wheel Signing" >/dev/null 2>&1; then
  export APPLE_SIGNING_IDENTITY="Convert Wheel Signing"
fi
npm ci --ignore-scripts
npx --no-install tauri build --bundles app -- --locked
built="src-tauri/target/release/bundle/macos/Convert Wheel.app"
target="/Applications/Convert Wheel.app"

# Quit a running copy so the new one can take its place.
if pgrep -f "$target/Contents/MacOS/" >/dev/null; then
  osascript -e 'tell application id "local.convertwheel" to quit' >/dev/null 2>&1 || true
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    pgrep -f "$target/Contents/MacOS/" >/dev/null || break
    sleep 0.5
  done
  pkill -f "$target/Contents/MacOS/" 2>/dev/null || true
fi

rm -rf "$target"
ditto "$built" "$target"
open "$target"
echo "Installed and started: $target"
