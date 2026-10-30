//! Which formats the wheel offers for a dragged file.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Audio,
    Video,
    Doc,
    Sheet,
    Slides,
    Pdf,
    Subtitles,
}

impl Kind {
    /// Short name used in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Audio => "audio",
            Kind::Video => "video",
            Kind::Doc => "doc",
            Kind::Sheet => "sheet",
            Kind::Slides => "slides",
            Kind::Pdf => "pdf",
            Kind::Subtitles => "subs",
        }
    }

    /// Name shown in the menu.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Image => "Images",
            Kind::Audio => "Audio",
            Kind::Video => "Video",
            Kind::Doc => "Documents",
            Kind::Sheet => "Spreadsheets",
            Kind::Slides => "Presentations",
            Kind::Pdf => "PDFs",
            Kind::Subtitles => "Subtitles",
        }
    }
}

/// What each kind of file is read from and can be turned into.
const KINDS: &[(Kind, &[&str], &[&str])] = &[
    (
        Kind::Image,
        &["jpg", "jpeg", "png", "heic", "heif", "webp", "tiff", "tif", "bmp", "gif", "avif"],
        &["jpg", "png", "heic", "webp", "tiff", "gif", "avif", "bmp", "ico", "pdf"],
    ),
    (
        Kind::Audio,
        &["mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg", "opus", "wma"],
        &["mp3", "m4a", "wav", "flac", "aiff", "ogg", "opus", "wma"],
    ),
    (
        Kind::Video,
        &["mp4", "mov", "m4v", "mkv", "webm", "avi", "wmv"],
        &["mp4", "mov", "webm", "mkv", "avi", "wmv", "gif", "mp3"],
    ),
    (
        Kind::Doc,
        &["docx", "doc", "odt", "rtf", "pages", "md", "markdown", "html", "htm", "txt", "epub"],
        &["pdf", "docx", "odt", "html", "md", "txt", "epub"],
    ),
    // Apple's Numbers and Keynote files go in through LibreOffice, which reads them.
    (Kind::Sheet, &["xlsx", "xls", "ods", "csv", "numbers"], &["pdf", "xlsx", "ods", "csv"]),
    (Kind::Slides, &["pptx", "ppt", "odp", "key"], &["pdf", "pptx", "odp"]),
    // Pictures of the pages, or the text (as a Word file it is text only: the PDF's
    // layout isn't kept).
    (Kind::Pdf, &["pdf"], &["docx", "jpg", "png", "txt"]),
    (Kind::Subtitles, &["srt", "vtt"], &["srt", "vtt"]),
];

/// The Tools wheel: what can be done to a file while it keeps its format, by the id
/// the wheel and the converter use, with the formats each tool takes. Animated GIFs are
/// left out: a tool would have to handle every frame (One PDF takes their first).
/// Merge and One PDF need two files or more.
const TOOLS: &[(&str, &[&str])] = &[
    (
        "tool:compress",
        &[
            "jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "mp4", "mov", "m4v", "mkv", "webm", "mp3",
            "m4a", "aac", "ogg", "opus", "wma", "flac",
        ],
    ),
    ("tool:half", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "bmp"]),
    (
        "tool:clean",
        &[
            "jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "mp4", "mov", "m4v", "mkv", "webm", "avi",
            "mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg", "opus",
        ],
    ),
    ("tool:rotate", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "bmp"]),
    ("tool:qr", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "bmp", "gif"]),
    ("tool:adjust", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "bmp"]),
    (
        "tool:crop",
        &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "bmp", "mp4", "mov", "m4v", "mkv", "webm", "avi", "wmv"],
    ),
    (
        "tool:trim",
        &[
            "mp4", "mov", "m4v", "mkv", "webm", "avi", "wmv", "mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg",
            "opus", "wma",
        ],
    ),
    (
        "tool:bleep",
        &[
            "mp4", "mov", "m4v", "mkv", "webm", "avi", "wmv", "mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg",
            "opus", "wma",
        ],
    ),
    (
        "tool:speed",
        &[
            "mp4", "mov", "m4v", "mkv", "webm", "avi", "wmv", "mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg",
            "opus", "wma",
        ],
    ),
    (
        "tool:parts",
        &[
            "mp4", "mov", "m4v", "mkv", "webm", "avi", "wmv", "mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg",
            "opus", "wma",
        ],
    ),
    ("tool:onepdf", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "bmp", "gif"]),
    ("tool:collage", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "bmp", "gif"]),
    ("tool:merge", &["pdf"]),
    ("tool:split", &["pdf"]),
    ("tool:smaller", &["pdf"]),
    ("tool:still", &["mp4", "mov", "m4v", "mkv", "webm", "avi"]),
    (
        "tool:target",
        &[
            "jpg", "jpeg", "heic", "heif", "webp", "avif", "mp4", "mov", "m4v", "mkv", "webm", "mp3", "m4a", "aac", "ogg",
            "opus", "wma",
        ],
    ),
    (
        "tool:join",
        &[
            "mp4", "mov", "m4v", "mkv", "webm", "avi", "wmv", "mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg",
            "opus", "wma",
        ],
    ),
    ("tool:loudness", &["mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg", "opus"]),
    ("tool:mono", &["mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg", "opus"]),
];

