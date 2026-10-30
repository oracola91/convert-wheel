//! Copy To and Move To on the Actions wheel: the dropped files and folders go into a
//! folder chosen in the system's folder picker. Nothing in that folder is replaced: a
//! name that is taken gets a number ("photo 2.jpg", "Holiday 2"). Move renames when it
//! can; onto another disk it copies, and removes the original only once the copy is
//! complete. Links are copied as links, never followed.

use std::ffi::OsStr;
use std::fs::{self, Metadata, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::log;

/// The folder chosen last, where the picker opens next time.
static LAST: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Asks for a folder, then copies or moves `files` into it on a thread of its own.
pub fn start(app: &AppHandle, moving: bool, files: Vec<PathBuf>) {
    let Some(first) = files.first() else {
        return;
    };
    let start = LAST
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .filter(|dir| dir.is_dir())
        .or_else(|| first.parent().map(Path::to_path_buf));
    let title = if moving { "Move to" } else { "Copy to" };
    let mut picker = app.dialog().file().set_title(title).set_can_create_directories(true);
    if let Some(dir) = start {
        picker = picker.set_directory(dir);
    }
    bring_forward(app);
    let app = app.clone();
    picker.pick_folder(move |folder| {
        let Some(dir) = folder.and_then(|f| f.into_path().ok()) else {
            return;
        };
        *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.clone());
        thread::spawn(move || {
            let (done, failed) = transfer(&files, &dir, moving);
            report(&app, moving, &dir, done, failed);
        });
    });
}

/// The app has no window of its own in front: without this, macOS opens the picker
/// behind the file manager the files came from.
fn bring_forward(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.run_on_main_thread(|| {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            #[allow(deprecated)]
            objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        }
    });
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// Copies or moves each file into `dir`: what it became, and what failed with why.
fn transfer(files: &[PathBuf], dir: &Path, moving: bool) -> (Vec<PathBuf>, Vec<(String, String)>) {
    let mut done = Vec::new();
    let mut failed = Vec::new();
    for file in files {
        let name = file_name(file);
        let result = if moving { move_into(file, dir) } else { copy_into(file, dir) };
        match result {
            Ok(dest) => {
                // File names only: the log never holds folders.
                log::line(&format!("{} {name}", if moving { "moved" } else { "copied" }));
                done.push(dest);
            }
            Err(e) => {
                log::line(&format!("couldn't {} {name}: {e}", if moving { "move" } else { "copy" }));
                failed.push((name, e));
            }
        }
    }
    (done, failed)
}

/// What `file` is, without following a link, and its name.
fn look(file: &Path) -> Result<(Metadata, &OsStr), String> {
    let meta = fs::symlink_metadata(file).map_err(|e| format!("can't read it: {e}"))?;
    let name = file.file_name().ok_or("it has no name")?;
    Ok((meta, name))
}

/// Whether `dir` is folder `file` or lies inside it, comparing real paths: on a Mac
/// /tmp is /private/tmp.
fn inside(dir: &Path, file: &Path) -> bool {
    let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    real(dir).starts_with(real(file))
}

/// A copy of `file` (a file, a link or a whole folder) in `dir`.
fn copy_into(file: &Path, dir: &Path) -> Result<PathBuf, String> {
    let (meta, name) = look(file)?;
    let cant = |e: io::Error| format!("can't make the copy: {e}");
    if meta.is_dir() {
        if inside(dir, file) {
            return Err("a folder can't be copied into itself".into());
        }
        let dest = claim_folder(dir, name).map_err(cant)?;
        copy_tree(file, &dest).map_err(|e| format!("couldn't copy all of it: {e}"))?;
        Ok(dest)
    } else if meta.is_symlink() {
        // Making a link fails if the name was taken meanwhile: nothing is replaced.
        let dest = free_name(dir, name).map_err(cant)?;
        copy_link(file, &dest).map_err(cant)?;
        Ok(dest)
    } else {
        let dest = claim_file(dir, name).map_err(cant)?;
        // Writes over the empty file just claimed, nothing else.
        fs::copy(file, &dest).map_err(|e| {
            let _ = fs::remove_file(&dest);
            format!("couldn't copy it: {e}")
        })?;
        Ok(dest)
    }
}

