#!/bin/sh
# Builds the macOS app, packs it in a .dmg and attaches it to the GitHub release of
# the current version, adding its checksum to SHA256SUMS.txt. Run on a Mac after the
# tag's build has published the release (GitHub's Mac build machines cost ten times
# the minutes, so the Mac build happens here).
set -eu
cd "$(dirname "$0")/.."

version=$(node -p "require('./src-tauri/tauri.conf.json').version")
tag="v$version"
gh release view "$tag" >/dev/null 2>&1 || { echo "No release $tag yet: push the tag and wait for its build."; exit 1; }

npx tauri build --bundles app
app="src-tauri/target/release/bundle/macos/Convert Wheel.app"
out="src-tauri/target/release/bundle/dmg"
dmg="$out/Convert.Wheel_${version}_aarch64.dmg"
mkdir -p "$out"
rm -f "$dmg"

# The disk image holds the app and a link to Applications to drag it onto.
staging=$(mktemp -d)
ditto "$app" "$staging/Convert Wheel.app"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "Convert Wheel" -srcfolder "$staging" -ov -format UDZO "$dmg" >/dev/null
rm -rf "$staging"

sums=$(mktemp -d)
gh release download "$tag" --pattern SHA256SUMS.txt --dir "$sums"
grep -v "_aarch64.dmg" "$sums/SHA256SUMS.txt" > "$sums/new.txt" || true
(cd "$out" && shasum -a 256 "$(basename "$dmg")") >> "$sums/new.txt"
mv "$sums/new.txt" "$sums/SHA256SUMS.txt"

gh release upload "$tag" "$dmg" "$sums/SHA256SUMS.txt" --clobber
rm -rf "$sums"
echo "Attached $(basename "$dmg") to $tag"
