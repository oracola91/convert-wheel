#!/bin/sh
# Finishes a release from a Mac. The tag's build on GitHub has created the release as a
# draft with the Windows and Linux files; this adds the macOS ones and publishes it:
# the .dmg, the signed update file (.app.tar.gz), the macOS entries in latest.json and
# the checksums. Installed copies see the release only once it is published, so they
# never find a latest.json without their system in it. (GitHub's Mac machines cost ten
# times the minutes, so the Mac build happens here.)
#
# Needs the update signing key: ~/.tauri/convert-wheel-updater.key, with its password
# in the macOS Keychain (service "convert-wheel-updater-key"). Only the final signing
# command sees them, never the build. And the code-signing certificate "Convert Wheel
# Signing" in the login keychain (scripts/macos-signing-cert.sh).
set -eu
cd "$(dirname "$0")/.."

version=$(node -p "require('./src-tauri/tauri.conf.json').version")
tag="v$version"

# Only a clean checkout of exactly the release tag, the same tag GitHub built from.
[ -z "$(git status --porcelain)" ] || { echo "Uncommitted changes: commit or stash them first."; exit 1; }
[ "$(git describe --tags --exact-match 2>/dev/null)" = "$tag" ] || { echo "HEAD is not $tag: check out the tag first."; exit 1; }
remote=$(git ls-remote origin "refs/tags/$tag^{}" | cut -f1)
[ -n "$remote" ] && [ "$remote" = "$(git rev-parse HEAD)" ] || { echo "Local $tag differs from GitHub's: fetch the tags again."; exit 1; }
[ "$(gh release view "$tag" --json isDraft -q .isDraft 2>/dev/null)" = "true" ] || { echo "No draft release $tag: push the tag and wait for its build (or it is already published)."; exit 1; }

key="$HOME/.tauri/convert-wheel-updater.key"
[ -f "$key" ] || { echo "Update signing key missing: $key"; exit 1; }

# Every release is signed with the same certificate, so macOS keeps the permissions it
# gave the app across updates.
security find-certificate -c "Convert Wheel Signing" >/dev/null 2>&1 ||
  { echo "Code-signing certificate missing: run scripts/macos-signing-cert.sh, or import its backup."; exit 1; }
export APPLE_SIGNING_IDENTITY="Convert Wheel Signing"

# The exact tool versions from the lock files, and no install scripts.
npm ci --ignore-scripts
npx --no-install tauri build --bundles app -- --locked

bundle="src-tauri/target/release/bundle/macos"
out="src-tauri/target/release/bundle/dmg"
dmg="$out/Convert.Wheel_${version}_aarch64.dmg"
upd="$out/Convert.Wheel_${version}_aarch64.app.tar.gz"
mkdir -p "$out"
rm -f "$dmg" "$upd" "$upd.sig"

# The update file: the .app in a tar.gz, as the updater expects. COPYFILE_DISABLE keeps
# macOS's ._ metadata files out of it.
COPYFILE_DISABLE=1 tar -czf "$upd" -C "$bundle" "Convert Wheel.app"

# The disk image holds the app and a link to Applications to drag it onto.
staging=$(mktemp -d)
ditto "$bundle/Convert Wheel.app" "$staging/Convert Wheel.app"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "Convert Wheel" -srcfolder "$staging" -ov -format UDZO "$dmg" >/dev/null 2>&1
rm -rf "$staging"

# Sign: the only command that sees the key. The signature records the version.
TAURI_SIGNING_PRIVATE_KEY=$(cat "$key") \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$(security find-generic-password -a convert-wheel -s convert-wheel-updater-key -w) \
  npx --no-install tauri signer sign --app-version "$version" "$upd" >/dev/null

work=$(mktemp -d)
gh release download "$tag" --pattern SHA256SUMS.txt --pattern latest.json --dir "$work"

# Add the macOS update to latest.json, then check every system is there before publishing.
python3 -I - "$work/latest.json" "$upd" "$upd.sig" "$tag" <<'PY'
import json, os, subprocess, sys
manifest_path, update, sig_path, tag = sys.argv[1:5]
repo = subprocess.run(["gh", "repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"],
                      capture_output=True, text=True, check=True).stdout.strip()
manifest = json.load(open(manifest_path))
entry = {
    "url": f"https://github.com/{repo}/releases/download/{tag}/{os.path.basename(update)}",
    "signature": open(sig_path).read().strip(),
}
manifest["platforms"]["darwin-aarch64-app"] = entry
manifest["platforms"]["darwin-aarch64"] = entry
needed = ["windows-x86_64-nsis", "windows-x86_64-msi", "linux-x86_64-appimage", "linux-x86_64-deb", "darwin-aarch64"]
missing = [k for k in needed if k not in manifest["platforms"]]
if missing:
    sys.exit(f"latest.json lacks: {', '.join(missing)}")
json.dump(manifest, open(manifest_path, "w"), indent=2)
PY

# Checksums: the CI list plus the macOS files and the final latest.json.
grep -v " latest.json" "$work/SHA256SUMS.txt" > "$work/sums.txt" || true
(cd "$out" && shasum -a 256 "$(basename "$dmg")" "$(basename "$upd")") >> "$work/sums.txt"
(cd "$work" && shasum -a 256 latest.json) >> "$work/sums.txt"
mv "$work/sums.txt" "$work/SHA256SUMS.txt"

# Still a draft, so replacing latest.json and the checksums here is safe.
gh release upload "$tag" "$dmg" "$upd" "$work/latest.json" "$work/SHA256SUMS.txt" --clobber
gh release edit "$tag" --draft=false --latest
rm -rf "$work"
echo "Published $tag with the macOS files."
