//! Shift+drag on GNOME under Wayland. Wayland keeps the pointer and the keys private,
//! so the app can't watch a drag the way it does on X11. GNOME Shell can, and a small
//! extension of the app's own (`gnome-shell/convert-wheel@oracola91.github.io`) tells
//! the app the keys held while a drag goes on (see linux.rs) and puts the wheel under
//! the pointer. The app copies the extension into the user's extension folder, keeps
//! it up to date, and turns it on the first time. GNOME Shell loads a new or changed
//! extension only at the next login.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::AppHandle;

use crate::wheel::Which;

pub const UUID: &str = "convert-wheel@oracola91.github.io";

const FILES: &[(&str, &str)] = &[
    ("metadata.json", include_str!("../gnome-shell/convert-wheel@oracola91.github.io/metadata.json")),
    ("extension.js", include_str!("../gnome-shell/convert-wheel@oracola91.github.io/extension.js")),
];

/// GNOME, by the desktop the session names ("ubuntu:GNOME" on Ubuntu).
pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|part| part.eq_ignore_ascii_case("gnome")))
}

/// Puts the extension in place or brings it up to date, turns it on the first time,
/// and says when a new login is needed to load it.
pub fn install(app: &AppHandle) {
    let Some(dir) = extensions_dir().map(|d| d.join(UUID)) else {
        crate::log::line("no home folder: the GNOME extension for Shift+drag isn't installed");
        return;
    };
    let first = !dir.exists();
    match write_files(&dir) {
        Ok(false) => return,
        Ok(true) => {}
        Err(e) => {
            crate::log::line(&format!("GNOME extension not installed ({e}); Shift+drag is off, the shortcut still works"));
            return;
        }
    }
    if first {
        if let Err(e) = turn_on() {
            crate::log::line(&format!("GNOME extension installed but not turned on ({e})"));
        }
    }
    let on = gsettings(&["get", "org.gnome.shell", "enabled-extensions"]).is_ok_and(|list| with_uuid(&list).is_none());
    crate::log::line(&format!(
        "GNOME extension {} in {} ({}); GNOME Shell loads it at the next login",
        if first { "installed" } else { "updated" },
        dir.display(),
        if on { "on" } else { "off in GNOME's Extensions app" },
    ));
    if on {
        crate::notify(
            app,
            "Log out and back in once for Shift+drag",
            "Then hold Shift while dragging a file, as on other systems. Until then, Super+Shift+C opens the wheel.",
        );
    }
}

/// Where GNOME Shell looks for the user's extensions.
fn extensions_dir() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))?;
    Some(data.join("gnome-shell/extensions"))
}

/// Writes the files that differ; true if any did.
fn write_files(dir: &Path) -> Result<bool, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut changed = false;
    for (name, text) in FILES {
        let path = dir.join(name);
        if fs::read_to_string(&path).ok().as_deref() != Some(*text) {
            fs::write(&path, text).map_err(|e| format!("{name}: {e}"))?;
            changed = true;
        }
    }
    Ok(changed)
}

/// Adds the extension to the ones GNOME Shell turns on at login.
fn turn_on() -> Result<(), String> {
    let list = gsettings(&["get", "org.gnome.shell", "enabled-extensions"])?;
    match with_uuid(&list) {
        Some(more) => gsettings(&["set", "org.gnome.shell", "enabled-extensions", &more]).map(|_| ()),
        None => Ok(()),
    }
}

