//! Turns one file into another format with ImageMagick, ffmpeg, pandoc or LibreOffice.
//!
//! Each tool writes into a private work folder. The result is then moved next to the
//! original under a free name ("report.pdf", "report 2.pdf", ...), so nothing is
//! ever overwritten. Tools get absolute paths as separate arguments, never a shell line.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::formats::{self, Kind};

/// Converts `src` to `target` (a format from the wheel) and returns the new file.
pub fn convert(src: &Path, target: &str) -> Result<PathBuf, String> {
    let ext = formats::extension(src).ok_or("the file has no extension")?;
    let kind = formats::kind_of(&ext).ok_or_else(|| format!(".{ext} files are not supported"))?;
    if !formats::outputs_for(&ext).contains(&target) {
        return Err(format!(".{ext} files can't become .{target}"));
    }
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("the file name is not valid text")?;
    let work = WorkDir::new()?;
    let out = work.path.join(format!("{stem}.{target}"));

    // Office files are zip archives. A broken one would otherwise come out as a PDF
    // of its raw bytes, because LibreOffice falls back to reading it as text.
    let zipped = ["docx", "xlsx", "pptx", "odt", "ods", "odp", "epub"];
    if zipped.contains(&ext.as_str()) && !read_head(src)?.starts_with(b"PK") {
        return Err(format!("this is not a valid .{ext} file"));
    }

    match kind {
        Kind::Image => image(src, target, &out)?,
        Kind::Audio => audio(src, target, &out)?,
        Kind::Video => video(src, &ext, target, &out)?,
        Kind::Doc => document(src, &ext, target, &out, &work)?,
        Kind::Sheet | Kind::Slides => libreoffice(src, target, &out)?,
    }
    if !out.is_file() {
        return Err("the converter finished but wrote no file".into());
    }
    place(&out, src, stem, target)
}

/// The image format a file really is, from its first bytes. Only plain raster
/// formats are recognised, so a disguised SVG or PostScript file is refused.
fn sniff_image(head: &[u8]) -> Option<&'static str> {
    let at = |i: usize, sig: &[u8]| head.get(i..i + sig.len()) == Some(sig);
    if at(0, &[0xFF, 0xD8, 0xFF]) {
        Some("jpeg")
    } else if at(0, b"\x89PNG") {
        Some("png")
    } else if at(0, b"GIF8") {
        Some("gif")
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        Some("webp")
    } else if at(0, b"II*\0") || at(0, b"MM\0*") {
        Some("tiff")
    } else if at(0, b"BM") {
        Some("bmp")
    } else if at(4, b"ftyp") {
        // HEIC and AVIF share a container; libheif reads both through the HEIC coder.
        match head.get(8..12)? {
            b"avif" | b"avis" => Some("avif"),
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" => Some("heic"),
            _ => None,
        }
    } else {
        None
    }
}

fn image(src: &Path, target: &str, out: &Path) -> Result<(), String> {
    // Naming the real input format stops ImageMagick from picking a decoder itself.
    // The real format, not the extension, so a PNG saved as .jpg still converts.
    let coder = sniff_image(&read_head(src)?).ok_or("this is not an image format the wheel can read")?;
    // Multi-page outputs keep every frame; the rest take the first one.
    let frame = if matches!(target, "pdf" | "tiff" | "gif") { "" } else { "[0]" };
    let mut cmd = Command::new(tool("magick")?);
    cmd.arg(format!("{coder}:{}{frame}", src.display())).arg("-auto-orient");
    if target == "jpg" {
        // JPEG has no transparency; without this, transparent areas turn black.
        cmd.args(["-background", "white", "-alpha", "remove", "-alpha", "off"]);
    }
    if matches!(target, "jpg" | "webp") {
        cmd.args(["-quality", "90"]);
    }
    let out_coder = if target == "jpg" { "jpeg" } else { target };
    cmd.arg(format!("{out_coder}:{}", out.display()));
    run(cmd)
}

fn audio_args(target: &str) -> &'static [&'static str] {
    match target {
        "mp3" => &["-c:a", "libmp3lame", "-q:a", "2"],
        "m4a" => &["-c:a", "aac", "-b:a", "256k"],
        "wav" => &["-c:a", "pcm_s16le"],
        "flac" => &["-c:a", "flac"],
        "aiff" => &["-c:a", "pcm_s16be"],
        // Homebrew's ffmpeg has no Vorbis encoder; Opus in an Ogg file plays everywhere Ogg does.
        "ogg" => &["-c:a", "libopus", "-b:a", "160k"],
        _ => &[],
    }
}

