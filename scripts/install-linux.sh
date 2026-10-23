#!/bin/sh
# Sets up Convert Wheel on Ubuntu or Debian in one go:
#
#   curl -fsSL https://raw.githubusercontent.com/oracola91/convert-wheel/main/scripts/install-linux.sh | sh
#
# 1. the converters: ffmpeg, ImageMagick, LibreOffice (Writer, Calc, Impress), MuPDF
#    and pandoc, pandoc from its own releases unless an up-to-date working one is there;
# 2. the newest Convert Wheel release, after checking its signature with the project's
#    update key (the same check the app makes on every update) and the version in it;
# 3. Open at Login;
# 4. on GNOME, the keyboard shortcut Super+Shift+C that opens the wheel, which Wayland
#    sessions need: there, apps can't see a Shift+drag;
# 5. starts the app.
#
# A version can be named instead of the newest: `... | sh -s -- 0.2.0`.
# Run it as yourself, not as root: it asks for your password for the installs.
# Running it again installs what's missing and turns Open at Login back on; a shortcut
# key you changed stays. After this the app updates itself.
#
# Everything runs from main() at the very end, so a download cut short runs nothing.

set -eu

REPO=oracola91/convert-wheel
EXE=/usr/bin/convert-wheel
# The public half of the project's update key: the same one built into the app.
UPDATE_KEY=RWSSm9yM9UYMp00bQLUwwNEMjwlQkpEI/3KR9vIf38mMNQSY7YaRG7Tf
# The app runs pandoc with --sandbox. That needs pandoc 2.15 or newer (Ubuntu 22.04 ships
# 2.9), built with its data files inside: Ubuntu's and Debian's own builds keep them
# outside, and in the sandbox they can't write Word files. So pandoc comes from its own
# releases, pinned to this checksum.
PANDOC_VERSION=3.12.1
PANDOC_SHA256=b4217ac7c54067c442f9d4cd38bcd0c184a43f3e7053431a12263a6f92a33ba2
KEYS=org.gnome.settings-daemon.plugins.media-keys
SLOT=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/convert-wheel/
SHORTCUT='<Shift><Super>c'

step() { printf '\n\033[1m%s\033[0m\n' "$*"; }
fail() {
    printf '\nConvert Wheel setup stopped: %s\n' "$*" >&2
    exit 1
}

# HTTPS only, redirects included, and no endless wait on a dead connection.
fetch() {
    curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 --connect-timeout 30 --retry 2 "$@"
}

# apt waits for another apt (an update running in the background) instead of failing,
# and never gets the terminal's input: with `curl ... | sh` that input is this script.
apt_get() {
    sudo env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=300 "$@" </dev/null
}

# Whether pandoc does what the app asks of it: write a Word file inside its sandbox.
# That fails both for a pandoc older than 2.15 and for one built without its data files.
pandoc_ok() {
    command -v pandoc >/dev/null || return 1
    printf 'check\n' | pandoc --sandbox -f markdown -t docx -o "$work/check.docx" 2>"$work/pandoc.err"
}

pandoc_older() {
    have=$(pandoc --version | sed -n '1s/^pandoc[^0-9]*//p')
    dpkg --compare-versions "$have" lt "$PANDOC_VERSION"
}

