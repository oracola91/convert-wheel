//! The user's choices from the menu, kept as JSON in the app's support folder.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::formats::Kind;
use crate::wheel::{lock, Which};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SaveTo {
    #[default]
    NextToOriginal,
    Downloads,
}

/// How a conversion trades file size for quality. Balanced is what the app has always
/// done.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    Smaller,
    #[default]
    Balanced,
    Best,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Smaller, Level::Balanced, Level::Best];

    pub fn key(self) -> &'static str {
        match self {
            Level::Smaller => "smaller",
            Level::Balanced => "balanced",
            Level::Best => "best",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Smaller => "Smaller Files",
            Level::Balanced => "Balanced",
            Level::Best => "Best Quality",
        }
    }

    /// One of three values, by level: the conversions' numbers.
    pub fn pick<T>(self, smaller: T, balanced: T, best: T) -> T {
        match self {
            Level::Smaller => smaller,
            Level::Balanced => balanced,
            Level::Best => best,
        }
    }
}

/// The quality level for each kind of file it matters for. Lossless formats (PNG,
/// TIFF, WAV, FLAC, AIFF) and documents don't have one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quality {
    pub pictures: Level,
    pub video: Level,
    pub audio: Level,
}

impl Quality {
    /// The kinds, by the key the menu uses, with their names.
    pub const KINDS: [(&'static str, &'static str); 3] = [("pictures", "Pictures"), ("video", "Video"), ("audio", "Audio")];

    pub fn get(&self, kind: &str) -> Option<Level> {
        match kind {
            "pictures" => Some(self.pictures),
            "video" => Some(self.video),
            "audio" => Some(self.audio),
            _ => None,
        }
    }

    pub fn set(&mut self, kind: &str, level: Level) {
        match kind {
            "pictures" => self.pictures = level,
            "video" => self.video = level,
            "audio" => self.audio = level,
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub save_to: SaveTo,
    /// Formats left off the wheel, as "kind:format" (for example "image:avif").
    pub hidden: BTreeSet<String>,
    pub quality: Quality,
    /// The shelf at the top of the screen is switched off (it is on unless chosen so).
    pub shelf_off: bool,
    pub highlight: Highlight,
    /// The wheel is a plain solid disc instead of Liquid Glass (macOS).
    pub no_liquid_glass: bool,
    /// How see-through the Liquid Glass wheel is (macOS), from the menu's slider.
    pub transparency: Transparency,
    /// Which keys open each wheel, from the menu's Keys.
    pub keys: Keys,
    /// The arrow keys move the wheel's highlight (macOS, Keys > Arrow Keys on the
    /// Wheel). Off unless switched on there, as reading them needs Input Monitoring.
    /// Windows and Linux read them always.
    pub arrow_keys: bool,
    /// Windows: the "Send to" entry for the keyboard wheel is taken out (it is there
    /// unless switched off in Settings).
    pub send_to_off: bool,
}

/// The keys held with Shift that open a wheel: Shift alone, with Option (Alt on Windows
/// and Linux), or with Control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Combo {
    Shift,
    ShiftOption,
    ShiftControl,
}

impl Combo {
    pub const ALL: [Combo; 3] = [Combo::Shift, Combo::ShiftOption, Combo::ShiftControl];

    /// The keys held with Shift now. Control wins when Option (Alt) is held too.
    pub fn from_keys(option: bool, control: bool) -> Self {
        match (option, control) {
            (_, true) => Combo::ShiftControl,
            (true, false) => Combo::ShiftOption,
            (false, false) => Combo::Shift,
        }
    }

    /// The name the menu and the settings file use.
    pub fn key(self) -> &'static str {
        match self {
            Combo::Shift => "shift",
            Combo::ShiftOption => "shift-option",
            Combo::ShiftControl => "shift-control",
        }
    }

    /// As the menu shows it, in the system's own names for the keys.
    pub fn label(self) -> &'static str {
        let mac = cfg!(target_os = "macos");
        match self {
            Combo::Shift => "Shift",
            Combo::ShiftOption if mac => "Shift+Option",
            Combo::ShiftOption => "Shift+Alt",
            Combo::ShiftControl if mac => "Shift+Control",
            Combo::ShiftControl => "Shift+Ctrl",
        }
    }
}

