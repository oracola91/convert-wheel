#!/bin/sh
# Builds Convert Wheel and installs it in /Applications, replacing an older copy,
# then starts it. Run again after pulling changes to update.
set -eu
cd "$(dirname "$0")/.."

npx tauri build --bundles app
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
