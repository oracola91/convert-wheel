//! Turns one file into another format with ImageMagick, ffmpeg, pandoc, LibreOffice or
//! MuPDF (for PDFs).
//!
//! Each conversion gets a private work folder. Except for audio and video (too big to
//! copy), the file is first copied in under a neutral name, so odd characters in the
//! name can't confuse a tool and the file can't change between the checks and the
//! conversion. The result is then moved next to the original under a free name
//! ("report.pdf", "report 2.pdf", ...), so nothing is ever overwritten. Tools get
//! paths as separate arguments, never a shell line, and each has a time limit.

use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

use crate::formats::{self, Kind};
use crate::settings::Quality;

/// How long each tool may run before it is stopped. Generous: a stuck tool should not
/// block every later conversion, but a long video must still finish.
const QUICK: Duration = Duration::from_secs(120);
const OFFICE: Duration = Duration::from_secs(180);
const AUDIO: Duration = Duration::from_secs(600);
const VIDEO: Duration = Duration::from_secs(3600);

/// The only containers ffmpeg may read: the ones the wheel accepts. Anything else, a
/// playlist or concat list disguised as a video for example, is refused instead of
/// being followed to other files or the network.
const FFMPEG_FORMATS: &str = "mov,mp4,m4a,matroska,webm,avi,mp3,wav,flac,aiff,ogg,aac,asf,srt,webvtt";

/// At most 1920 on the long side, and even sizes, which the video encoders need. For
/// Compress, and for the old formats (AVI, WMV), whose players can't take 4K.
const FIT_HD: &str = "scale=w='if(gte(iw,ih),min(1920,iw),-2)':h='if(gte(iw,ih),-2,min(1920,ih))',\
                      scale=trunc(iw/2)*2:trunc(ih/2)*2";

/// The sound a conversion encodes: the first track. Phones add more that ffmpeg can't
/// decode or other containers can't hold (an iPhone's spatial audio, for one).
const FIRST_SOUND: &str = "0:a:0?";

/// The error of a conversion ended with Stop.
pub const STOPPED: &str = "stopped";

/// Pictures of a PDF's pages: at most this many pages; print resolution; and a size
/// cap, so a page that claims to be metres wide can't need gigabytes of memory.
const MAX_PAGES: usize = 100;
const PAGE_DPI: &str = "150";
const PAGE_MAX_PIXELS: &str = "4000";
/// Text is small, so many more pages are read for it.
const MAX_TEXT_PAGES: usize = 2000;
/// mutool works through a PDF this many pages at a time, and after each batch the
/// pictures or text so far must fit these budgets, so a PDF built to fill the disk is
/// stopped early instead of after the fact.
const PAGE_BATCH: usize = 10;
const TEXT_BATCH: usize = 200;
const PAGES_BUDGET: u64 = 1024 * 1024 * 1024;
const TEXT_BUDGET: u64 = 50 * 1024 * 1024;
/// Everything a PDF needs (every mutool batch and, for JPG, every page through
/// ImageMagick) together.
const PDF_TOTAL: Duration = Duration::from_secs(600);

/// What a converter produced in the work folder.
enum Output {
    /// One file, `result.<target>`.
    File,
    /// Pictures of a PDF's pages, in page order. `cut`: the PDF has more pages than
    /// MAX_PAGES, and only the first ones are here.
    Pages { files: Vec<PathBuf>, cut: bool },
}

