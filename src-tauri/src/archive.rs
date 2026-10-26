//! Archives, from the Actions wheel: ZIP, TAR and TAR.GZ made of whatever is dragged,
//! and ZIP, TAR, TAR.GZ and RAR unpacked into a new folder next to them. Both go
//! through bsdtar (libarchive): built into macOS, and into Windows 10 and later as
//! tar.exe; the libarchive-tools package on Linux. bsdtar refuses names with ".." and
//! strips a leading "/", so nothing lands outside the new folder, and a size budget
//! stops an archive built to fill the disk. A plain .gz file (one file, not a TAR) is
//! unpacked here, within the same budget.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::convert;

/// Packing or unpacking a big folder takes a while; a stuck one must still end.
const LIMIT: Duration = Duration::from_secs(30 * 60);

/// What unpacking may write: 1 GB plus twenty times the archive. Real archives of
/// text and documents stay well inside; one built to explode does not.
fn budget(archive_size: u64) -> u64 {
    (1 << 30) + archive_size.saturating_mul(20)
}

/// What the archive types are, by name: "report.zip", "site.tar.gz".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Archive {
    /// Anything bsdtar reads: ZIP, TAR, TAR.GZ, RAR.
    Bsdtar,
    /// One gzip-compressed file.
    Gzip,
}

/// The kind of archive and the name without its archive ending ("site" for
/// "site.tar.gz", "log.txt" for "log.txt.gz").
fn split_name(name: &str) -> Option<(Archive, &str)> {
    let lower = name.to_ascii_lowercase();
    for (end, kind) in [
        (".tar.gz", Archive::Bsdtar),
        (".tgz", Archive::Bsdtar),
        (".zip", Archive::Bsdtar),
        (".tar", Archive::Bsdtar),
        (".rar", Archive::Bsdtar),
        (".gz", Archive::Gzip),
    ] {
        if lower.ends_with(end) && lower.len() > end.len() {
            return Some((kind, &name[..name.len() - end.len()]));
        }
    }
    None
}

/// Whether a file is an archive Unpack takes, by its name.
pub fn is_archive(path: &str) -> bool {
    Path::new(path).file_name().and_then(|n| n.to_str()).and_then(split_name).is_some()
}

#[cfg(target_os = "macos")]
fn bsdtar() -> Result<PathBuf, String> {
    Ok("/usr/bin/bsdtar".into())
}

