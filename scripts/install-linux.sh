#!/bin/sh
# Sets up Convert Wheel on Ubuntu or Debian in one go:
#
#   curl -fsSL https://raw.githubusercontent.com/oracola91/convert-wheel/main/scripts/install-linux.sh | sh
#
# 1. the converters: ffmpeg, ImageMagick, LibreOffice (Writer, Calc, Impress) and pandoc,
#    pandoc from its own releases unless a working one is already installed;
# 2. the newest Convert Wheel release, checked against the release's checksums;
# 3. Open at Login;
# 4. on GNOME (Ubuntu's desktop), the keyboard shortcut Super+Shift+C that opens the
#    wheel, which Wayland sessions need: there, apps can't see a Shift+drag;
# 5. starts the app.
#
# Run it as yourself, not as root: it asks for your password for the installs.
# Running it again installs what's missing and leaves the rest as it is. After this the
# app updates itself.
#
# Everything runs from main() at the very end, so a download cut short runs nothing.

set -eu

REPO=oracola91/convert-wheel
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

# apt never gets the terminal's input: with `curl ... | sh` that input is this script.
apt_install() {
    sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y "$@" </dev/null
}

# Checks a downloaded file against the line for it in a checksum list ("hash  name").
matches() { # file, sums file
    expected=$(awk -v f="$(basename "$1")" '$2 == f { print $1 }' "$2")
    [ -n "$expected" ] && [ "$(sha256sum "$1" | cut -d' ' -f1)" = "$expected" ]
}

# Whether pandoc does what the app asks of it: write a Word file inside its sandbox.
# That fails both for a pandoc older than 2.15 and for one built without its data files.
pandoc_ok() {
    command -v pandoc >/dev/null || return 1
    printf 'check\n' | pandoc --sandbox -f markdown -t docx -o "$work/check.docx" 2>/dev/null
}

# Adds the shortcut on GNOME. Returns 1 where that isn't possible (another desktop, or
# no desktop session reachable), and the caller prints how to add it by hand.
gnome_shortcut() {
    case "${XDG_CURRENT_DESKTOP:-}" in *GNOME*) ;; *) return 1 ;; esac
    command -v gsettings >/dev/null || return 1
    gsettings list-schemas 2>/dev/null | grep -qx "$KEYS" || return 1
    list=$(gsettings get "$KEYS" custom-keybindings) || return 1
    case "$list" in
        *"'$SLOT'"*) ;;
        "@as []" | "[]") gsettings set "$KEYS" custom-keybindings "['$SLOT']" ;;
        *) gsettings set "$KEYS" custom-keybindings "${list%]}, '$SLOT']" ;;
    esac
    one="$KEYS.custom-keybinding:$SLOT"
    gsettings set "$one" name 'Convert Wheel'
    gsettings set "$one" command 'convert-wheel --show'
    gsettings set "$one" binding "$SHORTCUT"
    # Without a running desktop session gsettings keeps changes in memory only.
    [ "$(gsettings get "$one" binding)" = "'$SHORTCUT'" ]
}

main() {
    [ "$(id -u)" -ne 0 ] || fail "run it as yourself, not as root or with sudo. It asks for your password when it needs it."
    [ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || fail "this is for 64-bit Intel or AMD Linux."
    command -v apt-get >/dev/null || fail "this is for Ubuntu and Debian. For other systems, see INSTALL.md."
    command -v curl >/dev/null || fail "curl is needed: sudo apt install curl"

    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    # apt reads local packages as its own user, so they must be readable.
    chmod 755 "$work"

    step "1/5 The converters: ffmpeg, ImageMagick, pandoc, LibreOffice (asks for your password)"
    sudo apt-get update </dev/null
    apt_install ffmpeg imagemagick libreoffice-writer libreoffice-calc libreoffice-impress
    if ! pandoc_ok; then
        step "    Adding pandoc $PANDOC_VERSION from pandoc's releases"
        pdeb="$work/pandoc-$PANDOC_VERSION-1-amd64.deb"
        curl -fsSL -o "$pdeb" "https://github.com/jgm/pandoc/releases/download/$PANDOC_VERSION/pandoc-$PANDOC_VERSION-1-amd64.deb"
        [ "$(sha256sum "$pdeb" | cut -d' ' -f1)" = "$PANDOC_SHA256" ] || fail "the pandoc download doesn't match its checksum."
        chmod 644 "$pdeb"
        apt_install "$pdeb"
        pandoc_ok || fail "pandoc still can't write Word files in its sandbox."
    fi

    step "2/5 Convert Wheel"
    latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")
    tag=${latest##*/}
    case "$tag" in v[0-9]*) ;; *) fail "couldn't find the newest release on GitHub." ;; esac
    deb="$work/Convert.Wheel_${tag#v}_amd64.deb"
    curl -fsSL -o "$deb" "https://github.com/$REPO/releases/download/$tag/$(basename "$deb")"
    curl -fsSL -o "$work/SHA256SUMS.txt" "https://github.com/$REPO/releases/download/$tag/SHA256SUMS.txt"
    matches "$deb" "$work/SHA256SUMS.txt" || fail "the download doesn't match the release's checksum."
    chmod 644 "$deb"
    apt_install "$deb"
    exe=$(command -v convert-wheel) || fail "convert-wheel isn't installed after all."

    # The same file the app's own Open at Login writes, so its menu shows it ticked.
    step "3/5 Open at Login"
    autostart="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
    mkdir -p "$autostart"
    printf '[Desktop Entry]\nType=Application\nName=Convert Wheel\nExec="%s"\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n' \
        "$exe" >"$autostart/convert-wheel.desktop"

    step "4/5 Keyboard shortcut"
    if gnome_shortcut; then
        shortcut=yes
        echo "Super+Shift+C opens the wheel."
    else
        shortcut=no
        echo "Not set here. Add it in your desktop's keyboard settings: command 'convert-wheel --show', key Super+Shift+C."
    fi

    step "5/5 Starting Convert Wheel"
    if pgrep -x convert-wheel >/dev/null; then
        echo "It's already running. Quit it from its menu and start it again to use the version just installed."
    elif [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
        setsid "$exe" >/dev/null 2>&1 </dev/null &
        echo "Started: its icon is in the top bar."
    else
        echo "No desktop session here: it starts at the next login."
    fi

    step "Done: Convert Wheel ${tag#v}"
    if [ "${XDG_SESSION_TYPE:-}" = x11 ]; then
        echo "Hold Shift while dragging a file in Files, and the wheel opens. Drop the file on a format."
    elif [ "$shortcut" = yes ]; then
        echo "Press Super+Shift+C, then drag a file onto the wheel and drop it on a format."
        echo "(This is a Wayland session, which hides drags from other apps, so Shift+drag can't work here.)"
    fi
    echo "Converted files land next to the original. The app updates itself from now on."
}

main "$@"
