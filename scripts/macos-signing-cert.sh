#!/bin/sh
# Makes Convert Wheel's own code-signing certificate on this Mac, once. The Mac install
# and release scripts sign with it, so macOS keeps the permissions it gave the app (the
# Downloads and Documents folders, for one) across updates: an app signed ad hoc looks
# new to macOS after every update and is asked again. The certificate is self-signed,
# so macOS still warns on the first open of a downloaded copy, as before.
#
# Leaves a backup of the key in the folder given (default ~/Downloads): a .p12 file and
# its password. Keep both in the password manager, then delete them. To sign on another
# Mac, import the .p12 there:
#   security import "Convert Wheel signing key.p12" -k ~/Library/Keychains/login.keychain-db -T /usr/bin/codesign
set -eu
name="Convert Wheel Signing"
out="${1:-$HOME/Downloads}"
# OpenSSL 3, for -addext and the keychain's legacy .p12 format.
openssl=/opt/homebrew/opt/openssl@3/bin/openssl
[ -x "$openssl" ] || openssl=openssl

if security find-certificate -c "$name" >/dev/null 2>&1; then
  echo "\"$name\" is already in the keychain."
  exit 0
fi
umask 077
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
"$openssl" rand -base64 24 >"$work/password"
"$openssl" req -x509 -newkey rsa:3072 -sha256 -days 7300 -nodes -subj "/CN=$name" \
  -addext "basicConstraints=critical,CA:false" -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning" -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
"$openssl" pkcs12 -export -legacy -name "$name" -inkey "$work/key.pem" -in "$work/cert.pem" \
  -passout "file:$work/password" -out "$work/key.p12"
# codesign may use the key; anything else has to ask.
security import "$work/key.p12" -k "$HOME/Library/Keychains/login.keychain-db" -f pkcs12 \
  -P "$(cat "$work/password")" -T /usr/bin/codesign >/dev/null
cp "$work/key.p12" "$out/Convert Wheel signing key.p12"
cp "$work/password" "$out/Convert Wheel signing key password.txt"
echo "Made \"$name\". Its backup is in $out: save both files in the password manager, then delete them."
