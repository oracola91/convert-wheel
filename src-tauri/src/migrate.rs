//! Moving from the app's earlier names. Up to 0.3.x the app had another name, the app
//! id local.convertwheel and the program file convert-wheel. Each start moves what the
//! old names still hold, when there is something to move: the settings and the shelf,
//! Open at Login, the Linux keyboard shortcut and the GNOME Shell extension. The old
//! log, cache and web view folders are deleted. Only this file, windows/hooks.nsh, the
//! Linux package's replaces line and CI's upgrade checks still know the old names.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// The app's name up to 0.3.x: Windows' Run value, Send To entry and Start menu.
#[cfg_attr(not(windows), allow(dead_code))]
pub const NAME: &str = "Convert Wheel";
/// Its app id: the folders of the settings, the log and the web view.
pub const ID: &str = "local.convertwheel";
/// Its program file.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub const PROGRAM: &str = "convert-wheel";
/// Its working-files folder on Windows and Linux (convert.rs, cache_dir).
const CACHE: &str = "convertwheel";
/// Its GNOME Shell extension.
#[cfg(any(target_os = "linux", test))]
const GNOME_UUID: &str = "convert-wheel@oracola91.github.io";

/// `path` with every folder named `new_id` named `old` instead; None when it has none.
fn with_old_id(path: &Path, new_id: &str, old: &str) -> Option<PathBuf> {
    let mut found = false;
    let swapped: PathBuf = path
        .components()
        .map(|c| match c {
            Component::Normal(part) if part == new_id => {
                found = true;
                Component::Normal(old.as_ref())
            }
            other => other,
        })
        .collect();
    found.then_some(swapped)
}

/// Deletes an old folder, only one whose own name is an old name.
fn remove(dir: &Path) {
    let named = dir.file_name().is_some_and(|n| n == ID || n == CACHE);
    if !named || !dir.is_dir() {
        return;
    }
    match fs::remove_dir_all(dir) {
        Ok(()) => crate::log::line(&format!("removed the folder of the app's earlier name: {}", dir.display())),
        Err(e) => crate::log::line(&format!("couldn't remove {}: {e}", dir.display())),
    }
}

/// Copies the settings and the shelf from the old id's folder into `config` (the
/// settings folder under the new id `new_id`), where they aren't yet, then deletes the
/// old folder. Runs before the settings are read.
pub fn settings(config: &Path, new_id: &str) {
    let Some(old) = with_old_id(config, new_id, ID).filter(|o| o.is_dir()) else {
        return;
    };
    let mut copied = true;
    for name in ["settings.json", "shelf.json"] {
        let (from, to) = (old.join(name), config.join(name));
        if from.is_file() && !to.exists() {
            if let Err(e) = fs::create_dir_all(config).and_then(|_| fs::copy(&from, &to)) {
                crate::log::line(&format!("couldn't move {name} from the earlier settings folder: {e}"));
                copied = false;
            }
        }
    }
    if copied {
        crate::log::line("settings moved from the app's earlier name");
        remove(&old);
    }
}

/// Deletes the old id's log, cache and web view folders, and the old working-files
/// folder. `dirs` are the app's own folders under the new id `new_id`.
pub fn leftovers(dirs: &[PathBuf], new_id: &str) {
    for dir in dirs {
        if let Some(old) = with_old_id(dir, new_id, ID) {
            // The folder named after the id, not a "logs" folder inside it.
            let top: PathBuf = old.ancestors().find(|a| a.file_name().is_some_and(|n| n == ID)).map_or(old.clone(), Path::to_path_buf);
            remove(&top);
        }
    }
    for cache in old_cache_dirs() {
        remove(&cache);
    }
}

/// The old working-files folder: %LOCALAPPDATA%\convertwheel, ~/.cache/convertwheel.
/// macOS kept it under the app id (Library/Caches), which `leftovers` covers.
fn old_cache_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        dirs.extend(std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(CACHE)));
    }
    if cfg!(target_os = "linux") {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")));
        dirs.extend(cache.map(|c| c.join(CACHE)));
    }
    dirs
}