fn gsettings(args: &[&str]) -> Result<String, String> {
    let out = Command::new("gsettings").args(args).output().map_err(|e| format!("gsettings: {e}"))?;
    if !out.status.success() {
        return Err(format!("gsettings: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The keys held, from one of the extension's "keys" lines: "100" is Shift alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keys {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
}

impl Keys {
    pub fn parse(text: &str) -> Option<Keys> {
        let held: Vec<bool> = text.chars().map(|c| ['0', '1'].contains(&c).then_some(c == '1')).collect::<Option<_>>()?;
        let [shift, alt, control] = held[..] else {
            return None;
        };
        Some(Keys { shift, alt, control })
    }
}

/// What one poll of a drag asks for.
#[derive(Debug, PartialEq)]
pub enum Step {
    Wait,
    Open(Which),
    /// The wheel is open: switch it if the keys ask for another.
    Follow,
}

/// One drag the extension reports, poll by poll, as the X11 trigger follows one: the
/// wheel opens once Shift has been held for KEYS_SETTLE polls, if the drag started in
/// a file manager.
pub struct Drag {
    from_files: bool,
    held: u8,
    pub shown: bool,
    /// Shift was held, but the drag didn't start in a file manager.
    pub refused: bool,
}

impl Drag {
    pub fn new(from_files: bool) -> Drag {
        Drag { from_files, held: 0, shown: false, refused: false }
    }

    pub fn poll(&mut self, keys: Keys) -> Step {
        if self.shown {
            return Step::Follow;
        }
        self.held = if keys.shift { self.held.saturating_add(1) } else { 0 };
        if self.held < crate::trigger::KEYS_SETTLE {
            return Step::Wait;
        }
        if !self.from_files {
            self.refused = true;
            return Step::Wait;
        }
        self.shown = true;
        Step::Open(Which::from_keys(keys.alt, keys.control))
    }
}

/// gsettings' list of strings `list` with this extension added; None when it is in it
/// already or the list can't be read.
fn with_uuid(list: &str) -> Option<String> {
    if list.contains(&format!("'{UUID}'")) {
        return None;
    }
    let inner = list.trim().strip_prefix("@as ").unwrap_or(list.trim());
    let inner = inner.strip_prefix('[')?.strip_suffix(']')?.trim();
    Some(if inner.is_empty() { format!("['{UUID}']") } else { format!("[{inner}, '{UUID}']") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_is_added_to_the_list_once() {
        assert_eq!(with_uuid("@as []").as_deref(), Some("['convert-wheel@oracola91.github.io']"));
        assert_eq!(
            with_uuid("['ding@rastersoft.com', 'ubuntu-dock@ubuntu.com']").as_deref(),
            Some("['ding@rastersoft.com', 'ubuntu-dock@ubuntu.com', 'convert-wheel@oracola91.github.io']")
        );
        assert_eq!(with_uuid("['convert-wheel@oracola91.github.io']"), None);
        // Something gsettings never prints: left alone.
        assert_eq!(with_uuid("nonsense"), None);
    }

    #[test]
    fn keys_come_as_three_digits() {
        assert_eq!(Keys::parse("100"), Some(Keys { shift: true, alt: false, control: false }));
        assert_eq!(Keys::parse("011"), Some(Keys { shift: false, alt: true, control: true }));
        for bad in ["", "10", "1000", "1a0", "ü00"] {
            assert_eq!(Keys::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_drag_opens_the_wheel_like_x11_does() {
        let keys = |shift, alt, control| Keys { shift, alt, control };
        // Shift has to be held for a moment; letting go starts the count again.
        let mut drag = Drag::new(true);
        assert_eq!(drag.poll(keys(false, false, false)), Step::Wait);
        assert_eq!(drag.poll(keys(true, false, false)), Step::Wait);
        assert_eq!(drag.poll(keys(false, false, false)), Step::Wait);
        assert_eq!(drag.poll(keys(true, true, false)), Step::Wait);
        assert_eq!(drag.poll(keys(true, true, false)), Step::Open(Which::Tools));
        // Open: from now on the keys switch the wheel.
        assert_eq!(drag.poll(keys(false, false, false)), Step::Follow);
        assert!(drag.shown && !drag.refused);
        // From an app that isn't a file manager: never.
        let mut drag = Drag::new(false);
        for _ in 0..5 {
            assert_eq!(drag.poll(keys(true, false, false)), Step::Wait);
        }
        assert!(drag.refused && !drag.shown);
    }

    /// The extension and the app must agree on the names they share.
    #[test]
    fn the_extension_matches_the_app() {
        let metadata: serde_json::Value = serde_json::from_str(FILES[0].1).unwrap();
        assert_eq!(metadata["uuid"], UUID);
        let script = FILES[1].1;
        assert!(script.contains("'convert-wheel.sock'"), "the socket in linux.rs");
        assert!(script.contains(&format!("const WHEEL_SIZE = {};", crate::wheel::SIZE)), "the wheel's size");
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let wheel = config["app"]["windows"].as_array().unwrap().iter().find(|w| w["label"] == "wheel").unwrap();
        assert!(script.contains(&format!("const WHEEL_TITLE = '{}';", wheel["title"].as_str().unwrap())));
    }
}
