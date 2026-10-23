//! Turns one file into another format with ImageMagick, ffmpeg, pandoc or LibreOffice.
//!
//! Each conversion gets a private work folder. Except for audio and video (too big to
//! copy), the file is first copied in under a neutral name, so odd characters in the
//! name can't confuse a tool and the file can't change between the checks and the
//! conversion. The result is then moved next to the original under a free name
//! ("report.pdf", "report 2.pdf", ...), so nothing is ever overwritten. Tools get
//! paths as separate arguments, never a shell line, and each has a time limit.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use crate::formats::{self, Kind};

/// How long each tool may run before it is stopped. Generous: a stuck tool should not
/// block every later conversion, but a long video must still finish.
const QUICK: Duration = Duration::from_secs(120);
const OFFICE: Duration = Duration::from_secs(180);
const AUDIO: Duration = Duration::from_secs(600);
const VIDEO: Duration = Duration::from_secs(3600);

/// The only containers ffmpeg may read: the ones the wheel accepts. Anything else, a
/// playlist or concat list disguised as a video for example, is refused instead of
/// being followed to other files or the network.
const FFMPEG_FORMATS: &str = "mov,mp4,m4a,matroska,webm,avi,mp3,wav,flac,aiff,ogg,aac";

/// Converts `src` to `target` (a format from the wheel) and returns the new file.
/// It goes in `save_in`, or next to the original when that is None. If the original's
/// folder can't be written to (a mounted disk image, another app's folder), it goes to
/// `fallback` (Downloads) instead.
pub fn convert(src: &Path, target: &str, save_in: Option<&Path>, fallback: &Path) -> Result<PathBuf, String> {
    let ext = formats::extension(src).ok_or("the file has no extension")?;
    let kind = formats::kind_of(&ext).ok_or_else(|| format!(".{ext} files are not supported"))?;
    if !formats::outputs_for(&ext).contains(&target) {
        return Err(format!(".{ext} files can't become .{target}"));
    }
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("the file name is not valid text")?;
    // Not a folder, a device or a pipe (a pipe would block the read below forever).
    let meta = fs::metadata(src).map_err(|e| format!("can't read the file: {e}"))?;
    if !meta.is_file() {
        return Err("this is not a regular file".into());
    }
    let work = tempfile::Builder::new()
        .prefix("convert-wheel-")
        .tempdir()
        .map_err(|e| format!("work folder: {e}"))?;
    let out = work.path().join(format!("result.{target}"));

    // Audio and video can be huge; ffmpeg reads them in place, locked down instead.
    let input = if matches!(kind, Kind::Audio | Kind::Video) {
        src.to_path_buf()
    } else {
        let staged = work.path().join(format!("input.{ext}"));
        fs::copy(src, &staged).map_err(|e| format!("can't read the file: {e}"))?;
        staged
    };

    // Office files must really be what their name says. LibreOffice otherwise picks a
    // reader from the content: a broken .docx comes out as a PDF of its raw bytes, and
    // a web page saved as .doc is read as a web page.
    let head = read_head(&input)?;
    let expected: &[u8] = match ext.as_str() {
        "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "epub" => b"PK",
        "doc" | "xls" | "ppt" => &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1],
        "rtf" => b"{\\rtf",
        _ => b"",
    };
    if !head.starts_with(expected) {
        return Err(format!("this is not a valid .{ext} file"));
    }

    let converted = match kind {
        Kind::Image => image(&input, target, &out),
        Kind::Audio => audio(&input, target, &out),
        Kind::Video => video(&input, &ext, target, &out),
        Kind::Doc => document(&input, &ext, target, &out, &work),
        Kind::Sheet | Kind::Slides => libreoffice(&input, target, &out),
    };
    // Tool messages often start with a full path; keep only file names.
    converted.map_err(|e| redact(&e, src, work.path()))?;
    if !out.is_file() {
        return Err("the converter finished but wrote no file".into());
    }
    let dir = match save_in {
        Some(dir) => dir.to_path_buf(),
        None => src.parent().ok_or("the file has no folder")?.to_path_buf(),
    };
    let placed = match place(&out, &dir, stem, target) {
        Err(e) if save_in.is_none() && is_no_write(&e) => {
            fs::create_dir_all(fallback).and_then(|_| place(&out, fallback, stem, target))
        }
        other => other,
    };
    placed.map_err(|e| format!("can't save the result: {e}"))
}

