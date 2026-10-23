#!/bin/sh
# Builds the macOS app from the release tag, packs it in a .dmg, and adds to the GitHub
# release of that version: the .dmg, the signed update file (.app.tar.gz), the macOS
# entry in latest.json, and their checksums in SHA256SUMS.txt. Run on a Mac after the
# tag's build has published the release (GitHub's Mac machines cost ten times the
# minutes, so the Mac build happens here).
#
# Needs the update signing key: ~/.tauri/convert-wheel-updater.key, with its password
# in the macOS Keychain (service "convert-wheel-updater-key").
set -eu
cd "$(dirname "$0")/.."

version=$(node -p "require('./src-tauri/tauri.conf.json').version")
tag="v$version"

# Only a clean checkout of exactly the release tag, so the .dmg matches the source.
[ -z "$(git status --porcelain)" ] || { echo "Uncommitted changes: commit or stash them first."; exit 1; }
[ "$(git describe --tags --exact-match 2>/dev/null)" = "$tag" ] || { echo "HEAD is not $tag: check out the tag first."; exit 1; }
gh release view "$tag" >/dev/null 2>&1 || { echo "No release $tag yet: push the tag and wait for its build."; exit 1; }

key="$HOME/.tauri/convert-wheel-updater.key"
[ -f "$key" ] || { echo "Update signing key missing: $key"; exit 1; }
TAURI_SIGNING_PRIVATE_KEY=$(cat "$key")
TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$(security find-generic-password -a convert-wheel -s convert-wheel-updater-key -w)
export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD

npx tauri build --bundles app
unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD

bundle="src-tauri/target/release/bundle/macos"
app="$bundle/Convert Wheel.app"
out="src-tauri/target/release/bundle/dmg"
dmg="$out/Convert.Wheel_${version}_aarch64.dmg"
upd="$out/Convert.Wheel_${version}_aarch64.app.tar.gz"
mkdir -p "$out"
rm -f "$dmg" "$upd"
cp "$bundle/Convert Wheel.app.tar.gz" "$upd"

# The disk image holds the app and a link to Applications to drag it onto.
staging=$(mktemp -d)
ditto "$app" "$staging/Convert Wheel.app"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "Convert Wheel" -srcfolder "$staging" -ov -format UDZO "$dmg" >/dev/null 2>&1
rm -rf "$staging"

work=$(mktemp -d)
gh release download "$tag" --pattern SHA256SUMS.txt --pattern latest.json --dir "$work"

# Add the macOS update to latest.json; the signature is checked by the app.
python3 -I - "$work/latest.json" "$upd" "$bundle/Convert Wheel.app.tar.gz.sig" "$tag" <<'PY'
import json, os, sys
manifest_path, update, sig_path, tag = sys.argv[1:5]
repo = os.popen("gh repo view --json nameWithOwner -q .nameWithOwner").read().strip()
manifest = json.load(open(manifest_path))
entry = {
    "url": f"https://github.com/{repo}/releases/download/{tag}/{os.path.basename(update)}",
    "signature": open(sig_path).read().strip(),
}
manifest["platforms"]["darwin-aarch64-app"] = entry
manifest["platforms"]["darwin-aarch64"] = entry
json.dump(manifest, open(manifest_path, "w"), indent=2)
PY

# Checksums: replace the lines for files this script (re)writes.
grep -v -e "_aarch64.dmg" -e "_aarch64.app.tar.gz" -e " latest.json" "$work/SHA256SUMS.txt" > "$work/sums.txt" || true
(cd "$out" && shasum -a 256 "$(basename "$dmg")" "$(basename "$upd")") >> "$work/sums.txt"
(cd "$work" && shasum -a 256 latest.json) >> "$work/sums.txt"
mv "$work/sums.txt" "$work/SHA256SUMS.txt"

gh release upload "$tag" "$dmg" "$upd" "$work/latest.json" "$work/SHA256SUMS.txt" --clobber
rm -rf "$work"
echo "Added $(basename "$dmg") and the macOS update to $tag"
