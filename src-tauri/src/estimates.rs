//! The middle of the wheel says roughly how big the result will be while a segment is
//! under the cursor. Working that out runs tools, so it happens on a thread of its own,
//! for one file at a time (the newest asked), and what it found is kept while the file
//! and the menu's quality stay the same.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::SystemTime;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::convert;
use crate::settings::Quality;
use crate::wheel::{self, WheelState, Which};

/// How many files' estimates are kept.
const KEPT: usize = 64;

/// The one file on the wheel and its segments.
struct Ask {
    path: String,
    targets: Vec<&'static str>,
}

/// For the page: estimated sizes in bytes by segment, for the file at `path`.
#[derive(Clone, Serialize)]
struct Found {
    path: String,
    sizes: HashMap<&'static str, u64>,
}

/// The same file, unchanged, at the same quality.
#[derive(PartialEq, Eq, Hash)]
struct Key {
    path: String,
    len: u64,
    modified: Option<SystemTime>,
    quality: [&'static str; 3],
}

fn key(path: &str, quality: Quality) -> Option<Key> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    Some(Key {
        path: path.to_string(),
        len: meta.len(),
        modified: meta.modified().ok(),
        quality: [quality.pictures.key(), quality.video.key(), quality.audio.key()],
    })
}

pub struct Estimates(Sender<Ask>);

pub fn start(app: AppHandle) -> Estimates {
    let (tx, rx) = mpsc::channel::<Ask>();
    thread::spawn(move || {
        // What was found for each segment, None where nothing can be told.
        let mut kept: HashMap<Key, HashMap<&'static str, Option<u64>>> = HashMap::new();
        while let Ok(mut ask) = rx.recv() {
            // Only the newest counts: the wheel shows one file at a time.
            while let Ok(newer) = rx.try_recv() {
                ask = newer;
            }
            let quality = app.state::<crate::settings::Store>().get().quality;
            let Some(key) = key(&ask.path, quality) else {
                continue;
            };
            if kept.len() >= KEPT && !kept.contains_key(&key) {
                kept.clear();
            }
            let known = kept.entry(key).or_default();
            let missing: Vec<&'static str> = ask.targets.iter().copied().filter(|t| !known.contains_key(t)).collect();
            if !missing.is_empty() {
                let found = convert::estimate::sizes(Path::new(&ask.path), &missing, quality);
                for target in missing {
                    known.insert(target, found.iter().find(|(t, _)| t.as_str() == target).map(|&(_, bytes)| bytes));
                }
            }
            let sizes = ask.targets.iter().filter_map(|&t| Some((t, (*known.get(t)?)?))).collect();
            if crate::trigger::showing(&app, &ask.path) {
                let _ = app.emit("wheel-estimates", Found { path: ask.path, sizes });
            }
        }
    });
    Estimates(tx)
}

/// Asks for estimates for the wheel that is showing, when it shows one file. The
/// Actions wheel makes nothing to estimate.
pub fn ask(app: &AppHandle) {
    let state = app.state::<WheelState>();
    let which = *wheel::lock(&state.which);
    let path = match wheel::lock(&state.paths).as_slice() {
        [path] if which != Which::Actions => path.clone(),
        _ => return,
    };
    let targets = wheel::lock(&state.formats).clone();
    let _ = app.state::<Estimates>().0.send(Ask { path, targets });
}
