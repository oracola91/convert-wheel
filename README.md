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
| Images: JPG, PNG, HEIC, WebP, TIFF, BMP, GIF, AVIF | JPG, PNG, HEIC, WebP, TIFF, GIF, AVIF, BMP, ICO, PDF | ImageMagick |
| Audio: MP3, WAV, M4A, AAC, FLAC, AIFF, OGG, Opus, WMA | MP3, M4A, WAV, FLAC, AIFF, OGG, Opus, WMA | ffmpeg |
| Video: MP4, MOV, M4V, MKV, WebM, AVI, WMV | MP4, MOV, WebM, MKV, AVI, WMV, GIF, MP3 (sound only) | ffmpeg |
| Documents: DOCX, DOC, ODT, RTF, Pages, Markdown, HTML, TXT, EPUB | PDF, DOCX, ODT, HTML, Markdown, TXT, EPUB | LibreOffice for PDF and office formats, pandoc for the rest |
| Spreadsheets: XLSX, XLS, ODS, CSV, Numbers | PDF, XLSX, ODS, CSV (first sheet) | LibreOffice |
| Presentations: PPTX, PPT, ODP, Keynote | PDF, PPTX, ODP | LibreOffice |
| PDF | JPG, PNG (a picture of each page), TXT, DOCX (the text only) | MuPDF (`mutool`); pandoc for DOCX |
| Subtitles: SRT, VTT | SRT, VTT | ffmpeg |

An icon (ICO) holds the picture at 256, 128, 64, 48, 32 and 16 pixels, fitted into a square with see-through edges rather than stretched. MKV is a fast copy of the streams when they fit, H.264 otherwise; AVI is MPEG-4 Part 2 (what Xvid makes) with MP3, and WMV is Windows Media 8 with WMA, for older players, at most 1920 pixels on the long side. Pages, Numbers and Keynote files go through LibreOffice, which reads them; a file saved as a package folder (older versions) is refused: save it as a single file. Subtitles should be UTF-8.

The wheel only shows formats that every dragged file can become. The new file lands next to the original. If the name is taken it becomes `name 2.ext`, `name 3.ext` and so on. Nothing is ever overwritten.

A PDF with several pages becomes a folder of pictures next to it, `report (PNG)` holding `report 1.png`, `report 2.png` and so on, at 150 dpi; at most the first 100 pages, and the folder name says so when there were more. Text is taken from PDFs of up to 2000 pages. As text or a Word file, a PDF keeps its words and paragraphs but not its layout, tables or pictures. A scanned PDF has no text to take out. A PDF that needs a password to open is refused with a note to save a copy without it. For a Word file that keeps the layout, open the PDF in Word itself.

A notification says when a drop is done or why it failed, and how the size changed ("2.2 MB → 410 KB · photo (compressed).jpg"; not for folders, such as a split PDF). A click on it opens the folder with the new files selected (on Windows only while the notification is on screen, not from the Action Center; on macOS also later from Notification Center). Video also gets a "Converting…" notification first, because it can take a while. Meanwhile the menu shows how far a drop is, and **Stop Converting** ends it.

## The Tools wheel

Hold Option as well as Shift while dragging (Alt+Shift on Windows and on Linux with X11) and a second wheel opens: what can be done to the file while it keeps its format. The result goes next to the original, named after the tool ("photo (half size).jpg"), and never replaces it. For pictures (JPEG, PNG, HEIC, WebP, AVIF and TIFF; BMP gets Half Size and Rotate only; animated GIFs none):