# Checks the downloaded .deb against its signature in the release's latest.json (the
# file installed copies read to find updates): made with the update key above, over
# exactly this file, for exactly this version. openssl does the Ed25519 check, so
# nothing else needs installing.
signed() { # deb, latest.json, version
    python3 -I - "$1" "$2" "$UPDATE_KEY" "$3" "$work" <<'PY'
import base64, hashlib, json, os, subprocess, sys

deb, manifest, key, version, work = sys.argv[1:6]

def fail(why):
    sys.exit("signature check: " + why)

try:
    release = json.load(open(manifest))
    entry = release["platforms"]["linux-x86_64-deb"]
    lines = base64.b64decode(entry["signature"]).decode().splitlines()
    sig = base64.b64decode(lines[1])
    comment = lines[2]
    global_sig = base64.b64decode(lines[3])
except Exception:
    fail("the release's signature can't be read")
if release.get("version") != version or os.path.basename(entry.get("url", "")) != os.path.basename(deb):
    fail("the release's list doesn't name this file")
public = base64.b64decode(key)
if len(sig) != 74 or len(global_sig) != 64 or not comment.startswith("trusted comment: "):
    fail("the release's signature can't be read")
if sig[2:10] != public[2:10]:
    fail("made with a different key")
data = open(deb, "rb").read()
if sig[:2] == b"ED":
    message = hashlib.blake2b(data, digest_size=64).digest()
elif sig[:2] == b"Ed":
    message = data
else:
    fail("unknown signature type")

pem = os.path.join(work, "update-key.pem")
with open(pem, "w") as f:
    der = bytes.fromhex("302a300506032b6570032100") + public[10:]
    f.write("-----BEGIN PUBLIC KEY-----\n" + base64.b64encode(der).decode() + "\n-----END PUBLIC KEY-----\n")

def verify(signed_bytes, signature, what):
    msg, sigfile = os.path.join(work, "signed.bin"), os.path.join(work, "signature.bin")
    open(msg, "wb").write(signed_bytes)
    open(sigfile, "wb").write(signature)
    check = ["openssl", "pkeyutl", "-verify", "-pubin", "-inkey", pem, "-rawin", "-in", msg, "-sigfile", sigfile]
    if subprocess.run(check, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode != 0:
        fail(what + " doesn't match the signature")

verify(message, sig[10:], "the file")
text = comment[len("trusted comment: "):]
verify(sig[10:] + text.encode(), global_sig, "the signed comment")
fields = dict(f.split(":", 1) for f in text.split("\t") if ":" in f)
if fields.get("version") != version:
    fail("signed for version %s, not %s" % (fields.get("version"), version))
PY
}

# Adds the shortcut on GNOME, or keeps it if it is there (with whatever key it has
# now). Returns 1 where that isn't possible (another desktop, or no desktop session
# reachable), and the caller prints how to add it by hand.
gnome_shortcut() {
    case "${XDG_CURRENT_DESKTOP:-}" in *GNOME*) ;; *) return 1 ;; esac
    command -v gsettings >/dev/null || return 1
    gsettings list-schemas 2>/dev/null | grep -qx "$KEYS" || return 1
    list=$(gsettings get "$KEYS" custom-keybindings) || return 1
    one="$KEYS.custom-keybinding:$SLOT"
    case "$list" in
        *"'$SLOT'"*) ;;
        *)
            case "$list" in
                "@as []" | "[]") list="['$SLOT']" ;;
                *) list="${list%]}, '$SLOT']" ;;
            esac
            gsettings set "$KEYS" custom-keybindings "$list" || return 1
            gsettings set "$one" binding "$SHORTCUT" || return 1
            ;;
    esac
    gsettings set "$one" name 'Convert Wheel' || return 1
    gsettings set "$one" command "$EXE --show" || return 1
    # Without a running desktop session gsettings keeps changes in memory only, so read
    # everything back.
    case "$(gsettings get "$KEYS" custom-keybindings)" in *"'$SLOT'"*) ;; *) return 1 ;; esac
    [ "$(gsettings get "$one" command)" = "'$EXE --show'" ]
}