/// Open at Login made by a copy under the old names: made again for this copy.
pub fn login() {
    #[cfg(windows)]
    crate::login::move_old_entry();
    #[cfg(target_os = "linux")]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
        let Some(old) = config.map(|c| c.join(format!("autostart/{PROGRAM}.desktop"))).filter(|p| p.is_file()) else {
            return;
        };
        if let Err(e) = fs::remove_file(&old) {
            crate::log::line(&format!("couldn't remove the earlier Open at Login entry: {e}"));
            return;
        }
        if crate::login::available() {
            match crate::login::set_enabled(true) {
                Ok(()) => crate::log::line("Open at Login moved from the app's earlier name"),
                Err(e) => crate::log::line(&format!("couldn't set Open at Login again: {e}")),
            }
        }
    }
}

/// The keyboard shortcut's command with the old program file swapped for `program`:
/// "/usr/bin/convert-wheel --show" to "/usr/bin/pirouette --show".
#[cfg(any(target_os = "linux", test))]
fn new_command(old: &str, program: &str) -> String {
    let (exe, rest) = old.split_once(' ').map_or((old, ""), |(e, r)| (e, r));
    let swapped = match exe.rsplit_once('/') {
        Some((dir, name)) if name == PROGRAM => format!("{dir}/{program}"),
        None if exe == PROGRAM => program.to_string(),
        _ => exe.to_string(),
    };
    if rest.is_empty() {
        swapped
    } else {
        format!("{swapped} {rest}")
    }
}

/// A gsettings list ("['a', 'b']" or "@as []") without `item`; None when it isn't there.
#[cfg(any(target_os = "linux", test))]
fn without(list: &str, item: &str) -> Option<String> {
    let quoted = format!("'{item}'");
    if !list.contains(&quoted) {
        return None;
    }
    let inner = list.trim().trim_start_matches("@as").trim().trim_start_matches('[').trim_end_matches(']');
    let kept: Vec<&str> = inner.split(',').map(str::trim).filter(|s| !s.is_empty() && *s != quoted).collect();
    Some(if kept.is_empty() { "@as []".to_string() } else { format!("[{}]", kept.join(", ")) })
}

