//! The Settings window (menu > Settings…): what the menu used to hold, in tabs
//! (General, Wheel, Quality, About), the same on every system. The page asks for the
//! current state with `settings_view` and sends each change with `settings_change`,
//! which checks it, saves it, applies it and answers with the new state.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::settings::{self, Combo, Highlight, Level, Quality, SaveTo, Store, Transparency};
use crate::wheel::Which;
use crate::{converters, formats, log, login, reveal};

const LABEL: &str = "settings";

/// The wheels, as the page and the settings name them.
const WHEELS: [(Which, &str, &str); 3] =
    [(Which::Formats, "formats", "Formats wheel"), (Which::Tools, "tools", "Tools wheel"), (Which::Actions, "actions", "Actions wheel")];

/// The Highlight colours as the page paints its swatches (the wheel's own shades).
const SWATCHES: [(&str, &str); 8] = [
    ("orange", "#f26a2c"),
    ("red", "#f2554f"),
    ("pink", "#f5639c"),
    ("purple", "#ab73f2"),
    ("blue", "#3f8ff5"),
    ("teal", "#32bdc2"),
    ("green", "#48c55c"),
    ("graphite", "#8e8e93"),
];

#[derive(Serialize)]
struct Toggle {
    available: bool,
    on: bool,
}

#[derive(Serialize)]
struct Choice {
    key: &'static str,
    label: &'static str,
}

#[derive(Serialize)]
struct FormatView {
    key: String,
    label: String,
    shown: bool,
}

#[derive(Serialize)]
struct KindView {
    label: &'static str,
    formats: Vec<FormatView>,
}

#[derive(Serialize)]
struct WheelKeys {
    wheel: &'static str,
    label: &'static str,
    combo: Option<&'static str>,
}

#[derive(Serialize)]
struct ColorView {
    key: &'static str,
    label: &'static str,
    color: &'static str,
}

#[derive(Serialize)]
struct QualityView {
    kind: &'static str,
    label: &'static str,
    level: &'static str,
}

/// Everything the page shows, as it is now.
#[derive(Serialize)]
pub struct View {
    app: &'static str,
    version: String,
    /// "macos", "windows" or "linux": the page names keys and glass the system's way.
    system: &'static str,
    login: Toggle,
    save_to: SaveTo,
    /// None where there is no shelf (Wayland).
    shelf: Option<bool>,
    /// "Update Converters…" works here.
    converters: bool,
    kinds: Vec<KindView>,
    keys: Vec<WheelKeys>,
    combos: Vec<Choice>,
    /// macOS only: the arrow keys switch (Input Monitoring). None elsewhere: always on.
    arrows: Option<bool>,
    highlight: &'static str,
    colors: Vec<ColorView>,
    /// Liquid Glass (macOS) or See-through (Windows, Linux).
    glass: bool,
    transparency: u8,
    quality: Vec<QualityView>,
    levels: Vec<Choice>,
}

/// One change from the page.
#[derive(Debug, Deserialize)]
#[serde(tag = "what", rename_all = "kebab-case")]
pub enum Change {
    Login { on: bool },
    SaveTo { to: SaveTo },
    Shelf { on: bool },
    Format { key: String, shown: bool },
    Keys { wheel: String, combo: Option<String> },
    Arrows { on: bool },
    Highlight { color: String },
    Glass { on: bool },
    Transparency { value: u8 },
    Quality { kind: String, level: String },
}

fn system() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// Every system with a shelf: not Wayland, which places no window at the top.
fn shelf_here() -> bool {
    #[cfg(target_os = "linux")]
    return !crate::linux::is_wayland();
    #[cfg(not(target_os = "linux"))]
    true
}

fn view(app: &AppHandle) -> View {
    let s = app.state::<Store>().get();
    let kinds = formats::all()
        .map(|(kind, outputs)| KindView {
            label: kind.label(),
            formats: outputs
                .iter()
                .map(|f| FormatView { key: settings::key(kind, f), label: f.to_uppercase(), shown: s.shows(kind, f) })
                .collect(),
        })
        .collect();
    View {
        app: crate::APP_NAME,
        version: app.package_info().version.to_string(),
        system: system(),
        login: Toggle { available: login::available(), on: login::is_enabled() },
        save_to: s.save_to,
        shelf: shelf_here().then_some(!s.shelf_off),
        converters: converters::available(),
        kinds,
        keys: WHEELS.iter().map(|(which, wheel, label)| WheelKeys { wheel, label, combo: s.keys.get(*which).map(Combo::key) }).collect(),
        combos: Combo::ALL.iter().map(|c| Choice { key: c.key(), label: c.label() }).collect(),
        arrows: cfg!(target_os = "macos").then_some(s.arrow_keys),
        highlight: s.highlight.key(),
        colors: Highlight::ALL
            .iter()
            .map(|h| ColorView {
                key: h.key(),
                label: h.label(),
                color: SWATCHES.iter().find(|(k, _)| *k == h.key()).map_or("#8e8e93", |(_, c)| *c),
            })
            .collect(),
        glass: !s.no_liquid_glass,
        transparency: s.transparency.0.min(100),
        quality: Quality::KINDS
            .iter()
            .map(|(kind, label)| QualityView { kind, label, level: s.quality.get(kind).unwrap_or_default().key() })
            .collect(),
        levels: Level::ALL.iter().map(|l| Choice { key: l.key(), label: l.label() }).collect(),
    }
}