/// Converts `src` to `target` (a format from the wheel) and returns the new file, or
/// for a PDF with several pages turned into pictures, the new folder that holds them.
/// A Tools wheel `target` ("tool:half") keeps the format and names the result after
/// the tool instead: "photo (half size).jpg".
/// It goes in `save_in`, or next to the original when that is None. If the original's
/// folder can't be written to (a mounted disk image, another app's folder), it goes to
/// `fallback` (Downloads) instead.
pub fn convert(src: &Path, target: &str, save_in: Option<&Path>, fallback: &Path) -> Result<PathBuf, String> {
    let ext = formats::extension(src).ok_or("the file has no extension")?;
    let kind = formats::kind_of(&ext).ok_or_else(|| format!(".{ext} files are not supported"))?;
    let tool = formats::is_tool(target).then_some(target);
    match tool {
        Some(t) if !formats::tool_takes(t, &ext) => return Err(format!(".{ext} files can't be {}", tool_done(t))),
        Some(t) if formats::combines(t) => return Err(combine_words(t).map_or("", |w| w.0).into()),
        None if !formats::outputs_for(&ext).contains(&target) => return Err(format!(".{ext} files can't become .{target}")),
        _ => {}
    }
    let base = src
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("the file name is not valid text")?;
    let (stem, out_ext) = match tool {
        Some(t) => (format!("{base} ({})", tool_suffix(t)), tool_out_ext(t, &ext)),
        None => (base.to_string(), target.to_string()),
    };
    // Not a folder, a device or a pipe (a pipe would block the read below forever).
    let meta = fs::metadata(src).map_err(|e| format!("can't read the file: {e}"))?;
    if !meta.is_file() {
        return Err("this is not a regular file".into());
    }
    let work = tempfile::Builder::new()
        .prefix("convert-wheel-")
        .tempdir()
        .map_err(|e| format!("work folder: {e}"))?;
    let out = work.path().join(format!("result.{out_ext}"));

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
        // Pages, Numbers and Keynote save a ZIP file too (an older package folder is
        // refused above, as not a regular file).
        "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "epub" | "pages" | "numbers" | "key" => b"PK",
        "doc" | "xls" | "ppt" => &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1],
        "rtf" => b"{\\rtf",
        "pdf" => b"%PDF-",
        _ => b"",
    };
    if !head.starts_with(expected) {
        return Err(format!("this is not a valid .{ext} file"));
    }

    let file = |done: Result<(), String>| done.map(|()| Output::File);
    let converted = match (tool, kind) {
        (Some(t), Kind::Image) => file(image_tool(&input, t, &out)),
        (Some(t), Kind::Pdf) => pdf_tool(&input, t, &out, &work),
        (Some(t), Kind::Audio | Kind::Video) => file(media_tool(&input, &ext, t, &out)),
        (Some(_), _) => Err("the Tools wheel has no tools for this kind of file yet".to_string()),
        (None, Kind::Image) => file(image(&input, target, &out)),
        (None, Kind::Audio) => file(audio(&input, target, &out)),
        (None, Kind::Video) => file(video(&input, &ext, target, &out)),
        (None, Kind::Doc) => file(document(&input, &ext, target, &out, &work)),
        (None, Kind::Sheet | Kind::Slides) => file(libreoffice(&input, target, &out)),
        (None, Kind::Pdf) => pdf(&input, target, &out, &work),
        (None, Kind::Subtitles) => file(ffmpeg(&input, &["-map", "0:s:0"], &out, QUICK)),
    };
    // Tool messages often start with a full path; keep only file names.
    let output = converted.map_err(|e| redact(&e, src, work.path()))?;
    if matches!(output, Output::File) && !out.is_file() {
        return Err("the converter finished but wrote no file".into());
    }
    // A compressed copy that is no smaller is no use: say so instead of saving it.
    if let Some(t @ ("tool:compress" | "tool:smaller")) = tool {
        if fs::metadata(&out).map_or(true, |m| m.len() >= meta.len()) {
            let name = if t == "tool:compress" { "Compress" } else { "Smaller" };
            return Err(format!("it is already as small as {name} makes it"));
        }
    }
    let dir = match save_in {
        Some(dir) => dir.to_path_buf(),
        None => src.parent().ok_or("the file has no folder")?.to_path_buf(),
    };
    let save = |dir: &Path| match &output {
        Output::File => place(&out, dir, &stem, &out_ext),
        // A split PDF: "report (pages)/report 01.pdf". A PDF's pages as pictures:
        // "report (PNG)/report 01.png".
        Output::Pages { files, cut } if tool.is_some() => place_pages(files, *cut, dir, base, "pages", &out_ext),
        Output::Pages { files, cut } => place_pages(files, *cut, dir, base, &target.to_uppercase(), target),
    };
    let placed = match save(&dir) {
        Err(e) if save_in.is_none() && is_no_write(&e) => fs::create_dir_all(fallback).and_then(|_| save(fallback)),
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
pub(crate) fn is_no_write(e: &io::Error) -> bool {
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
    cmd.args(IMAGE_LIMITS);
    cmd.arg(format!("{coder}:{}{frame}", src.display())).arg("-auto-orient");
    if target == "jpg" {
        // JPEG has no transparency; without this, transparent areas turn black.
        cmd.args(["-background", "white", "-alpha", "remove", "-alpha", "off"]);
    }
    let level = quality().pictures;
    if matches!(target, "jpg" | "webp") {
        cmd.args(["-quality", level.pick("75", "90", "97")]);
    }
    // ImageMagick's own default for these (libheif's 50) is Balanced.
    if let (true, Some(q)) = (matches!(target, "heic" | "avif"), level.pick(Some("40"), None, Some("85"))) {
        cmd.args(["-quality", q]);
    }
    if target == "ico" {
        // Icons are square, 256 at most, and hold several sizes. A picture that isn't
        // square is fitted in and padded with transparency rather than stretched.
        cmd.args(["-resize", "256x256", "-background", "none", "-gravity", "center", "-extent", "256x256"]);
        cmd.args(["-define", "icon:auto-resize=256,128,64,48,32,16"]);
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

/// Caps for a small file that claims to be a huge picture, before it is decoded.
const IMAGE_LIMITS: &[&str] = &[
    "-limit", "memory", "1GiB", "-limit", "map", "2GiB", "-limit", "disk", "4GiB",
    "-limit", "width", "32KP", "-limit", "height", "32KP", "-limit", "area", "1GP",
    "-limit", "time", "110",
];

/// What a Tools wheel tool adds to the file name: "photo (half size).jpg".
fn tool_suffix(tool: &str) -> &'static str {
    match tool {
        "tool:compress" => "compressed",
        "tool:half" => "half size",
        "tool:clean" => "no metadata",
        "tool:rotate" => "rotated",
        "tool:merge" => "merged",
        "tool:onepdf" => "one PDF",
        "tool:smaller" => "smaller",
        "tool:still" => "first frame",
        "tool:loudness" => "even loudness",
        "tool:mono" => "mono",
        "tool:crop" => "cropped",
        _ => "changed",
    }
}

/// What a tool does, for messages: ".gif files can't be rotated".
fn tool_done(tool: &str) -> &'static str {
    match tool {
        "tool:compress" => "compressed",
        "tool:half" => "resized",
        "tool:clean" => "cleaned of metadata",
        "tool:rotate" => "rotated",
        "tool:merge" => "merged",
        "tool:onepdf" => "put into one PDF",
        "tool:split" => "split",
        "tool:smaller" => "made smaller",
        "tool:still" => "turned into a picture",
        "tool:loudness" => "evened out",
        "tool:mono" => "mixed down to mono",
        "tool:crop" => "cropped",
        _ => "changed",
    }
}

/// The format a tool's result is in: the file's own, except First Frame's picture.
fn tool_out_ext(tool_id: &str, ext: &str) -> String {
    match tool_id {
        "tool:still" => "jpg".into(),
        _ => formats::canonical(ext).into(),
    }
}

/// Loudness the way podcasts and streaming services aim for it (EBU R128 style):
/// -16 LUFS, peaks at most -1.5 dB, a loudness range of 11.
const LOUDNORM: &str = "loudnorm=I=-16:TP=-1.5:LRA=11";

/// Runs a Tools wheel tool on audio or video (`ext`, its own format). Like the
/// conversions, it goes through `ffmpeg`, locked down to local files and our formats.
fn media_tool(src: &Path, ext: &str, tool_id: &str, out: &Path) -> Result<(), String> {
    let own = formats::canonical(ext);
    let video = formats::kind_of(ext) == Some(Kind::Video);
    match tool_id {
        // The picture and sound as they are, without the file's and the streams'
        // metadata (where and when it was recorded, the device, tags, cover art) or
        // the data tracks phones keep it in. A copy: quick, nothing is encoded again.
        "tool:clean" => {
            let streams: &[&str] = if video { &["-map", "0:v", "-map", "0:a?"] } else { &["-map", "0:a"] };
            let mut args = streams.to_vec();
            args.extend(["-map_metadata", "-1", "-map_chapters", "-1", "-c", "copy", "-fflags", "+bitexact"]);
            ffmpeg(src, &args, out, AUDIO)
        }
        "tool:compress" => {
            let mut args = vec!["-map", "0:v:0", "-map", FIRST_SOUND, "-vf", FIT_HD];
            if own == "webm" {
                args.extend(["-c:v", "libvpx-vp9", "-crf", "38", "-b:v", "0", "-deadline", "good", "-cpu-used", "4"]);
                args.extend(["-row-mt", "1", "-c:a", "libopus", "-b:a", "96k"]);
            } else {
                args.extend(["-c:v", "libx264", "-preset", "medium", "-crf", "28", "-pix_fmt", "yuv420p"]);
                args.extend(["-c:a", "aac", "-b:a", "128k"]);
                if own != "mkv" {
                    args.extend(["-movflags", "+faststart"]);
                }
            }
            ffmpeg(src, &args, out, VIDEO)
        }
        "tool:still" => ffmpeg(src, &["-map", "0:v:0", "-frames:v", "1", "-q:v", "2", "-update", "1"], out, QUICK),
        "tool:loudness" | "tool:mono" => {
            let mut args = vec!["-map", "0:a:0", "-map_metadata", "0"];
            let rate;
            if tool_id == "tool:loudness" {
                // loudnorm works at 192 kHz; the result goes back to the file's own rate.
                rate = sample_rate(src, out).unwrap_or(48_000).to_string();
                args.extend(["-af", LOUDNORM, "-ar", &rate]);
            } else {
                args.extend(["-ac", "1"]);
            }
            args.extend(audio_args(own));
            ffmpeg(src, &args, out, AUDIO)
        }
        other => Err(format!("unknown tool {other}")),
    }
}

/// The sample rate of a file's first audio stream, from ffprobe, locked down like
/// ffmpeg. None when ffprobe is missing or can't tell.
fn sample_rate(src: &Path, out: &Path) -> Option<u32> {
    let answer = out.with_file_name("rate.txt");
    let mut cmd = Command::new(tool("ffprobe").ok()?);
    cmd.args(["-v", "error", "-protocol_whitelist", "file", "-format_whitelist", FFMPEG_FORMATS])
        .args(["-select_streams", "a:0", "-show_entries", "stream=sample_rate"])
        .args(["-of", "default=noprint_wrappers=1:nokey=1", "-o"])
        .arg(format!("file:{}", answer.display()))
        .arg(format!("file:{}", src.display()));
    run(cmd, QUICK).ok()?;
    fs::read_to_string(&answer).ok()?.trim().parse().ok().filter(|r| (8_000..=384_000).contains(r))
}

/// Runs a Tools wheel tool on a picture. The result is written in the picture's real
/// format, upright, with its colour profile; other metadata stays too, except with
/// Remove Metadata.
fn image_tool(src: &Path, tool: &str, out: &Path) -> Result<(), String> {
    let coder = sniff_image(&read_head(src)?).ok_or("this is not an image format the wheel can read")?;
    let mut cmd = Command::new(magick()?);
    cmd.args(IMAGE_LIMITS);
    // Upright first, so a photo that was only turned by its EXIF tag still looks the
    // same once the tag is gone or the picture is changed.
    cmd.arg(format!("{coder}:{}[0]", src.display())).arg("-auto-orient");
    match tool {
        // PNG has no quality setting: fewer colours (256, dithered) is what shrinks it.
        "tool:compress" if coder == "png" => cmd.args(["-colors", "256"]),
        "tool:compress" if coder == "tiff" => cmd.args(["-compress", "zip"]),
        "tool:compress" if coder == "jpeg" => cmd.args(["-quality", "70"]),
        // The newer formats look as good as JPEG at a lower number. HEIC's numbers run
        // higher than the others': at the same one its files are about twice the size.
        "tool:compress" if coder == "heic" => cmd.args(["-quality", "45"]),
        "tool:compress" => cmd.args(["-quality", "55"]),
        "tool:half" => cmd.args(["-resize", "50%"]),
        // Every profile but the colour one (EXIF with the camera and location, XMP,
        // IPTC), and comments.
        "tool:clean" => cmd.args(["+profile", "!icc,*", "+set", "comment"]),
        "tool:rotate" => cmd.args(["-rotate", "90"]),
        "tool:crop" => {
            let crop = control().and_then(|c| c.crop).ok_or("Crop opens its window from the wheel")?;
            if crop.turns > 0 {
                cmd.args(["-rotate", &(90 * u32::from(crop.turns)).to_string()]);
            }
            cmd.args(["-crop", &format!("{}x{}+{}+{}", crop.width, crop.height, crop.x, crop.y), "+repage"])
        }
        other => return Err(format!("unknown tool {other}")),
    };
    // Written again, a lossy picture should look and weigh about the same. JPEG keeps
    // the original's own quality when none is given; for the newer formats ImageMagick
    // can't tell. For WebP and AVIF 80 is close to what browsers use (90 made files three
    // times bigger); HEIC needs 60 for about the size an iPhone writes (80 made them two
    // and a half times bigger).
    if tool != "tool:compress" && matches!(coder, "webp" | "heic" | "avif") {
        cmd.args(["-quality", if coder == "heic" { "60" } else { "80" }]);
    }
    if coder == "png" {
        // ImageMagick would otherwise add its own dates to a PNG.
        cmd.args(["-define", "png:exclude-chunks=date,time"]);
    }
    cmd.arg(format!("{coder}:{}", out.display()));
    run(cmd, QUICK)
}

/// The encoder for an audio format, at the menu's audio quality. WAV, FLAC and AIFF
/// are lossless and have none.
fn audio_args(target: &str) -> Vec<&'static str> {
    let level = quality().audio;
    match target {
        // VBR: about 130, 190 and 245 kbit/s.
        "mp3" => vec!["-c:a", "libmp3lame", "-q:a", level.pick("5", "2", "0")],
        // Tools only for "aac": they keep the file's own format.
        "m4a" | "aac" => vec!["-c:a", "aac", "-b:a", level.pick("160k", "256k", "320k")],
        "wav" => vec!["-c:a", "pcm_s16le"],
        "flac" => vec!["-c:a", "flac"],
        "aiff" => vec!["-c:a", "pcm_s16be"],
        // Homebrew's ffmpeg has no Vorbis encoder; Opus in an Ogg file plays everywhere Ogg does.
        "ogg" | "opus" => vec!["-c:a", "libopus", "-b:a", level.pick("96k", "160k", "256k")],
        "wma" => vec!["-c:a", "wmav2", "-b:a", level.pick("128k", "192k", "256k")],
        _ => Vec::new(),
    }
}

fn ffmpeg(src: &Path, args: &[&str], out: &Path, limit: Duration) -> Result<(), String> {
    let mut cmd = Command::new(tool("ffmpeg")?);
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
    // A long run writes how far it is to a file next to the result, for the menu. Quick
    // ones (a stream copy) aren't worth asking ffprobe for the length first.
    let progress = out.with_extension("progress");
    let watch = match control() {
        Some(control) if limit > QUICK => media_length(src, out).map(|length| (control, length)),
        _ => None,
    };
    if watch.is_some() {
        cmd.arg("-progress").arg(format!("file:{}", progress.display()));
    }
    // Local files only, our containers only, and the input named as a file so that a
    // name like "concat:..." can't be read as an instruction.
    cmd.args(["-protocol_whitelist", "file", "-format_whitelist", FFMPEG_FORMATS, "-i"])
        .arg(format!("file:{}", src.display()))
        .args(args)
        .arg(out);
    let Some((control, length)) = watch else {
        return run(cmd, limit);
    };
    // Twice a second, the time ffmpeg has reached against the whole length.
    let finished = AtomicBool::new(false);
    let result = thread::scope(|s| {
        s.spawn(|| {
            while !finished.load(Ordering::SeqCst) {
                if let Some(at) = progress_at(&file_tail(&progress)) {
                    control.set_percent(Some(percent_of(at, length)));
                }
                thread::sleep(Duration::from_millis(500));
            }
        });
        let result = run(cmd, limit);
        finished.store(true, Ordering::SeqCst);
        result
    });
    control.set_percent(None);
    result
}