#[cfg(target_os = "linux")]
fn gsettings(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("gsettings").args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The GNOME keyboard shortcut the setup script made for the old program file (see
/// scripts/install-linux.sh): the same keys under the new slot, starting this program.
#[cfg(target_os = "linux")]
pub fn shortcut() {
    const KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
    const BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
    let (old_slot, new_slot) = (format!("{BASE}{PROGRAM}/"), format!("{BASE}pirouette/"));
    let Some(list) = gsettings(&["get", KEYS, "custom-keybindings"]) else {
        return;
    };
    let Some(rest) = without(&list, &old_slot) else {
        return;
    };
    let one = |slot: &str| format!("{KEYS}.custom-keybinding:{slot}");
    let unquote = |v: String| v.trim_matches('\'').to_string();
    let Some(binding) = gsettings(&["get", &one(&old_slot), "binding"]).map(unquote) else {
        return;
    };
    let command = gsettings(&["get", &one(&old_slot), "command"]).map(unquote).unwrap_or_default();
    let program = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "pirouette".into());
    let list = if rest.contains(&format!("'{new_slot}'")) {
        rest
    } else if rest == "@as []" {
        format!("['{new_slot}']")
    } else {
        format!("{}, '{new_slot}']", rest.trim_end_matches(']'))
    };
    let moved = gsettings(&["set", &one(&new_slot), "name", crate::APP_NAME]).is_some()
        && gsettings(&["set", &one(&new_slot), "command", &new_command(&command, &program)]).is_some()
        && gsettings(&["set", &one(&new_slot), "binding", &binding]).is_some()
        && gsettings(&["set", KEYS, "custom-keybindings", &list]).is_some();
    if moved {
        for key in ["name", "command", "binding"] {
            let _ = gsettings(&["reset", &one(&old_slot), key]);
        }
        crate::log::line("keyboard shortcut moved from the app's earlier name");
    } else {
        crate::log::line("couldn't move the keyboard shortcut from the app's earlier name");
    }
}

/// The GNOME Shell extension under the old name: switched off and its files deleted
/// (gnome_shell.rs installs the one under the new name).
#[cfg(target_os = "linux")]
pub fn gnome_extension() {
    if let Some(list) = gsettings(&["get", "org.gnome.shell", "enabled-extensions"]) {
        if let Some(rest) = without(&list, GNOME_UUID) {
            let _ = gsettings(&["set", "org.gnome.shell", "enabled-extensions", &rest]);
        }
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    if let Some(dir) = data.map(|d| d.join("gnome-shell/extensions").join(GNOME_UUID)).filter(|d| d.is_dir()) {
        match fs::remove_dir_all(&dir) {
            Ok(()) => crate::log::line("removed the GNOME Shell extension of the app's earlier name"),
            Err(e) => crate::log::line(&format!("couldn't remove the earlier GNOME Shell extension: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: &str = "io.github.oracola91.pirouette";

    #[test]
    fn the_old_id_takes_the_new_ones_place() {
        let mac = Path::new("/Users/a/Library/Application Support").join(NEW);
        assert_eq!(with_old_id(&mac, NEW, ID).unwrap(), Path::new("/Users/a/Library/Application Support/local.convertwheel"));
        let logs = Path::new("C:/Users/a/AppData/Local").join(NEW).join("logs");
        assert_eq!(with_old_id(&logs, NEW, ID).unwrap(), Path::new("C:/Users/a/AppData/Local/local.convertwheel/logs"));
        assert_eq!(with_old_id(Path::new("/tmp/other"), NEW, ID), None);
    }

    #[test]
    fn settings_move_once_and_the_old_folder_goes() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (root.path().join(ID), root.path().join(NEW));
        fs::create_dir(&old).unwrap();
        fs::write(old.join("settings.json"), r#"{"save_to":"downloads"}"#).unwrap();
        fs::write(old.join("shelf.json"), "[]").unwrap();
        settings(&new, NEW);
        assert_eq!(fs::read_to_string(new.join("settings.json")).unwrap(), r#"{"save_to":"downloads"}"#);
        assert!(new.join("shelf.json").is_file());
        assert!(!old.exists());
        // Settings already under the new id are never overwritten.
        fs::create_dir(&old).unwrap();
        fs::write(old.join("settings.json"), "{}").unwrap();
        settings(&new, NEW);
        assert_eq!(fs::read_to_string(new.join("settings.json")).unwrap(), r#"{"save_to":"downloads"}"#);
    }

    #[test]
    fn leftovers_delete_only_old_named_folders() {
        let root = tempfile::tempdir().unwrap();
        let old_logs = root.path().join(ID).join("logs");
        fs::create_dir_all(&old_logs).unwrap();
        fs::write(old_logs.join("old.log"), "x").unwrap();
        let other = root.path().join("keep-me");
        fs::create_dir(&other).unwrap();
        leftovers(&[root.path().join(NEW).join("logs"), other.clone()], NEW);
        assert!(!root.path().join(ID).exists());
        assert!(other.is_dir());
    }

    #[test]
    fn the_shortcut_command_gets_the_new_program() {
        assert_eq!(new_command("/usr/bin/convert-wheel --show", "pirouette"), "/usr/bin/pirouette --show");
        assert_eq!(new_command("convert-wheel --show tools", "pirouette"), "pirouette --show tools");
        assert_eq!(new_command("/opt/other --show", "pirouette"), "/opt/other --show");
    }

    #[test]
    fn lists_lose_one_item() {
        assert_eq!(without("['/a/', '/b/']", "/a/").as_deref(), Some("['/b/']"));
        assert_eq!(without("['/a/']", "/a/").as_deref(), Some("@as []"));
        assert_eq!(without("['/a/']", "/c/"), None);
        assert_eq!(without("@as []", "/a/"), None);
        assert_eq!(without(&format!("['ding@rastersoft.com', '{GNOME_UUID}']"), GNOME_UUID).as_deref(), Some("['ding@rastersoft.com']"));
    }
}