main() {
    [ "$(id -u)" -ne 0 ] || fail "run it as yourself, not as root or with sudo. It asks for your password when it needs it."
    [ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || fail "this is for 64-bit Intel or AMD Linux."
    command -v apt-get >/dev/null || fail "this is for Ubuntu and Debian. For other systems, see INSTALL.md."
    command -v curl >/dev/null || fail "curl is needed: sudo apt install curl"
    command -v sudo >/dev/null || fail "sudo is needed, and your account must be allowed to use it."
    if [ $# -gt 0 ]; then
        printf '%s' "$1" | grep -Eqx '[0-9]+\.[0-9]+\.[0-9]+' || fail "\"$1\" isn't a version like 0.2.0."
    fi

    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    # apt reads local packages as its own user, so they must be readable.
    chmod 755 "$work"

    step "1/5 The converters: ffmpeg, ImageMagick, LibreOffice, MuPDF, pandoc (asks for your password)"
    # A broken extra package source makes the update fail but leaves the rest usable.
    apt_get update || echo "(apt-get update reported a problem, above; going on with the package lists there are.)"
    apt_get install -y ffmpeg imagemagick libreoffice-writer libreoffice-calc libreoffice-impress mupdf-tools \
        python3 openssl
    if ! pandoc_ok || pandoc_older; then
        step "    Adding pandoc $PANDOC_VERSION from pandoc's releases"
        pdeb="$work/pandoc-$PANDOC_VERSION-1-amd64.deb"
        fetch -o "$pdeb" "https://github.com/jgm/pandoc/releases/download/$PANDOC_VERSION/pandoc-$PANDOC_VERSION-1-amd64.deb"
        [ "$(sha256sum "$pdeb" | cut -d' ' -f1)" = "$PANDOC_SHA256" ] || fail "the pandoc download doesn't match its checksum."
        chmod 644 "$pdeb"
        apt_get install -y "$pdeb"
        pandoc_ok || fail "pandoc still can't write Word files in its sandbox: $(tail -n 1 "$work/pandoc.err")"
    fi

    step "2/5 Convert Wheel"
    if [ $# -gt 0 ]; then
        tag="v$1"
    else
        latest=$(fetch -I -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")
        tag=${latest##*/}
        printf '%s' "$tag" | grep -Eqx 'v[0-9]+\.[0-9]+\.[0-9]+' || fail "couldn't find the newest release on GitHub."
    fi
    version=${tag#v}
    deb="$work/Convert.Wheel_${version}_amd64.deb"
    fetch -o "$deb" "https://github.com/$REPO/releases/download/$tag/$(basename "$deb")" ||
        fail "couldn't download Convert Wheel $version."
    fetch -o "$work/latest.json" "https://github.com/$REPO/releases/download/$tag/latest.json"
    signed "$deb" "$work/latest.json" "$version" || fail "the download isn't the signed release; nothing was installed."
    echo "Signature checked: Convert Wheel $version, signed with the project's update key."
    chmod 644 "$deb"
    apt_get install -y "$deb"
    [ -x "$EXE" ] || fail "$EXE isn't there after installing."

    # The same file the app's own Open at Login writes, so its menu shows it ticked.
    step "3/5 Open at Login"
    autostart="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
    mkdir -p "$autostart"
    printf '[Desktop Entry]\nType=Application\nName=Convert Wheel\nExec="%s"\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n' \
        "$EXE" >"$autostart/convert-wheel.desktop"
    echo "On. To turn it off, untick Open at Login in the Convert Wheel menu."

    step "4/5 Keyboard shortcut"
    key=""
    if gnome_shortcut; then
        key=$(gsettings get "$KEYS.custom-keybinding:$SLOT" binding | tr -d "'")
        [ "$key" = "$SHORTCUT" ] && key="Super+Shift+C"
        echo "$key opens the wheel."
    else
        echo "Not set here. Add one in your desktop's keyboard settings: command '$EXE --show', key Super+Shift+C."
    fi

    step "5/5 Starting Convert Wheel"
    if pgrep -x convert-wheel >/dev/null; then
        echo "It's already running. Quit it from its menu and start it again to use the version just installed."
    elif [ -n "${SSH_CONNECTION:-}" ] || [ -z "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
        echo "Not started from here (this terminal isn't on the desktop): it starts at your next login."
    else
        setsid "$EXE" >/dev/null 2>&1 </dev/null &
        sleep 2
        if pgrep -x convert-wheel >/dev/null; then
            echo "Started: its icon is in the top bar."
        else
            echo "It didn't stay running. Run convert-wheel in a terminal to see why."
        fi
    fi

    step "Done: Convert Wheel $version"
    case "${XDG_SESSION_TYPE:-}" in
        x11) echo "Hold Shift while dragging a file in Files, and the wheel opens. Drop the file on a format." ;;
        wayland)
            if [ -n "$key" ]; then
                echo "Press $key, then drag a file onto the wheel and drop it on a format."
            else
                echo "Press the shortcut you add, then drag a file onto the wheel and drop it on a format."
            fi
            echo "(Wayland hides drags from other apps, so Shift+drag can't work in this session.)"
            ;;
        *) echo "On the desktop: Shift+drag a file in an X11 session, or press the shortcut in a Wayland one." ;;
    esac
    if grep -Eqs 'rights="none"[[:space:]]+pattern="PDF"' /etc/ImageMagick-6/policy.xml; then
        echo "Note: this system's ImageMagick isn't allowed to write PDFs, so images can't become PDFs here (see INSTALL.md)."
    fi
    echo "Converted files land next to the original. The app updates itself from now on."
}

main "$@"
