# Convert Wheel

Hold Shift while dragging a file. A wheel of formats appears under the cursor. Drop the file on a format to convert it.

This is our own build, written from scratch. The idea comes from what the paid Tangerine app for macOS visibly does. No Tangerine code, assets or binaries were used or inspected.

## Status (macOS)

| Step | State |
|---|---|
| 1. Proof: Shift+drag shows the wheel, a drop hands over the file path | Done, tested by hand |
| 2. Converter core (ImageMagick, ffmpeg, pandoc, LibreOffice) | Done. All 49 format pairs pass from the command line; a drop on the wheel still needs a hands-on test |
| 3. Finished wheel | Not started |
| 4. Menu bar app: start at login, settings | Quit menu only |
| 5. Testing with real files | Not started |
| 6. Install as an app | Not started |

Windows and Linux come after macOS. The trigger in `src-tauri/src/trigger.rs` is the only part that differs per OS.

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

## Run it

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
3. Move over the segments. The one under the cursor turns orange.
4. Drop on a segment. The new file appears next to the original and a notification confirms it.
5. Drop in the middle, or let go anywhere else. Nothing happens, and the wheel closes.

Also worth trying: several files at once, a file with no extension, text dragged from a browser (no wheel should appear), a second screen, and LibreOffice open at the same time.

## Safety

- Tools get the file paths as separate arguments, never through a shell, so odd file names can't run commands.
- ImageMagick is told the real image format, read from the file's first bytes. Only plain raster formats are accepted, so a disguised SVG or PostScript file is refused instead of decoded.
- pandoc runs with `--sandbox`. It reads only the file it was given, never the network or other files.
- Office files must really be zip archives. Otherwise LibreOffice would quietly turn a broken file into a PDF of garbage.
- LibreOffice uses its own profile in `~/Library/Caches/local.convertwheel`, so it doesn't interfere with a LibreOffice window you have open.
- Each conversion works in its own temporary folder, which is deleted afterwards whether it succeeded or not.

## How it works

- **Trigger** (`src-tauri/src/trigger.rs`): every 30 ms it reads the drag pasteboard's change count, the mouse buttons and the Shift key. These are shared by the window server, so no permission prompt is needed.
- **Formats** (`src-tauri/src/formats.rs`): what each kind of file can become.
- **Converter** (`src-tauri/src/convert.rs`): picks the tool and its settings, then places the result safely.
- **Queue** (`src-tauri/src/main.rs`): one worker converts drops in order, then posts the notification.
- **Wheel** (`src/`): plain HTML, CSS and JavaScript in a transparent window. No frontend framework and no npm runtime packages.

## Layout

```
src/                 wheel UI (index.html, wheel.css, wheel.js)
src-tauri/           Rust app
  src/main.rs        app setup, menu bar icon, drop queue, notifications, --convert
  src/trigger.rs     Shift+drag detection (macOS)
  src/formats.rs     formats offered per file type
  src/convert.rs     the conversions
  tauri.conf.json    window and bundle settings
```
