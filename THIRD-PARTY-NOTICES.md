# Third-party notices

Pirouette is MIT OR Apache-2.0 (see LICENSE-MIT and LICENSE-APACHE). It is built with
Rust crates and web assets that keep their own licences. Most are MIT or Apache-2.0
(Tauri, objc2, the windows crate, serde and others; the full list with versions is in
`src-tauri/Cargo.lock`). The ones under other or narrower terms:

| Part | Licence | Where |
|---|---|---|
| Lucide icons (the wheel's and windows' icons) | ISC | `src/icons-LICENSE.txt` |
| flo_curves (through vtracer, SVG tracing) | Apache-2.0 only | crates.io |
| rqrr (reading QR codes) | (MIT OR Apache-2.0) AND ISC | crates.io |
| vtracer, visioncortex (SVG tracing) | MIT OR Apache-2.0 | crates.io |

The converters the app runs (ffmpeg, ImageMagick, pandoc, LibreOffice, MuPDF, ExifTool,
bsdtar) are not part of it: they are separate programs you install (the app can ask
Homebrew, winget or apt to), started as separate processes, each under its own licence.

Before a release, a full list of the crates in the build can be made with
`cargo about` or `cargo license` from `src-tauri/`.