/// Replaces full paths in a tool's message with the file name.
fn redact(message: &str, src: &Path, work: &Path) -> String {
    let mut text = message.to_string();
    if let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) {
        text = text.replace(&src.display().to_string(), &name);
    }
    // Tools write either separator, whatever the system's own is.
    for dir in [src.parent(), Some(work)].into_iter().flatten() {
        for sep in ['/', '\\'] {
            text = text.replace(&format!("{}{sep}", dir.display()), "");
        }
    }
    text.replace("file:", "")
}

/// The original's folder can't take the result (read-only, or a name too long for
/// it), so Downloads gets it instead.
fn is_no_write(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem | io::ErrorKind::InvalidFilename
    )
}

/// Downloads by name, for the command line. The app asks the system instead, which
/// also finds a Downloads folder that was moved or renamed.
pub fn downloads() -> PathBuf {
    home().join("Downloads")
}

/// The user's home folder (HOME, or USERPROFILE on Windows).
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
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
    let mut cmd = Command::new(magick()?);
    // Caps for a small file that claims to be a huge picture, before it is decoded.
    cmd.args([
        "-limit", "memory", "1GiB", "-limit", "map", "2GiB", "-limit", "disk", "4GiB",
        "-limit", "width", "32KP", "-limit", "height", "32KP", "-limit", "area", "1GP",
        "-limit", "time", "110",
    ]);
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
    run(cmd, QUICK).map_err(|e| {
        if target == "pdf" && e.contains("not authorized") {
            "this computer's ImageMagick security policy doesn't allow writing PDF (common on Ubuntu)".to_string()
        } else {
            e
        }
    })
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

fn ffmpeg(src: &Path, args: &[&str], out: &Path, limit: Duration) -> Result<(), String> {
    let mut cmd = Command::new(tool("ffmpeg")?);
    // Local files only, our containers only, and the input named as a file so that a
    // name like "concat:..." can't be read as an instruction.
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error"])
        .args(["-protocol_whitelist", "file", "-format_whitelist", FFMPEG_FORMATS, "-i"])
        .arg(format!("file:{}", src.display()))
        .args(args)
        .arg(out);
    run(cmd, limit)
}

fn audio(src: &Path, target: &str, out: &Path) -> Result<(), String> {
    let mut args = vec!["-vn", "-map_metadata", "0"];
    args.extend_from_slice(audio_args(target));
    ffmpeg(src, &args, out, AUDIO)
}

fn video(src: &Path, ext: &str, target: &str, out: &Path) -> Result<(), String> {
    // H.264 in yuv420p needs even width and height; phone and screen recordings often
    // aren't, so round down by a pixel where needed.
    const H264: &[&str] = &[
        "-map", "0:v:0", "-map", "0:a?", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", "-c:v", "libx264",
        "-preset", "medium", "-crf", "20", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k",
        "-movflags", "+faststart",
    ];
    match target {
        "mp4" | "mov" => {
            // MP4, MOV and M4V usually hold the same streams: try a fast copy first.
            if matches!(ext, "mp4" | "mov" | "m4v") {
                let copy = ["-map", "0:v", "-map", "0:a?", "-c", "copy", "-movflags", "+faststart"];
                // A copy is quick or fails quickly; it never needs the video limit.
                if ffmpeg(src, &copy, out, QUICK).is_ok() {
                    return Ok(());
                }
                let _ = fs::remove_file(out);
            }
            ffmpeg(src, H264, out, VIDEO)
        }
        "webm" => ffmpeg(
            src,
            &[
                "-map", "0:v:0", "-map", "0:a?", "-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0",
                "-deadline", "good", "-cpu-used", "4", "-row-mt", "1", "-c:a", "libopus", "-b:a", "128k",
            ],
            out,
            VIDEO,
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
            VIDEO,
        ),
        "mp3" => audio(src, target, out),
        other => Err(format!("video can't become .{other}")),
    }
}