fn ffmpeg(src: &Path, args: &[&str], out: &Path) -> Result<(), String> {
    let mut cmd = Command::new(tool("ffmpeg")?);
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(src)
        .args(args)
        .arg(out);
    run(cmd)
}

fn audio(src: &Path, target: &str, out: &Path) -> Result<(), String> {
    let mut args = vec!["-vn", "-map_metadata", "0"];
    args.extend_from_slice(audio_args(target));
    ffmpeg(src, &args, out)
}

fn video(src: &Path, ext: &str, target: &str, out: &Path) -> Result<(), String> {
    const H264: &[&str] = &[
        "-map", "0:v:0", "-map", "0:a?", "-c:v", "libx264", "-preset", "medium", "-crf", "20",
        "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart",
    ];
    match target {
        "mp4" | "mov" => {
            // MP4, MOV and M4V usually hold the same streams: try a fast copy first.
            if matches!(ext, "mp4" | "mov" | "m4v") {
                let copy = ["-map", "0:v", "-map", "0:a?", "-c", "copy", "-movflags", "+faststart"];
                if ffmpeg(src, &copy, out).is_ok() {
                    return Ok(());
                }
                let _ = fs::remove_file(out);
            }
            ffmpeg(src, H264, out)
        }
        "webm" => ffmpeg(
            src,
            &[
                "-map", "0:v:0", "-map", "0:a?", "-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0",
                "-row-mt", "1", "-c:a", "libopus", "-b:a", "128k",
            ],
            out,
        ),
        "gif" => ffmpeg(
            src,
            &[
                "-an",
                "-vf",
                "fps=12,scale='min(720,iw)':-2:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse",
                "-loop",
                "0",
            ],
            out,
        ),
        "mp3" => audio(src, target, out),
        other => Err(format!("video can't become .{other}")),
    }
}

/// Word-processing files. LibreOffice keeps the layout (PDF, Word, ODT from office
/// files); pandoc is better at the plain formats (HTML, Markdown, text, EPUB).
fn document(src: &Path, ext: &str, target: &str, out: &Path, work: &WorkDir) -> Result<(), String> {
    let ext = formats::canonical(ext);
    let office_in = matches!(ext, "docx" | "doc" | "odt" | "rtf");
    match target {
        "pdf" if office_in || matches!(ext, "txt" | "html") => libreoffice(src, target, out),
        "pdf" => {
            // Markdown and EPUB: pandoc to Word first, then LibreOffice lays it out.
            let docx = work.path.join("intermediate.docx");
            pandoc(src, ext, "docx", &docx)?;
            libreoffice(&docx, target, out)
        }
        "docx" | "odt" if office_in => libreoffice(src, target, out),
        _ if ext == "doc" => {
            // pandoc can't read the old .doc format: LibreOffice turns it into .docx first.
            let docx = work.path.join("intermediate.docx");
            libreoffice(src, "docx", &docx)?;
            pandoc(&docx, "docx", target, out)
        }
        _ => pandoc(src, ext, target, out),
    }
}

fn pandoc(src: &Path, ext: &str, target: &str, out: &Path) -> Result<(), String> {
    let reader = match ext {
        "md" | "txt" => "markdown",
        other => other,
    };
    let writer = match target {
        "html" => "html5",
        "md" => "gfm",
        "txt" => "plain",
        "epub" => "epub3",
        other => other,
    };
    let mut cmd = Command::new(tool("pandoc")?);
    // --sandbox: pandoc reads only the input file, never the network or other files.
    cmd.args(["--sandbox", "-f", reader, "-t", writer]);
    if target == "html" {
        cmd.args(["--standalone", "--embed-resources"]);
    }
    cmd.arg("-o").arg(out).arg(src);
    run(cmd)
}

fn libreoffice(src: &Path, target: &str, out: &Path) -> Result<(), String> {
    let outdir = out.parent().ok_or("no work folder")?.join("libreoffice");
    fs::create_dir_all(&outdir).map_err(|e| format!("work folder: {e}"))?;
    let mut cmd = Command::new(soffice()?);
    // A profile of our own, so it works while LibreOffice is also open in the normal way.
    cmd.arg(format!("-env:UserInstallation={}", file_url(&libreoffice_profile())))
        .args(["--headless", "--norestore", "--nolockcheck", "--convert-to", target, "--outdir"])
        .arg(&outdir)
        .arg(src);
    run(cmd)?;
    // LibreOffice names the result after the input and can exit 0 without writing it.
    let stem = src.file_stem().ok_or("no file name")?.to_string_lossy();
    let written = outdir.join(format!("{stem}.{target}"));
    if !written.is_file() {
        return Err("LibreOffice could not convert this file".into());
    }
    fs::rename(&written, out).map_err(|e| format!("work folder: {e}"))
}

