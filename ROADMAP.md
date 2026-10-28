# Roadmap

The aim: everything a good drag-and-drop file tool does, on macOS, Windows and Linux, and then more. Files never leave the computer, results never replace the original, and every new tool gets the same safety rules as the converters (see the Safety section of the README).

## Three wheels

Today Shift+drag opens one wheel: the formats a file can become. Two more are planned, opened the same way with another key:

| Keys while dragging | Wheel | Holds |
|---|---|---|
| Shift | Formats (done) | What the file can become |
| Option+Shift (Alt+Shift on Windows and Linux) | Tools | What can be done to the file: compress, resize, merge, split, clean |
| Control | Actions | What can be done with the file: copy, move, share, copy its path |

## Stages, in order

**1. Show what happened.** Clicking the notification opens the folder with the result selected. A progress line for long jobs and batches, and a way to stop a job that runs too long.

**2. The Tools wheel, with what the installed tools already do.**
- Images: compress (a few quality steps), half size, remove metadata (camera, location), rotate.
- PDF: merge the dropped PDFs into one, split into one file per page, make smaller, turn dropped images into one PDF.
- Video: compress, a still image from the first frame, remove metadata.
- Audio: even out loudness, mono or stereo.

**3. The Actions wheel.** Copy, Move, Copy Path, Show in Folder, open a terminal there, Share (AirDrop on macOS, the Share panel on Windows), and a shelf that holds files for the next drag. The shelf came first, on macOS (at the top of the screen, see README); Windows and Linux follow.

**4. More formats.**
- Archives: make ZIP, TAR and GZIP; unpack ZIP, TAR, GZIP and RAR.
- Images: BMP and ICO out; SVG out (traced) later.
- Video: MKV, AVI and WMV out. Audio: Opus and WMA.
- Subtitles: SRT and VTT both ways.
- Apple Pages, Numbers and Keynote files in (LibreOffice reads them).

**5. Settings.** Quality per kind of file, which keys open which wheel, and choosing a segment with the arrow keys, Enter and Esc.

**6. One-step setup on macOS and Windows,** like `scripts/install-linux.sh`: one command or one click installs the converters. Where the licences allow, ship them with the app instead.

**7. An editing window,** for what a wheel can't ask: cut and trim video and audio, split and join clips, change speed, bleep a word, crop, annotate and redact photos, reorder and redact PDF pages, compress to an exact file size.

**8. Smaller tools:** read QR codes, read and edit metadata.

## Beyond that

- Text from scanned PDFs and photos (OCR), into text, Word or a searchable PDF.
- A second way in: the right-click menu of Finder, File Explorer and the Linux file managers.
- Presets: a named set of tool and format steps, run with one drop.
- The command line can do everything the wheels do (`convert-wheel --convert` today).
- Translations and full keyboard and screen-reader support.

## How each item ships

One pull request per item, with tests, through the same checks as everything else. A release goes out when a stage, or a useful part of one, is done.