/// How long an audio or video file plays, from ffprobe, locked down like ffmpeg. None
/// when ffprobe is missing or can't tell: the conversion then runs without a percent.
fn media_length(src: &Path, out: &Path) -> Option<Duration> {
    let answer = out.with_file_name("length.txt");
    let mut cmd = Command::new(tool("ffprobe").ok()?);
    cmd.args(["-v", "error", "-protocol_whitelist", "file", "-format_whitelist", FFMPEG_FORMATS])
        .args(["-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1", "-o"])
        .arg(format!("file:{}", answer.display()))
        .arg(format!("file:{}", src.display()));
    run(cmd, QUICK).ok()?;
    let seconds: f64 = fs::read_to_string(&answer).ok()?.trim().parse().ok()?;
    (seconds.is_finite() && seconds > 0.0).then(|| Duration::from_secs_f64(seconds))
}

/// The end of a file that keeps growing, without reading all of it each time.
fn file_tail(path: &Path) -> String {
    let Ok(mut file) = fs::File::open(path) else {
        return String::new();
    };
    let len = file.seek(SeekFrom::End(0)).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(2048)));
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The time ffmpeg has reached, from the last complete `out_time_us=` line of its
/// -progress output. It says "N/A" before the first frame.
fn progress_at(text: &str) -> Option<Duration> {
    text.lines()
        .rev()
        .filter_map(|l| l.strip_prefix("out_time_us="))
        .find_map(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_micros)
}

/// 99 at most: the last percent comes when the file is really in place.
fn percent_of(at: Duration, length: Duration) -> u8 {
    (at.as_secs_f64() / length.as_secs_f64() * 100.0).clamp(0.0, 99.0) as u8
}

fn audio(src: &Path, target: &str, out: &Path) -> Result<(), String> {
    let mut args = vec!["-vn", "-map_metadata", "0"];
    args.extend(audio_args(target));
    ffmpeg(src, &args, out, AUDIO)
}

/// A fast copy of the picture and sound into another container: with every sound
/// track, or else with the first only, as a phone's extra tracks may not fit. False
/// when neither works and the video has to be encoded.
fn copy_streams(src: &Path, extra: &[&str], out: &Path) -> bool {
    for sound in ["0:a?", FIRST_SOUND] {
        let mut args = vec!["-map", "0:v", "-map", sound, "-c", "copy"];
        args.extend(extra);
        // A copy is quick or fails quickly; it never needs the video limit.
        if ffmpeg(src, &args, out, QUICK).is_ok() {
            return true;
        }
        let _ = fs::remove_file(out);
    }
    false
}

