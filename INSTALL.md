# Installing Pirouette

Every version is on the repo's [Releases page](https://github.com/oracola91/pirouette/releases).

**From version 0.1.1 the app updates itself.** It asks github.com for the newest version shortly after it starts and then once a day; when a new version is out, a notification says so and the menu offers **Install Update … and Restart**. So install once, by hand, and later versions arrive through the app. Copies of 0.1.0 have no updater and need one manual install of 0.1.1 or newer.

| System | File |
|---|---|
| macOS, Apple silicon | `Pirouette_<version>_aarch64.dmg` (`Pirouette_…` up to 0.2.1) |
| Windows, one user, no admin rights | `Pirouette_<version>_x64-setup.exe` |
| Windows, all users, management tools | `Pirouette_<version>_x64_en-US.msi` |
| Ubuntu, Debian | `Pirouette_<version>_amd64.deb` |
| Other Linux | `Pirouette_<version>_amd64.AppImage` |

None of the files is signed with a certificate the systems trust, so Windows and macOS warn once on first launch. This stays so: see [Why the files aren't signed](#why-the-files-arent-signed), which also says how to open them safely. Check a download against `SHA256SUMS.txt` first ([Checking a download](#checking-a-download)).

Pirouette does the converting with five free tools that are installed separately: **ffmpeg** (audio, video), **ImageMagick** (images), **pandoc** (documents), **LibreOffice** (office files, and documents to PDF) and **MuPDF** (PDFs to pictures and text). Install them first; the commands are in each section. A format whose tool is missing fails with a notification. On macOS and Windows that notification points to **Install …** in the app's menu, which installs the tool with Homebrew or winget; on Linux it names the install command.

## macOS

**1. The converters** ([Homebrew](https://brew.sh) needed):

```sh
brew install ffmpeg imagemagick pandoc mupdf-tools
brew install --cask libreoffice
```

Or install only the ones you need, when you need them: when a drop needs a missing one, the menu offers **Install …** and runs the same `brew install` for you. ExifTool (`brew install exiftool`) is only needed to change a picture's or video's date and place in Info; the menu offers it the same way.

**2. The app:** open the `.dmg` and drag Pirouette onto Applications. Delete an older copy first if there is one.

**3. First launch:** macOS blocks apps that aren't signed by an Apple developer. Open Pirouette once, then go to System Settings > Privacy & Security and click "Open Anyway" next to Pirouette. Or, in Terminal:

```sh
xattr -dr com.apple.quarantine "/Applications/Pirouette.app"
```

**4. Use it:** a ring-and-dot icon appears in the menu bar (top right); there is no Dock icon. Hold Shift while dragging a file in Finder and the wheel opens; hold Option+Shift for the Tools wheel (pictures: compress, half size, remove metadata, rotate, read a QR code, crop, compress to a size, one PDF; PDFs: merge, split, smaller; video: compress, remove metadata, trim, first frame, join; audio: compress, remove metadata, trim, join, loudness, mono); start a drag and then hold Control+Shift for the Actions wheel (copy path, show in folder, terminal, AirDrop, share, make a ZIP / TAR.GZ / TAR, unpack). The keys can change the wheel during the drag, without letting go of the file. Allow notifications when macOS asks. Open at Login is in the ring icon's menu.

**Without a drag:** select files in Finder, right-click > Services > **Open in Pirouette Wheel**: the wheel opens with the keyboard (arrows pick, Enter applies, Esc closes). A key for it can be set in System Settings > Keyboard > Keyboard Shortcuts > Services. Everything else is in the menu's **Settings…**.

**From the source instead:** `./scripts/install-macos.sh` builds the app on this Mac, installs it in Applications and starts it. A local build needs no Gatekeeper step.

**Update:** the app updates itself (menu > Install Update). By hand: install the new `.dmg` the same way, or run the script again. A copy that updated itself from 0.2.x keeps its old folder name, `Pirouette.app`, but shows Pirouette inside; the script replaces it with `Pirouette.app`.

**Uninstall:** turn off Open at login in Settings, quit, and delete Pirouette (or Pirouette) from Applications. Its settings, working files and log are in `~/Library/Application Support/io.github.oracola91.pirouette`, `~/Library/Caches/io.github.oracola91.pirouette` and `~/Library/Logs/io.github.oracola91.pirouette`; delete them too for a clean removal.

## Windows

**1. The converters**, in a terminal:

```
winget install Gyan.FFmpeg ImageMagick.ImageMagick JohnMacFarlane.Pandoc TheDocumentFoundation.LibreOffice ArtifexSoftware.mutool
```

Sign out and back in afterwards, so apps started from the Start menu see the new tools.

Or install only the ones you need, when you need them: when a drop needs a missing one, the menu offers **Install …** and runs the same `winget install` for you (no sign-out needed). Packages that install for all users, such as LibreOffice, still ask for admin rights.

**2. The app**, one of:

- **`.exe`**: double-click it. It installs for the signed-in user, without admin rights.
- **`.msi`**: installs for all users in Program Files and needs admin rights. Silently, for a management tool or a script:

  ```
  msiexec /i "Pirouette_<version>_x64_en-US.msi" /qn
  ```

**3. First launch:** SmartScreen warns about an unknown publisher. Check the file first ([Checking a download](#checking-a-download)), then click "More info", then "Run anyway". Or take the "downloaded from the internet" mark off the checked file before opening it, and SmartScreen doesn't ask: right-click it > Properties > tick **Unblock** > OK, or in PowerShell:

  ```
  Unblock-File .\Pirouette_<version>_x64-setup.exe
  ```

**4. Use it:** the app's icon sits in the taskbar's notification area (it may be under the ^ arrow). Hold Shift while dragging a file in File Explorer or on the desktop and the wheel opens. It says "Drop here" until the file reaches it, then shows the formats.

**Without a drag:** select files in File Explorer, right-click > **Send to > Pirouette** (on Windows 11 under "Show more options"): the wheel opens with the keyboard (arrows pick, Enter applies, Esc closes). The Send To entry is made at start; Settings > General switches it off.

**Update:** the app updates itself (menu > Install Update), and Open at Login stays as it was. By hand: run the new installer over the old one. The `.exe` then removes the old version first ("Uninstall before installing" is ticked); your settings stay, unless you tick "Delete the application data", but Open at Login is switched off, so turn it on again in the menu.

**Uninstall:** Settings > Apps > Installed apps > Pirouette > Uninstall. The `.exe`'s uninstaller also removes Open at Login. For the `.msi`, turn off Open at Login in the icon's menu first (its uninstaller doesn't remove it), then uninstall, or `msiexec /x "Pirouette_<version>_x64_en-US.msi" /qn`. Settings are in `%APPDATA%\io.github.oracola91.pirouette`, working files in `%LOCALAPPDATA%\pirouette`, the log in `%LOCALAPPDATA%\io.github.oracola91.pirouette\logs`.

## Linux

**One command (Ubuntu, Debian):** in a terminal on the desktop, as yourself (not root):

```sh
curl -fsSL https://raw.githubusercontent.com/oracola91/pirouette/main/scripts/install-linux.sh | sh
```

It asks for your password, then installs the converters (pandoc from pandoc's own releases, see below) and the newest release, turns on Open at Login, adds the keyboard shortcut Super+Shift+C on GNOME, and starts the app. Before installing the release it checks the release's signature, made with the project's update key (the same check the app makes on every update), and the version in it; a file that fails is not installed. To install a given version instead of the newest, add it at the end: `... | sh -s -- 0.2.0`. Running it again is safe: it installs what's missing and turns Open at Login back on, and keeps a shortcut key you changed. To read it before it runs: `curl -fsSLO <the same address>`, open `install-linux.sh`, then `sh install-linux.sh`.

The steps it takes, by hand:

**1. The converters** (Ubuntu and Debian):

```sh
sudo apt install ffmpeg imagemagick libreoffice mupdf-tools libarchive-tools
```

On Ubuntu 24.04 and later, iPhone photos (HEIC) also need libheif's HEVC add-ons, which ImageMagick doesn't bring:

```sh
sudo apt install libheif-plugin-libde265 libheif-plugin-x265
```

pandoc comes from [pandoc's own releases](https://github.com/jgm/pandoc/releases), not from Ubuntu or Debian. The app runs pandoc in its sandbox, which needs 2.15 or newer (Ubuntu 22.04 has 2.9), and Ubuntu's and Debian's builds can't write Word files there because they keep pandoc's data files outside the program:

```sh
curl -fsSLO https://github.com/jgm/pandoc/releases/download/3.12.1/pandoc-3.12.1-1-amd64.deb
echo "b4217ac7c54067c442f9d4cd38bcd0c184a43f3e7053431a12263a6f92a33ba2  pandoc-3.12.1-1-amd64.deb" | sha256sum -c
sudo apt install ./pandoc-3.12.1-1-amd64.deb
```

**2. The app:**

- **Ubuntu, Debian:** apt installs the downloaded file and whatever it needs:

  ```sh
  sudo apt install ./Pirouette_<version>_amd64.deb
  ```

  Start it from the app menu, or run `pirouette`.

- **Other Linux:** make the AppImage runnable and start it. Ubuntu 22.04 and later also need FUSE 2 for AppImages (`sudo apt install libfuse2`, or `libfuse2t64` on 24.04).

  ```sh
  chmod +x Pirouette_<version>_amd64.AppImage
  ./Pirouette_<version>_amd64.AppImage
  ```

**3. Use it:** how the wheel opens depends on the desktop session. `echo $XDG_SESSION_TYPE` says which one is running.

**Without a drag (any session):** in Files, right-click a file > **Open With** > Pirouette: the wheel opens with the keyboard (arrows pick, Enter applies, Esc closes).

- **x11:** hold Shift while dragging a file in the file manager (Files, Nemo, Caja, Thunar, Dolphin, PCManFM) or on the desktop.
- **wayland** (the default on current Ubuntu and Fedora): Wayland doesn't let apps watch the mouse, so the wheel opens from a keyboard shortcut. In Settings > Keyboard > View and Customize Shortcuts > Custom Shortcuts, add one with the command `pirouette --show` (for the AppImage: its full path, then ` --show`) and a key such as Super+Shift+C. Press it, then drag a file onto the wheel. Esc or a click in the middle closes it; it also closes by itself after 20 seconds.

The menu icon needs AppIndicator support: on by default in Ubuntu, an extension on plain GNOME.

**Update:** the app updates itself (menu > Install Update). For the `.deb` this installs as root, so it asks for your password: normally in the system's own dialog; where that isn't available, in a plain box titled "Authentication Required". Enter it only right after you chose Install Update. By hand: install the newer `.deb` the same way.

**Uninstall:** turn off Open at Login in the icon's menu (the package doesn't remove it), then `sudo apt remove pirouette` or delete the AppImage. Remove the Pirouette shortcut in Settings > Keyboard > View and Customize Shortcuts > Custom Shortcuts, and if nothing else needs them, the converters (`sudo apt remove pandoc mupdf-tools` and so on). Settings are in `~/.config/io.github.oracola91.pirouette`, working files in `~/.cache/pirouette`, the log in `~/.local/share/io.github.oracola91.pirouette/logs`.

**Known gaps:** Ubuntu's ImageMagick 6 refuses to write PDFs (a security policy) and may lack HEIC and AVIF support.

## Checking a download

Each release has `SHA256SUMS.txt`. In the folder with the downloads:

```sh
shasum -a 256 -c SHA256SUMS.txt --ignore-missing     # macOS
sha256sum -c SHA256SUMS.txt --ignore-missing         # Linux
```

On Windows, compare `Get-FileHash .\<file> -Algorithm SHA256` with the file's line in `SHA256SUMS.txt`.

## Why the files aren't signed

Pirouette is a personal, open-source project. A code-signing certificate that Windows SmartScreen and macOS Gatekeeper trust costs money every year, its key has to live on a hardware token or a paid cloud signing service (a CA/Browser Forum rule since June 2023), and Microsoft's own signing service takes individuals only in the USA and Canada. So the Windows installers are not signed, and the Mac app is signed only with the project's own certificate, which Gatekeeper doesn't know. The warnings say exactly that: the publisher is unknown, not that the file is harmful.

What protects a download instead:

- **The checksums.** Every release lists the SHA-256 of each file in `SHA256SUMS.txt`, made by the same GitHub build that made the files. Check the file before opening it (above). If it doesn't match, delete it.
- **The update signature.** Once installed, the app updates itself only with files signed with the project's update key, and checks that signature itself (see *What protects an update* in the README). Updates carry no download mark, so no warning appears for them.

Getting past the warning, for a file you checked:

- **Windows:** "More info" > "Run anyway" in the SmartScreen window, or before opening: right-click > Properties > **Unblock**, or `Unblock-File .\<file>` in PowerShell. This only removes the mark from that one file. Don't switch SmartScreen off for the whole computer.
- **macOS:** open it once, then System Settings > Privacy & Security > "Open Anyway", or `xattr -dr com.apple.quarantine "/Applications/Pirouette.app"` (macOS steps above).
- **Linux:** no warning. The `.deb` installs as usual; an AppImage only needs `chmod +x`.

## Making a release

1. On a new branch, raise the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json` (all three the same), and in the two lock files that repeat it: `package-lock.json` (twice, at the top) and `src-tauri/Cargo.lock` (the `pirouette` entry). The builds use the lock files as they are, so a version left behind there stops them.
2. Push the branch and merge it through a pull request: `main` takes changes only that way. Then tag the merged `main` and push the tag:

   ```sh
   git switch main && git pull
   git tag -a v0.2.0 -m "Pirouette 0.2.0"
   git push origin v0.2.0
   ```

   Release tags can't be moved or deleted once pushed, so a release with a mistake is fixed with the next version.

3. GitHub builds the Windows and Linux files (about 20 minutes), signs the update files in a separate step, and creates the release as a **draft** with them, `latest.json` (what installed copies read to find the update) and `SHA256SUMS.txt`. Installed copies don't see a draft. The build stops early if the tag and the version don't match.
4. On the release Mac, with the tag checked out and nothing uncommitted, run `./scripts/release-macos.sh`. It builds and signs the macOS files, adds them and their entry in `latest.json`, checks every system is listed, and publishes the release. It needs the update signing key (`~/.tauri/pirouette-updater.key`, password in the Keychain) and the code-signing certificate "Pirouette Signing" in the login keychain (`scripts/macos-signing-cert.sh` makes it once; on another Mac, import its .p12 backup).

A build without a release, for testing: Actions > Build > Run workflow, or `gh workflow run build.yml -f platform=windows` (`linux`, `both`). Its files stay on the run's page for 14 days.