/// `file` moved into `dir`: renamed where it can be, otherwise copied and then removed.
fn move_into(file: &Path, dir: &Path) -> Result<PathBuf, String> {
    let (meta, name) = look(file)?;
    if file.parent().is_some_and(|parent| inside(parent, dir) && inside(dir, parent)) {
        return Err("it is already in that folder".into());
    }
    if meta.is_dir() && inside(dir, file) {
        return Err("a folder can't be moved into itself".into());
    }
    // A file's name is claimed with an empty file the rename then takes the place of.
    let claimed = meta.is_file();
    let dest = if claimed { claim_file(dir, name) } else { free_name(dir, name) }.map_err(|e| format!("can't move it there: {e}"))?;
    let undo = |dest: &Path| {
        if claimed {
            let _ = fs::remove_file(dest);
        }
    };
    match fs::rename(file, &dest) {
        Ok(()) => Ok(dest),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            undo(&dest);
            let copy = copy_into(file, dir).map_err(|e| format!("{e}, so the original stays"))?;
            let removed = if meta.is_dir() { fs::remove_dir_all(file) } else { fs::remove_file(file) };
            removed.map_err(|e| format!("copied, but the original couldn't be removed: {e}"))?;
            Ok(copy)
        }
        Err(e) => {
            undo(&dest);
            Err(format!("couldn't move it: {e}"))
        }
    }
}

/// Copies what is in folder `from` into the new, empty folder `to`.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let (src, dest) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            fs::create_dir(&dest)?;
            copy_tree(&src, &dest)?;
        } else if kind.is_symlink() {
            copy_link(&src, &dest)?;
        } else {
            fs::copy(&src, &dest)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_link(src: &Path, dest: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(src)?, dest)
}

/// Making a link on Windows needs rights an ordinary account may not have.
#[cfg(windows)]
fn copy_link(src: &Path, _dest: &Path) -> io::Result<()> {
    Err(io::Error::other(format!("{} is a link, and Windows doesn't let links be copied here", file_name(src))))
}

/// "photo.jpg", then "photo 2.jpg", ...: the names a copy of `name` may take.
fn candidates(name: &OsStr) -> impl Iterator<Item = String> {
    let name = name.to_string_lossy().into_owned();
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (name[..dot].to_string(), name[dot..].to_string()),
        _ => (name.clone(), String::new()),
    };
    (1..1000).map(move |n| if n == 1 { format!("{stem}{ext}") } else { format!("{stem} {n}{ext}") })
}

/// An empty file under the first free name: create_new claims the name atomically, so
/// a file that appears meanwhile is safe.
fn claim_file(dir: &Path, name: &OsStr) -> io::Result<PathBuf> {
    for candidate in candidates(name) {
        let dest = dir.join(candidate);
        match OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(_) => return Ok(dest),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many files with that name already"))
}

/// A new, empty folder under the first free name.
fn claim_folder(dir: &Path, name: &OsStr) -> io::Result<PathBuf> {
    for candidate in candidates(name) {
        let dest = dir.join(candidate);
        match fs::create_dir(&dest) {
            Ok(()) => return Ok(dest),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "too many folders with that name already"))
}

/// The first name nothing has yet, for a folder or a link, which can't be claimed
/// ahead the way a file is. Renaming never replaces a folder with anything in it, so
/// at worst a moved folder lands on an empty one made in the same instant.
fn free_name(dir: &Path, name: &OsStr) -> io::Result<PathBuf> {
    candidates(name)
        .map(|candidate| dir.join(candidate))
        .find(|dest| fs::symlink_metadata(dest).is_err())
        .ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "too many folders with that name already"))
}

