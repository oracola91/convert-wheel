//! The user's choices from the menu, kept as JSON in the app's support folder.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::formats::Kind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SaveTo {
    #[default]
    NextToOriginal,
    Downloads,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub save_to: SaveTo,
    /// Formats left off the wheel, as "kind:format" (for example "image:avif").
    pub hidden: BTreeSet<String>,
}

pub fn key(kind: Kind, format: &str) -> String {
    format!("{}:{format}", kind.key())
}

impl Settings {
    pub fn shows(&self, kind: Kind, format: &str) -> bool {
        !self.hidden.contains(&key(kind, format))
    }
}

pub struct Store {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl Store {
    /// Reads the settings file. A missing or unreadable file gives the defaults.
    pub fn load(path: PathBuf) -> Self {
        let current = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, current: Mutex::new(current) }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Changes the settings and writes them straight away.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Settings {
        let Ok(mut current) = self.current.lock() else {
            return Settings::default();
        };
        change(&mut current);
        let written = self
            .path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| fs::write(&self.path, serde_json::to_string_pretty(&*current).unwrap_or_default()));
        if let Err(e) = written {
            eprintln!("convert-wheel: settings not saved: {e}");
        }
        current.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_restart_and_bad_files_give_defaults() {
        let dir = std::env::temp_dir().join(format!("convert-wheel-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let store = Store::load(path.clone());
        assert_eq!(store.get(), Settings::default());

        store.update(|s| {
            s.save_to = SaveTo::Downloads;
            s.hidden.insert(key(Kind::Image, "avif"));
        });
        let again = Store::load(path.clone()).get();
        assert_eq!(again.save_to, SaveTo::Downloads);
        assert!(!again.shows(Kind::Image, "avif"));
        assert!(again.shows(Kind::Image, "png"));

        fs::write(&path, "not json").unwrap();
        assert_eq!(Store::load(path).get(), Settings::default());
        let _ = fs::remove_dir_all(dir);
    }
}
