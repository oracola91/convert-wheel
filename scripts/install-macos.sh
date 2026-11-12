#!/bin/sh
# Builds Pirouette (called Convert Wheel before 0.3.0) and installs it in /Applications,
# replacing an older copy under either name, then starts it. Run again after pulling changes to update.
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
built="src-tauri/target/release/bundle/macos/Pirouette.app"
target="/Applications/Pirouette.app"
# The same app (same bundle identifier) under its name before 0.3.0. An update from
# inside the app keeps that folder name, so it can still be there.
old="/Applications/Convert Wheel.app"

# Quit a running copy so the new one can take its place.
for app in "$target" "$old"; do
  if pgrep -f "$app/Contents/MacOS/" >/dev/null; then
    osascript -e 'tell application id "local.convertwheel" to quit' >/dev/null 2>&1 || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      pgrep -f "$app/Contents/MacOS/" >/dev/null || break
      sleep 0.5
    done
    pkill -f "$app/Contents/MacOS/" 2>/dev/null || true
  fi
done

rm -rf "$target" "$old"
ditto "$built" "$target"
open "$target"
echo "Installed and started: $target"