fn folder_name(dir: &Path) -> String {
    dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The notification: where the files went, or what failed. A click shows them.
fn report(app: &AppHandle, moving: bool, dir: &Path, done: Vec<PathBuf>, failed: Vec<(String, String)>) {
    let (verb, failed_verb) = if moving { ("Moved", "move") } else { ("Copied", "copy") };
    let folder = folder_name(dir);
    let (title, body) = match (done.as_slice(), failed.as_slice()) {
        ([one], []) => (format!("{verb} to {folder}"), file_name(one)),
        (many, []) => (format!("{verb} to {folder}"), format!("{} files", many.len())),
        ([], [(name, why)]) => (format!("Couldn't {failed_verb} {name}"), why.clone()),
        (_, [(name, why), ..]) => (
            format!("{} of {} files failed", failed.len(), done.len() + failed.len()),
            format!("{name}: {why}"),
        ),
    };
    crate::notify_done(app, &title, &body, done);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_never_replaces_anything() {
        let work = tempfile::tempdir().unwrap();
        let (from, to) = (work.path().join("from"), work.path().join("to"));
        fs::create_dir_all(from.join("Holiday/inside")).unwrap();
        fs::write(from.join("photo.jpg"), "new").unwrap();
        fs::write(from.join("Holiday/inside/a.txt"), "a").unwrap();
        fs::create_dir_all(to.join("Holiday")).unwrap();
        fs::write(to.join("photo.jpg"), "old").unwrap();

        let (done, failed) = transfer(&[from.join("photo.jpg"), from.join("Holiday")], &to, false);
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(done, [to.join("photo 2.jpg"), to.join("Holiday 2")]);
        assert_eq!(fs::read_to_string(to.join("photo.jpg")).unwrap(), "old");
        assert_eq!(fs::read_to_string(to.join("photo 2.jpg")).unwrap(), "new");
        assert_eq!(fs::read_to_string(to.join("Holiday 2/inside/a.txt")).unwrap(), "a");
        // The originals stay.
        assert!(from.join("photo.jpg").is_file() && from.join("Holiday/inside/a.txt").is_file());
    }

    #[test]
    fn a_move_takes_the_original_along() {
        let work = tempfile::tempdir().unwrap();
        let (from, to) = (work.path().join("from"), work.path().join("to"));
        fs::create_dir_all(from.join("Holiday")).unwrap();
        fs::create_dir_all(&to).unwrap();
        fs::write(from.join("report.pdf"), "pdf").unwrap();
        fs::write(from.join("Holiday/a.txt"), "a").unwrap();
        fs::write(to.join("report.pdf"), "old").unwrap();

        let (done, failed) = transfer(&[from.join("report.pdf"), from.join("Holiday")], &to, true);
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(done, [to.join("report 2.pdf"), to.join("Holiday")]);
        assert_eq!(fs::read_to_string(to.join("report 2.pdf")).unwrap(), "pdf");
        assert_eq!(fs::read_to_string(to.join("report.pdf")).unwrap(), "old");
        assert_eq!(fs::read_to_string(to.join("Holiday/a.txt")).unwrap(), "a");
        assert!(!from.join("report.pdf").exists() && !from.join("Holiday").exists());
    }

    #[test]
    fn a_folder_never_goes_into_itself_and_a_move_in_place_is_refused() {
        let work = tempfile::tempdir().unwrap();
        let folder = work.path().join("Holiday");
        fs::create_dir_all(folder.join("inside")).unwrap();
        fs::write(work.path().join("a.txt"), "a").unwrap();

        let (_, failed) = transfer(std::slice::from_ref(&folder), &folder.join("inside"), false);
        assert_eq!(failed, [("Holiday".to_string(), "a folder can't be copied into itself".to_string())]);
        let (_, failed) = transfer(std::slice::from_ref(&folder), &folder.join("inside"), true);
        assert_eq!(failed, [("Holiday".to_string(), "a folder can't be moved into itself".to_string())]);
        let (_, failed) = transfer(&[work.path().join("a.txt")], work.path(), true);
        assert_eq!(failed, [("a.txt".to_string(), "it is already in that folder".to_string())]);
        assert!(work.path().join("a.txt").is_file());
        // Copying into the same folder makes a numbered copy.
        let (done, _) = transfer(&[work.path().join("a.txt")], work.path(), false);
        assert_eq!(done, [work.path().join("a 2.txt")]);
    }

    #[cfg(unix)]
    #[test]
    fn links_are_copied_as_links() {
        let work = tempfile::tempdir().unwrap();
        let folder = work.path().join("Holiday");
        fs::create_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", folder.join("hosts")).unwrap();
        let to = work.path().join("to");
        fs::create_dir_all(&to).unwrap();

        let (done, failed) = transfer(&[folder, work.path().join("Holiday/hosts")], &to, false);
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(fs::read_link(done[0].join("hosts")).unwrap(), Path::new("/etc/hosts"));
        assert_eq!(fs::read_link(&done[1]).unwrap(), Path::new("/etc/hosts"));
    }

    #[test]
    fn names_are_numbered_before_the_extension() {
        let names: Vec<String> = candidates(OsStr::new("photo.jpg")).take(2).collect();
        assert_eq!(names, ["photo.jpg", "photo 2.jpg"]);
        let names: Vec<String> = candidates(OsStr::new(".hidden")).take(2).collect();
        assert_eq!(names, [".hidden", ".hidden 2"]);
    }
}