fn video(src: &Path, ext: &str, target: &str, out: &Path) -> Result<(), String> {
    // H.264 in yuv420p needs even width and height; phone and screen recordings often
    // aren't, so round down by a pixel where needed.
    let level = quality().video;
    let h264: Vec<&str> = vec![
        "-map", "0:v:0", "-map", FIRST_SOUND, "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", "-c:v", "libx264",
        "-preset", "medium", "-crf", level.pick("26", "20", "17"), "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k",
        "-movflags", "+faststart",
    ];
    // MPEG-4 Part 2 and Windows Media 8 take a fixed quantiser: lower is better.
    let quantiser = level.pick("5", "3", "2");
    match target {
        "mp4" | "mov" => {
            // MP4, MOV and M4V usually hold the same streams: try a fast copy first.
            if matches!(ext, "mp4" | "mov" | "m4v") && copy_streams(src, &["-movflags", "+faststart"], out) {
                return Ok(());
            }
            ffmpeg(src, &h264, out, VIDEO)
        }
        "webm" => ffmpeg(
            src,
            &[
                "-map", "0:v:0", "-map", FIRST_SOUND, "-c:v", "libvpx-vp9", "-crf", level.pick("38", "32", "26"), "-b:v", "0",
                "-deadline", "good", "-cpu-used", "4", "-row-mt", "1", "-c:a", "libopus", "-b:a", "128k",
            ],
            out,
            VIDEO,
        ),
        "mkv" => {
            // Matroska holds nearly any streams: a fast copy first, as for MP4.
            if copy_streams(src, &[], out) {
                return Ok(());
            }
            // H.264 without MP4's own "+faststart".
            ffmpeg(src, &h264[..h264.len() - 2], out, VIDEO)
        }
        // For older players: MPEG-4 Part 2 (what Xvid makes) with MP3, and Windows
        // Media 8 with WMA.
        "avi" => ffmpeg(
            src,
            &[
                "-map", "0:v:0", "-map", FIRST_SOUND, "-vf", FIT_HD, "-c:v", "mpeg4", "-vtag", "xvid",
                "-q:v", quantiser, "-c:a", "libmp3lame", "-q:a", "4",
            ],
            out,
            VIDEO,
        ),
        "wmv" => ffmpeg(
            src,
            &[
                "-map", "0:v:0", "-map", FIRST_SOUND, "-vf", FIT_HD, "-c:v", "wmv2", "-q:v", quantiser,
                "-c:a", "wmav2", "-b:a", "192k",
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
    let office_in = matches!(ext, "docx" | "doc" | "odt" | "rtf" | "pages");
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
        _ if matches!(ext, "doc" | "rtf" | "pages") => {
            // pandoc can't read .doc or Pages, and reads RTF only in recent versions:
            // LibreOffice turns them into .docx first.
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
    run(cmd, QUICK).map_err(|e| {
        // Ubuntu's and Debian's pandoc keeps its data files outside the program, and in
        // the sandbox it can't reach them.
        if e.contains("Could not find data file") {
            "this pandoc can't write that format in its sandbox (Ubuntu's and Debian's builds can't): \
             install pandoc from github.com/jgm/pandoc/releases"
                .to_string()
        } else {
            e
        }
    })
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

/// PDFs, read with MuPDF's mutool: a picture of each page, or the text. The text comes
/// out as paragraphs, with the lines of each joined back up, as plain text or as a Word
/// file; the PDF's layout, pictures and tables aren't kept. (LibreOffice can open a PDF
/// too, but makes every line a separate floating text box.)
fn pdf(src: &Path, target: &str, out: &Path, work: &TempDir) -> Result<Output, String> {
    let mutool = Mutool { path: tool("mutool")?, src, deadline: Instant::now() + PDF_TOTAL };
    let count = mutool.page_count(work.path())?;
    if count == 0 {
        return Err("this PDF has no pages".into());
    }
    match target {
        "png" | "jpg" => {
            let dir = work.path().join("pages");
            fs::create_dir(&dir).map_err(|e| format!("work folder: {e}"))?;
            let last = count.min(MAX_PAGES);
            for first in (1..=last).step_by(PAGE_BATCH) {
                let pages = format!("{first}-{}", (first + PAGE_BATCH - 1).min(last));
                // Inside the pages folder, so the "%d" in the output name is the only
                // "%" mutool sees, whatever the work folder's path holds.
                mutool.run(&dir, &["draw", "-q", "-r", PAGE_DPI, "-w", PAGE_MAX_PIXELS, "-h", PAGE_MAX_PIXELS, "-o", "%d.png"], &pages)?;
                if folder_size(&dir)? > PAGES_BUDGET {
                    return Err("this PDF's pages are too large to draw: the pictures would take more than 1 GB".into());
                }
            }
            let mut files: Vec<PathBuf> = (1..=last).map(|n| dir.join(format!("{n}.png"))).collect();
            if let Some(missing) = files.iter().position(|p| !p.is_file()) {
                return Err(format!("MuPDF didn't draw page {} of this PDF", missing + 1));
            }
            if target == "jpg" {
                for png in &mut files {
                    mutool.time_left()?;
                    let jpg = png.with_extension("jpg");
                    image(png, "jpg", &jpg)?;
                    *png = jpg;
                }
            }
            if let [one] = files.as_slice() {
                fs::rename(one, out).map_err(|e| format!("work folder: {e}"))?;
                return Ok(Output::File);
            }
            Ok(Output::Pages { files, cut: count > MAX_PAGES })
        }
        "txt" | "docx" => {
            if count > MAX_TEXT_PAGES {
                return Err(format!("this PDF has {count} pages: the text can be taken out of PDFs up to {MAX_TEXT_PAGES} pages"));
            }
            let mut bytes = Vec::new();
            for first in (1..=count).step_by(TEXT_BATCH) {
                let pages = format!("{first}-{}", (first + TEXT_BATCH - 1).min(count));
                mutool.run(work.path(), &["draw", "-q", "-F", "txt", "-o", "text.txt"], &pages)?;
                let part = work.path().join("text.txt");
                let size = fs::metadata(&part).map_err(|e| format!("work folder: {e}"))?.len();
                if bytes.len() as u64 + size > TEXT_BUDGET {
                    return Err("this PDF holds more than 50 MB of text, too much to take out".into());
                }
                bytes.extend(fs::read(&part).map_err(|e| format!("work folder: {e}"))?);
            }
            let text = paragraphs(&String::from_utf8_lossy(&bytes));
            if text.is_empty() {
                return Err("this PDF has no text to take out (its pages may be scans)".into());
            }
            if target == "txt" {
                fs::write(out, text.join("\n\n") + "\n").map_err(|e| format!("work folder: {e}"))?;
            } else {
                let html = work.path().join("text.html");
                fs::write(&html, paragraphs_html(&text)).map_err(|e| format!("work folder: {e}"))?;
                pandoc(&html, "html", "docx", out)?;
            }
            Ok(Output::File)
        }
        other => Err(format!("a PDF can't become .{other}")),
    }
}

/// Most pages Split makes files of; the folder says so when the PDF had more.
const MAX_SPLIT: usize = 100;

/// Smaller: pictures above 225 dpi are scaled down to 150 dpi, and photos are saved
/// again as JPEG at quality 60, each only where that makes it smaller. Fonts and
/// streams are compressed, unused and duplicate objects go.
const SMALLER: &[&str] = &[
    "-gggg", "-z", "-f", "-i", "-Z",
    "--color-lossy-image-subsample-dpi", "225,150", "--color-lossy-image-recompress-method", "jpeg:60",
    "--gray-lossy-image-subsample-dpi", "225,150", "--gray-lossy-image-recompress-method", "jpeg:60",
    "--color-lossless-image-subsample-dpi", "225,150", "--gray-lossless-image-subsample-dpi", "225,150",
];

/// Older MuPDF (Ubuntu's own packages) has none of the picture options: there Smaller
/// only tidies the file, which seldom makes it much smaller.
const SMALLER_OLD_MUPDF: &[&str] = &["-ggg", "-z"];

/// Runs a Tools wheel tool on a PDF (Merge, which takes several, is `combine`).
fn pdf_tool(src: &Path, tool_id: &str, out: &Path, work: &TempDir) -> Result<Output, String> {
    let mutool = Mutool { path: tool("mutool")?, src, deadline: Instant::now() + PDF_TOTAL };
    let count = mutool.page_count(work.path())?;
    if count == 0 {
        return Err("this PDF has no pages".into());
    }
    let out_name = out.file_name().and_then(|n| n.to_str()).ok_or("work folder name")?;
    match tool_id {
        "tool:split" => {
            if count == 1 {
                return Err("this PDF has only one page".into());
            }
            let dir = work.path().join("pages");
            fs::create_dir(&dir).map_err(|e| format!("work folder: {e}"))?;
            let last = count.min(MAX_SPLIT);
            let files: Vec<PathBuf> = (1..=last).map(|n| dir.join(format!("{n}.pdf"))).collect();
            for (n, file) in (1..=last).zip(&files) {
                let name = format!("{n}.pdf");
                mutool.run_args(&dir, &["merge", "-o", &name], &[&n.to_string()])?;
                if !file.is_file() {
                    return Err(format!("MuPDF didn't copy page {n} of this PDF"));
                }
            }
            Ok(Output::Pages { files, cut: count > MAX_SPLIT })
        }
        "tool:smaller" => {
            let mut args = vec!["clean"];
            args.extend(SMALLER);
            match mutool.run_failure(work.path(), &args, &[out_name]) {
                Err(f) if f.output.contains("usage:") => {
                    let mut args = vec!["clean"];
                    args.extend(SMALLER_OLD_MUPDF);
                    mutool.run_args(work.path(), &args, &[out_name])?;
                }
                other => other.map_err(|f| f.message)?,
            }
            Ok(Output::File)
        }
        other => Err(format!("unknown tool {other}")),
    }
}

/// Most pictures One PDF takes.
const MAX_PICTURES: usize = 200;

/// For the tools that make one file of a whole drop: what to say when there are too
/// few files, and when one doesn't belong.
fn combine_words(tool_id: &str) -> Option<(&'static str, &'static str)> {
    match tool_id {
        "tool:merge" => Some(("Merge needs two PDFs or more", "only PDFs can be merged")),
        "tool:onepdf" => Some(("One PDF needs two pictures or more", "only pictures can go into one PDF")),
        _ => None,
    }
}

/// Makes one PDF of the dropped files, in the order they were dropped: the Tools
/// wheel's Merge (PDFs) and One PDF (pictures, a page each). The result goes next to
/// the first file, named after it: "report (merged).pdf", "IMG_1234 (one PDF).pdf".
/// MuPDF's merge leaves out a PDF it can't read and still succeeds, so the pages are
/// counted before and after.
pub fn combine(srcs: &[PathBuf], target: &str, save_in: Option<&Path>, fallback: &Path) -> Result<PathBuf, String> {
    let (too_few, only) = combine_words(target).ok_or_else(|| format!("{target} doesn't combine files"))?;
    let [first, _, ..] = srcs else {
        return Err(too_few.into());
    };
    let pictures = target == "tool:onepdf";
    if pictures && srcs.len() > MAX_PICTURES {
        return Err(format!("One PDF takes up to {MAX_PICTURES} pictures"));
    }
    let work = tempfile::Builder::new()
        .prefix("convert-wheel-")
        .tempdir()
        .map_err(|e| format!("work folder: {e}"))?;
    let deadline = Instant::now() + PDF_TOTAL;
    let mutool = tool("mutool")?;
    let mut inputs = Vec::new();
    let mut pages = 0;
    for (i, src) in srcs.iter().enumerate() {
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let fail = |e: String| format!("{name}: {}", redact(&e, src, work.path()));
        let ext = formats::extension(src).unwrap_or_default();
        if !formats::tool_takes(target, &ext) {
            return Err(fail(only.into()));
        }
        let meta = fs::metadata(src).map_err(|e| fail(format!("can't read the file: {e}")))?;
        if !meta.is_file() {
            return Err(fail("this is not a regular file".into()));
        }
        let staged = work.path().join(format!("input-{i}.{ext}"));
        fs::copy(src, &staged).map_err(|e| fail(format!("can't read the file: {e}")))?;
        if pictures {
            let page = picture_page(&staged, work.path(), i).map_err(fail)?;
            let as_pdf = format!("page-{i}.pdf");
            let picture = Mutool { path: mutool.clone(), src: &page, deadline };
            picture.run_args(work.path(), &["convert", "-o", &as_pdf], &[]).map_err(fail)?;
            inputs.push(work.path().join(as_pdf));
            pages += 1;
        } else {
            if !read_head(&staged)?.starts_with(b"%PDF-") {
                return Err(fail("this is not a valid .pdf file".into()));
            }
            let pdf = Mutool { path: mutool.clone(), src: &staged, deadline };
            pages += pdf.page_count(work.path()).map_err(fail)?;
            inputs.push(staged);
        }
    }
    if pages > MAX_TEXT_PAGES {
        return Err(format!("together these PDFs have {pages} pages: Merge takes up to {MAX_TEXT_PAGES}"));
    }
    let out = work.path().join("result.pdf");
    let mut cmd = Command::new(&mutool);
    cmd.current_dir(work.path()).args(["merge", "-o", "result.pdf"]).args(&inputs);
    let limit = deadline.saturating_duration_since(Instant::now()).min(OFFICE);
    run_tool(cmd, limit).map_err(|f| redact(&password_message(f), first, work.path()))?;
    let merged = Mutool { path: mutool.clone(), src: &out, deadline };
    if !out.is_file() || merged.page_count(work.path())? != pages {
        let what = if pictures { "pictures" } else { "PDFs" };
        return Err(format!("MuPDF couldn't read one of the {what}"));
    }
    let stem = first.file_stem().and_then(|s| s.to_str()).ok_or("the file name is not valid text")?;
    let stem = format!("{stem} ({})", tool_suffix(target));
    let dir = match save_in {
        Some(dir) => dir.to_path_buf(),
        None => first.parent().ok_or("the file has no folder")?.to_path_buf(),
    };
    let placed = match place(&out, &dir, &stem, "pdf") {
        Err(e) if save_in.is_none() && is_no_write(&e) => fs::create_dir_all(fallback).and_then(|_| place(&out, fallback, &stem, "pdf")),
        other => other,
    };
    placed.map_err(|e| format!("can't save the result: {e}"))
}

/// A picture as page `n` of One PDF, in `work`: upright, on white where it was see-through,
/// and marked with the resolution that makes its long side as long as an A4 page's
/// (29.7 cm), which MuPDF then takes as the page size. MuPDF won't go below 72 dpi, so
/// a picture under 842 pixels keeps a smaller page. Photos go in as JPEG, the rest as
/// PNG, so screenshots stay sharp.
fn picture_page(src: &Path, work: &Path, n: usize) -> Result<PathBuf, String> {
    let coder = sniff_image(&read_head(src)?).ok_or("this is not an image format the wheel can read")?;
    let photo = matches!(coder, "jpeg" | "heic" | "webp" | "avif");
    let page = work.join(format!("page-{n}.{}", if photo { "jpg" } else { "png" }));
    let mut cmd = Command::new(magick()?);
    cmd.args(IMAGE_LIMITS);
    cmd.arg(format!("{coder}:{}[0]", src.display())).arg("-auto-orient");
    cmd.args(["-background", "white", "-alpha", "remove", "-alpha", "off"]);
    cmd.args(["-units", "PixelsPerInch", "-set", "density", "%[fx:max(w,h)/11.69]"]);
    if photo {
        cmd.args(["-quality", "92"]);
    }
    cmd.arg(format!("{}:{}", if photo { "jpeg" } else { "png" }, page.display()));
    run(cmd, QUICK)?;
    Ok(page)
}

/// mutool's last line on a failure is only "cannot draw ...": say why when the reason
/// is a known one.
fn password_message(f: Failure) -> String {
    if f.output.contains("cannot authenticate password") {
        "this PDF is password-protected: open it, save a copy without the password, and convert that".into()
    } else {
        f.message
    }
}

/// mutool for one PDF, within the time all of that PDF's work may take.
struct Mutool<'a> {
    path: PathBuf,
    src: &'a Path,
    deadline: Instant,
}

impl Mutool<'_> {
    /// The time left for this PDF, or an error once it is used up.
    fn time_left(&self) -> Result<Duration, String> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("this PDF took longer than {} to convert and was stopped", span(PDF_TOTAL)));
        }
        Ok(left)
    }

    /// Runs `mutool <args> <the PDF> <pages>` in `dir` (output names are relative to it).
    fn run(&self, dir: &Path, args: &[&str], pages: &str) -> Result<(), String> {
        self.run_args(dir, args, &[pages])
    }

    /// Runs `mutool <args> <the PDF> <after>` in `dir`.
    fn run_args(&self, dir: &Path, args: &[&str], after: &[&str]) -> Result<(), String> {
        self.run_failure(dir, args, after).map_err(password_message)
    }

    /// `run_args`, keeping mutool's own output on a failure.
    fn run_failure(&self, dir: &Path, args: &[&str], after: &[&str]) -> Result<(), Failure> {
        let mut cmd = Command::new(&self.path);
        cmd.current_dir(dir).args(args).arg(self.src).args(after);
        let limit = self.time_left()?.min(OFFICE);
        run_tool(cmd, limit)
    }

    /// The number of pages, as MuPDF reads the PDF. On a PDF that needs a password
    /// `mutool show` only warns and prints "null"; `mutool info` fails and says why.
    fn page_count(&self, work: &Path) -> Result<usize, String> {
        self.run(work, &["show", "-o", "count.txt"], "trailer/Root/Pages/Count")?;
        let text = fs::read_to_string(work.join("count.txt")).unwrap_or_default();
        if let Some(count) = parse_count(&text) {
            return Ok(count);
        }
        self.run_args(work, &["info"], &[])?;
        Err("MuPDF couldn't tell how many pages this PDF has".to_string())
    }
}

/// The page count from `mutool show`: the number itself, or, when the PDF keeps it in
/// an object of its own, the number between "6 0 obj" and "endobj".
fn parse_count(text: &str) -> Option<usize> {
    text.lines().map(str::trim).find(|l| !l.is_empty() && !l.ends_with("obj"))?.parse().ok()
}