- **Compress**: JPEG at quality 70; WebP and AVIF at 55, HEIC at 45 (its numbers run higher); PNG down to 256 colours (dithered); TIFF with lossless ZIP. A result that isn't smaller is not saved, and the notification says so.
- **Half size**: half the width and height.
- **Remove metadata**: takes out EXIF (camera, date, location), XMP, IPTC and comments, and keeps the colour profile. The picture is turned upright first, so it looks the same without the EXIF turn.
- **Rotate**: a quarter turn clockwise.
- **QR code** (GIFs too, their first frame): reads every QR code in the picture. The text is saved next to it ("photo (QR code).txt"), copied to the clipboard and shown in the notification; a link in it is never opened. Read by [rqrr](https://github.com/WanzenBug/rqrr), in the app itself.
- **Adjust** (one picture at a time): opens a window with the picture and sliders for brightness (50 to 150%), contrast (50 to 150%) and saturation (0 to 200%), shown on the picture as they move; Reset puts them back, Enter adjusts, Esc cancels. ImageMagick then does the same steps as the preview, in the same order: "photo (adjusted).jpg". The window only sends the three numbers back.
- **Mark up** (one picture at a time): opens a window with the picture to draw on: a pen, arrows, boxes and words, in red, yellow, blue, green, black or white, thin or thick, with Undo (Cmd/Ctrl+Z). **Black out** covers a part with solid black. The window only sends the shapes back; the app checks each one and ImageMagick draws them, the blacked-out parts last so nothing lands on top of them. A picture with a blacked-out part also loses its metadata, keeping only the colour profile: the small preview a camera keeps in the EXIF would otherwise still show what was covered. Words are drawn in a font the system has (Arial or Helvetica on macOS, Arial on Windows, DejaVu, Liberation or Noto Sans on Linux). The result is "photo (marked up).jpg".
- **Crop** (one picture at a time): opens a window with the picture and a box to drag by its middle, corners and sides. Free, the picture's own shape, square, 4:3, 3:2 or 16:9; turn it left or right; Enter crops, Esc cancels. The result is "photo (cropped).jpg". The window only sends the box back: which file it applies to stays in the app. A video can be cropped the same way (one at a time), on a typical frame of it; the video is turned and cropped, then encoded again in its own format at the menu's quality: "clip (cropped).mp4".

The other tools keep the metadata. Every tool saves the picture again: JPEG at its own quality, HEIC, WebP and AVIF at 80.

**One PDF** (two pictures or more dropped together): a PDF with one picture per page, in the order dropped, next to the first: "IMG_1234 (one PDF).pdf". Each page is as long as an A4 page on its long side (a picture under 842 pixels keeps a smaller page); photos go in as JPEG, screenshots and other pictures as PNG. Up to 200 pictures. Needs ImageMagick and MuPDF.

**Collage** (two pictures or more dropped together): one picture with all of them in a grid on white, in the order dropped, each fitted into a square of its own, next to the first: "IMG_1234 (collage).jpg". Two make one row, five make a row of three and a row of two (a short last row sits in the middle). Up to 36 pictures. Needs ImageMagick.

For PDFs (with MuPDF):

- **Merge** (two PDFs or more dropped together): one PDF with all their pages, in the order dropped, next to the first: "report (merged).pdf". Up to 2000 pages in all. A PDF that MuPDF can't read stops the merge with its name, instead of being left out quietly.
- **Split**: one PDF per page in a new folder, "report (pages)/report 01.pdf"; the first 100 pages, and the folder name says so when there were more.
- **Reorder** (one PDF of 2 to 100 pages at a time): opens a window with a small picture of every page, to drag into a new order. A page picked with a click or the arrow keys also moves with Shift and the arrow keys; Reverse turns the whole order round, Reset puts it back. Enter reorders, Esc cancels. Bookmarks follow their pages, and the title and author stay: "report (reordered).pdf". The window only sends the new order back.
- **Smaller**: pictures above 225 dpi scaled down to 150 dpi and photos saved again as JPEG at quality 60, each only where that is smaller; fonts and streams compressed, unused and duplicate parts dropped. Text stays text. Older MuPDF without the picture options (Ubuntu's own packages) only tidies the file, which seldom makes it much smaller. Not saved if it isn't smaller.

For video and audio (with ffmpeg):

- **Compress** (MP4, MOV, M4V, MKV, WebM): at most 1920 pixels on the long side; H.264 at CRF 28 with AAC 128k, or for WebM VP9 at CRF 38 with Opus 96k. Not saved if it isn't smaller.
- **Remove metadata** (video and audio): a copy of the picture and sound without the file's and the streams' metadata (where and when it was recorded, the device, tags, cover art, chapters) or the data tracks phones keep it in. Nothing is encoded again, so it is quick.
- **First frame** (video): the first frame as a JPEG: "clip (first frame).jpg".
- **Loudness** (audio): evened out to -16 LUFS with peaks at most -1.5 dB (ffmpeg's loudnorm), at the file's own sample rate, tags kept.
- **Mono** (audio): both channels mixed into one.
- **Compress** (MP3, M4A, AAC, Ogg, Opus, WMA, FLAC): the same format at a lower bit rate (MP3, M4A and WMA 96k, Ogg and Opus 64k), tags kept; FLAC stays lossless at its strongest packing. Not saved if it isn't smaller. WAV and AIFF have nothing to compress with.
- **To a size** (JPEG, HEIC, WebP, AVIF pictures; MP4, MOV, M4V, MKV, WebM videos; MP3, M4A, AAC, Ogg, Opus, WMA sound; PDFs; one file at a time): a small window asks for a size ("2 MB", or Half, Quarter, 25 MB, 10 MB, ...) and the result comes in at that or under: "photo (under 2 MB).jpg". Pictures get the highest quality that fits (found by halving 25 to 92), and are made smaller only if even the lowest is too big. Video gets the bit rate the size allows over its length, in two passes, with a smaller picture as the rate drops (1920, 1280, 854); sound gets the bit rate that fits. A PDF is tidied like Smaller and its pictures are saved again, step by step from 150 dpi at JPEG quality 70 down to 50 dpi at 25, until it fits; text stays text, so a PDF of text alone may not get much smaller (needs a MuPDF with the picture options, not Ubuntu 22.04's). The window says how small a long file can go.
- **Join** (two or more sound files, or two or more videos): one after another, in the order dropped, as one file in the first one's format: "clip (joined).mp4". Every part is encoded again, so different formats, sizes and rates fit: videos take the first one's size (the others fitted in, with bars) and frame rate, and a video without sound gets silence for its length. Up to 50 files.
- **Trim** (audio and video, one file at a time): opens a window with the sound's loudness along its length, a video's frames above it, and the part to keep between two ends to drag (or times to type, "1:23.4"). Play plays that part first (files up to 20 minutes). Enter trims, Esc cancels, Space plays. The cut is exact, not at the nearest keyframe: the part is encoded again in the file's own format at the menu's quality. The result is "clip (trimmed).mp4".
- **Bleep** (audio and video, one file at a time): opens the Trim window to pick the part to bleep out (a word, a name), with Play to find it. That part of the sound becomes a 1 kHz tone; the rest stays as it was, and a video's picture is copied as it is, not encoded again. The result is "clip (bleeped).mp4".
- **Speed** (audio and video, one file at a time): opens a small window with the speeds 0.25x, 0.5x, 0.75x, 1.25x, 1.5x, 2x and 4x (arrow keys move between them) and how long the file will play at the one picked. The sound keeps its pitch. Encoded again in the file's own format at the menu's quality: "clip (2x speed).mp4".
- **Split** (audio and video, one file at a time): opens a small window to pick how many equal parts (2 to 10), or how long each part is (10 seconds to 30 minutes). Each part is cut exactly, like Trim, and they go in a new folder next to the file: "clip (parts)/clip 1.mp4", "clip 2.mp4", ... Up to 50 parts of a second or more; a few moments left at the end go onto the last part.
- **Info** (pictures, sound, video and PDFs, one file at a time): opens a window with what the file says about itself. A picture: its size, when it was taken, the camera and lens, and where it was taken (or "none in the file"). A video: its tags, when and on what it was recorded, and where. A PDF: its pages, title, author, what made it and its dates. A sound file's title, artist, album, year, genre and comment, and a PDF's title, author, subject and keywords, can be changed: they go into a copy, "song (new info).mp3", with everything else as it was (ffmpeg copies the sound, nothing is encoded again). A picture's and a video's details are only shown. Older MuPDF (Ubuntu 22.04's) only shows a PDF's details too.

Loudness and Mono save the audio again in its own format (MP3 VBR quality 2, AAC 256k, Opus 160k; WAV and AIFF as 16-bit). On Wayland, Shift+drag (with Option or Control for these wheels) needs GNOME and its helper extension (see Linux below); the keyboard shortcut opens only the Formats wheel.

## The Actions wheel

Hold Control+Shift while dragging and a third wheel opens: what can be done with the dragged files themselves, whatever they are, folders too. None of it changes a file.

- **Copy path**: the full path on the clipboard, one per line for several files.
- **Show**: opens the folder with the files selected.
- **Copy to** and **Move to**: the system's folder picker opens (where the last one went), and the files and folders go into the folder chosen. Nothing there is replaced: a name that is taken gets a number ("photo 2.jpg"). Move renames on the same disk; onto another disk it copies, and removes the original only once the copy is complete. Links are copied as links, never followed.
- **Terminal**: a terminal in the file's folder (in a dragged folder itself): Terminal on macOS, PowerShell on Windows, GNOME Terminal, Console, Konsole or the system's default on Linux. The folder is handed over as the starting folder, never inside a command line.
- **AirDrop** (macOS): the AirDrop window for the files.

- **ZIP**, **TAR.GZ**, **TAR**: an archive of everything dragged, files and folders, next to the first: "Photos.zip", "notes.txt.zip", or "Archive.zip" for several.
- **Unpack** (when every dragged file is a ZIP, TAR, TAR.GZ, GZ or RAR): what the archive holds at the top goes next to it, when that is one folder or file, as Finder's Archive Utility does it; otherwise a new folder named after the archive. Nothing already there is replaced ("Photos 2").

Archives go through bsdtar (libarchive): built into macOS and into Windows 10 and later (tar.exe), `libarchive-tools` on Linux (the setup script installs it). It refuses names with ".." and strips a leading "/", so nothing lands outside the new folder; an archive that does try is refused as a whole and leaves nothing behind. Unpacking may write at most 1 GB plus twenty times the archive's size, so a "zip bomb" is stopped. Unpacking first goes into a hidden folder next to the archive, so a half-done unpack is never mistaken for a finished one. A password-protected archive is refused. A plain .gz (one file, not a TAR) is unpacked by the app itself, within the same budget.

On a Mac, start the drag first and then press Control+Shift: Control held before the click turns it into a right-click. Copying and moving files and sharing on Windows come later ([ROADMAP](ROADMAP.md), stage 3). For moving files by hand there is the shelf.

## The shelf (macOS)

Drag files toward the top of the screen and the shelf opens to take them: out of the notch on a MacBook, under the ring icon on other screens (in the top right corner when the icon can't be seen). Drop them there, go to the folder where they belong, and drag them out of the shelf again.

- Holding files only remembers where they are. Nothing is copied or moved until they are dragged out, and the drag's source is told it was a copy, so it never removes the originals.
- Dragged out, Finder does what it does with its own drags: moves them on the same disk, copies them to another one, and Option makes a copy. Files that landed somewhere leave the shelf. The shelf itself never moves or deletes anything.
- While it holds files a small sign stays: the newest file's icon and how many, beside the notch or in a pill under the ring icon. Point at it to see them, click to show them in Finder, and the × lets go of them all.
- What it holds survives a restart (`shelf.json` next to the settings, the paths only). A file that was moved, renamed or deleted in the meantime is dropped from the shelf, never guessed at.
- With Shift held the drag belongs to the wheel and the shelf steps aside.
- To keep it away, untick **Shelf at the Top of the Screen** in the menu.

Windows (above the clock) and Linux follow.

### Switching wheels in one drag

Once a wheel is open, the keys keep choosing: add Option (Alt) and it becomes the Tools wheel, press Control instead and it becomes the Actions wheel, keep only Shift and it is the formats again. With no keys held the wheel stays as it is, so they can be let go on the way to a segment. On Linux this works with X11; the Wayland shortcut opens the formats wheel only.

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
- **Wayland on GNOME:** GNOME Shell can see drags, so the app installs a small GNOME Shell extension of its own (`src-tauri/gnome-shell/`, `src-tauri/src/gnome_shell.rs`) into `~/.local/share/gnome-shell/extensions` and turns it on the first time. While a drag goes on, the extension sends the app which app the drag started in and the keys held, over the same socket every 30 ms, and moves the wheel the app opens under the pointer (a Wayland app can't place its own windows). The app opens and switches the wheel from that as it does on X11. GNOME Shell loads a new or updated extension at the next login; the app says so once.
- **Drop target:** our own GTK handlers instead of WebKit's. They answer "copy" over a format and nothing elsewhere, never "move" even with Shift held, and never ask the file manager to delete the original.
- **Menu bar icon:** shown through AppIndicator.
- **Open at Login:** an entry in `~/.config/autostart`.
- **Converter:** falls back to ImageMagick 6's `convert` and the `libreoffice` command, checks pandoc is 2.15 or newer, and Aptos stands in as Noto Sans.

## Updates

From version 0.1.1 the app updates itself from this repo's [Releases page](https://github.com/oracola91/convert-wheel/releases). Thirty seconds after it starts, then once a day, it reads `latest.json` from the newest published release (a request to github.com, through the system proxy). When there is a newer version, a notification says so and the menu shows **Install Update … and Restart**. Nothing installs until the user chooses it, or starts the app with `convert-wheel --update`. Conversions still running are finished first.

**What protects an update.** Every update file is signed with the project's update key (minisign, Ed25519), and the signature records the version it was made for. The app checks the signature against the public key built into it (`plugins.updater.pubkey` in `tauri.conf.json`) and refuses an update whose signature or version doesn't match (`requireSignedVersion`), so a changed file, a fake one, or an older build relabelled as newer is not installed. Only newer versions install. Files the app downloads itself carry no "downloaded from the internet" mark, so updates don't go through SmartScreen or Gatekeeper: the signature check is the control.

**Where the key is.** Two copies: a secret in this repo's `release` environment on GitHub, which only `v*` tags can use and only the release job's signing step sees (the builds never do); and `~/.tauri/convert-wheel-updater.key` on the release Mac, with its password in the Keychain, used only by the last command of `scripts/release-macos.sh`. Keep an offline backup of the key file and its password in a password manager.

**The Mac code-signing certificate.** macOS remembers which folders an app may open (Downloads, Documents) by the app's signature. An app signed ad hoc gets a new signature with every build, so after every update macOS would ask again. Releases and `scripts/install-macos.sh` builds are therefore signed with the project's own certificate, "Convert Wheel Signing", made once with `scripts/macos-signing-cert.sh` and kept in the release Mac's login keychain, with a backup (.p12 and its password) in a password manager. It is self-signed, not an Apple Developer ID: it keeps the permissions across updates, but macOS still asks for "Open Anyway" on the first launch of a downloaded copy.

**If the key is lost,** installed copies can no longer be updated: publish a build with a new key and ask everyone to install it once by hand. **If the key may have leaked,** do the same at once: generate a new key, put its public half in `tauri.conf.json`, replace both copies of the private key, release, and tell users to reinstall from the Releases page; copies with the old key can be offered files signed with the leaked one until they do.

On Windows the update runs the installer in passive mode (a progress bar, no questions). On macOS the app is replaced in place. On Linux the AppImage is replaced, or the `.deb` is installed as root: the system asks for the admin password (pkexec), or if that isn't available, a plain password box appears and the password goes to `sudo`.

## The menu

The ring icon in the menu bar opens:

- **Open at Login**: adds or removes Convert Wheel in System Settings > General > Login Items. Only works for the installed app.
- **Shelf at the Top of the Screen** (macOS): on unless unticked.
- **Save Converted Files**: next to the original (default) or in Downloads. When the original's folder can't be written to, the file goes to Downloads either way and the notification says so.
- **Formats on the Wheel**: tick or untick formats per kind of file. Unticked formats stay off the wheel.
- **Quality**: Smaller Files, Balanced (the default, what the app always did) or Best Quality, for Pictures, Video and Audio separately. It holds for a whole drop, from when it starts.

  | | Smaller Files | Balanced | Best Quality |
  |---|---|---|---|
  | JPEG, WebP | 75 | 90 | 97 |
  | HEIC, AVIF | 40 | ImageMagick's own (50) | 85 |
  | H.264 (MP4, MOV, MKV) | CRF 26 | CRF 20 | CRF 17 |
  | VP9 (WebM) | CRF 38 | CRF 32 | CRF 26 |
  | AVI, WMV | quantiser 5 | 3 | 2 |
  | MP3 (VBR) | about 130 kbit/s | 190 | 245 |
  | AAC (M4A) | 160 kbit/s | 256 | 320 |
  | Opus, OGG | 96 kbit/s | 160 | 256 |
  | WMA | 128 kbit/s | 192 | 256 |

  Lossless formats (PNG, TIFF, WAV, FLAC, AIFF) and documents have no quality to choose. The Tools wheel's Compress keeps its own, lower settings.
- **Highlight Color**: what colour the segment under the cursor lights up in: Orange (the default), Red, Pink, Purple, Blue, Teal, Green or Graphite. The next wheel opens in it.
- **Wheel Glass** (macOS): Liquid Glass (the default) or No Liquid Glass, a plain solid wheel. With Liquid Glass, the Transparency slider under them sets how see-through it is, from frosted (left: the frosted material behind it and more white on the segments, easiest to read) to clear (right: no blur; the glass stays frosted only right behind each word, so the words still read). Saved when the knob is let go; the next wheel opens with it.
- **Last: …**: how the last drop went, for when notifications are off. A click on it opens the folder with the new files selected, like a click on the notification. While a drop converts, the line shows how far it is: "Converting 3 of 10 to PNG…", with a percent for audio and video, or the time so far for a tool that doesn't report one.
- **Stop Converting**: ends the drop being converted now. Files already done stay; the one in progress leaves nothing behind. Drops queued after it still run.
- **Install ffmpeg…** (macOS and Windows): appears when a drop needed a converter that isn't installed, and the notification points to it. It runs `brew install` on macOS or `winget install` on Windows for that converter (several in a row: "Install ffmpeg and MuPDF…"), checks the app can now find it, and says so; then drop the files again. Without Homebrew it opens brew.sh; without winget it says to get App Installer from the Microsoft Store. On Windows, choosing it accepts the package's terms as typing the command would, and packages that install for all users, such as LibreOffice, still ask for admin rights. The package manager's last lines go to the log. On Linux the notification still names the `apt` command (it needs an admin password); the setup script installs everything at once.
- **Check for Updates… / Install Update … and Restart**.
- **Quit Convert Wheel**.

Settings are kept in `~/Library/Application Support/local.convertwheel/settings.json` (Windows: `%APPDATA%\local.convertwheel`, Linux: `~/.config/local.convertwheel`). A settings file that can't be used is kept aside as `settings.json.bad` (or, if it can't be read at all, left alone and never saved over), and a notification says the settings were reset.

**Show Log** opens the folder with the log of starts, conversions, errors and update checks: `~/Library/Logs/local.convertwheel/convert-wheel.log` (Windows: `%LOCALAPPDATA%\local.convertwheel\logs`, Linux: `~/.local/share/local.convertwheel/logs`). It holds file names, not folders (tool messages are stripped of paths), is readable by the user only, and keeps at most about 1 MB.

**Quit** while conversions are running asks for a second Quit within 10 seconds, then stops them.

Only one copy runs at a time: starting the app again just says it is already running. On macOS this uses a fixed lock name in `/tmp` (from the single-instance plugin); on a Mac shared by several user accounts, another account could block it. That doesn't matter on a one-person laptop.

## Run it from the source

Needs Rust, Node.js and the Xcode Command Line Tools, plus the converters:

```sh
brew install ffmpeg imagemagick pandoc mupdf-tools
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
3. Over the wheel, the dragged file shrinks to a small icon so its name doesn't cover the wheel. Move over the segments. The one under the cursor turns orange, moves out toward the cursor, and the middle says what it will become ("To PDF"). For one file it says roughly how big it will be instead, once the app has worked that out ("2.2 MB → ~410 KB"): for pictures (a piece of the middle converted and scaled up), sound (its length at the bit rate used; MP3's rate follows the music, so that one is rougher) and video that is only copied into another container. Video that is encoded again, documents and PDFs have no estimate. On a Force Touch trackpad each new segment gives a light tick. Over the middle it says "Cancel".
4. Drop on a segment. The new file appears next to the original and a notification confirms it. Click the notification: Finder opens with the new file selected.
5. Drop in the middle. The file slides back and nothing happens. Outside the wheel the drop works as usual in macOS: it slides back, or Finder moves the file if you let go over a folder.

6. Drag a file without Shift toward the top of the screen. The shelf opens ("Drop to hold here"); drop it there. The file stays where it was, and a small sign stays at the top.
7. Open another folder on the same disk, point at the sign and drag the shelf out into that folder. The file moves there and the shelf empties. With Option held it is copied instead.

Also worth trying: several files at once, a file with no extension, text dragged from a browser (no wheel should appear), a second screen, and LibreOffice open at the same time.

## Safety

- Tools get the file paths as separate arguments, never through a shell, so odd file names can't run commands.
- Except audio and video (too big), each file is first copied into a private work folder under a neutral name, so characters like `%` or `[1]` in a name can't confuse a tool, and the file can't change between the checks and the conversion.
- Only regular files are converted: a folder, device or pipe is refused.
- ImageMagick is told the real image format, read from the file's first bytes. Only plain raster formats are accepted, so a disguised SVG or PostScript file is refused instead of decoded.
- ffmpeg may read only local files, and only the containers the wheel accepts. A playlist or concat list disguised as a video is refused instead of being followed to other files or the network. ffprobe, which reads how long a file plays for the percent in the menu, gets the same limits.
- pandoc runs with `--sandbox`. It reads only the file it was given, never the network or other files. HTML goes to PDF through pandoc for this reason. A pandoc older than 2.15, or one whose version can't be read, is refused. Ubuntu's and Debian's own pandoc can't write Word files in the sandbox (its data files are kept outside the program), so on Linux pandoc comes from pandoc's own releases.
- LibreOffice uses its own profile in the app's cache folder, so it doesn't interfere with a LibreOffice window you have open. In that profile macros never run, links in documents are never updated when they open, and LibreOffice is told to block content linked from outside a document (`BlockUntrustedRefererLinks`). A test with LibreOffice 26.8 on macOS fetched nothing from a web page or ODT linking a remote image; other versions, spreadsheet DDE links and Windows network shares are not tested yet.
- Office files must really be what their name says: zip formats must start as zip archives, `.doc`/`.xls`/`.ppt` as OLE files, `.rtf` as RTF. Text and CSV are opened with a fixed text reader. Otherwise LibreOffice would pick a reader from the content, and a broken file would come out as a PDF of garbage.
- PDFs must start as PDFs (`%PDF-`). MuPDF only draws the pages or takes out the text. Its draw command doesn't run scripts in a PDF and doesn't open files a PDF points to outside itself (checked in MuPDF's source, not yet tested with crafted PDFs). mutool works through a PDF ten pages at a time (text: 200) and stops once the pictures pass 1 GB or the text 50 MB, so a PDF built to fill the disk is cut off early. Pages are capped at 4000 pixels a side, to keep a PDF claiming huge pages from needing huge amounts of memory, and a whole PDF, JPG pages included, gets 10 minutes.
- Every tool has a time limit (2 minutes for images and documents, 3 for LibreOffice and PDFs, 10 for audio, 60 for video). A tool that hangs is stopped with everything it started, so it can't block later conversions. Quitting, or Stop Converting in the menu, stops any tool still running the same way.
- ImageMagick gets memory, disk, size and time limits, so a small file claiming to be a huge picture can't take the machine down.
- Each conversion works in its own private temporary folder (random name, owner-only on macOS and Linux), deleted afterwards whether it succeeded or not.
- The web page in the wheel may only listen for the app's events; its content security policy allows nothing from outside the app.
- Every build runs `cargo audit` against the RustSec database. It reports, without blocking, a few notices from Tauri's own Linux libraries: gtk-rs 0.18 and the D-Bus crates are marked unmaintained, and glib 0.18 has an unsoundness note in an iterator this app doesn't use. They go away when Tauri moves to newer versions. The updater is built without its zip support, so the zip crate with open advisories isn't in the app at all.

## Fonts in PDFs

LibreOffice running without a window only sees its own bundled fonts. The converter gives it a font list (`~/Library/Caches/local.convertwheel/fontconfig/fonts.conf`) with the macOS font folders, so documents keep their fonts.

Word's default font since 2023, Aptos, ships inside Word only, and its licence limits it to Microsoft products. So it is not handed to LibreOffice. Helvetica Neue, which comes with macOS, stands in for it. If Aptos is ever properly installed on the Mac, the real font is used instead. Calibri and Cambria need nothing, because LibreOffice ships look-alikes with the same spacing.

## How it works

- **Trigger** (`src-tauri/src/trigger.rs`): every 30 ms it reads the drag pasteboard's change count, the mouse buttons and the Shift key. These are shared by the window server, so no permission prompt is needed.
- **Wheel window** (`src-tauri/src/wheel_window.rs`): the native parts. Glass behind the ring: Apple's Liquid Glass on macOS 26 and later, the frosted material before (both follow light/dark mode and Reduce Transparency), the dragged file's icon in the middle, placement at the cursor kept inside the screen, and the fade in and out.
- **Drop target** (`src-tauri/src/drop_view.rs`): a transparent native view on top of the wheel takes the drop. It shrinks the dragged file to a small icon, tells the page which segment to light up, and queues the conversion.
- **Formats** (`src-tauri/src/formats.rs`): what each kind of file can become, and which tools the Tools wheel offers for it.
- **Converter** (`src-tauri/src/convert.rs`): picks the tool and its settings, then places the result safely.
- **Queue** (`src-tauri/src/main.rs`): one worker converts drops in order, keeps the menu's progress line current, then posts the notification.
- **Installing converters** (`src-tauri/src/converters.rs`): remembers the converters drops found missing, offers them in the menu, and runs `brew install` or `winget install` with fixed package names, never a shell line.
- **Showing results** (`src-tauri/src/reveal.rs`): opens Finder, Explorer or the Linux file manager with the new files selected, through each system's own call rather than a command line.
- **Shelf** (`src-tauri/src/shelf.rs`, `shelf_window.rs`): the held paths and where the shelf goes (the notch, under the menu bar icon, or the corner), and on macOS a native panel above the menu bar that takes drops and starts the drag out. The trigger tells it when a file drag comes near.
- **Wheel** (`src/`): plain HTML, CSS and JavaScript in a transparent window. No frontend framework and no npm runtime packages. It opens and closes along the same path with no bounce, and follows Reduce Motion, Increase Contrast and dark mode.

## Layout

```
src/                       wheel UI (index.html, wheel.css, wheel.js)
src-tauri/                 Rust app
  src/main.rs              app setup, drop queue, notifications, --convert, --show, --update
  src/update.rs            update checks and installs
SECURITY.md                how to report a security problem
  src/log.rs               the log file
  src/trigger.rs           Shift+drag on macOS; opening, closing and dropping for all
  src/wheel.rs             wheel geometry: which segment is under a point
  src/formats.rs           formats and tools offered per file type
  src/convert.rs           the conversions
  src/menu.rs              menu bar icon and menu
  src/converters.rs        installing a missing converter from the menu
  src/reveal.rs            showing results selected in the file manager
  src/mac_notification.rs  macOS notifications, whose click shows the files
  src/settings.rs          saved choices (where files go, hidden formats)
  src/login.rs             Open at Login on each system
  src/drop_view.rs         macOS: the drop target over the wheel
  src/shelf.rs             the shelf: held files, where it goes
  src/shelf_window.rs      macOS: the shelf panel, drop in and drag out
  src/wheel_window.rs      macOS: glass, hub icon, placement, fades
  src/win32.rs             Windows: trigger, placement, drop target
  src/linux.rs             Linux: X11 trigger, Wayland shortcut and drags, placement, drop target
  src/gnome_shell.rs       Linux: installs the GNOME Shell extension, reads its drag reports
  gnome-shell/             the GNOME Shell extension (Shift+drag on Wayland)
  tauri.conf.json          window and bundle settings
  tauri.windows.conf.json  Windows window settings and installer types
  tauri.linux.conf.json    Linux window settings and package types
  Info.plist               marks it a menu bar app on macOS (no Dock icon)
.github/workflows/build.yml  Windows and Linux builds; a version tag publishes a release
.github/workflows/test.yml   unit tests on Linux, Windows and macOS for every pull request
scripts/install-macos.sh   build and install in /Applications on this Mac
scripts/install-linux.sh   one-command setup on Ubuntu and Debian
scripts/release-macos.sh   add the macOS .dmg to a release
scripts/macos-signing-cert.sh  make the Mac code-signing certificate, once
INSTALL.md                 installing, updating, uninstalling, making a release
ROADMAP.md                 what comes next, in order
```