/// A format key the wheel really has, such as "image:avif".
fn known_format(key: &str) -> bool {
    formats::all().any(|(kind, outputs)| outputs.iter().any(|f| settings::key(kind, f) == key))
}

/// What a change does to the saved settings.
type Edit = Box<dyn FnOnce(&mut settings::Settings)>;

/// Checks a change and turns it into what to save. Open at Login and the side effects
/// of a change are done by `apply`; this part only reads, so it can be tested.
fn edit(change: &Change) -> Result<Edit, String> {
    Ok(match change {
        Change::SaveTo { to } => {
            let to = *to;
            Box::new(move |s| s.save_to = to)
        }
        Change::Shelf { on } => {
            let off = !on;
            Box::new(move |s| s.shelf_off = off)
        }
        Change::Format { key, shown } => {
            if !known_format(key) {
                return Err(format!("no format {key} on the wheel"));
            }
            let (key, shown) = (key.clone(), *shown);
            Box::new(move |s| {
                if shown {
                    s.hidden.remove(&key);
                } else {
                    s.hidden.insert(key);
                }
            })
        }
        Change::Keys { wheel, combo } => {
            let which = WHEELS.iter().find(|w| w.1 == wheel).map(|w| w.0).ok_or_else(|| format!("no wheel {wheel}"))?;
            let combo = match combo {
                None => None,
                Some(c) => Some(Combo::ALL.into_iter().find(|k| k.key() == c).ok_or_else(|| format!("no keys {c}"))?),
            };
            // Keys another wheel had swap over to it, so no two wheels share keys.
            Box::new(move |s| s.keys.assign(which, combo))
        }
        Change::Arrows { on } => {
            if !cfg!(target_os = "macos") {
                return Err("the arrow keys are always on here".into());
            }
            let on = *on;
            Box::new(move |s| s.arrow_keys = on)
        }
        Change::Highlight { color } => {
            let color = Highlight::ALL.into_iter().find(|h| h.key() == color).ok_or_else(|| format!("no colour {color}"))?;
            Box::new(move |s| s.highlight = color)
        }
        Change::Glass { on } => {
            let plain = !on;
            Box::new(move |s| s.no_liquid_glass = plain)
        }
        Change::Transparency { value } => {
            let value = (*value).min(100);
            Box::new(move |s| s.transparency = Transparency(value))
        }
        Change::Quality { kind, level } => {
            let kind = Quality::KINDS.iter().find(|(k, _)| k == kind).map(|(k, _)| *k).ok_or_else(|| format!("no kind {kind}"))?;
            let level = Level::ALL.into_iter().find(|l| l.key() == level).ok_or_else(|| format!("no level {level}"))?;
            Box::new(move |s| s.quality.set(kind, level))
        }
        Change::Login { .. } => Box::new(|_| {}),
    })
}

/// Saves a change and does what it takes to show: the shelf comes or goes, the wheel
/// keys and the arrow keys take effect, Open at Login is set with the system.
fn apply(app: &AppHandle, change: Change) -> Result<(), String> {
    if let Change::Login { on } = change {
        return login::set_enabled(on).map_err(|e| format!("Couldn't change Open at Login: {e}"));
    }
    let store = app.state::<Store>();
    let saved = store.update(edit(&change)?).map_err(|e| format!("Couldn't save the setting: {e}"))?;
    match change {
        Change::Shelf { .. } => {
            #[cfg(target_os = "macos")]
            crate::shelf_window::setting_changed(app);
            #[cfg(not(target_os = "macos"))]
            crate::shelf_web::setting_changed(app);
        }
        Change::Keys { .. } => crate::wheel::set_keys(saved.keys),
        #[cfg(target_os = "macos")]
        Change::Arrows { on } => {
            crate::trigger::set_mac_arrows(on);
            // Asked here and never by the wheel: macOS shows its dialog the first time,
            // after that the switch only lives in System Settings.
            if on && !crate::trigger::ask_input_monitoring() {
                crate::notify(
                    app,
                    "Arrow keys need Input Monitoring",
                    "Allow the app in System Settings > Privacy & Security > Input Monitoring. Until then the wheel can't see the arrow keys while another app is in front.",
                );
            }
        }
        _ => {}
    }
    Ok(())
}