/// The total size of the files in a folder.
fn folder_size(dir: &Path) -> Result<u64, String> {
    let mut total = 0;
    for entry in fs::read_dir(dir).map_err(|e| format!("work folder: {e}"))? {
        total += entry.and_then(|e| e.metadata()).map_err(|e| format!("work folder: {e}"))?.len();
    }
    Ok(total)
}

/// Paragraphs from mutool's text: they are separated by empty lines (and pages by a
/// form feed), and the lines inside one are joined back up. A word broken with a
/// hyphen at a line end ("conver-" / "sion") joins the next line without a space and
/// keeps the hyphen, so nothing is lost; a dash after a space or a digit ("2020 -")
/// keeps its space. Control characters, which Word files can't hold, are left out.
fn paragraphs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let line: String = line.chars().filter(|c| !c.is_control() || *c == '\t').collect();
        let line = line.trim();
        if line.is_empty() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        let broken_word = current
            .strip_suffix('-')
            .and_then(|before| before.chars().last())
            .is_some_and(char::is_alphabetic);
        if !current.is_empty() && !broken_word {
            current.push(' ');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// A plain web page of the paragraphs, for pandoc to turn into a Word file.
fn paragraphs_html(paragraphs: &[String]) -> String {
    let mut html = String::from("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title></title></head><body>\n");
    for p in paragraphs {
        html.push_str(&format!("<p>{}</p>\n", xml_text(p)));
    }
    html.push_str("</body></html>\n");
    html
}

fn read_head(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut head = Vec::with_capacity(16);
    fs::File::open(path)
        .and_then(|f| f.take(16).read_to_end(&mut head))
        .map_err(|e| format!("can't read the file: {e}"))?;
    Ok(head)
}

/// What the app sees of a drop being converted, and can do to it: how far its tool
/// has got, when the tool says, and Stop. Conversions run under one with `under`.
pub struct Control {
    stop: AtomicBool,
    /// Percent done of the tool running now, or NO_PERCENT.
    percent: AtomicU8,
    /// The quality levels chosen in the menu when the drop started.
    quality: Quality,
    /// The box chosen in the editor, for Crop.
    crop: Option<Crop>,
}

/// A crop chosen in the editor: the picture, upright, is turned `turns` quarter turns
/// clockwise, then cut to the box, given in pixels of the turned picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub turns: u8,
}

impl Crop {
    /// The box lies inside a picture of `width` x `height` (upright) once it is turned.
    pub fn check(&self, width: u32, height: u32) -> Result<(), String> {
        let (w, h) = if self.turns % 2 == 1 { (height, width) } else { (width, height) };
        let inside = self.turns < 4
            && self.width > 0
            && self.height > 0
            && self.x.checked_add(self.width).is_some_and(|right| right <= w)
            && self.y.checked_add(self.height).is_some_and(|bottom| bottom <= h);
        if inside {
            Ok(())
        } else {
            Err("the crop box is outside the picture".into())
        }
    }
}

const NO_PERCENT: u8 = u8::MAX;

impl Control {
    #[cfg(test)]
    pub fn new() -> Arc<Self> {
        Self::with_edit(Quality::default(), None)
    }

    /// A drop's control: the menu's quality, and for Crop the box from the editor.
    pub fn with_edit(quality: Quality, crop: Option<Crop>) -> Arc<Self> {
        Arc::new(Control { stop: AtomicBool::new(false), percent: AtomicU8::new(NO_PERCENT), quality, crop })
    }

    /// Ends the tool running now, with everything it started, and keeps the drop from
    /// starting another. What it wrote so far stays in the work folder, which goes.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// How far the running tool is, in percent, for tools that report it (ffmpeg).
    pub fn percent(&self) -> Option<u8> {
        Some(self.percent.load(Ordering::SeqCst)).filter(|&p| p != NO_PERCENT)
    }

    fn set_percent(&self, percent: Option<u8>) {
        self.percent.store(percent.map_or(NO_PERCENT, |p| p.min(100)), Ordering::SeqCst);
    }
}

thread_local! {
    /// The Control of the drop this thread is converting. One per thread, so tests that
    /// run side by side can't stop each other's tools.
    static CONTROL: RefCell<Option<Arc<Control>>> = const { RefCell::new(None) };
}

/// Runs `convert` calls made inside `f` under `control`.
pub fn under<T>(control: &Arc<Control>, f: impl FnOnce() -> T) -> T {
    /// Clears the thread's Control however `f` ends, a panic included.
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CONTROL.with(|c| c.borrow_mut().take());
        }
    }
    CONTROL.with(|c| *c.borrow_mut() = Some(Arc::clone(control)));
    let _reset = Reset;
    f()
}

fn control() -> Option<Arc<Control>> {
    CONTROL.with(|c| c.borrow().clone())
}

/// The quality levels for this drop: Balanced everywhere outside one (the command line,
/// tests).
fn quality() -> Quality {
    control().map(|c| c.quality).unwrap_or_default()
}

pub(crate) fn stop_asked() -> bool {
    control().is_some_and(|c| c.stopped())
}

/// A tool that didn't finish well: the message for the notification, and the end of
/// its error output, for callers that look for a known cause in it.
struct Failure {
    message: String,
    output: String,
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure { message, output: String::new() }
    }
}

/// Runs a tool to the end or until `limit`, whichever comes first. On a timeout the
/// tool and everything it started are stopped. Only the last line of its error
/// output is kept, for the notification.
fn run(cmd: Command, limit: Duration) -> Result<(), String> {
    run_tool(cmd, limit).map_err(|f| f.message)
}