/// Word-processing files. LibreOffice keeps the layout (PDF, Word, ODT from office
/// files); pandoc is better at the plain formats (HTML, Markdown, text, EPUB).
fn document(src: &Path, ext: &str, target: &str, out: &Path, work: &TempDir) -> Result<(), String> {
    let ext = formats::canonical(ext);
    let office_in = matches!(ext, "docx" | "doc" | "odt" | "rtf");
    match target {
        // Plain text keeps its exact characters through LibreOffice; pandoc would read
        // it as Markdown and change *, # and _.
        "pdf" | "docx" | "odt" if office_in || ext == "txt" => libreoffice(src, target, out),
        "pdf" => {
            // Markdown, HTML and EPUB: pandoc (sandboxed, so a web page can't make it
            // fetch anything) to Word first, then LibreOffice lays it out.
            let docx = work.path().join("intermediate.docx");
            pandoc(src, ext, "docx", &docx)?;
            libreoffice(&docx, target, out)
        }
        _ if ext == "doc" || ext == "rtf" => {
            // pandoc can't read .doc, and reads RTF only in recent versions: LibreOffice
            // turns them into .docx first.
            let docx = work.path().join("intermediate.docx");
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
    let pandoc = tool("pandoc")?;
    check_pandoc_version(&pandoc)?;
    let mut cmd = Command::new(pandoc);
    // --sandbox: pandoc reads only the input file, never the network or other files.
    cmd.args(["--sandbox", "-f", reader, "-t", writer]);
    if target == "html" {
        cmd.args(["--standalone", "--embed-resources"]);
    }
    cmd.arg("-o").arg(out).arg(src);
    run(cmd, QUICK)
}

fn libreoffice(src: &Path, target: &str, out: &Path) -> Result<(), String> {
    let outdir = out.parent().ok_or("no work folder")?.join("libreoffice");
    fs::create_dir_all(&outdir).map_err(|e| format!("work folder: {e}"))?;
    let mut cmd = Command::new(soffice()?);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    cmd.env("FONTCONFIG_FILE", font_config()?);
    // A profile of our own, so it works while LibreOffice is also open in the normal
    // way, and locked down: no macros, no content linked from outside the document.
    let profile = cache_dir().join("libreoffice");
    harden_libreoffice(&profile)?;
    cmd.arg(format!("-env:UserInstallation={}", file_url(&profile)))
        .args(["--headless", "--norestore", "--nolockcheck"]);
    // Text and CSV are read as exactly that, never sniffed as something richer.
    match formats::extension(src).as_deref() {
        Some("txt") => {
            cmd.arg("--infilter=Text (encoded):UTF8");
        }
        Some("csv") => {
            cmd.arg("--infilter=CSV:44,34,76,1");
        }
        _ => {}
    }
    cmd.args(["--convert-to", target, "--outdir"]).arg(&outdir).arg(src);
    run(cmd, OFFICE)?;
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

/// Runs a tool to the end or until `limit`, whichever comes first. On a timeout the
/// tool and everything it started are stopped. Only the last line of its error
/// output is kept, for the notification.
fn run(mut cmd: Command, limit: Duration) -> Result<(), String> {
    let name = Path::new(cmd.get_program())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Error output goes to a temporary file, not a pipe: a chatty tool can't fill the
    // pipe and stall while we wait for it.
    let mut errors = tempfile::tempfile().map_err(|e| format!("work file: {e}"))?;
    let errors_for_tool = errors.try_clone().map_err(|e| format!("work file: {e}"))?;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(errors_for_tool);
    quiet(&mut cmd);
    // A group of its own on macOS and Linux, so a timeout can stop LibreOffice's
    // helper processes too.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not start {name}: {e}"))?;
    let id = child.id();
    running().push(id);
    let result = wait(&mut child, &name, limit, &mut errors);
    running().retain(|&p| p != id);
    result
}

/// Tools running right now, so quitting can stop them instead of leaving them behind.
fn running() -> std::sync::MutexGuard<'static, Vec<u32>> {
    static RUNNING: Mutex<Vec<u32>> = Mutex::new(Vec::new());
    RUNNING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Stops every tool still running, with whatever it started. Called when the app quits.
pub fn stop_all() {
    for id in running().drain(..) {
        kill_tree(id);
    }
}

fn wait(child: &mut Child, name: &str, limit: Duration, errors: &mut fs::File) -> Result<(), String> {
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                stop(child);
                return Err(format!("{name} took longer than {} min and was stopped", limit.as_secs() / 60));
            }
            Err(e) => {
                stop(child);
                return Err(format!("{name}: {e}"));
            }
        }
    };
    if status.success() {
        return Ok(());
    }
    // The end of the error output is enough: that is where tools say what went wrong.
    let len = errors.seek(SeekFrom::End(0)).unwrap_or(0);
    let _ = errors.seek(SeekFrom::Start(len.saturating_sub(16 * 1024)));
    let mut text = String::new();
    let _ = errors.read_to_string(&mut text);
    let last = text.lines().rev().find(|l| !l.trim().is_empty()).map(str::trim);
    match last {
        Some(line) => Err(format!("{name} failed: {line}")),
        None => Err(format!("{name} failed ({status})")),
    }
}