/// A tool that turns the whole drop into one file, instead of working on each file.
pub fn combines(id: &str) -> bool {
    matches!(id, "tool:merge" | "tool:onepdf" | "tool:collage" | "tool:join" | "act:zip" | "act:tgz" | "act:tar")
}

/// A Tools wheel id ("tool:half") rather than a format.
pub fn is_tool(id: &str) -> bool {
    id.starts_with("tool:")
}

/// Whether `tool` works on .`ext` files.
pub fn tool_takes(tool: &str, ext: &str) -> bool {
    TOOLS.iter().any(|(id, ins)| *id == tool && ins.contains(&ext))
}

/// Tools that work on every dragged file, in wheel order.
pub fn tools_for(paths: &[String]) -> Vec<&'static str> {
    let exts: Vec<String> = paths.iter().filter_map(|p| extension(Path::new(p))).collect();
    if exts.is_empty() || exts.len() != paths.len() {
        return Vec::new();
    }
    TOOLS
        .iter()
        .filter(|(id, ins)| {
            exts.iter().all(|e| ins.contains(&e.as_str()))
                && (!combines(id) || paths.len() > 1)
                && (!opens_editor(id) || paths.len() == 1)
                // Join takes sound files or video files, not the two together.
                && (*id != "tool:join" || exts.iter().all(|e| kind_of(e) == kind_of(&exts[0])))
        })
        .map(|(id, _)| *id)
        .take(MAX_SEGMENTS)
        .collect()
}

/// A tool that first opens the editing window (one file at a time) instead of running.
pub fn opens_editor(id: &str) -> bool {
    matches!(id, "tool:crop" | "tool:adjust" | "tool:trim" | "tool:bleep" | "tool:speed" | "tool:parts" | "tool:target")
}

/// The Actions wheel: what can be done with the dragged files themselves, whatever
/// they are. AirDrop is macOS only; Unpack only when every file is an archive.
const ACTIONS: &[&str] = &[
    "act:path", "act:show", "act:copyto", "act:moveto", "act:terminal", "act:airdrop", "act:zip", "act:tgz", "act:tar", "act:unpack",
];

/// An Actions wheel id ("act:path").
pub fn is_action(id: &str) -> bool {
    id.starts_with("act:")
}

/// The actions for the dragged files, in wheel order.
pub fn actions_for(paths: &[String]) -> Vec<&'static str> {
    if paths.is_empty() {
        return Vec::new();
    }
    let archives = paths.iter().all(|p| crate::archive::is_archive(p));
    ACTIONS
        .iter()
        .copied()
        .filter(|a| match *a {
            "act:airdrop" => cfg!(target_os = "macos"),
            "act:unpack" => archives,
            _ => true,
        })
        .collect()
}

/// Every kind with the formats it can become, in menu order.
pub fn all() -> impl Iterator<Item = (Kind, &'static [&'static str])> {
    KINDS.iter().map(|(kind, _, outs)| (*kind, *outs))
}

/// Most segments the wheel draws, so labels stay readable. Pictures need all ten.
pub const MAX_SEGMENTS: usize = 10;

pub fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

pub fn kind_of(ext: &str) -> Option<Kind> {
    KINDS.iter().find(|(_, ins, _)| ins.contains(&ext)).map(|(k, _, _)| *k)
}

pub fn outputs_for(ext: &str) -> &'static [&'static str] {
    KINDS
        .iter()
        .find(|(_, ins, _)| ins.contains(&ext))
        .map(|(_, _, outs)| *outs)
        .unwrap_or(&[])
}

/// Same name for the same format, so "jpeg" in and "jpg" out don't both show.
pub fn canonical(ext: &str) -> &str {
    match ext {
        "jpeg" => "jpg",
        "tif" => "tiff",
        "heif" => "heic",
        "aif" => "aiff",
        "htm" => "html",
        "markdown" => "md",
        other => other,
    }
}

