# Installing Convert Wheel

Every version is on the repo's [Releases page](https://github.com/oracola91/convert-wheel/releases). The repo is private, so sign in to GitHub with an account that has access.

| System | File |
|---|---|
| macOS, Apple silicon | `Convert.Wheel_<version>_aarch64.dmg` |
| Windows, one user, no admin rights | `Convert.Wheel_<version>_x64-setup.exe` |
| Windows, all users, Intune | `Convert.Wheel_<version>_x64_en-US.msi` |
| Ubuntu, Debian | `Convert.Wheel_<version>_amd64.deb` |
| Other Linux | `Convert.Wheel_<version>_amd64.AppImage` |

None of the files is code-signed yet, so each system warns once on first launch. The steps below say how to get past that.

Convert Wheel does the converting with four free tools that are installed separately: **ffmpeg** (audio, video), **ImageMagick** (images), **pandoc** (documents) and **LibreOffice** (PDF and office files). Install them first; the commands are in each section. A format whose tool is missing fails with a notification that names the install command.

## macOS

**1. The converters** ([Homebrew](https://brew.sh) needed):

```sh
brew install ffmpeg imagemagick pandoc
brew install --cask libreoffice
```

**2. The app:** open the `.dmg` and drag Convert Wheel onto Applications.

**3. First launch:** macOS blocks apps that aren't signed by an Apple developer. Open Convert Wheel once, then go to System Settings > Privacy & Security and click "Open Anyway" next to Convert Wheel. Or, in Terminal:

```sh
xattr -dr com.apple.quarantine "/Applications/Convert Wheel.app"
```

**4. Use it:** a ring icon appears in the menu bar (top right); there is no Dock icon. Hold Shift while dragging a file in Finder and the wheel opens. Allow notifications when macOS asks. Open at Login is in the ring icon's menu.

**From the source instead:** `./scripts/install-macos.sh` builds the app on this Mac, installs it in Applications and starts it. A local build needs no Gatekeeper step.

**Update:** install the new `.dmg` the same way, or run the script again.

**Uninstall:** turn off Open at Login in the menu, quit, and delete Convert Wheel from Applications. Its settings and working files are in `~/Library/Application Support/local.convertwheel` and `~/Library/Caches/local.convertwheel`.

## Windows

**1. The converters**, in a terminal:

```
winget install Gyan.FFmpeg ImageMagick.ImageMagick JohnMacFarlane.Pandoc TheDocumentFoundation.LibreOffice
```

Sign out and back in afterwards, so apps started from the Start menu see the new tools.

**2. The app**, one of:

- **`.exe`**: double-click it. It installs for the signed-in user, without admin rights.
- **`.msi`**: installs for all users in Program Files and needs admin rights. Silently, for Intune or a script:

  ```
  msiexec /i "Convert.Wheel_<version>_x64_en-US.msi" /qn
  ```

**3. First launch:** SmartScreen warns about an unknown publisher. Click "More info", then "Run anyway".

**4. Use it:** the app's icon sits in the taskbar's notification area (it may be under the ^ arrow). Hold Shift while dragging a file in File Explorer or on the desktop and the wheel opens. It says "Drop here" until the file reaches it, then shows the formats.

**Update:** run the new installer over the old one.

**Uninstall:** turn off Open at Login in the icon's menu first, then Settings > Apps > Installed apps > Convert Wheel > Uninstall (for the `.msi`: `msiexec /x "Convert.Wheel_<version>_x64_en-US.msi" /qn`). Settings are in `%APPDATA%\local.convertwheel`, working files in `%LOCALAPPDATA%\convertwheel`.

## Linux

**1. The converters** (Ubuntu and Debian):

```sh
sudo apt install ffmpeg imagemagick pandoc libreoffice
```

pandoc must be 2.15 or newer: check with `pandoc --version`. Ubuntu 24.04 is fine; Ubuntu 22.04 ships 2.9, so take a newer `.deb` from [pandoc's releases](https://github.com/jgm/pandoc/releases).

**2. The app:**

- **Ubuntu, Debian:** apt installs the downloaded file and whatever it needs:

  ```sh
  sudo apt install ./Convert.Wheel_<version>_amd64.deb
  ```

  Start it from the app menu, or run `convert-wheel`.

- **Other Linux:** make the AppImage runnable and start it. Ubuntu 22.04 and later also need FUSE 2 for AppImages (`sudo apt install libfuse2`, or `libfuse2t64` on 24.04).

  ```sh
  chmod +x Convert.Wheel_<version>_amd64.AppImage
  ./Convert.Wheel_<version>_amd64.AppImage
  ```

**3. Use it:** how the wheel opens depends on the desktop session. `echo $XDG_SESSION_TYPE` says which one is running.

- **x11:** hold Shift while dragging a file in the file manager (Files, Nemo, Caja, Thunar, Dolphin, PCManFM) or on the desktop.
- **wayland** (the default on current Ubuntu and Fedora): Wayland doesn't let apps watch the mouse, so the wheel opens from a keyboard shortcut. In Settings > Keyboard > View and Customize Shortcuts > Custom Shortcuts, add one with the command `convert-wheel --show` (for the AppImage: its full path, then ` --show`) and a key such as Super+Shift+C. Press it, then drag a file onto the wheel. Esc or a click in the middle closes it; it also closes by itself after 20 seconds.

The menu icon needs AppIndicator support: on by default in Ubuntu, an extension on plain GNOME.

**Update:** install the newer `.deb` the same way.

**Uninstall:** turn off Open at Login in the icon's menu, then `sudo apt remove convert-wheel` (or delete the AppImage). Settings are in `~/.config/local.convertwheel`, working files in `~/.cache/convertwheel`.

**Known gaps:** Ubuntu's ImageMagick 6 refuses to write PDFs (a security policy) and may lack HEIC and AVIF support.

## Checking a download

Each release has `SHA256SUMS.txt`. In the folder with the downloads:

```sh
shasum -a 256 -c SHA256SUMS.txt --ignore-missing     # macOS
sha256sum -c SHA256SUMS.txt --ignore-missing         # Linux
```

On Windows, compare `Get-FileHash .\<file> -Algorithm SHA256` with the file's line in `SHA256SUMS.txt`.

## Making a release

1. Raise the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json` (all three the same).
2. Commit, then tag and push the tag:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

3. GitHub builds the Windows and Linux files (about 20 minutes) and publishes the release with them and `SHA256SUMS.txt`. The build stops early if the tag and the version don't match.
4. On a Mac, add the `.dmg`: `./scripts/release-macos.sh`.

A build without a release, for testing: Actions > Build > Run workflow, or `gh workflow run build.yml -f platform=windows` (`linux`, `both`). Its files stay on the run's page for 14 days.
