# Convert Wheel

Hold Shift while dragging a file. A wheel of formats appears under the cursor. Drop the file on a format to convert it.

This is our own build, written from scratch. The idea comes from what the paid Tangerine app for macOS visibly does. No Tangerine code, assets or binaries were used or inspected.

**To install it, see [INSTALL.md](INSTALL.md).** Installers for every system are on the [Releases page](https://github.com/oracola91/convert-wheel/releases).

## Status (macOS)

| Step | State |
|---|---|
| 1. Proof: Shift+drag shows the wheel, a drop hands over the file path | Done, tested by hand |
| 2. Converter core (ImageMagick, ffmpeg, pandoc, LibreOffice) | Done. All 49 format pairs pass from the command line; Word to PDF tested by hand |
| 3. Finished wheel: glass, motion, hub icon, haptics, screen edges | Done, tested by hand |
| 4. Menu bar app: Open at Login, where files go, which formats show | Done, tested by hand |
| 5. Testing with real files | In daily use |
| 6. Install as an app | Done: `scripts/install-macos.sh` |

**Windows** and **Linux:** built on GitHub (`.exe`, `.msi`, `.deb`, `.AppImage`), waiting on hands-on tests.

## What converts to what

| Files | Can become | Done by |
|---|---|---|
| Images: JPG, PNG, HEIC, WebP, TIFF, BMP, GIF, AVIF | JPG, PNG, HEIC, WebP, TIFF, GIF, AVIF, PDF | ImageMagick |
| Audio: MP3, WAV, M4A, AAC, FLAC, AIFF, OGG, Opus | MP3, M4A, WAV, FLAC, AIFF, OGG | ffmpeg |
| Video: MP4, MOV, M4V, MKV, WebM, AVI | MP4, MOV, WebM, GIF, MP3 (sound only) | ffmpeg |
| Documents: DOCX, DOC, ODT, RTF, Markdown, HTML, TXT, EPUB | PDF, DOCX, ODT, HTML, Markdown, TXT, EPUB | LibreOffice for PDF and office formats, pandoc for the rest |
| Spreadsheets: XLSX, XLS, ODS, CSV | PDF, XLSX, ODS, CSV (first sheet) | LibreOffice |
| Presentations: PPTX, PPT, ODP | PDF, PPTX, ODP | LibreOffice |

The wheel only shows formats that every dragged file can become. The new file lands next to the original. If the name is taken it becomes `name 2.ext`, `name 3.ext` and so on. Nothing is ever overwritten.

A notification says when a drop is done or why it failed. Video also gets a "Converting…" notification first, because it can take a while.

## On Windows and Linux

Install steps are in [INSTALL.md](INSTALL.md). What works differently from macOS:

**Windows** (`src-tauri/src/win32.rs`)

- **Trigger:** Windows has no shared drag pasteboard to watch, so the wheel opens when a drag that started in File Explorer, on the desktop or in an Open/Save dialog has Shift held. Which files are dragged becomes known when the drag reaches the wheel; until then it says "Drop here".
- **Drop target:** our own, on the wheel window and the windows WebView2 creates inside it. It always reports "copy" (or "refuse" in the middle). Tauri's built-in drop handler leaves the result unset, so Windows hands File Explorer the source's own allowed effects back, which can include "move" and so delete the original. That is why it is not used here.
- **Look:** no system glass, so the page draws a solid disc with a shadow. No file icon in the middle.
- **Open at Login:** a value under the user's Run key in the registry.
- **No console windows:** the converters run without flashing a console.

**Linux** (`src-tauri/src/linux.rs`)

- **X11:** like Windows. A drag that starts in a file manager (Files/Nautilus, Nemo, Caja, Thunar, Dolphin, PCManFM) or on the desktop, with Shift held, opens the wheel at the cursor.
- **Wayland:** apps can't watch the pointer, so the wheel opens from a keyboard shortcut running `convert-wheel --show`, which reaches the running app over a socket in `XDG_RUNTIME_DIR`. That wheel waits for a file; Esc, a click in the middle or 20 seconds close it.
- **Drop target:** our own GTK handlers instead of WebKit's. They answer "copy" over a format and nothing elsewhere, never "move" even with Shift held, and never ask the file manager to delete the original.
- **Menu bar icon:** shown through AppIndicator.
- **Open at Login:** an entry in `~/.config/autostart`.
- **Converter:** falls back to ImageMagick 6's `convert` and the `libreoffice` command, checks pandoc is 2.15 or newer, and Aptos stands in as Noto Sans.

## The menu

The ring icon in the menu bar opens:

- **Open at Login**: adds or removes Convert Wheel in System Settings > General > Login Items. Only works for the installed app.
- **Save Converted Files**: next to the original (default) or in Downloads. When the original's folder can't be written to, the file goes to Downloads either way and the notification says so.
- **Formats on the Wheel**: tick or untick formats per kind of file. Unticked formats stay off the wheel.
- **Quit Convert Wheel**.

Settings are kept in `~/Library/Application Support/local.convertwheel/settings.json`.

## Run it from the source

Needs Rust, Node.js and the Xcode Command Line Tools, plus the converters:

```sh
brew install ffmpeg imagemagick pandoc
brew install --cask libreoffice
```

Then:

```sh
npm install --ignore-scripts
cd src-tauri
cargo build
./target/debug/convert-wheel
```

The app has no Dock icon. It shows an orange icon in the menu bar, and that menu has Quit.

### Without the wheel

The same converter runs from the command line. It prints the path of each new file:

```sh
./target/debug/convert-wheel --convert pdf "report.docx" "budget.xlsx"
```

## Test it by hand

1. Start it from Terminal as above.
2. Drag a file in Finder and hold Shift while dragging. The wheel appears under the cursor.
3. Over the wheel, the dragged file shrinks to a small icon so its name doesn't cover the wheel. Move over the segments. The one under the cursor turns orange, moves out toward the cursor, and the middle says what it will become ("To PDF"). On a Force Touch trackpad each new segment gives a light tick. Over the middle it says "Cancel".
4. Drop on a segment. The new file appears next to the original and a notification confirms it.
5. Drop in the middle. The file slides back and nothing happens. Outside the wheel the drop works as usual in macOS: it slides back, or Finder moves the file if you let go over a folder.

Also worth trying: several files at once, a file with no extension, text dragged from a browser (no wheel should appear), a second screen, and LibreOffice open at the same time.

## Safety

- Tools get the file paths as separate arguments, never through a shell, so odd file names can't run commands.
- ImageMagick is told the real image format, read from the file's first bytes. Only plain raster formats are accepted, so a disguised SVG or PostScript file is refused instead of decoded.
- pandoc runs with `--sandbox`. It reads only the file it was given, never the network or other files.
- Office files must really be zip archives. Otherwise LibreOffice would quietly turn a broken file into a PDF of garbage.
- LibreOffice uses its own profile in `~/Library/Caches/local.convertwheel`, so it doesn't interfere with a LibreOffice window you have open.
- Each conversion works in its own temporary folder, which is deleted afterwards whether it succeeded or not.

## Fonts in PDFs

LibreOffice running without a window only sees its own bundled fonts. The converter gives it a font list (`~/Library/Caches/local.convertwheel/fontconfig/fonts.conf`) with the macOS font folders, so documents keep their fonts.

Word's default font since 2023, Aptos, ships inside Word only, and its licence limits it to Microsoft products. So it is not handed to LibreOffice. Helvetica Neue, which comes with macOS, stands in for it. If Aptos is ever properly installed on the Mac, the real font is used instead. Calibri and Cambria need nothing, because LibreOffice ships look-alikes with the same spacing.

## How it works

- **Trigger** (`src-tauri/src/trigger.rs`): every 30 ms it reads the drag pasteboard's change count, the mouse buttons and the Shift key. These are shared by the window server, so no permission prompt is needed.
- **Wheel window** (`src-tauri/src/wheel_window.rs`): the native parts. Frosted glass behind the ring (the system material, so it follows light/dark mode and Reduce Transparency), the dragged file's icon in the middle, placement at the cursor kept inside the screen, and the fade in and out.
- **Drop target** (`src-tauri/src/drop_view.rs`): a transparent native view on top of the wheel takes the drop. It shrinks the dragged file to a small icon, tells the page which segment to light up, and queues the conversion.
- **Formats** (`src-tauri/src/formats.rs`): what each kind of file can become.
- **Converter** (`src-tauri/src/convert.rs`): picks the tool and its settings, then places the result safely.
- **Queue** (`src-tauri/src/main.rs`): one worker converts drops in order, then posts the notification.
- **Wheel** (`src/`): plain HTML, CSS and JavaScript in a transparent window. No frontend framework and no npm runtime packages. It opens and closes along the same path with no bounce, and follows Reduce Motion, Increase Contrast and dark mode.

## Layout

```
src/                       wheel UI (index.html, wheel.css, wheel.js)
src-tauri/                 Rust app
  src/main.rs              app setup, drop queue, notifications, --convert, --show
  src/trigger.rs           Shift+drag on macOS; opening, closing and dropping for all
  src/wheel.rs             wheel geometry: which segment is under a point
  src/formats.rs           formats offered per file type
  src/convert.rs           the conversions
  src/menu.rs              menu bar icon and menu
  src/settings.rs          saved choices (where files go, hidden formats)
  src/login.rs             Open at Login on each system
  src/drop_view.rs         macOS: the drop target over the wheel
  src/wheel_window.rs      macOS: glass, hub icon, placement, fades
  src/win32.rs             Windows: trigger, placement, drop target
  src/linux.rs             Linux: X11 trigger, Wayland shortcut, placement, drop target
  tauri.conf.json          window and bundle settings
  tauri.windows.conf.json  Windows window settings and installer types
  tauri.linux.conf.json    Linux window settings and package types
  Info.plist               marks it a menu bar app on macOS (no Dock icon)
.github/workflows/build.yml  Windows and Linux builds; a version tag publishes a release
scripts/install-macos.sh   build and install in /Applications on this Mac
scripts/release-macos.sh   add the macOS .dmg to a release
INSTALL.md                 installing, updating, uninstalling, making a release
```
