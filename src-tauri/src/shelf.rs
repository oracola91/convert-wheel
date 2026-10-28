//! The shelf: files held at the top of the screen for a moment, to drag out again
//! somewhere else. Holding a file only remembers where it is: nothing is copied or
//! moved until the files are dragged out of the shelf. What is held survives a
//! restart; files that are gone by then are dropped, never guessed at.
//!
//! On a screen with a notch the shelf grows out of the notch. Elsewhere it sits under
//! the app's menu bar icon, or in the top right corner when the icon can't be seen.
//! The placement maths is here so it can be tested anywhere; the window itself is in
//! shelf_window.rs (macOS).

// Windows and Linux hold the list but have no shelf window yet.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::wheel::lock;

/// The extra width each side of the notch while the shelf holds files. Points.
pub const WING: f64 = 38.0;
/// The open shelf: its width, and the height of its body under the notch or menu bar.
pub const OPEN_WIDTH: f64 = 360.0;
pub const BODY: f64 = 96.0;
/// The small pill under the menu bar icon while the shelf holds files.
pub const PILL_WIDTH: f64 = 68.0;
pub const PILL_HEIGHT: f64 = 30.0;
/// Space between the menu bar and the shelf, and between the shelf and a screen edge.
const GAP: f64 = 6.0;
const MARGIN: f64 = 8.0;
/// How close a file drag must come to the notch (sideways, downwards) or to the
/// shelf's place under the menu bar before the shelf opens for the drop.
const ZONE_SIDE: f64 = 120.0;
const ZONE_DOWN: f64 = 110.0;
const ZONE_AROUND: f64 = 60.0;

/// A rectangle in screen points, origin at the bottom left like AppKit's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Area {
    pub fn max_x(&self) -> f64 {
        self.x + self.w
    }

    pub fn max_y(&self) -> f64 {
        self.y + self.h
    }

    pub fn mid_x(&self) -> f64 {
        self.x + self.w / 2.0
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.max_x() && y >= self.y && y <= self.max_y()
    }
}

/// Where the shelf lives on a screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    /// The screen has a notch, here: the shelf grows out of it.
    Notch(Area),
    /// Under the menu bar, centred under `x` but kept inside `visible` (the screen
    /// without the menu bar and Dock).
    Below { x: f64, visible: Area, screen_top: f64 },
}

impl Anchor {
    /// The anchor on a screen: its `notch` if it has one, else under the menu bar
    /// `icon` if that is on this screen, else the top right corner.
    pub fn on(screen: Area, visible: Area, notch: Option<Area>, icon: Option<Area>) -> Anchor {
        if let Some(notch) = notch.filter(|n| n.w > 0.0 && n.h > 0.0) {
            return Anchor::Notch(notch);
        }
        let x = match icon {
            Some(icon) if screen.contains(icon.mid_x(), icon.y) => icon.mid_x(),
            _ => visible.max_x() - MARGIN - PILL_WIDTH / 2.0,
        };
        Anchor::Below { x, visible, screen_top: screen.max_y() }
    }

    /// The shelf's window for `look`, or None while it is hidden.
    pub fn frame(&self, look: Look) -> Option<Area> {
        let open = matches!(look, Look::Open | Look::Drop { .. });
        match (*self, look) {
            (_, Look::Hidden) => None,
            (Anchor::Notch(n), _) if !open => Some(Area { x: n.x - WING, y: n.y, w: n.w + 2.0 * WING, h: n.h }),
            (Anchor::Notch(n), _) => {
                let w = (n.w + 2.0 * WING).max(OPEN_WIDTH);
                let h = n.h + BODY;
                Some(Area { x: n.mid_x() - w / 2.0, y: n.max_y() - h, w, h })
            }
            (Anchor::Below { x, visible, .. }, _) => {
                let (w, h) = if open { (OPEN_WIDTH, BODY) } else { (PILL_WIDTH, PILL_HEIGHT) };
                let left = (x - w / 2.0).min(visible.max_x() - MARGIN - w).max(visible.x + MARGIN);
                Some(Area { x: left, y: visible.max_y() - GAP - h, w, h })
            }
        }
    }