/// The keys that open each wheel, or None when that wheel is switched off. No two
/// wheels share keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub formats: Option<Combo>,
    pub tools: Option<Combo>,
    pub actions: Option<Combo>,
}

impl Default for Keys {
    fn default() -> Self {
        Keys::STANDARD
    }
}

impl Keys {
    /// Shift for the formats, with Option (Alt) for the tools, with Control the actions.
    pub const STANDARD: Keys =
        Keys { formats: Some(Combo::Shift), tools: Some(Combo::ShiftOption), actions: Some(Combo::ShiftControl) };

    pub fn get(&self, which: Which) -> Option<Combo> {
        match which {
            Which::Formats => self.formats,
            Which::Tools => self.tools,
            Which::Actions => self.actions,
        }
    }

    fn slot(&mut self, which: Which) -> &mut Option<Combo> {
        match which {
            Which::Formats => &mut self.formats,
            Which::Tools => &mut self.tools,
            Which::Actions => &mut self.actions,
        }
    }

    /// The wheel these keys open, if any (the first of Formats, Tools, Actions, should
    /// a hand-edited settings file give two wheels the same keys).
    pub fn wheel(&self, combo: Combo) -> Option<Which> {
        Which::ALL.into_iter().find(|w| self.get(*w) == Some(combo))
    }

    /// Puts `which` on `combo`, or switches it off with None. A wheel that had those
    /// keys takes the ones `which` had, so two wheels never share keys.
    pub fn assign(&mut self, which: Which, combo: Option<Combo>) {
        let old = self.get(which);
        if let Some(combo) = combo {
            for other in Which::ALL {
                if other != which && self.get(other) == Some(combo) {
                    *self.slot(other) = old;
                }
            }
        }
        *self.slot(which) = combo;
    }
}

/// The colour the segment under the cursor lights up in. src/wheel.css has each one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Highlight {
    #[default]
    Orange,
    Red,
    Pink,
    Purple,
    Blue,
    Teal,
    Green,
    Graphite,
}

impl Highlight {
    pub const ALL: [Highlight; 8] = [
        Highlight::Orange,
        Highlight::Red,
        Highlight::Pink,
        Highlight::Purple,
        Highlight::Blue,
        Highlight::Teal,
        Highlight::Green,
        Highlight::Graphite,
    ];

    /// The name the page and the settings file use.
    pub fn key(self) -> &'static str {
        match self {
            Highlight::Orange => "orange",
            Highlight::Red => "red",
            Highlight::Pink => "pink",
            Highlight::Purple => "purple",
            Highlight::Blue => "blue",
            Highlight::Teal => "teal",
            Highlight::Green => "green",
            Highlight::Graphite => "graphite",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Highlight::Orange => "Orange",
            Highlight::Red => "Red",
            Highlight::Pink => "Pink",
            Highlight::Purple => "Purple",
            Highlight::Blue => "Blue",
            Highlight::Teal => "Teal",
            Highlight::Green => "Green",
            Highlight::Graphite => "Graphite",
        }
    }
}

/// How see-through the Liquid Glass wheel is: 0 is frosted (the frosted material at
/// full strength behind it and the most white on the segments), 100 is clear (no
/// blur and next to no white; only the patches behind the words stay frosted).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Transparency(pub u8);

impl Default for Transparency {
    fn default() -> Self {
        Transparency(50)
    }
}

impl Transparency {
    fn share(self) -> f64 {
        f64::from(self.0.min(100)) / 100.0
    }

    /// How strongly the frosted material shows behind the wheel, 0 to 1.
    pub fn blur(self) -> f64 {
        1.0 - self.share()
    }

    /// How much white the segments carry.
    pub fn paint(self) -> f64 {
        0.40 - 0.34 * self.share()
    }

