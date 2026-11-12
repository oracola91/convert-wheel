//! Windows: notifications whose click also works later, from the notification list.
//!
//! The notification plugin only hears a click while the notification is on screen:
//! once it has moved to the list (Action Center), the click went nowhere. Here the
//! notification carries a link instead, `pirouette:show?<start>-<id>`: Windows opens
//! the link with this program, the running copy gets it (single instance) and shows
//! the files that notification reported. The link names a notification, never a file,
//! so a link from anywhere else can at most show a recent result again; one from an
//! earlier start of the app (another `<start>`) does nothing.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

/// The link's scheme. The uninstaller removes its registration (windows/hooks.nsh).
const SCHEME: &str = "pirouette";
const KEY: PCWSTR = w!("Software\\Classes\\pirouette");
const COMMAND_KEY: PCWSTR = w!("Software\\Classes\\pirouette\\shell\\open\\command");

/// This start of the app, so a link from an earlier one is told apart.
fn start() -> u64 {
    static START: OnceLock<u64> = OnceLock::new();
    *START.get_or_init(|| SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

fn link(start: u64, id: i32) -> String {
    format!("{SCHEME}:show?{start}-{id}")
}

/// Whether `arg` is one of these links, from this start or an earlier one.
pub fn is_link(arg: &str) -> bool {
    arg.get(..SCHEME.len() + 1).is_some_and(|s| s.eq_ignore_ascii_case(&format!("{SCHEME}:")))
}

/// The notification a link names, if the link came from this start of the app.
pub fn clicked(arg: &str) -> Option<i32> {
    parse(arg, start())
}

fn parse(arg: &str, start: u64) -> Option<i32> {
    if !is_link(arg) {
        return None;
    }
    let rest = arg[SCHEME.len() + 1..].strip_prefix("show?")?.trim_end_matches('/');
    let (from, id) = rest.split_once('-')?;
    let from: u64 = from.parse().ok()?;
    let id: i32 = id.parse().ok()?;
    (from == start && id > 0).then_some(id)
}

/// Shows a notification under the app's own name (the id the installers give its
/// Start menu entry), with the link for a click.
pub fn show(app_id: &str, id: i32, title: &str, body: &str) -> windows::core::Result<()> {
    let xml = format!(
        r#"<toast launch="{}" activationType="protocol"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
        escape(&link(start(), id)),
        escape(title),
        escape(body)
    );
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?.Show(&toast)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Makes Windows open the links with this copy of the app (again on each start, so it
/// follows an update to another folder). Only the user's own part of the registry.
pub fn register() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let command = format!("\"{}\" \"%1\"", exe.display());
    let set = |key: PCWSTR, name: PCWSTR, value: &str| {
        let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
        // SAFETY: a NUL-terminated UTF-16 string, its length in bytes.
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key, name, REG_SZ.0, Some(data.as_ptr().cast()), (data.len() * 2) as u32) }.ok()
    };
    let results = [
        set(KEY, PCWSTR::null(), &format!("URL:{} notification", crate::APP_NAME)),
        set(KEY, w!("URL Protocol"), ""),
        set(COMMAND_KEY, PCWSTR::null(), &command),
    ];
    if let Some(Err(e)) = results.into_iter().find(Result::is_err) {
        crate::log::line(&format!("couldn't register the notification link: {}", e.message()));
    }
}

/// Whether this copy runs from a build folder: it has no Start menu entry, and
/// Windows drops notifications under an app id nothing registered.
pub fn from_build_folder() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.to_string_lossy().to_ascii_lowercase()))
        .is_some_and(|d| d.ends_with("\\target\\debug") || d.ends_with("\\target\\release"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_from_this_start_names_its_notification() {
        assert_eq!(parse(&link(77, 12), 77), Some(12));
        assert_eq!(parse("pirouette:show?77-12/", 77), Some(12));
        assert_eq!(parse("PIROUETTE:show?77-12", 77), Some(12));
    }

    #[test]
    fn other_links_name_nothing() {
        assert_eq!(parse(&link(76, 12), 77), None);
        assert_eq!(parse("pirouette:show?77-0", 77), None);
        assert_eq!(parse("pirouette:show?77-C:\\x.jpg", 77), None);
        assert_eq!(parse("pirouette:open?77-12", 77), None);
        assert_eq!(parse("C:\\photo.jpg", 77), None);
        assert!(is_link("pirouette:anything"));
        assert!(!is_link("pir"));
    }

    #[test]
    fn text_is_escaped_for_the_notification() {
        assert_eq!(escape(r#"a<b>&"c"'"#), "a&lt;b&gt;&amp;&quot;c&quot;&apos;");
    }
}