    /// Where a file drag opens the shelf for a drop: around the notch, or around the
    /// open shelf's place up to the top of the screen.
    pub fn zone(&self) -> Area {
        match *self {
            Anchor::Notch(n) => Area {
                x: n.x - ZONE_SIDE,
                y: n.y - ZONE_DOWN,
                w: n.w + 2.0 * ZONE_SIDE,
                h: n.h + ZONE_DOWN,
            },
            Anchor::Below { screen_top, .. } => {
                let open = self.frame(Look::Open).unwrap_or(Area { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
                let bottom = open.y - ZONE_AROUND;
                Area { x: open.x - ZONE_AROUND, y: bottom, w: open.w + 2.0 * ZONE_AROUND, h: screen_top - bottom }
            }
        }
    }

    pub fn is_notch(&self) -> bool {
        matches!(self, Anchor::Notch(_))
    }
}

/// How the shelf shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Hidden,
    /// Holds files: a small sign of that beside the notch or under the menu bar icon.
    Compact,
    /// The pointer is over it: what it holds, ready to drag out.
    Open,
    /// A file drag came near: ready to take the files. `over`: the drag is on it.
    Drop { over: bool },
}

/// The held files, saved as a JSON list of paths.
pub struct Shelf {
    file: Option<PathBuf>,
    held: Mutex<Vec<String>>,
}

impl Shelf {
    /// Reads what was held before. Files that are gone since are dropped; a damaged
    /// list starts the shelf empty. Both are logged.
    pub fn load(file: PathBuf) -> Shelf {
        let held: Vec<String> = match fs::read_to_string(&file) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                crate::log::line(&format!("shelf list unreadable ({e}); starting empty"));
                Vec::new()
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                crate::log::line(&format!("shelf list unreadable ({e}); starting empty"));
                Vec::new()
            }
        };
        let shelf = Shelf { file: Some(file), held: Mutex::new(held) };
        let gone = shelf.prune();
        if gone > 0 {
            crate::log::line(&format!("shelf: {gone} held file(s) no longer there"));
        }
        shelf
    }

    /// A shelf that is never saved.
    #[cfg(test)]
    fn in_memory() -> Shelf {
        Shelf { file: None, held: Mutex::new(Vec::new()) }
    }

    pub fn held(&self) -> Vec<String> {
        lock(&self.held).clone()
    }

    pub fn is_empty(&self) -> bool {
        lock(&self.held).is_empty()
    }

    /// Holds `paths` too, after the ones held already; a path held already stays
    /// where it is. Returns how many were new.
    pub fn add(&self, paths: Vec<String>) -> usize {
        let mut held = lock(&self.held);
        let before = held.len();
        for p in paths {
            if !held.contains(&p) {
                held.push(p);
            }
        }
        let added = held.len() - before;
        if added > 0 {
            self.save(&held);
        }
        added
    }

    pub fn remove(&self, paths: &[String]) {
        let mut held = lock(&self.held);
        let before = held.len();
        held.retain(|p| !paths.contains(p));
        if held.len() != before {
            self.save(&held);
        }
    }

    pub fn clear(&self) {
        let mut held = lock(&self.held);
        held.clear();
        self.save(&held);
    }

    /// Lets go of files that are no longer where they were (moved, renamed or deleted
    /// since). Returns how many.
    pub fn prune(&self) -> usize {
        let mut held = lock(&self.held);
        let before = held.len();
        held.retain(|p| Path::new(p).exists());
        let gone = before - held.len();
        if gone > 0 {
            self.save(&held);
        }
        gone
    }

    /// Writes a temporary file first, then swaps it in, like the settings. A failure
    /// only costs the list after a restart, so it is logged, not shown.
    fn save(&self, held: &[String]) {
        let Some(file) = &self.file else {
            return;
        };
        let result = (|| {
            let text = serde_json::to_string_pretty(held).map_err(io::Error::other)?;
            if let Some(dir) = file.parent() {
                fs::create_dir_all(dir)?;
            }
            let tmp = file.with_extension("json.tmp");
            fs::write(&tmp, text)?;
            fs::rename(&tmp, file)
        })();
        if let Err(e) = result {
            crate::log::line(&format!("couldn't save the shelf list: {e}"));
        }
    }
}

/// The shelf's title for what it holds: the name of one file, or "3 files" ("3
/// items" once a folder is among them).
pub fn title(held: &[String]) -> String {
    match held {
        [] => "Nothing held".to_string(),
        [one] => file_name(one),
        many if many.iter().any(|p| Path::new(p).is_dir()) => format!("{} items", many.len()),
        many => format!("{} files", many.len()),
    }
}

/// The count beside the notch or in the pill: "3", or "99+".
pub fn count(n: usize) -> String {
    if n > 99 {
        "99+".to_string()
    } else {
        n.to_string()
    }
}