/// Without this, Windows flashes a console window for every tool that runs.
fn quiet(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd;
}

/// Stops a tool and the processes it started.
fn stop(child: &mut Child) {
    kill_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

fn kill_tree(id: u32) {
    #[cfg(unix)]
    unsafe {
        // SAFETY: plain syscall; the negative id addresses the group created in run().
        libc::kill(-(id as libc::pid_t), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        // The system copy, not whatever "taskkill" comes first on PATH.
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        let mut kill = Command::new(system.join("System32\\taskkill.exe"));
        kill.args(["/PID", &id.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        quiet(&mut kill);
        let _ = kill.status();
    }
}

/// Moves the result into `dir` without replacing anything there.
fn place(tmp: &Path, dir: &Path, stem: &str, target: &str) -> io::Result<PathBuf> {
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
            Err(e) => return Err(e),
        };
        // Same disk: swap our placeholder for the result. Other disk: copy the bytes.
        if fs::rename(tmp, &dest).is_ok() {
            return Ok(dest);
        }
        if let Err(e) = fs::File::open(tmp).and_then(|mut f| io::copy(&mut f, &mut file)) {
            let _ = fs::remove_file(&dest);
            return Err(e);
        }
        return Ok(dest);
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many files with that name already"))
}

/// Finds a command-line tool. Apps opened from Finder or the Start menu don't always
/// get the shell's PATH, so the places the usual installers use are checked as well.
fn tool(name: &str) -> Result<PathBuf, String> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let path = std::env::var_os("PATH").unwrap_or_default();
    // Only absolute folders: an empty or relative PATH entry would mean "wherever the
    // app happens to be started from".
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .chain(extra_tool_dirs())
        .map(|dir| dir.join(&file))
        .find(|p| p.is_file())
        .ok_or_else(|| format!("{name} is not installed ({})", install_hint(name)))
}

#[cfg(target_os = "macos")]
fn extra_tool_dirs() -> Vec<PathBuf> {
    vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()]
}

/// winget puts ffmpeg behind a link folder, pandoc in the user's AppData and
/// ImageMagick in a versioned Program Files folder.
#[cfg(windows)]
fn extra_tool_dirs() -> Vec<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
    let programs = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| "C:\\Program Files".into());
    let mut dirs = vec![local.join("Microsoft\\WinGet\\Links"), local.join("Pandoc"), programs.join("Pandoc")];
    if let Ok(entries) = fs::read_dir(&programs) {
        dirs.extend(
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("ImageMagick"))
                .map(|e| e.path()),
        );
    }
    dirs
}

#[cfg(not(any(target_os = "macos", windows)))]
fn extra_tool_dirs() -> Vec<PathBuf> {
    vec!["/usr/local/bin".into()]
}

/// ImageMagick 7 is `magick`. Linux distributions that still ship ImageMagick 6 call
/// it `convert`, with the same options. (On Windows `convert` is a disk tool, so no.)
fn magick() -> Result<PathBuf, String> {
    tool("magick").or_else(|e| if cfg!(target_os = "linux") { tool("convert").map_err(|_| e) } else { Err(e) })
}

/// `--sandbox` arrived in pandoc 2.15; older pandoc stops at the unknown option with a
/// confusing message, so say it plainly. Ubuntu 22.04, for one, ships 2.9. A version
/// that can't be read is refused too: the sandbox is what makes pandoc safe here.
fn check_pandoc_version(pandoc: &Path) -> Result<(), String> {
    static CHECKED: OnceLock<()> = OnceLock::new();
    if CHECKED.get().is_some() {
        return Ok(());
    }
    let mut cmd = Command::new(pandoc);
    cmd.arg("--version").stdin(Stdio::null()).stderr(Stdio::null());
    quiet(&mut cmd);
    let out = cmd.output().map_err(|e| format!("could not start pandoc: {e}"))?;
    let first = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().to_string();
    let version = parse_pandoc_version(&first).ok_or("can't tell which pandoc version is installed")?;
    if version < (2, 15) {
        return Err(format!("pandoc {}.{} is too old: 2.15 or newer is needed", version.0, version.1));
    }
    let _ = CHECKED.set(());
    Ok(())
}