/// `run`, keeping the end of the tool's error output on a failure.
fn run_tool(mut cmd: Command, limit: Duration) -> Result<(), Failure> {
    let name = Path::new(cmd.get_program())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if stop_asked() {
        return Err(STOPPED.to_string().into());
    }
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

fn wait(child: &mut Child, name: &str, limit: Duration, errors: &mut fs::File) -> Result<(), Failure> {
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if stop_asked() => {
                stop(child);
                return Err(STOPPED.to_string().into());
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            Ok(None) => {
                stop(child);
                return Err(format!("{name} took longer than {} and was stopped", span(limit)).into());
            }
            Err(e) => {
                stop(child);
                return Err(format!("{name}: {e}").into());
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
    let message = match last {
        Some(line) => format!("{name} failed: {line}"),
        None => format!("{name} failed ({status})"),
    };
    Err(Failure { message, output: text })
}

/// "2 min", or "45 s" for less than a minute.
fn span(limit: Duration) -> String {
    match limit.as_secs() {
        s if s < 60 => format!("{s} s"),
        s => format!("{} min", s / 60),
    }
}

/// Without this, Windows flashes a console window for every tool that runs.
pub(crate) fn quiet(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd;
}

/// Stops a tool and the processes it started.
pub(crate) fn stop(child: &mut Child) {
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
pub(crate) fn place(tmp: &Path, dir: &Path, stem: &str, target: &str) -> io::Result<PathBuf> {
    for n in 1..1000 {
        let name = if n == 1 {
            format!("{stem}.{target}")
        } else {
            format!("{stem} {n}.{target}")
        };
        let dest = dir.join(name);
        // create_new claims the name atomically, so a file that appears meanwhile is safe.
        let file = match OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        move_into(tmp, file, &dest)?;
        return Ok(dest);
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many files with that name already"))
}

/// Puts the pictures of a PDF's pages in a new folder in `dir`, named after the PDF and
/// the format ("report (PNG)", then "report (PNG) 2", ...; "report (PNG, pages 1-100)"
/// when the PDF had more), as "report 1.png", "report 2.png", ... The numbers are
/// padded ("report 01.png") so the pages sort in order everywhere.
fn place_pages(files: &[PathBuf], cut: bool, dir: &Path, stem: &str, label: &str, target: &str) -> io::Result<PathBuf> {
    let base = if cut {
        format!("{stem} ({label}, pages 1-{})", files.len())
    } else {
        format!("{stem} ({label})")
    };
    let width = files.len().to_string().len();
    for n in 1..1000 {
        let folder = dir.join(if n == 1 { base.clone() } else { format!("{base} {n}") });
        // create_dir fails on an existing folder, so it claims the name like create_new.
        match fs::create_dir(&folder) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
        let mut placed = Vec::new();
        let filled = files.iter().enumerate().try_for_each(|(i, page)| {
            let dest = folder.join(format!("{stem} {:0width$}.{target}", i + 1));
            let file = OpenOptions::new().write(true).create_new(true).open(&dest)?;
            move_into(page, file, &dest)?;
            placed.push((page, dest));
            Ok(())
        });
        if let Err(e) = filled {
            // Pages that were moved go back, so saving them elsewhere (Downloads) still
            // has them. Then the folder goes: it is our own new one, nothing else is in it.
            for (page, dest) in placed {
                if !page.exists() {
                    let _ = fs::rename(dest, page);
                }
            }
            let _ = fs::remove_dir_all(&folder);
            return Err(e);
        }
        return Ok(folder);
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many folders with that name already"))
}

/// Swaps the empty placeholder `file` at `dest` for `tmp`: a rename on the same disk,
/// a copy of the bytes from another one.
fn move_into(tmp: &Path, mut file: fs::File, dest: &Path) -> io::Result<()> {
    if fs::rename(tmp, dest).is_ok() {
        return Ok(());
    }
    if let Err(e) = fs::File::open(tmp).and_then(|mut f| io::copy(&mut f, &mut file)) {
        let _ = fs::remove_file(dest);
        return Err(e);
    }
    Ok(())
}

/// Finds a command-line tool. Apps opened from Finder or the Start menu don't always
/// get the shell's PATH, so the places the usual installers use are checked as well.
pub(crate) fn tool(name: &str) -> Result<PathBuf, String> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let path = std::env::var_os("PATH").unwrap_or_default();
    // Only absolute folders: an empty or relative PATH entry would mean "wherever the
    // app happens to be started from".
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .chain(extra_tool_dirs())
        .map(|dir| dir.join(&file))
        .find(|p| p.is_file())
        .ok_or_else(|| not_installed(name))
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
/// For the editor: the picture's size upright, in pixels, and a JPEG of it, upright
/// too, at most 1400 pixels across and on white where it is see-through.
pub fn preview(src: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let coder = sniff_image(&read_head(src)?).ok_or("this is not an image format the wheel can read")?;
    let work = tempfile::tempdir().map_err(|e| format!("work folder: {e}"))?;
    let (jpeg, size) = (work.path().join("preview.jpg"), work.path().join("size.txt"));
    let mut cmd = Command::new(magick()?);
    cmd.args(IMAGE_LIMITS)
        .arg(format!("{coder}:{}[0]", src.display()))
        .args(["-auto-orient", "-format", "%w %h", "-write"])
        .arg(format!("info:{}", size.display()))
        .args(["-background", "white", "-flatten", "-resize", "1400x1400>", "-quality", "85"])
        .arg(format!("jpeg:{}", jpeg.display()));
    run(cmd, QUICK)?;
    let size = fs::read_to_string(&size).unwrap_or_default();
    let mut numbers = size.split_whitespace().map(|n| n.parse::<u32>().ok());
    let (Some(Some(width)), Some(Some(height))) = (numbers.next(), numbers.next()) else {
        return Err("ImageMagick didn't say how big the picture is".into());
    };
    let bytes = fs::read(&jpeg).map_err(|e| format!("work file: {e}"))?;
    Ok((width, height, bytes))
}

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

/// A converter the app runs: the name `tool` looks for, the name people know it by, and
/// the packages that install it.
pub struct Converter {
    pub tool: &'static str,
    pub name: &'static str,
    /// Homebrew's arguments after `brew install`.
    pub brew: &'static str,
    pub winget: &'static str,
    pub apt: &'static str,
}

pub const CONVERTERS: &[Converter] = &[
    Converter { tool: "ffmpeg", name: "ffmpeg", brew: "ffmpeg", winget: "Gyan.FFmpeg", apt: "ffmpeg" },
    Converter { tool: "magick", name: "ImageMagick", brew: "imagemagick", winget: "ImageMagick.ImageMagick", apt: "imagemagick" },
    Converter { tool: "pandoc", name: "pandoc", brew: "pandoc", winget: "JohnMacFarlane.Pandoc", apt: "pandoc" },
    Converter {
        tool: "soffice",
        name: "LibreOffice",
        brew: "--cask libreoffice",
        winget: "TheDocumentFoundation.LibreOffice",
        apt: "libreoffice",
    },
    Converter { tool: "mutool", name: "MuPDF", brew: "mupdf-tools", winget: "ArtifexSoftware.mutool", apt: "mupdf-tools" },
    // Archives. macOS and Windows have it built in; only Linux ever misses it.
    Converter { tool: "bsdtar", name: "bsdtar", brew: "libarchive", winget: "", apt: "libarchive-tools" },
];

fn converter(tool: &str) -> Option<&'static Converter> {
    CONVERTERS.iter().find(|c| c.tool == tool)
}

/// "ffmpeg is not installed (brew install ffmpeg)". `missing` reads it back.
fn not_installed(tool: &str) -> String {
    let name = converter(tool).map_or(tool, |c| c.name);
    format!("{name} is not installed ({})", install_hint(tool))
}

/// The converter a conversion's error says is not installed, if it says so.
pub fn missing(error: &str) -> Option<&'static Converter> {
    CONVERTERS.iter().find(|c| error.starts_with(&format!("{} is not installed (", c.name)))
}

/// Whether the app can find `c` now, where a conversion would look for it.
pub fn installed(c: &Converter) -> bool {
    match c.tool {
        "magick" => magick().is_ok(),
        "soffice" => soffice().is_ok(),
        tool_name => tool(tool_name).is_ok(),
    }
}

fn install_hint(name: &str) -> String {
    let c = converter(name);
    if cfg!(target_os = "macos") {
        format!("brew install {}", c.map_or(name, |c| c.brew))
    } else if cfg!(windows) {
        format!("winget install {}", c.map_or(name, |c| c.winget))
    } else {
        let package = c.map_or(name, |c| c.apt);
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
                .map_err(|_| not_installed("soffice"))
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

/// Makes text safe inside an XML or HTML element.
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
    #[cfg(unix)]
    fn stop_ends_the_running_tool_and_starts_no_other() {
        let control = Control::new();
        let stopper = Arc::clone(&control);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            stopper.stop();
        });
        let started = Instant::now();
        under(&control, || {
            let mut cmd = Command::new("sleep");
            cmd.arg("30");
            assert_eq!(run(cmd, Duration::from_secs(60)).unwrap_err(), STOPPED);
            assert!(started.elapsed() < Duration::from_secs(5));
            // The next tool of the same drop doesn't start at all.
            let mut next = Command::new("sleep");
            next.arg("30");
            assert_eq!(run(next, Duration::from_secs(60)).unwrap_err(), STOPPED);
        });
        assert!(started.elapsed() < Duration::from_secs(5));
        // Outside the drop, tools run again.
        assert!(super::control().is_none());
    }

    /// A real video conversion reports how far it is, and Stop ends it without leaving
    /// anything next to the original. Skipped where ffmpeg or ffprobe is missing.
    #[test]
    fn a_video_reports_its_percent_and_can_be_stopped() {
        let (Ok(ffmpeg), Ok(_)) = (tool("ffmpeg"), tool("ffprobe")) else {
            eprintln!("ffmpeg or ffprobe isn't installed: video progress not tested");
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let src = work.path().join("clip.mov");
        let made = Command::new(ffmpeg)
            .args(["-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=120:size=640x360:rate=30"])
            .args(["-c:v", "mpeg4", "-q:v", "5"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(made.success());

        let control = Control::new();
        let watcher = Arc::clone(&control);
        let seen = thread::spawn(move || {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(60) {
                if let Some(p) = watcher.percent() {
                    watcher.stop();
                    return Some(p);
                }
                thread::sleep(Duration::from_millis(100));
            }
            watcher.stop();
            None
        });
        let result = under(&control, || convert(&src, "webm", None, work.path()));
        let percent = seen.join().unwrap();
        assert!(percent.is_some_and(|p| p < 100), "no percent came: {percent:?}");
        assert_eq!(result.unwrap_err(), STOPPED);
        assert_eq!(control.percent(), None);
        let left: Vec<_> = fs::read_dir(work.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("clip.mov")]);
    }

    #[test]
    fn ffmpeg_progress_is_read_from_its_last_complete_line() {
        assert_eq!(progress_at(""), None);
        assert_eq!(progress_at("out_time_us=N/A\nprogress=continue\n"), None);
        let text = "out_time_us=1500000\nprogress=continue\nout_time_us=3000000\nprogress=continue\nout_ti";
        assert_eq!(progress_at(text), Some(Duration::from_secs(3)));
        let length = Duration::from_secs(12);
        assert_eq!(percent_of(Duration::from_secs(3), length), 25);
        // Not 100 before the result is in place, and never more.
        assert_eq!(percent_of(Duration::from_secs(12), length), 99);
        assert_eq!(percent_of(Duration::from_secs(30), length), 99);
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

        let fake = work.path().join("scan.pdf");
        fs::write(&fake, "<html></html>").unwrap();
        let err = convert(&fake, "png", None, work.path()).unwrap_err();
        assert!(err.contains("not a valid .pdf"), "{err}");
    }

    #[test]
    fn pdf_text_comes_back_as_paragraphs() {
        let text = "Quarterly\nreport\n\nThe first line\nand the sec-\nond line.\n\n\u{c}\nNext page \u{1}text\n";
        assert_eq!(
            paragraphs(text),
            ["Quarterly report", "The first line and the sec-ond line.", "Next page text"]
        );
        assert!(paragraphs("\n \n\u{c}\n").is_empty());
        // A dash after a space or a digit is not a broken word.
        assert_eq!(paragraphs("From 2020 -\n2021, and 3-\n4"), ["From 2020 - 2021, and 3- 4"]);
        let html = paragraphs_html(&["a < b & c".to_string()]);
        assert!(html.contains("<p>a &lt; b &amp; c</p>"), "{html}");
    }

    #[test]
    fn page_counts_are_read_direct_or_from_their_own_object() {
        assert_eq!(parse_count("4\n"), Some(4));
        assert_eq!(parse_count("6 0 obj\n12\nendobj\n"), Some(12));
        assert_eq!(parse_count("null\n"), None);
        assert_eq!(parse_count(""), None);
    }

    #[test]
    fn page_pictures_go_in_a_new_folder_in_page_order() {
        let work = tempfile::tempdir().unwrap();
        let pages: Vec<PathBuf> = (1..=12)
            .map(|n| {
                let p = work.path().join(format!("{n}.png"));
                fs::write(&p, n.to_string()).unwrap();
                p
            })
            .collect();
        let out = work.path().join("out");
        fs::create_dir_all(out.join("report (PNG)")).unwrap();
        let folder = place_pages(&pages, false, &out, "report", "PNG", "png").unwrap();
        assert_eq!(folder, out.join("report (PNG) 2"));
        assert_eq!(fs::read_to_string(folder.join("report 01.png")).unwrap(), "1");
        assert_eq!(fs::read_to_string(folder.join("report 12.png")).unwrap(), "12");
        assert!(fs::read_dir(out.join("report (PNG)")).unwrap().next().is_none(), "the old folder is untouched");

        let pages: Vec<PathBuf> = (1..=2)
            .map(|n| {
                let p = work.path().join(format!("{n}.jpg"));
                fs::write(&p, "x").unwrap();
                p
            })
            .collect();
        let folder = place_pages(&pages, true, &out, "report", "JPG", "jpg").unwrap();
        assert_eq!(folder, out.join("report (JPG, pages 1-2)"));
        assert!(folder.join("report 1.jpg").is_file());
    }

    /// A small PDF with one line of text on each page.
    fn tiny_pdf(pages: &[&str]) -> Vec<u8> {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {} >>",
                (0..pages.len()).map(|i| format!("{} 0 R", 4 + 2 * i)).collect::<Vec<_>>().join(" "),
                pages.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        for (i, text) in pages.iter().enumerate() {
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                5 + 2 * i
            ));
            let stream = format!("BT /F1 12 Tf 10 50 Td ({text}) Tj ET");
            objects.push(format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()));
        }
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{object}\nendobj\n", i + 1).bytes());
        }
        let xref = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            pdf.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        pdf.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
        pdf
    }

    /// Needs MuPDF. Where it isn't installed this passes without checking anything,
    /// unless REQUIRE_MUTOOL is set (CI sets it where it installs MuPDF).
    #[test]
    fn pdf_pages_become_pictures_and_text() {
        if tool("mutool").is_err() {
            let required = std::env::var_os("REQUIRE_MUTOOL").is_some_and(|v| !v.is_empty());
            assert!(!required, "mutool should be installed here");
            eprintln!("mutool isn't installed: PDF conversion not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let src = work.path().join("two pages.pdf");
        fs::write(&src, tiny_pdf(&["First page", "Second page"])).unwrap();

        let folder = convert(&src, "png", None, work.path()).unwrap();
        assert_eq!(folder, work.path().join("two pages (PNG)"));
        for n in 1..=2 {
            let page = fs::read(folder.join(format!("two pages {n}.png"))).unwrap();
            assert!(page.starts_with(b"\x89PNG"));
        }
        let text = convert(&src, "txt", None, work.path()).unwrap();
        assert_eq!(fs::read_to_string(text).unwrap(), "First page\n\nSecond page\n");

        let one = work.path().join("one.pdf");
        fs::write(&one, tiny_pdf(&["Only page"])).unwrap();
        assert_eq!(convert(&one, "png", None, work.path()).unwrap(), work.path().join("one.png"));

        let empty = work.path().join("empty.pdf");
        fs::write(&empty, tiny_pdf(&[])).unwrap();
        let err = convert(&empty, "png", None, work.path()).unwrap_err();
        assert!(err.contains("no pages"), "{err}");

        // More pages than are drawn: the first 100, and the folder says so.
        let many = work.path().join("long.pdf");
        let lines: Vec<String> = (1..=101).map(|n| format!("Page {n}")).collect();
        fs::write(&many, tiny_pdf(&lines.iter().map(String::as_str).collect::<Vec<_>>())).unwrap();
        let folder = convert(&many, "png", None, work.path()).unwrap();
        assert_eq!(folder, work.path().join("long (PNG, pages 1-100)"));
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 100);
        assert!(folder.join("long 001.png").is_file() && folder.join("long 100.png").is_file());
        let text = fs::read_to_string(convert(&many, "txt", None, work.path()).unwrap()).unwrap();
        assert!(text.starts_with("Page 1\n\nPage 2\n") && text.ends_with("Page 101\n"), "{text}");

        // A PDF that needs a password says so. With its objects in object streams (-Z,
        // where MuPDF has it) even the page count can't be read without the password.
        let locked = work.path().join("locked.pdf");
        let lock = |extra: &[&str]| {
            let mut clean = Command::new(tool("mutool").unwrap());
            clean.arg("clean").args(extra).args(["-E", "aes-256", "-U", "secret", "-O", "owner"]).arg(&src).arg(&locked);
            run(clean, QUICK)
        };
        lock(&["-Z"]).or_else(|_| lock(&[])).unwrap();
        let err = convert(&locked, "txt", None, work.path()).unwrap_err();
        assert!(err.contains("password-protected"), "{err}");
        let err = convert(&locked, "png", None, work.path()).unwrap_err();
        assert!(err.contains("password-protected"), "{err}");
    }

    /// Needs MuPDF and a pandoc that works in its sandbox. Where they aren't installed
    /// this passes without checking anything, unless REQUIRE_PANDOC is set.
    #[test]
    fn pdf_text_becomes_a_word_file() {
        let ready = tool("mutool").is_ok() && tool("pandoc").is_ok_and(|p| check_pandoc_version(&p).is_ok());
        if !ready {
            let required = std::env::var_os("REQUIRE_PANDOC").is_some_and(|v| !v.is_empty());
            assert!(!required, "mutool and pandoc should be installed here");
            eprintln!("mutool or pandoc isn't installed: PDF to Word not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let src = work.path().join("letter.pdf");
        fs::write(&src, tiny_pdf(&["Dear reader", "Kind regards"])).unwrap();
        let docx = convert(&src, "docx", None, work.path()).unwrap();
        assert_eq!(docx, work.path().join("letter.docx"));
        assert!(fs::read(&docx).unwrap().starts_with(b"PK"));
    }

    /// Width x height, as ImageMagick reads it.
    fn picture_size(path: &Path) -> String {
        let out = Command::new(magick().unwrap()).arg(path).args(["-format", "%wx%h", "info:"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn a_crop_box_must_lie_inside_the_turned_picture() {
        let crop = |x, y, width, height, turns| Crop { x, y, width, height, turns };
        assert!(crop(0, 0, 300, 200, 0).check(300, 200).is_ok());
        assert!(crop(1, 0, 300, 200, 0).check(300, 200).is_err(), "one pixel too far right");
        // Turned a quarter, a 300 x 200 picture is 200 x 300.
        assert!(crop(0, 0, 200, 300, 1).check(300, 200).is_ok());
        assert!(crop(0, 0, 300, 200, 1).check(300, 200).is_err());
        assert!(crop(0, 0, 0, 10, 0).check(300, 200).is_err(), "empty");
        assert!(crop(0, 0, 10, 10, 4).check(300, 200).is_err(), "turns are 0 to 3");
        assert!(crop(u32::MAX, 0, 10, 10, 0).check(300, 200).is_err(), "no overflow");
    }

    /// Needs ImageMagick. The editor's box, turned and cut, and its preview's size.
    #[test]
    fn crop_turns_then_cuts_the_box() {
        let Ok(magick) = magick() else {
            eprintln!("ImageMagick isn't installed: crop not tested");
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let wide = work.path().join("wide.png");
        let mut make = Command::new(&magick);
        make.args(["-size", "300x200", "gradient:red-blue"]).arg(&wide);
        run(make, QUICK).unwrap();
        assert_eq!(preview(&wide).map(|(w, h, jpeg)| (w, h, jpeg.starts_with(&[0xFF, 0xD8]))), Ok((300, 200, true)));

        let crop = Crop { x: 10, y: 20, width: 150, height: 120, turns: 1 };
        let control = Control::with_edit(Quality::default(), Some(crop));
        let out = under(&control, || convert(&wide, "tool:crop", None, work.path())).unwrap();
        assert_eq!(out, work.path().join("wide (cropped).png"));
        // ImageMagick 6 (Ubuntu) has no "magick identify": ask the way the other tests do.
        assert_eq!(picture_size(&out), "150x120");
        // Without the editor there is no box to cut.
        assert_eq!(convert(&wide, "tool:crop", None, work.path()).unwrap_err(), "Crop opens its window from the wheel");
    }

    #[test]
    fn image_tools_keep_the_format_and_name_the_result() {
        if magick().is_err() {
            eprintln!("ImageMagick isn't installed: image tools not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let plain = work.path().join("plain.jpg");
        let mut make = Command::new(magick().unwrap());
        make.args(["-size", "40x30", "gradient:red-blue"]).arg(&plain);
        assert!(make.status().unwrap().success());
        // A camera's EXIF block, with a marker standing in for the location.
        let jpeg = fs::read(&plain).unwrap();
        let payload = [&b"Exif\0\0II*\0\x08\0\0\0\0\0\0\0"[..], b"WHERE-I-WAS"].concat();
        let mut photo_bytes = jpeg[..2].to_vec();
        photo_bytes.extend([0xFF, 0xE1]);
        photo_bytes.extend(((payload.len() + 2) as u16).to_be_bytes());
        photo_bytes.extend(&payload);
        photo_bytes.extend(&jpeg[2..]);
        let photo = work.path().join("photo.jpeg");
        fs::write(&photo, &photo_bytes).unwrap();
        let has = |path: &Path, needle: &[u8]| fs::read(path).unwrap().windows(needle.len()).any(|w| w == needle);

        let half = convert(&photo, "tool:half", None, work.path()).unwrap();
        assert_eq!(half, work.path().join("photo (half size).jpg"));
        assert_eq!(picture_size(&half), "20x15");
        assert!(has(&half, b"WHERE-I-WAS"), "only Remove Metadata removes metadata");

        let rotated = convert(&photo, "tool:rotate", None, work.path()).unwrap();
        assert_eq!(rotated, work.path().join("photo (rotated).jpg"));
        assert_eq!(picture_size(&rotated), "30x40");

        let clean = convert(&photo, "tool:clean", None, work.path()).unwrap();
        assert_eq!(clean, work.path().join("photo (no metadata).jpg"));
        assert!(!has(&clean, b"WHERE-I-WAS") && !has(&clean, b"Exif\0\0"));
        assert_eq!(picture_size(&clean), "40x30");

        let small = convert(&photo, "tool:compress", None, work.path()).unwrap();
        assert_eq!(small, work.path().join("photo (compressed).jpg"));
        assert!(fs::metadata(&small).unwrap().len() < photo_bytes.len() as u64);

        // Nothing replaces anything: the same tool again makes a second file.
        assert_eq!(convert(&photo, "tool:half", None, work.path()).unwrap(), work.path().join("photo (half size) 2.jpg"));

        let bmp = work.path().join("old.bmp");
        let mut make = Command::new(magick().unwrap());
        make.args(["-size", "8x8", "xc:white"]).arg(format!("bmp3:{}", bmp.display()));
        assert!(make.status().unwrap().success());
        assert_eq!(convert(&bmp, "tool:compress", None, work.path()).unwrap_err(), ".bmp files can't be compressed");
        let gif = work.path().join("anim.gif");
        fs::write(&gif, b"GIF89a").unwrap();
        assert_eq!(convert(&gif, "tool:rotate", None, work.path()).unwrap_err(), ".gif files can't be rotated");
    }

    #[test]
    fn pdf_tools_split_merge_and_shrink() {
        if tool("mutool").is_err() {
            let required = std::env::var_os("REQUIRE_MUTOOL").is_some_and(|v| !v.is_empty());
            assert!(!required, "mutool should be installed here");
            eprintln!("mutool isn't installed: PDF tools not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let pages = |path: &Path| {
            let m = Mutool { path: tool("mutool").unwrap(), src: path, deadline: Instant::now() + PDF_TOTAL };
            m.page_count(tempfile::tempdir().unwrap().path()).unwrap()
        };
        let three = work.path().join("report.pdf");
        fs::write(&three, tiny_pdf(&["One", "Two", "Three"])).unwrap();
        let two = work.path().join("annex.pdf");
        fs::write(&two, tiny_pdf(&["Four", "Five"])).unwrap();

        let folder = convert(&three, "tool:split", None, work.path()).unwrap();
        assert_eq!(folder, work.path().join("report (pages)"));
        for n in 1..=3 {
            assert_eq!(pages(&folder.join(format!("report {n}.pdf"))), 1);
        }
        let one = work.path().join("one.pdf");
        fs::write(&one, tiny_pdf(&["Only"])).unwrap();
        assert_eq!(convert(&one, "tool:split", None, work.path()).unwrap_err(), "this PDF has only one page");

        let merged = combine(&[three.clone(), two.clone()], "tool:merge", None, work.path()).unwrap();
        assert_eq!(merged, work.path().join("report (merged).pdf"));
        assert_eq!(pages(&merged), 5);
        // A PDF MuPDF can't read stops the merge instead of being left out.
        let broken = work.path().join("broken.pdf");
        fs::write(&broken, b"%PDF-1.4\nnot really a PDF\n").unwrap();
        let err = combine(&[three.clone(), broken], "tool:merge", None, work.path()).unwrap_err();
        assert!(err.starts_with("broken.pdf: "), "{err}");
        let err = combine(&[three.clone(), work.path().join("x.png")], "tool:merge", None, work.path()).unwrap_err();
        assert_eq!(err, "x.png: only PDFs can be merged");
        assert_eq!(convert(&three, "tool:merge", None, work.path()).unwrap_err(), "Merge needs two PDFs or more");

        // Pictures, one page each, in the order dropped.
        if magick().is_ok() {
            let mut pictures = Vec::new();
            for (name, spec) in [("wide.jpg", "60x40"), ("tall.png", "30x50")] {
                let path = work.path().join(name);
                let mut make = Command::new(magick().unwrap());
                make.args(["-size", spec, "xc:orange"]).arg(&path);
                assert!(make.status().unwrap().success());
                pictures.push(path);
            }
            let one = combine(&pictures, "tool:onepdf", None, work.path()).unwrap();
            assert_eq!(one, work.path().join("wide (one PDF).pdf"));
            assert_eq!(pages(&one), 2);
            let err = combine(&[pictures[0].clone(), three.clone()], "tool:onepdf", None, work.path()).unwrap_err();
            assert_eq!(err, "report.pdf: only pictures can go into one PDF");
        }

        // A tiny text-only PDF has nothing to shrink, or very little.
        match convert(&three, "tool:smaller", None, work.path()) {
            Ok(smaller) => assert_eq!(pages(&smaller), 3),
            Err(e) => assert_eq!(e, "it is already as small as Smaller makes it"),
        }
    }

    /// What ffprobe says about a file's first audio stream and its tags.
    fn probe(path: &Path) -> String {
        let out = Command::new(tool("ffprobe").unwrap())
            .args(["-v", "error", "-show_entries", "format_tags:stream=channels", "-of", "flat"])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Codec names of a file's streams, one per line, in order.
    fn streams(path: &Path) -> String {
        let out = Command::new(tool("ffprobe").unwrap())
            .args(["-v", "error", "-show_entries", "stream=codec_name", "-of", "csv=p=0"])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// A second sound track that MP4 and Matroska can't hold, like an iPhone's spatial
    /// audio, must not stop a copy or an encode: they keep the first track.
    #[test]
    fn extra_sound_tracks_dont_stop_a_video() {
        if tool("ffmpeg").is_err() || tool("ffprobe").is_err() {
            eprintln!("ffmpeg isn't installed: sound tracks not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let clip = work.path().join("phone.mov");
        let mut make = Command::new(tool("ffmpeg").unwrap());
        make.args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25:duration=2"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-f", "lavfi", "-i", "sine=frequency=660:duration=2"])
            .args(["-map", "0", "-map", "1", "-map", "2", "-shortest"])
            .args(["-c:v", "mpeg4", "-c:a:0", "aac", "-c:a:1", "adpcm_ima_qt"])
            .arg(&clip);
        assert!(make.status().unwrap().success());
        assert_eq!(streams(&clip), "mpeg4\naac\nadpcm_ima_qt\n");

        // Still a copy: the picture isn't encoded again.
        let mp4 = convert(&clip, "mp4", None, work.path()).unwrap();
        assert_eq!(streams(&mp4), "mpeg4\naac\n");
        let mkv = convert(&clip, "mkv", None, work.path()).unwrap();
        assert_eq!(streams(&mkv), "mpeg4\naac\n");
        let small = convert(&clip, "tool:compress", None, work.path()).unwrap();
        assert_eq!(streams(&small), "h264\naac\n");
    }

    /// AVI and WMV are for old players, which can't take 4K: the picture is made to fit
    /// 1920, keeping its shape.
    #[test]
    fn old_video_formats_fit_1920() {
        if tool("ffmpeg").is_err() || tool("ffprobe").is_err() {
            eprintln!("ffmpeg isn't installed: old video formats not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let clip = work.path().join("big.mp4");
        let mut make = Command::new(tool("ffmpeg").unwrap());
        make.args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=2560x1440:rate=10:duration=1"])
            .args(["-c:v", "libx264", "-preset", "ultrafast"])
            .arg(&clip);
        assert!(make.status().unwrap().success());
        for target in ["avi", "wmv"] {
            let out = convert(&clip, target, None, work.path()).unwrap();
            let size = Command::new(tool("ffprobe").unwrap())
                .args(["-v", "error", "-show_entries", "stream=width,height", "-of", "csv=p=0"])
                .arg(&out)
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&size.stdout).trim(), "1920,1080", "{target}");
        }
    }

    #[test]
    fn media_tools_on_a_short_clip() {
        if tool("ffmpeg").is_err() || tool("ffprobe").is_err() {
            eprintln!("ffmpeg isn't installed: media tools not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let clip = work.path().join("clip.mp4");
        let mut make = Command::new(tool("ffmpeg").unwrap());
        make.args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25:duration=2"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-shortest"])
            .args(["-c:v", "libx264", "-crf", "18", "-c:a", "aac", "-metadata", "location=+48.85+002.29/"])
            .args(["-metadata", "title=Holiday"])
            .arg(&clip);
        assert!(make.status().unwrap().success());
        assert!(probe(&clip).contains("Holiday"));

        let clean = convert(&clip, "tool:clean", None, work.path()).unwrap();
        assert_eq!(clean, work.path().join("clip (no metadata).mp4"));
        let tags = probe(&clean);
        assert!(!tags.contains("Holiday") && !tags.contains("48.85"), "{tags}");

        let still = convert(&clip, "tool:still", None, work.path()).unwrap();
        assert_eq!(still, work.path().join("clip (first frame).jpg"));
        assert!(fs::read(&still).unwrap().starts_with(&[0xFF, 0xD8]));

        let small = convert(&clip, "tool:compress", None, work.path()).unwrap();
        assert_eq!(small, work.path().join("clip (compressed).mp4"));

        let song = work.path().join("song.mp3");
        let mut make = Command::new(tool("ffmpeg").unwrap());
        make.args(["-v", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-ac", "2"])
            .args(["-metadata", "title=Tune"])
            .arg(&song);
        assert!(make.status().unwrap().success());
        let even = convert(&song, "tool:loudness", None, work.path()).unwrap();
        assert_eq!(even, work.path().join("song (even loudness).mp3"));
        assert!(probe(&even).contains("Tune"), "loudness keeps the tags");
        let mono = convert(&song, "tool:mono", None, work.path()).unwrap();
        assert!(probe(&mono).contains("channels=1"), "{}", probe(&mono));
        assert_eq!(convert(&song, "tool:still", None, work.path()).unwrap_err(), ".mp3 files can't be turned into a picture");
    }

    #[test]
    fn subtitles_go_both_ways() {
        if tool("ffmpeg").is_err() {
            eprintln!("ffmpeg isn't installed: subtitles not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let srt = work.path().join("film.srt");
        fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,500\nHello there\n").unwrap();
        let vtt = convert(&srt, "vtt", None, work.path()).unwrap();
        let text = fs::read_to_string(&vtt).unwrap();
        assert!(text.starts_with("WEBVTT") && text.contains("00:01.000 --> 00:02.500") && text.contains("Hello there"), "{text}");
        fs::remove_file(&srt).unwrap();
        let back = fs::read_to_string(convert(&vtt, "srt", None, work.path()).unwrap()).unwrap();
        assert!(back.contains("00:00:01,000 --> 00:00:02,500") && back.contains("Hello there"), "{back}");
    }

    #[test]
    fn icons_are_square_and_hold_every_size() {
        if magick().is_err() {
            eprintln!("ImageMagick isn't installed: icons not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let wide = work.path().join("logo.png");
        let mut make = Command::new(magick().unwrap());
        make.args(["-size", "300x200", "xc:orange"]).arg(&wide);
        assert!(make.status().unwrap().success());
        let ico = convert(&wide, "ico", None, work.path()).unwrap();
        let out = Command::new(magick().unwrap()).arg(&ico).args(["-format", "%wx%h,", "info:"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "256x256,128x128,64x64,48x48,32x32,16x16,");
    }

    #[test]
    fn the_quality_level_changes_the_result() {
        use crate::settings::Level;
        if magick().is_err() {
            eprintln!("ImageMagick isn't installed: quality not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let src = work.path().join("noise.png");
        let mut make = Command::new(magick().unwrap());
        make.args(["-size", "200x200", "-seed", "3", "plasma:fractal"]).arg(&src);
        assert!(make.status().unwrap().success());
        let size_at = |pictures: Level| {
            let control = Control::with_edit(Quality { pictures, ..Quality::default() }, None);
            let out = under(&control, || convert(&src, "jpg", None, work.path())).unwrap();
            fs::metadata(out).unwrap().len()
        };
        let (smaller, balanced, best) = (size_at(Level::Smaller), size_at(Level::Balanced), size_at(Level::Best));
        assert!(smaller < balanced && balanced < best, "{smaller} {balanced} {best}");
    }

    #[test]
    fn a_missing_converter_is_read_back_from_its_message() {
        for c in CONVERTERS {
            assert_eq!(missing(&not_installed(c.tool)).map(|m| m.tool), Some(c.tool));
        }
        assert!(missing(&not_installed("ffprobe")).is_none());
        assert!(missing("pandoc 2.9 is too old: 2.15 or newer is needed").is_none());
        assert!(missing("ffmpeg failed: Invalid data found when processing input").is_none());
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