/// Formats every dragged file can be turned into and the user has left on the wheel,
/// minus the format they already have when they all share one.
pub fn formats_for(paths: &[String], shows: impl Fn(Kind, &str) -> bool) -> Vec<&'static str> {
    let exts: Vec<String> = paths.iter().filter_map(|p| extension(Path::new(p))).collect();
    if exts.is_empty() || exts.len() != paths.len() {
        return Vec::new();
    }
    let mut common: Vec<&'static str> = outputs_for(&exts[0]).to_vec();
    for ext in &exts {
        let outs = outputs_for(ext);
        let kind = kind_of(ext);
        common.retain(|f| outs.contains(f) && kind.is_some_and(|k| shows(k, f)));
    }
    let first = canonical(&exts[0]);
    if exts.iter().all(|e| canonical(e) == first) {
        common.retain(|f| *f != first);
    }
    common.truncate(MAX_SEGMENTS);
    common
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    fn all_shown(_: Kind, _: &str) -> bool {
        true
    }

    #[test]
    fn hidden_formats_leave_the_wheel() {
        let f = formats_for(&v(&["/tmp/a.png"]), |k, f| !(k == Kind::Image && f == "avif"));
        assert!(!f.contains(&"avif") && f.contains(&"jpg"));
        // hiding PDF for documents only still leaves it for spreadsheets
        let f = formats_for(&v(&["/tmp/a.xlsx"]), |k, f| !(k == Kind::Doc && f == "pdf"));
        assert!(f.contains(&"pdf"));
    }

    #[test]
    fn image_drops_its_own_format() {
        let f = formats_for(&v(&["/tmp/a.JPEG"]), all_shown);
        assert!(!f.contains(&"jpg"));
        assert!(f.contains(&"png"));
    }

    #[test]
    fn mixed_kinds_share_only_common_formats() {
        // a video and an audio file only share mp3
        assert_eq!(formats_for(&v(&["/tmp/a.mov", "/tmp/b.wav"]), all_shown), vec!["mp3"]);
        // office files of any kind can all become a PDF
        assert_eq!(formats_for(&v(&["/tmp/a.docx", "/tmp/b.xlsx", "/tmp/c.pptx"]), all_shown), vec!["pdf"]);
    }

    #[test]
    fn unknown_or_missing_extension_gives_nothing() {
        assert!(formats_for(&v(&["/tmp/a.xyz"]), all_shown).is_empty());
        assert!(formats_for(&v(&["/tmp/Makefile"]), all_shown).is_empty());
        assert!(formats_for(&v(&["/tmp/a.png", "/tmp/Makefile"]), all_shown).is_empty());
    }

    #[test]
    fn mixed_image_formats_keep_all_outputs() {
        let f = formats_for(&v(&["/tmp/a.png", "/tmp/b.jpg"]), all_shown);
        assert!(f.contains(&"png") && f.contains(&"jpg"));
    }

    #[test]
    fn new_formats_are_offered() {
        assert!(formats_for(&v(&["/tmp/a.png"]), all_shown).ends_with(&["bmp", "ico", "pdf"]));
        assert!(formats_for(&v(&["/tmp/a.mov"]), all_shown).contains(&"wmv"));
        assert_eq!(formats_for(&v(&["/tmp/a.srt"]), all_shown), ["vtt"]);
        assert!(formats_for(&v(&["/tmp/a.pages"]), all_shown).contains(&"docx"));
        assert_eq!(formats_for(&v(&["/tmp/a.key", "/tmp/b.numbers"]), all_shown), ["pdf"]);
    }

    #[test]
    fn word_files_offer_pdf() {
        assert!(formats_for(&v(&["/tmp/a.docx"]), all_shown).contains(&"pdf"));
        assert!(formats_for(&v(&["/tmp/a.doc"]), all_shown).contains(&"docx"));
    }

    #[test]
    fn tools_are_offered_when_every_file_takes_them() {
        let one = ["tool:compress", "tool:half", "tool:clean", "tool:rotate", "tool:qr", "tool:adjust", "tool:crop", "tool:target"];
        assert_eq!(tools_for(&v(&["/tmp/a.JPG"])), one);
        let two = ["tool:compress", "tool:half", "tool:clean", "tool:rotate", "tool:qr", "tool:onepdf", "tool:collage"];
        assert_eq!(tools_for(&v(&["/tmp/a.JPG", "/tmp/b.heic"])), two);
        // BMP has no metadata to remove and nothing to compress it with.
        // Crop opens a window for one picture: not offered for two.
        assert_eq!(
            tools_for(&v(&["/tmp/a.png", "/tmp/b.bmp"])),
            ["tool:half", "tool:rotate", "tool:qr", "tool:onepdf", "tool:collage"]
        );
        assert_eq!(tools_for(&v(&["/tmp/a.bmp"])), ["tool:half", "tool:rotate", "tool:qr", "tool:adjust", "tool:crop"]);
        // An animated GIF's first frame can still hold a QR code.
        assert_eq!(tools_for(&v(&["/tmp/a.gif"])), ["tool:qr"]);
        assert_eq!(tools_for(&v(&["/tmp/a.gif", "/tmp/b.jpg"])), ["tool:qr", "tool:onepdf", "tool:collage"]);
        // Trim opens a window for one file.
        assert_eq!(
            tools_for(&v(&["/tmp/a.MOV"])),
            ["tool:compress", "tool:clean", "tool:crop", "tool:trim", "tool:bleep", "tool:speed", "tool:parts", "tool:still", "tool:target"]
        );
        assert_eq!(
            tools_for(&v(&["/tmp/a.avi"])),
            ["tool:clean", "tool:crop", "tool:trim", "tool:bleep", "tool:speed", "tool:parts", "tool:still"]
        );
        assert_eq!(
            tools_for(&v(&["/tmp/a.mp3"])),
            ["tool:compress", "tool:clean", "tool:trim", "tool:bleep", "tool:speed", "tool:parts", "tool:target", "tool:loudness", "tool:mono"]
        );
        assert_eq!(tools_for(&v(&["/tmp/a.mp3", "/tmp/b.flac"])), ["tool:compress", "tool:clean", "tool:join", "tool:loudness", "tool:mono"]);
        // WAV and AIFF are uncompressed by design: nothing to compress them with.
        assert_eq!(
            tools_for(&v(&["/tmp/a.wav"])),
            ["tool:clean", "tool:trim", "tool:bleep", "tool:speed", "tool:parts", "tool:loudness", "tool:mono"]
        );
        assert_eq!(tools_for(&v(&["/tmp/a.mp4", "/tmp/b.MOV"])), ["tool:compress", "tool:clean", "tool:still", "tool:join"]);
        // Sound and video together: each is compressed or cleaned in its own way, but
        // Join takes one kind only.
        assert_eq!(tools_for(&v(&["/tmp/a.mp3", "/tmp/b.mp4"])), ["tool:compress", "tool:clean"]);
        assert!(tools_for(&v(&["/tmp/a.png", "/tmp/b.pdf"])).is_empty());
        // Merge only once there is something to merge.
        assert_eq!(tools_for(&v(&["/tmp/a.pdf"])), ["tool:split", "tool:smaller"]);
        assert_eq!(tools_for(&v(&["/tmp/a.pdf", "/tmp/b.PDF"])), ["tool:merge", "tool:split", "tool:smaller"]);
        assert!(tools_for(&v(&["/tmp/Makefile"])).is_empty());
        assert!(tool_takes("tool:half", "jpeg") && !tool_takes("tool:half", "pdf"));
        assert!(TOOLS.iter().all(|(id, _)| is_tool(id)));
        // No kind of file gets more tools than the wheel draws, even with Merge or One PDF.
        for (_, ins, _) in KINDS {
            for ext in *ins {
                let two = v(&[&format!("/tmp/a.{ext}"), &format!("/tmp/b.{ext}")]);
                assert!(TOOLS.iter().filter(|(id, _)| tool_takes(id, ext)).count() <= MAX_SEGMENTS, "{ext}");
                assert!(tools_for(&two).len() <= MAX_SEGMENTS);
            }
        }
    }

    #[test]
    fn actions_are_offered_for_any_file() {
        assert!(actions_for(&[]).is_empty());
        let actions = actions_for(&v(&["/tmp/a.txt", "/tmp/Photos"]));
        assert_eq!(actions[..5], ["act:path", "act:show", "act:copyto", "act:moveto", "act:terminal"]);
        assert_eq!(actions.contains(&"act:airdrop"), cfg!(target_os = "macos"));
        assert!(actions.ends_with(&["act:zip", "act:tgz", "act:tar"]));
        let archives = actions_for(&v(&["/tmp/a.zip", "/tmp/b.tar.gz"]));
        assert!(archives.ends_with(&["act:tar", "act:unpack"]));
        // The most an Actions wheel holds (archives, on a Mac) still fits.
        assert!(archives.len() <= MAX_SEGMENTS);
        assert!(actions.len() <= MAX_SEGMENTS && actions.iter().all(|a| is_action(a) && !is_tool(a)));
    }

    #[test]
    fn no_kind_offers_more_than_the_wheel_draws() {
        for (_, _, outs) in KINDS {
            assert!(outs.len() <= MAX_SEGMENTS);
        }
    }

    #[test]
    fn the_page_names_and_draws_every_tool_and_action() {
        let words = include_str!("../../src/wheel.js");
        let icons = include_str!("../../src/icons.js");
        for id in TOOLS.iter().map(|(id, _)| *id).chain(ACTIONS.iter().copied()) {
            assert!(words.contains(&format!("'{id}': [")), "wheel.js has no words for {id}");
            assert!(icons.contains(&format!("\"{id}\": ")), "icons.js has no icon for {id}");
        }
    }
}