/// "pandoc 3.12.1" gives (3, 12).
fn parse_pandoc_version(line: &str) -> Option<(u32, u32)> {
    let mut numbers = line.split_whitespace().nth(1)?.split('.').map(|n| n.parse::<u32>().ok());
    Some((numbers.next()??, numbers.next()??))
}

fn install_hint(name: &str) -> String {
    let (brew, winget) = match name {
        "magick" => ("imagemagick", "ImageMagick.ImageMagick"),
        "ffmpeg" => ("ffmpeg", "Gyan.FFmpeg"),
        "pandoc" => ("pandoc", "JohnMacFarlane.Pandoc"),
        "soffice" => ("--cask libreoffice", "TheDocumentFoundation.LibreOffice"),
        other => (other, other),
    };
    if cfg!(target_os = "macos") {
        format!("brew install {brew}")
    } else if cfg!(windows) {
        format!("winget install {winget}")
    } else {
        let package = match name {
            "magick" => "imagemagick",
            "soffice" => "libreoffice",
            other => other,
        };
        format!("sudo apt install {package}, or your distribution's package manager")
    }
}

fn soffice() -> Result<PathBuf, String> {
    let known: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![
            "/Applications/LibreOffice.app/Contents/MacOS/soffice".into(),
            home().join("Applications/LibreOffice.app/Contents/MacOS/soffice"),
        ]
    } else if cfg!(windows) {
        ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|dir| PathBuf::from(dir).join("LibreOffice\\program\\soffice.exe"))
            .collect()
    } else {
        Vec::new()
    };
    known
        .into_iter()
        .find(|p| p.is_file())
        .map(Ok)
        .unwrap_or_else(|| {
            tool("soffice")
                .or_else(|_| tool("libreoffice"))
                .map_err(|_| format!("LibreOffice is not installed ({})", install_hint("soffice")))
        })
}

/// Our own working files: LibreOffice's profile and, on macOS, the font list.
fn cache_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Caches/local.convertwheel")
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(home).join("convertwheel")
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".cache"))
            .join("convertwheel")
    }
}

/// LibreOffice settings for converting files from anywhere: macros never run, and
/// pictures or other content linked from outside the document are not fetched (a web
/// address, or on Windows a network share, which would also send the user's sign-in).
const LIBREOFFICE_SETTINGS: &[(&str, &str, &str)] = &[
    ("/org.openoffice.Office.Common/Security/Scripting", "BlockUntrustedRefererLinks", "true"),
    ("/org.openoffice.Office.Common/Security/Scripting", "DisableMacrosExecution", "true"),
    ("/org.openoffice.Office.Common/Security/Scripting", "MacroSecurityLevel", "3"),
    // Never update links when a document opens: Writer's "never" is 0, Calc's is 1.
    ("/org.openoffice.Office.Writer/Content/Update", "Link", "0"),
    ("/org.openoffice.Office.Calc/Content/Update", "Link", "1"),
];

/// Writes the settings above into our LibreOffice profile, once. LibreOffice keeps
/// them when it rewrites the file.
fn harden_libreoffice(profile: &Path) -> Result<(), String> {
    let file = profile.join("user/registrymodifications.xcu");
    let existing = fs::read_to_string(&file).unwrap_or_default();
    let item = |(path, name, value): &(&str, &str, &str)| {
        format!("<item oor:path=\"{path}\"><prop oor:name=\"{name}\" oor:op=\"fuse\"><value>{value}</value></prop></item>")
    };
    // Every setting with exactly our value; anything else (missing, or changed by
    // something else) is written again, and the later entry wins.
    let items: String = LIBREOFFICE_SETTINGS
        .iter()
        .map(item)
        .filter(|line| !existing.contains(line.as_str()))
        .map(|line| line + "\n")
        .collect();
    if items.is_empty() {
        return Ok(());
    }
    let content = match existing.rfind("</oor:items>") {
        Some(end) => format!("{}{items}{}", &existing[..end], &existing[end..]),
        None => format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<oor:items \
             xmlns:oor=\"http://openoffice.org/2001/registry\" \
             xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" \
             xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n{items}</oor:items>\n"
        ),
    };
    fs::create_dir_all(file.parent().ok_or("no profile folder")?)
        .and_then(|_| fs::write(&file, content))
        .map_err(|e| format!("LibreOffice settings: {e}"))
}

