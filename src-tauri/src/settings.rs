//! The user's choices from the menu, kept as JSON in the app's support folder.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::formats::Kind;
use crate::wheel::lock;

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
    /// The file exists but couldn't be read: never save over it.
    locked: bool,
}

impl Store {
    /// Reads the settings file. A missing file gives the defaults. Otherwise, when the
    /// file can't be used, the defaults apply and the second value says why, for a
    /// notification: a file that isn't valid settings is kept aside as
    /// settings.json.bad; a file that can't be read at all is left alone and never
    /// saved over.
    pub fn load(path: PathBuf) -> (Self, Option<String>) {
        let mut locked = false;
        let (current, problem) = match fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(settings) => (settings, None),
                Err(e) => {
                    let bad = path.with_extension("json.bad");
                    let problem = match fs::rename(&path, &bad) {
                        Ok(()) => format!("Your settings were reset: the file was damaged ({e}). It is kept as settings.json.bad."),
                        Err(r) => {
                            locked = true;
                            format!("Your settings were reset: the file was damaged ({e}) and couldn't be set aside ({r}), so it won't be saved over.")
                        }
                    };
                    (Settings::default(), Some(problem))
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (Settings::default(), None),
            Err(e) => {
                locked = true;
                (
                    Settings::default(),
                    Some(format!("Your settings couldn't be read ({e}); defaults are in use and the file won't be saved over.")),
                )
            }
        };
        if let Some(problem) = &problem {
            crate::log::line(problem);
        }
        (Self { path, current: Mutex::new(current), locked }, problem)
    }

    pub fn get(&self) -> Settings {
        lock(&self.current).clone()
    }

    /// Changes the settings and saves them straight away. If saving fails, nothing
    /// changes, so what the menu shows always matches what is on disk.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<Settings, String> {
        if self.locked {
            return Err("the settings file couldn't be read, so it isn't saved over".into());
        }
        let mut current = lock(&self.current);
        let mut next = current.clone();
        change(&mut next);
        self.save(&next).map_err(|e| format!("can't save settings: {e}"))?;
        *current = next.clone();
        Ok(next)
    }

    /// Writes to a temporary file first, then swaps it in, so a crash halfway never
    /// leaves a cut-off settings file.
    fn save(&self, settings: &Settings) -> io::Result<()> {
        let text = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
        let dir = self.path.parent().ok_or_else(|| io::Error::other("no settings folder"))?;
        fs::create_dir_all(dir)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_restart_and_bad_files_are_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let (store, problem) = Store::load(path.clone());
        assert_eq!(store.get(), Settings::default());
        assert!(problem.is_none());

        store
            .update(|s| {
                s.save_to = SaveTo::Downloads;
                s.hidden.insert(key(Kind::Image, "avif"));
            })
            .unwrap();
        let again = Store::load(path.clone()).0.get();
        assert_eq!(again.save_to, SaveTo::Downloads);
        assert!(!again.shows(Kind::Image, "avif"));
        assert!(again.shows(Kind::Image, "png"));

        fs::write(&path, "not json").unwrap();
        let (store, problem) = Store::load(path.clone());
        assert_eq!(store.get(), Settings::default());
        assert!(problem.unwrap().contains("settings.json.bad"));
        assert!(path.with_extension("json.bad").exists());
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the settings file should be: the save must fail.
        let path = dir.path().join("settings.json");
        fs::create_dir(&path).unwrap();
        let store = Store { path, current: Mutex::new(Settings::default()), locked: false };
        assert!(store.update(|s| s.save_to = SaveTo::Downloads).is_err());
        assert_eq!(store.get().save_to, SaveTo::NextToOriginal);
    }
}