    /// Windows and Linux: how opaque the page's own disc is, with no blur behind it.
    /// Never below half, so the words stay readable on a busy background.
    pub fn disc(self) -> f64 {
        0.92 - 0.40 * self.share()
    }
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
    fn quality_is_kept_and_older_files_get_balanced() {
        let old: Settings = serde_json::from_str(r#"{"save_to":"downloads","hidden":[]}"#).unwrap();
        assert_eq!(old.quality, Quality::default());
        assert_eq!(old.quality.video, Level::Balanced);
        let mut s = Settings::default();
        s.quality.set("audio", Level::Best);
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains(r#""audio":"best""#), "{text}");
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.quality.get("audio"), Some(Level::Best));
        assert_eq!(Level::Smaller.pick(1, 2, 3), 1);
    }

    #[test]
    fn each_wheel_keeps_its_own_keys() {
        // Older settings files have no keys: the standard ones.
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.keys, Keys::STANDARD);
        let mut keys = Keys::STANDARD;
        assert_eq!(keys.wheel(Combo::ShiftOption), Some(Which::Tools));
        // Tools on Shift alone: the formats take Shift+Option, the keys Tools had.
        keys.assign(Which::Tools, Some(Combo::Shift));
        assert_eq!((keys.formats, keys.tools, keys.actions), (Some(Combo::ShiftOption), Some(Combo::Shift), Some(Combo::ShiftControl)));
        // Actions off: Shift+Control opens nothing, and nothing else moves.
        keys.assign(Which::Actions, None);
        assert_eq!(keys.wheel(Combo::ShiftControl), None);
        assert_eq!(keys.wheel(Combo::Shift), Some(Which::Tools));
        // Actions back on Shift alone: Tools had them and Actions had none, so Tools is off.
        keys.assign(Which::Actions, Some(Combo::Shift));
        assert_eq!((keys.tools, keys.actions), (None, Some(Combo::Shift)));
        let saved: Settings = serde_json::from_str(&serde_json::to_string(&Settings { keys, ..Settings::default() }).unwrap()).unwrap();
        assert_eq!(saved.keys, keys);
        assert_eq!(Combo::from_keys(true, true), Combo::ShiftControl);
    }

    #[test]
    fn the_shelf_is_on_unless_switched_off() {
        let old: Settings = serde_json::from_str(r#"{"save_to":"downloads","hidden":[]}"#).unwrap();
        assert!(!old.shelf_off);
        let off = Settings { shelf_off: true, ..Settings::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
        assert!(back.shelf_off);
    }

    #[test]
    fn the_arrow_keys_are_off_unless_switched_on() {
        let old: Settings = serde_json::from_str(r#"{"save_to":"downloads","hidden":[]}"#).unwrap();
        assert!(!old.arrow_keys);
        let on = Settings { arrow_keys: true, ..Settings::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&on).unwrap()).unwrap();
        assert!(back.arrow_keys);
    }

    #[test]
    fn the_highlight_is_orange_unless_chosen_and_the_page_knows_every_colour() {
        let old: Settings = serde_json::from_str(r#"{"save_to":"downloads","hidden":[]}"#).unwrap();
        assert_eq!(old.highlight, Highlight::Orange);
        let blue = Settings { highlight: Highlight::Blue, ..Settings::default() };
        let text = serde_json::to_string(&blue).unwrap();
        assert!(text.contains(r#""highlight":"blue""#), "{text}");
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap().highlight, Highlight::Blue);
        // The page accepts each colour and the stylesheet draws each one but the default.
        let js = include_str!("../../src/wheel.js");
        let css = include_str!("../../src/wheel.css");
        for h in Highlight::ALL {
            assert!(js.contains(&format!("'{}'", h.key())), "wheel.js lacks {}", h.key());
            if h != Highlight::default() {
                assert!(css.contains(&format!("[data-highlight=\"{}\"]", h.key())), "wheel.css lacks {}", h.key());
            }
        }
    }

    #[test]
    fn liquid_glass_is_on_at_half_transparency_unless_chosen() {
        // Older settings, also ones with the "glass" choice of earlier test builds.
        for old in [r#"{"save_to":"downloads","hidden":[]}"#, r#"{"glass":"medium"}"#] {
            let s: Settings = serde_json::from_str(old).unwrap();
            assert!(!s.no_liquid_glass);
            assert_eq!(s.transparency, Transparency(50));
        }
        let set = Settings { no_liquid_glass: true, transparency: Transparency(80), ..Settings::default() };
        let text = serde_json::to_string(&set).unwrap();
        assert!(text.contains(r#""transparency":80"#), "{text}");
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), set);
        // From frosted to clear: less blur and paint; never past 100.
        let (frosted, clear) = (Transparency(0), Transparency(100));
        assert_eq!((frosted.blur(), clear.blur()), (1.0, 0.0));
        assert!(frosted.paint() > Transparency(50).paint() && Transparency(50).paint() > clear.paint());
        assert_eq!(Transparency(250).blur(), 0.0);
    }

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