/// Makes text safe inside an XML element.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Stand-ins for fonts a document asks for but this computer doesn't have. Aptos,
/// Word's default since 2023, ships inside Word only and its licence limits it to
/// Microsoft products, so a similar font that comes with the system stands in.
/// Calibri and Cambria need nothing: LibreOffice bundles look-alikes for them.
#[cfg(target_os = "macos")]
const FONT_SWAPS: &[(&str, &str)] = &[
    ("Aptos", "Helvetica Neue"),
    ("Aptos Display", "Helvetica Neue"),
    ("Aptos Narrow", "Helvetica Neue"),
    ("Aptos Light", "Helvetica Neue"),
    ("Aptos SemiBold", "Helvetica Neue"),
    ("Aptos ExtraBold", "Helvetica Neue"),
    ("Aptos Black", "Helvetica Neue"),
    ("Aptos Serif", "Georgia"),
    ("Aptos Mono", "Menlo"),
];
#[cfg(target_os = "linux")]
const FONT_SWAPS: &[(&str, &str)] = &[
    ("Aptos", "Noto Sans"),
    ("Aptos Display", "Noto Sans"),
    ("Aptos Narrow", "Noto Sans"),
    ("Aptos Light", "Noto Sans"),
    ("Aptos SemiBold", "Noto Sans"),
    ("Aptos ExtraBold", "Noto Sans"),
    ("Aptos Black", "Noto Sans"),
    ("Aptos Serif", "Noto Serif"),
    ("Aptos Mono", "DejaVu Sans Mono"),
];

/// Font setup for LibreOffice. On macOS, without a window it finds fonts through its
/// own copy of fontconfig, which knows only LibreOffice's bundled fonts: every macOS
/// font is missing and documents come out in a serif fallback, so this file adds the
/// macOS font folders. On Linux it builds on the system's own setup. Both add the
/// stand-ins above ("accept" means the real font still wins when it is installed).
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn font_config() -> Result<PathBuf, String> {
    let dir = cache_dir().join("fontconfig");
    let mut conf = String::from("<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n<fontconfig>\n");
    if cfg!(target_os = "macos") {
        conf.push_str("<dir>/System/Library/Fonts</dir>\n<dir>/Library/Fonts</dir>\n<dir>~/Library/Fonts</dir>\n");
        conf.push_str(&format!("<cachedir>{}</cachedir>\n", xml_text(&dir.join("cache").display().to_string())));
    } else {
        conf.push_str("<include ignore_missing=\"yes\">/etc/fonts/fonts.conf</include>\n");
    }
    for (font, stand_in) in FONT_SWAPS {
        conf.push_str(&format!(
            "<alias binding=\"same\"><family>{font}</family><accept><family>{stand_in}</family></accept></alias>\n"
        ));
    }
    if cfg!(target_os = "macos") {
        conf.push_str(
            "<alias><family>sans-serif</family><prefer><family>Helvetica Neue</family></prefer></alias>\n\
             <alias><family>serif</family><prefer><family>Times New Roman</family></prefer></alias>\n\
             <alias><family>monospace</family><prefer><family>Menlo</family></prefer></alias>\n",
        );
    }
    conf.push_str("</fontconfig>\n");
    let file = dir.join("fonts.conf");
    if fs::read_to_string(&file).ok().as_deref() != Some(conf.as_str()) {
        fs::create_dir_all(&dir)
            .and_then(|_| fs::write(&file, conf))
            .map_err(|e| format!("font settings: {e}"))?;
    }
    Ok(file)
}