#[tauri::command]
pub fn settings_view(app: AppHandle) -> View {
    view(&app)
}

#[tauri::command]
pub fn settings_change(app: AppHandle, change: Change) -> Result<View, String> {
    if let Err(e) = apply(&app, change) {
        log::line(&format!("settings: {e}"));
        return Err(e);
    }
    Ok(view(&app))
}

/// The window's buttons: "log" (Show Log), "update-converters", and "close" (Esc).
#[tauri::command]
pub fn settings_action(app: AppHandle, action: String) -> Result<(), String> {
    match action.as_str() {
        "log" => {
            let file = log::path().ok_or("no log yet")?;
            reveal::show(&app, vec![file]);
            Ok(())
        }
        "update-converters" if converters::available() => {
            converters::update(&app);
            Ok(())
        }
        "close" => {
            if let Some(window) = app.get_webview_window(LABEL) {
                let _ = window.close();
            }
            Ok(())
        }
        _ => Err(format!("no action {action}")),
    }
}

/// Tells an open Settings window that "Update Converters…" is running or done, so
/// its button waits meanwhile.
pub fn converters_running(app: &AppHandle, running: bool) {
    let _ = app.emit_to(LABEL, "converters-updating", running);
}

/// Opens the Settings window, or brings the open one to the front.
pub fn open(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || show(&handle));
}

fn show(app: &AppHandle) {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => match WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("settings.html".into()))
            .title(format!("{} Settings", crate::APP_NAME))
            .inner_size(600.0, 560.0)
            .min_inner_size(520.0, 440.0)
            .center()
            .build()
        {
            Ok(window) => window,
            Err(e) => {
                log::line(&format!("couldn't open Settings: {e}"));
                crate::notify(app, "Couldn't open Settings", &e.to_string());
                return;
            }
        },
    };
    let _ = window.show();
    // A menu bar app is never the active one; without this the window opens behind.
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        #[allow(deprecated)]
        objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
    let _ = window.set_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed(change: Change) -> settings::Settings {
        let mut s = settings::Settings::default();
        edit(&change).unwrap()(&mut s);
        s
    }

    #[test]
    fn the_page_sends_changes_by_name() {
        let c: Change = serde_json::from_str(r#"{"what":"save-to","to":"downloads"}"#).unwrap();
        assert_eq!(changed(c).save_to, SaveTo::Downloads);
        let c: Change = serde_json::from_str(r#"{"what":"keys","wheel":"tools","combo":null}"#).unwrap();
        assert_eq!(changed(c).keys.tools, None);
        let c: Change = serde_json::from_str(r#"{"what":"quality","kind":"video","level":"best"}"#).unwrap();
        assert_eq!(changed(c).quality.video, Level::Best);
        let c: Change = serde_json::from_str(r#"{"what":"transparency","value":250}"#).unwrap();
        assert_eq!(changed(c).transparency, Transparency(100));
    }

    #[test]
    fn a_format_goes_off_the_wheel_and_back() {
        let off = changed(Change::Format { key: "image:avif".into(), shown: false });
        assert!(off.hidden.contains("image:avif"));
        let mut s = off;
        edit(&Change::Format { key: "image:avif".into(), shown: true }).unwrap()(&mut s);
        assert!(s.hidden.is_empty());
    }

    #[test]
    fn unknown_names_are_refused() {
        assert!(edit(&Change::Format { key: "image:exe".into(), shown: false }).is_err());
        assert!(edit(&Change::Keys { wheel: "nope".into(), combo: None }).is_err());
        assert!(edit(&Change::Keys { wheel: "tools".into(), combo: Some("shift-cmd".into()) }).is_err());
        assert!(edit(&Change::Highlight { color: "gold".into() }).is_err());
        assert!(edit(&Change::Quality { kind: "fonts".into(), level: "best".into() }).is_err());
        assert_eq!(edit(&Change::Arrows { on: true }).is_ok(), cfg!(target_os = "macos"));
    }

    #[test]
    fn every_highlight_has_a_swatch() {
        for h in Highlight::ALL {
            assert!(SWATCHES.iter().any(|(k, _)| *k == h.key()), "{}", h.key());
        }
    }
}