fn read_head(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut head = Vec::with_capacity(16);
    fs::File::open(path)
        .and_then(|f| f.take(16).read_to_end(&mut head))
        .map_err(|e| format!("can't read the file: {e}"))?;
    Ok(head)
}

fn run(mut cmd: Command) -> Result<(), String> {
    let name = Path::new(cmd.get_program())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let output = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not start {name}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no details");
    Err(format!("{name} failed: {}", last.trim()))
}

/// Moves the result next to the original without replacing anything there.
fn place(tmp: &Path, src: &Path, stem: &str, target: &str) -> Result<PathBuf, String> {
    let dir = src.parent().ok_or("the file has no folder")?;
    for n in 1..1000 {
        let name = if n == 1 {
            format!("{stem}.{target}")
        } else {
            format!("{stem} {n}.{target}")
        };
        let dest = dir.join(name);
        // create_new claims the name atomically, so a file that appears meanwhile is safe.
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("can't write in {}: {e}", dir.display())),
        };
        // Same disk: swap our placeholder for the result. Other disk: copy the bytes.
        if fs::rename(tmp, &dest).is_ok() {
            return Ok(dest);
        }
        let copied = fs::File::open(tmp).and_then(|mut f| io::copy(&mut f, &mut file));
        if let Err(e) = copied {
            let _ = fs::remove_file(&dest);
            return Err(format!("can't write in {}: {e}", dir.display()));
        }
        return Ok(dest);
    }
    Err("too many files with that name already".into())
}

/// A temporary folder that is removed when the conversion ends, however it ends.
struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("convert-wheel-{}-{n}", std::process::id()));
        fs::create_dir(&path).map_err(|e| format!("work folder: {e}"))?;
        Ok(Self { path })
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Finds a command-line tool. Apps opened from Finder don't get the shell's PATH,
/// so Homebrew's folders are checked as well.
fn tool(name: &str) -> Result<PathBuf, String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from))
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            let formula = if name == "magick" { "imagemagick" } else { name };
            format!("{name} is not installed (brew install {formula})")
        })
}

fn soffice() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    [
        PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice"),
        home.join("Applications/LibreOffice.app/Contents/MacOS/soffice"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .map(Ok)
    .unwrap_or_else(|| {
        tool("soffice").map_err(|_| "LibreOffice is not installed (brew install --cask libreoffice)".into())
    })
}

fn libreoffice_profile() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Library/Caches/local.convertwheel/libreoffice")
}

fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => url.push(b as char),
            _ => url.push_str(&format!("%{b:02X}")),
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_never_overwrites() {
        let work = WorkDir::new().unwrap();
        let src = work.path.join("report.docx");
        fs::write(&src, "x").unwrap();
        fs::write(work.path.join("report.pdf"), "keep me").unwrap();
        let tmp = work.path.join("result.tmp");
        fs::write(&tmp, "new").unwrap();

        let dest = place(&tmp, &src, "report", "pdf").unwrap();
        assert_eq!(dest, work.path.join("report 2.pdf"));
        assert_eq!(fs::read_to_string(work.path.join("report.pdf")).unwrap(), "keep me");
        assert_eq!(fs::read_to_string(dest).unwrap(), "new");
    }

    #[test]
    fn sniff_reads_the_real_format_and_refuses_the_rest() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpeg"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(sniff_image(b"\0\0\0\x18ftypheic"), Some("heic"));
        assert_eq!(sniff_image(b"\0\0\0\x1cftypavif"), Some("avif"));
        assert_eq!(sniff_image(b"<svg xmlns="), None);
        assert_eq!(sniff_image(b"%!PS-Adobe"), None);
        assert_eq!(sniff_image(b""), None);
    }

    #[test]
    fn file_url_escapes_spaces() {
        assert_eq!(file_url(Path::new("/a b/c")), "file:///a%20b/c");
    }

    #[test]
    fn unsupported_files_are_refused_before_any_tool_runs() {
        assert!(convert(Path::new("/tmp/a.xyz"), "pdf").is_err());
        assert!(convert(Path::new("/tmp/Makefile"), "pdf").is_err());
        assert!(convert(Path::new("/tmp/a.png"), "mp3").is_err());
    }

    #[test]
    fn libreoffice_output_keeps_dots_in_the_name() {
        // "v1.2 notes.docx" must look for "v1.2 notes.pdf", not "v1.pdf"
        let stem = Path::new("/x/v1.2 notes.docx").file_stem().unwrap().to_string_lossy();
        assert_eq!(format!("{stem}.pdf"), "v1.2 notes.pdf");
    }
}