/// "/a b/c" becomes "file:///a%20b/c"; "C:\\a b" becomes "file:///C:/a%20b".
fn file_url(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/");
    if !text.starts_with('/') {
        text.insert(0, '/');
    }
    let mut url = String::from("file://");
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => url.push(b as char),
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
        let work = tempfile::tempdir().unwrap();
        let src = work.path().join("report.docx");
        fs::write(&src, "x").unwrap();
        fs::write(work.path().join("report.pdf"), "keep me").unwrap();
        let tmp = work.path().join("result.tmp");
        fs::write(&tmp, "new").unwrap();

        let dest = place(&tmp, work.path(), "report", "pdf").unwrap();
        assert_eq!(dest, work.path().join("report 2.pdf"));
        assert_eq!(fs::read_to_string(work.path().join("report.pdf")).unwrap(), "keep me");
        assert_eq!(fs::read_to_string(dest).unwrap(), "new");
    }

    #[test]
    fn folders_and_missing_files_are_refused() {
        let work = tempfile::tempdir().unwrap();
        let folder = work.path().join("photos.png");
        fs::create_dir(&folder).unwrap();
        assert!(convert(&folder, "jpg", None, work.path()).unwrap_err().contains("not a regular file"));
        assert!(convert(&work.path().join("gone.png"), "jpg", None, work.path()).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn a_tool_that_hangs_is_stopped() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let started = Instant::now();
        let err = run(cmd, Duration::from_millis(500)).unwrap_err();
        assert!(err.contains("took longer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn paths_are_kept_out_of_messages() {
        let src = Path::new("/Users/someone/Secret Projects/clip.mov");
        let work = Path::new("/tmp/convert-wheel-abc");
        let msg = "ffmpeg failed: file:/Users/someone/Secret Projects/clip.mov: Invalid data";
        assert_eq!(redact(msg, src, work), "ffmpeg failed: clip.mov: Invalid data");
        assert_eq!(redact("magick: /tmp/convert-wheel-abc/input.png bad", src, work), "magick: input.png bad");
    }

    #[test]
    fn renamed_office_files_are_refused() {
        let work = tempfile::tempdir().unwrap();
        let fake = work.path().join("page.doc");
        fs::write(&fake, "<html><img src=x></html>").unwrap();
        let err = convert(&fake, "pdf", None, work.path()).unwrap_err();
        assert!(err.contains("not a valid .doc"), "{err}");
    }

    #[test]
    fn pandoc_versions_are_read() {
        assert_eq!(parse_pandoc_version("pandoc 3.12.1"), Some((3, 12)));
        assert_eq!(parse_pandoc_version("pandoc 2.9.2.1"), Some((2, 9)));
        assert_eq!(parse_pandoc_version("pandoc"), None);
        assert_eq!(parse_pandoc_version(""), None);
    }

    #[test]
    fn libreoffice_hardening_is_written_once_and_keeps_other_settings() {
        let work = tempfile::tempdir().unwrap();
        let file = work.path().join("user/registrymodifications.xcu");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "<oor:items>\n<item oor:path=\"/keep/me\"/>\n</oor:items>\n").unwrap();
        harden_libreoffice(work.path()).unwrap();
        harden_libreoffice(work.path()).unwrap();
        let s = fs::read_to_string(&file).unwrap();
        assert!(s.contains("/keep/me"));
        assert_eq!(s.matches("BlockUntrustedRefererLinks").count(), 1);
        assert_eq!(s.matches("Content/Update").count(), 2);
        assert!(s.trim_end().ends_with("</oor:items>"));
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
    #[cfg(target_os = "macos")]
    fn font_config_lists_mac_fonts_and_keeps_real_aptos_first() {
        let conf = fs::read_to_string(font_config().unwrap()).unwrap();
        assert!(conf.contains("<dir>/System/Library/Fonts</dir>"));
        // "accept" puts the stand-in after Aptos, so an installed Aptos is still used
        assert!(conf.contains("<family>Aptos</family><accept><family>Helvetica Neue</family></accept>"));
        assert!(!conf.contains("Microsoft Word.app"));
    }

    #[test]
    fn file_url_escapes_spaces() {
        assert_eq!(file_url(Path::new("/a b/c")), "file:///a%20b/c");
        assert_eq!(file_url(Path::new("C:\\Users\\a b")), "file:///C:/Users/a%20b");
    }

    #[test]
    fn unsupported_files_are_refused_before_any_tool_runs() {
        let here = Path::new("/tmp");
        assert!(convert(Path::new("/tmp/a.xyz"), "pdf", None, here).is_err());
        assert!(convert(Path::new("/tmp/Makefile"), "pdf", None, here).is_err());
        assert!(convert(Path::new("/tmp/a.png"), "mp3", None, here).is_err());
    }

    #[test]
    fn libreoffice_output_keeps_dots_in_the_name() {
        // "v1.2 notes.docx" must look for "v1.2 notes.pdf", not "v1.pdf"
        let stem = Path::new("/x/v1.2 notes.docx").file_stem().unwrap().to_string_lossy();
        assert_eq!(format!("{stem}.pdf"), "v1.2 notes.pdf");
    }
}