fn file_name(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Area = Area { x: 0.0, y: 0.0, w: 1512.0, h: 982.0 };
    /// A 14-inch MacBook Pro: menu bar 37 points high, notch about 185 wide.
    const NOTCH: Area = Area { x: 663.5, y: 945.0, w: 185.0, h: 37.0 };
    const VISIBLE: Area = Area { x: 0.0, y: 0.0, w: 1512.0, h: 945.0 };

    #[test]
    fn a_notch_screen_grows_the_shelf_out_of_the_notch() {
        let anchor = Anchor::on(SCREEN, VISIBLE, Some(NOTCH), None);
        assert!(anchor.is_notch());
        assert_eq!(anchor.frame(Look::Hidden), None);
        // Compact: the notch with a wing each side, flush with the top of the screen.
        let compact = anchor.frame(Look::Compact).unwrap();
        assert_eq!(compact, Area { x: 663.5 - WING, y: 945.0, w: 185.0 + 2.0 * WING, h: 37.0 });
        // Open: wider, centred on the notch, hanging down from the top.
        let open = anchor.frame(Look::Open).unwrap();
        assert_eq!(open.w, OPEN_WIDTH);
        assert_eq!(open.mid_x(), NOTCH.mid_x());
        assert_eq!(open.max_y(), SCREEN.max_y());
        assert_eq!(open.h, 37.0 + BODY);
        assert_eq!(anchor.frame(Look::Drop { over: true }), Some(open));
        // A drag near the notch opens it; one in the middle of the screen doesn't.
        let zone = anchor.zone();
        assert!(zone.contains(NOTCH.mid_x(), 900.0));
        assert!(zone.contains(open.x, open.y) && zone.contains(open.max_x(), open.y));
        assert!(!zone.contains(NOTCH.mid_x(), 500.0));
        assert!(!zone.contains(100.0, 970.0));
    }

    #[test]
    fn a_wide_notch_keeps_its_wings_when_open() {
        let wide = Area { w: 320.0, ..NOTCH };
        let open = Anchor::Notch(wide).frame(Look::Open).unwrap();
        assert_eq!(open.w, 320.0 + 2.0 * WING);
    }

    #[test]
    fn without_a_notch_the_shelf_sits_under_the_menu_bar_icon() {
        let icon = Area { x: 1440.0, y: 945.0, w: 30.0, h: 37.0 };
        let anchor = Anchor::on(SCREEN, VISIBLE, None, Some(icon));
        let pill = anchor.frame(Look::Compact).unwrap();
        assert_eq!((pill.w, pill.h), (PILL_WIDTH, PILL_HEIGHT));
        assert_eq!(pill.mid_x(), icon.mid_x());
        assert_eq!(pill.max_y(), VISIBLE.max_y() - GAP);
        // Open near the right edge: moved left to stay on the screen, still under the icon.
        let open = anchor.frame(Look::Open).unwrap();
        assert_eq!(open.max_x(), VISIBLE.max_x() - MARGIN);
        assert!(open.x < icon.mid_x() && icon.mid_x() < open.max_x());
        // The zone reaches up to the top of the screen, over the menu bar.
        let zone = anchor.zone();
        assert!(zone.contains(icon.mid_x(), 975.0));
        assert!(zone.contains(open.x, open.y));
        assert!(!zone.contains(200.0, 900.0));
    }

    #[test]
    fn an_icon_on_another_screen_or_none_means_the_top_right_corner() {
        let elsewhere = Area { x: 2000.0, y: 1400.0, w: 30.0, h: 24.0 };
        for icon in [None, Some(elsewhere)] {
            let pill = Anchor::on(SCREEN, VISIBLE, None, icon).frame(Look::Compact).unwrap();
            assert_eq!(pill.max_x(), VISIBLE.max_x() - MARGIN);
        }
        // An empty notch area (a screen without one) is no notch.
        let none = Area { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        assert!(!Anchor::on(SCREEN, VISIBLE, Some(none), None).is_notch());
    }

    #[test]
    fn holding_remembers_paths_once_and_lets_go_of_them() {
        let shelf = Shelf::in_memory();
        assert!(shelf.is_empty());
        assert_eq!(shelf.add(vec!["/a".into(), "/b".into()]), 2);
        // Dropping a file held already doesn't hold it twice or move it.
        assert_eq!(shelf.add(vec!["/b".into(), "/c".into()]), 1);
        assert_eq!(shelf.held(), vec!["/a", "/b", "/c"]);
        shelf.remove(&["/a".to_string(), "/x".to_string()]);
        assert_eq!(shelf.held(), vec!["/b", "/c"]);
        shelf.clear();
        assert!(shelf.is_empty());
    }

    #[test]
    fn the_shelf_survives_a_restart_without_files_that_are_gone() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        let list = dir.path().join("shelf.json");
        let shelf = Shelf::load(list.clone());
        assert!(shelf.is_empty());
        let paths = vec![a.display().to_string(), b.display().to_string()];
        shelf.add(paths.clone());
        assert_eq!(Shelf::load(list.clone()).held(), paths);
        // b was moved away while the app was closed: only a comes back.
        fs::rename(&b, dir.path().join("b moved.txt")).unwrap();
        assert_eq!(Shelf::load(list.clone()).held(), vec![paths[0].clone()]);
        // A damaged list starts the shelf empty instead of failing.
        fs::write(&list, "not json").unwrap();
        assert!(Shelf::load(list).is_empty());
    }

    #[test]
    fn the_title_and_count_say_what_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display().to_string();
        assert_eq!(title(&[]), "Nothing held");
        assert_eq!(title(&["/Users/me/Report.docx".into()]), "Report.docx");
        assert_eq!(title(&["/a.pdf".into(), "/b.pdf".into()]), "2 files");
        assert_eq!(title(&["/a.pdf".into(), folder]), "2 items");
        assert_eq!(count(3), "3");
        assert_eq!(count(120), "99+");
    }
}
