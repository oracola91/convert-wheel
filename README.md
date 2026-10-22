# Convert Wheel

Hold Shift while dragging a file. A wheel of formats appears under the cursor. Drop the file on a format to convert it.

This is our own build, written from scratch. The idea comes from what the paid Tangerine app for macOS visibly does. No Tangerine code, assets or binaries were used or inspected.

## Status

**Proof step (macOS).** The wheel appears on Shift+drag and reports which format a file was dropped on. Nothing is converted yet.

| Step | State |
|---|---|
| 1. Proof: Shift+drag shows the wheel, a drop hands over the file path | Built, waiting on a hands-on test |
| 2. Converter core (ffmpeg, ImageMagick, pandoc) | Not started |
| 3. Finished wheel | Not started |
| 4. Menu bar app: start at login, notifications | Quit menu only |
| 5. Testing with real files | Not started |
| 6. Install | Not started |

Windows and Linux come after macOS. The trigger in `src-tauri/src/trigger.rs` is the only part that differs per OS.

## Run it

Needs Rust, Node.js and the Xcode Command Line Tools. The converter step will also need `ffmpeg`, `imagemagick` and `pandoc` from Homebrew.

```sh
npm install --ignore-scripts
cd src-tauri
cargo build
./target/debug/convert-wheel
```

The app has no Dock icon. It shows an orange icon in the menu bar, and that menu has Quit.

## Test the proof step

1. Start it from Terminal as above and keep the Terminal window visible.
2. Drag a file in Finder. While dragging, hold Shift. The wheel should appear under the cursor.
3. Move over the segments. The one under the cursor turns orange.
4. Drop on a segment. Terminal prints `convert-wheel: 1 file(s) -> png: [...]` and the wheel closes.
5. Drop in the middle, or let go anywhere else. Nothing happens, and the wheel closes.

Also worth trying: several files at once, a file with no extension, dragging text from a browser (no wheel should appear), and a second screen.

## How it works

- **Trigger** (`src-tauri/src/trigger.rs`): every 30 ms it reads the drag pasteboard's change count, the mouse buttons and the Shift key. These are shared by the window server, so no permission prompt is needed. A new drag with Shift held shows the wheel at the cursor.
- **Formats** (`src-tauri/src/formats.rs`): picks what the dragged files can become. With several files, only formats they all support are shown.
- **Wheel** (`src/`): plain HTML, CSS and JavaScript in a transparent window. No frontend framework and no npm runtime packages.

## Layout

```
src/                 wheel UI (index.html, wheel.css, wheel.js)
src-tauri/           Rust app
  src/main.rs        app setup, menu bar icon, drop handler
  src/trigger.rs     Shift+drag detection (macOS)
  src/formats.rs     formats offered per file type, with tests
  tauri.conf.json    window and bundle settings
```