/// Windows' own tar.exe is bsdtar, from Windows 10 1803 on.
#[cfg(windows)]
fn bsdtar() -> Result<PathBuf, String> {
    let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    let tar = system.join("System32\\tar.exe");
    if tar.is_file() {
        Ok(tar)
    } else {
        Err("Windows' own tar.exe is missing (it comes with Windows 10 1803 and later)".into())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn bsdtar() -> Result<PathBuf, String> {
    convert::tool("bsdtar")
}

/// Packs `srcs` (files or folders) into one archive: `target` is "act:zip", "act:tar"
/// or "act:tgz". The archive goes next to the first, named after it ("Photos.zip",
/// "notes.txt.zip"), or "Archive.zip" for several.
pub fn make(srcs: &[PathBuf], target: &str, save_in: Option<&Path>, fallback: &Path) -> Result<PathBuf, String> {
    let (ext, format): (&str, &[&str]) = match target {
        "act:zip" => ("zip", &["--format", "zip"]),
        "act:tar" => ("tar", &["--format", "pax"]),
        "act:tgz" => ("tar.gz", &["-z"]),
        other => return Err(format!("{other} doesn't make an archive")),
    };
    let [first, ..] = srcs else {
        return Err("nothing to pack".into());
    };
    let work = tempfile::Builder::new()
        .prefix("convert-wheel-")
        .tempdir()
        .map_err(|e| format!("work folder: {e}"))?;
    let out = work.path().join(format!("result.{ext}"));
    let mut cmd = Command::new(bsdtar()?);
    cmd.arg("-c").arg("-f").arg(&out).args(format);
    for src in srcs {
        let what = || src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        fs::symlink_metadata(src).map_err(|e| format!("{}: can't read it: {e}", what()))?;
        let (Some(parent), Some(name)) = (src.parent(), src.file_name()) else {
            return Err(format!("{}: can't pack this", what()));
        };
        // From each item's own folder, so the archive holds "Photos/..." rather than
        // the whole path. A name starting with "-" or "@" would be read as an option
        // or as another archive to copy from, so it gets a "./" in front.
        let mut entry = std::ffi::OsString::new();
        if name.to_string_lossy().starts_with(['-', '@']) {
            entry.push("./");
        }
        entry.push(name);
        cmd.arg("-C").arg(parent).arg(entry);
    }
    run(cmd, None, &[work.path()])?;
    if !out.is_file() {
        return Err("bsdtar finished but wrote no archive".into());
    }
    let stem = match srcs {
        [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into()),
        _ => "Archive".to_string(),
    };
    let dir = match save_in {
        Some(dir) => dir.to_path_buf(),
        None => first.parent().ok_or("the file has no folder")?.to_path_buf(),
    };
    let placed = match convert::place(&out, &dir, &stem, ext) {
        Err(e) if save_in.is_none() && convert::is_no_write(&e) => {
            fs::create_dir_all(fallback).and_then(|_| convert::place(&out, fallback, &stem, ext))
        }
        other => other,
    };
    placed.map_err(|e| format!("can't save the archive: {e}"))
}

/// Unpacks an archive next to it: what it holds at the top, when that is one folder or
/// file, or else a new folder named after it ("site" for "site.tar.gz", then "site 2",
/// ...). A plain .gz file becomes the one file it holds. Anything that goes wrong
/// leaves nothing behind.
pub fn unpack(src: &Path, save_in: Option<&Path>, fallback: &Path) -> Result<PathBuf, String> {
    let name = src.file_name().and_then(|n| n.to_str()).ok_or("the file name is not valid text")?;
    let (kind, stem) = split_name(name).ok_or("this is not an archive Convert Wheel unpacks")?;
    let meta = fs::metadata(src).map_err(|e| format!("can't read the file: {e}"))?;
    if !meta.is_file() {
        return Err("this is not a regular file".into());
    }
    let dir = match save_in {
        Some(dir) => dir.to_path_buf(),
        None => src.parent().ok_or("the file has no folder")?.to_path_buf(),
    };
    let budget = budget(meta.len());
    let attempt = |dir: &Path| -> Result<PathBuf, Failure> {
        match kind {
            Archive::Gzip => gunzip(src, dir, stem, budget),
            Archive::Bsdtar => unpack_into_new_folder(src, dir, stem, budget),
        }
    };
    match attempt(&dir) {
        Err(Failure::Io(e)) if save_in.is_none() && convert::is_no_write(&e) => {
            fs::create_dir_all(fallback).map_err(|e| format!("can't save the result: {e}"))?;
            attempt(fallback).map_err(Failure::into_message)
        }
        other => other.map_err(Failure::into_message),
    }
}

/// Why unpacking failed: the folder can't be written to (then Downloads is tried), or
/// anything else.
enum Failure {
    Io(io::Error),
    Other(String),
}

impl Failure {
    fn into_message(self) -> String {
        match self {
            Failure::Io(e) => format!("can't save the result: {e}"),
            Failure::Other(message) => message,
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::Other(message)
    }
}

/// Unpacks into a hidden folder next to the archive first. When the archive holds one
/// folder or file at the top, as most do, that goes next to the archive by its own
/// name, the way Finder's Archive Utility does it; otherwise the hidden folder becomes
/// the new folder, named after the archive. A failure removes the hidden folder with
/// whatever got into it.
fn unpack_into_new_folder(src: &Path, dir: &Path, stem: &str, budget: u64) -> Result<PathBuf, Failure> {
    let tar = bsdtar()?;
    let work = tempfile::Builder::new()
        .prefix(".convert-wheel-")
        .tempdir_in(dir)
        .map_err(Failure::Io)?;
    let mut cmd = Command::new(tar);
    // -o: files belong to the user, whatever the archive says.
    cmd.args(["-x", "-o", "-f"]).arg(src).arg("-C").arg(work.path());
    let parent = src.parent().unwrap_or(dir);
    run(cmd, Some((work.path(), budget)), &[work.path(), parent])?;
    let entries: Vec<PathBuf> = fs::read_dir(work.path())
        .map_err(Failure::Io)?
        .flatten()
        .map(|e| e.path())
        .collect();
    match entries.as_slice() {
        [] => Err("the archive is empty".to_string().into()),
        [one] => {
            let name = one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| stem.into());
            move_to_free_name(one, dir, &name).map_err(Failure::Io)
        }
        _ => {
            let folder = work.keep();
            move_to_free_name(&folder, dir, stem).map_err(|e| {
                let _ = fs::remove_dir_all(&folder);
                Failure::Io(e)
            })
        }
    }
}

/// Moves `from` into `dir` as `name`, or "name 2", "name 3", ... ("report 2.pdf" for a
/// file), never over anything already there.
fn move_to_free_name(from: &Path, dir: &Path, name: &str) -> io::Result<PathBuf> {
    let file_parts = match name.rsplit_once('.') {
        Some((stem, ext)) if from.is_file() && !stem.is_empty() => Some((stem, ext)),
        _ => None,
    };
    for n in 1..1000 {
        let candidate = match (n, file_parts) {
            (1, _) => name.to_string(),
            (n, Some((stem, ext))) => format!("{stem} {n}.{ext}"),
            (n, None) => format!("{name} {n}"),
        };
        let dest = dir.join(candidate);
        // A rename can replace an empty folder, so a name that is taken is skipped
        // before trying, and one taken in the meantime after.
        if dest.symlink_metadata().is_ok() {
            continue;
        }
        match fs::rename(from, &dest) {
            Ok(()) => return Ok(dest),
            Err(_) if dest.symlink_metadata().is_ok() => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many files with that name already"))
}

/// A plain .gz: the one file it holds, next to it under its own name ("log.txt" from
/// "log.txt.gz"; "log 2.txt" when that is taken).
fn gunzip(src: &Path, dir: &Path, stem: &str, budget: u64) -> Result<PathBuf, Failure> {
    let input = fs::File::open(src).map_err(|e| format!("can't read the file: {e}"))?;
    let work = tempfile::Builder::new()
        .prefix("convert-wheel-")
        .tempdir_in(dir)
        .map_err(Failure::Io)?;
    let tmp = work.path().join("result");
    let mut out = fs::File::create(&tmp).map_err(Failure::Io)?;
    // One more byte than allowed tells a file that is too big from one that just fits.
    let mut reader = flate2::read::MultiGzDecoder::new(input).take(budget + 1);
    let mut buf = vec![0u8; 1 << 16];
    let mut written = 0u64;
    loop {
        if convert::stop_asked() {
            return Err(convert::STOPPED.to_string().into());
        }
        let n = reader.read(&mut buf).map_err(|e| format!("this is not a valid .gz file ({e})"))?;
        if n == 0 {
            break;
        }
        written += n as u64;
        if written > budget {
            return Err(too_big(budget).into());
        }
        out.write_all(&buf[..n]).map_err(Failure::Io)?;
    }
    drop(out);
    let (name, ext) = match stem.rsplit_once('.') {
        Some((name, ext)) if !name.is_empty() => (name, Some(ext)),
        _ => (stem, None),
    };
    let placed = match ext {
        Some(ext) => convert::place(&tmp, dir, name, ext),
        None => place_bare(&tmp, dir, name),
    };
    placed.map_err(Failure::Io)
}

/// `convert::place` for a name with no extension: "data", "data 2", ...
fn place_bare(tmp: &Path, dir: &Path, name: &str) -> io::Result<PathBuf> {
    for n in 1..1000 {
        let dest = dir.join(if n == 1 { name.to_string() } else { format!("{name} {n}") });
        match fs::OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
        return fs::rename(tmp, &dest).map(|()| dest);
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many files with that name already"))
}

fn too_big(budget: u64) -> String {
    format!(
        "this archive unpacks to more than {} GB, far more than its own size: stopped, in case it was built to fill the disk",
        budget >> 30
    )
}

/// Runs bsdtar to the end, LIMIT or a Stop. With `budget`, a folder that may grow to so
/// many bytes at most. Paths in `hide` are left out of the message.
fn run(mut cmd: Command, budget: Option<(&Path, u64)>, hide: &[&Path]) -> Result<(), String> {
    let mut errors = tempfile::tempfile().map_err(|e| format!("work file: {e}"))?;
    let for_tool = errors.try_clone().map_err(|e| format!("work file: {e}"))?;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(for_tool);
    convert::quiet(&mut cmd);
    // A group of its own, so a stop ends whatever it started too.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not start bsdtar: {e}"))?;
    let deadline = Instant::now() + LIMIT;
    let mut measured = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                let over = budget.filter(|_| measured.elapsed() >= Duration::from_millis(500)).map(|(dir, max)| {
                    measured = Instant::now();
                    size_of(dir) > max
                });
                let failure = if convert::stop_asked() {
                    Some(convert::STOPPED.to_string())
                } else if Instant::now() > deadline {
                    Some(format!("bsdtar took longer than {} min and was stopped", LIMIT.as_secs() / 60))
                } else if let (Some(true), Some((_, max))) = (over, budget) {
                    Some(too_big(max))
                } else {
                    None
                };
                if let Some(failure) = failure {
                    convert::stop(&mut child);
                    return Err(failure);
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                convert::stop(&mut child);
                return Err(format!("bsdtar: {e}"));
            }
        }
    };
    if status.success() {
        return Ok(());
    }
    // The first complaint says what went wrong; bsdtar's last line only says that it
    // went wrong.
    let len = errors.seek(SeekFrom::End(0)).unwrap_or(0);
    let _ = errors.seek(SeekFrom::Start(len.saturating_sub(16 * 1024)));
    let mut text = String::new();
    let _ = errors.read_to_string(&mut text);
    let first = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.contains("Error exit delayed"));
    let mut message = match first {
        Some(line) => line.trim_start_matches("bsdtar: ").trim_start_matches("tar.exe: ").to_string(),
        None => format!("bsdtar failed ({status})"),
    };
    for path in hide {
        for sep in ['/', '\\'] {
            message = message.replace(&format!("{}{sep}", path.display()), "");
        }
    }
    Err(message)
}

/// Bytes in the files under `dir`, links not followed. What can't be read counts as 0.
fn size_of(dir: &Path) -> u64 {
    let mut total = 0;
    let mut todo = vec![dir.to_path_buf()];
    while let Some(dir) = todo.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.path().symlink_metadata() {
                Ok(meta) if meta.is_dir() => todo.push(entry.path()),
                Ok(meta) => total += meta.len(),
                Err(_) => {}
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_names_are_read() {
        assert_eq!(split_name("site.tar.gz"), Some((Archive::Bsdtar, "site")));
        assert_eq!(split_name("Site.TGZ"), Some((Archive::Bsdtar, "Site")));
        assert_eq!(split_name("log.txt.gz"), Some((Archive::Gzip, "log.txt")));
        assert_eq!(split_name("photos.rar"), Some((Archive::Bsdtar, "photos")));
        assert_eq!(split_name(".zip"), None);
        assert_eq!(split_name("notes.txt"), None);
        assert!(is_archive("/tmp/a.ZIP") && !is_archive("/tmp/a.pdf"));
    }

    #[test]
    fn the_budget_grows_with_the_archive() {
        assert_eq!(budget(0), 1 << 30);
        assert_eq!(budget(1 << 30), 21 << 30);
    }

    #[test]
    fn pack_and_unpack() {
        if bsdtar().is_err() {
            eprintln!("bsdtar isn't installed: archives not tested");
            return;
        }
        let work = tempfile::tempdir().unwrap();
        let photos = work.path().join("Photos");
        fs::create_dir_all(photos.join("2026")).unwrap();
        fs::write(photos.join("2026").join("a.txt"), b"one").unwrap();
        let dash = work.path().join("-notes.txt");
        fs::write(&dash, b"two").unwrap();

        let zip = make(std::slice::from_ref(&photos), "act:zip", None, work.path()).unwrap();
        assert_eq!(zip, work.path().join("Photos.zip"));
        let both = make(&[photos.clone(), dash.clone()], "act:tgz", None, work.path()).unwrap();
        assert_eq!(both, work.path().join("Archive.tar.gz"));
        let tar = make(std::slice::from_ref(&dash), "act:tar", None, work.path()).unwrap();
        assert_eq!(tar, work.path().join("-notes.txt.tar"));

        // One folder at the top comes out as itself, never over what is there:
        // "Photos" exists, so "Photos 2", without a second "Photos" inside.
        let out = unpack(&zip, None, work.path()).unwrap();
        assert_eq!(out, work.path().join("Photos 2"));
        assert_eq!(fs::read(out.join("2026/a.txt")).unwrap(), b"one");
        // Several things at the top go in a folder named after the archive.
        let out = unpack(&both, None, work.path()).unwrap();
        assert_eq!(out, work.path().join("Archive"));
        assert_eq!(fs::read(out.join("-notes.txt")).unwrap(), b"two");
        // One file at the top: "-notes 2.txt" next to the original.
        assert_eq!(unpack(&tar, None, work.path()).unwrap(), work.path().join("-notes 2.txt"));

        // Something that only says it is an archive leaves nothing behind.
        let fake = work.path().join("fake.zip");
        fs::write(&fake, b"not an archive").unwrap();
        assert!(unpack(&fake, None, work.path()).is_err());
        let left: Vec<_> = fs::read_dir(work.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(left.iter().all(|n| !n.to_string_lossy().starts_with(".convert-wheel-") && n != "fake"), "{left:?}");
    }

    #[test]
    fn a_plain_gz_becomes_its_file_within_the_budget() {
        let work = tempfile::tempdir().unwrap();
        let gz = work.path().join("log.txt.gz");
        let mut enc = flate2::write::GzEncoder::new(fs::File::create(&gz).unwrap(), flate2::Compression::default());
        enc.write_all(b"line one\nline two\n").unwrap();
        enc.finish().unwrap();
        let out = unpack(&gz, None, work.path()).unwrap();
        assert_eq!(out, work.path().join("log.txt"));
        assert_eq!(fs::read(&out).unwrap(), b"line one\nline two\n");
        assert_eq!(unpack(&gz, None, work.path()).unwrap(), work.path().join("log 2.txt"));

        // Zeros pack small: over a tiny budget it stops, and leaves nothing.
        let bomb = work.path().join("zeros.gz");
        let mut enc = flate2::write::GzEncoder::new(fs::File::create(&bomb).unwrap(), flate2::Compression::best());
        enc.write_all(&vec![0u8; 4 << 20]).unwrap();
        enc.finish().unwrap();
        match gunzip(&bomb, work.path(), "zeros", 1 << 20) {
            Err(Failure::Other(e)) => assert!(e.contains("fill the disk"), "{e}"),
            _ => panic!("the budget should stop it"),
        }
        assert!(!work.path().join("zeros").exists());
    }
}
