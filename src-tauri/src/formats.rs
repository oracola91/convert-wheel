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
        }
    }
}

/// What each kind of file is read from and can be turned into.
const KINDS: &[(Kind, &[&str], &[&str])] = &[
    (
        Kind::Image,
        &["jpg", "jpeg", "png", "heic", "heif", "webp", "tiff", "tif", "bmp", "gif", "avif"],
        &["jpg", "png", "heic", "webp", "tiff", "gif", "avif", "pdf"],
    ),
    (
        Kind::Audio,
        &["mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg", "opus"],
        &["mp3", "m4a", "wav", "flac", "aiff", "ogg"],
    ),
    (
        Kind::Video,
        &["mp4", "mov", "m4v", "mkv", "webm", "avi"],
        &["mp4", "mov", "webm", "gif", "mp3"],
    ),
    (
        Kind::Doc,
        &["docx", "doc", "odt", "rtf", "md", "markdown", "html", "htm", "txt", "epub"],
        &["pdf", "docx", "odt", "html", "md", "txt", "epub"],
    ),
    (Kind::Sheet, &["xlsx", "xls", "ods", "csv"], &["pdf", "xlsx", "ods", "csv"]),
    (Kind::Slides, &["pptx", "ppt", "odp"], &["pdf", "pptx", "odp"]),
    // Pictures of the pages, or the text (as a Word file it is text only: the PDF's
    // layout isn't kept).
    (Kind::Pdf, &["pdf"], &["docx", "jpg", "png", "txt"]),
];

/// The Tools wheel: what can be done to a file while it keeps its format, by the id
/// the wheel and the converter use, with the formats each tool takes. Animated GIFs are
/// left out: a tool would have to handle every frame (One PDF takes their first).
/// Merge and One PDF need two files or more.
const TOOLS: &[(&str, &[&str])] = &[
    ("tool:compress", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif"]),
    ("tool:half", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "bmp"]),
    ("tool:clean", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif"]),
    ("tool:rotate", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "bmp"]),
    ("tool:onepdf", &["jpg", "jpeg", "png", "heic", "heif", "webp", "avif", "tiff", "tif", "bmp", "gif"]),
    ("tool:merge", &["pdf"]),
    ("tool:split", &["pdf"]),
    ("tool:smaller", &["pdf"]),
];

/// A tool that turns the whole drop into one file, instead of working on each file.
pub fn combines(id: &str) -> bool {
    matches!(id, "tool:merge" | "tool:onepdf")
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
        .filter(|(id, ins)| exts.iter().all(|e| ins.contains(&e.as_str())) && (!combines(id) || paths.len() > 1))
        .map(|(id, _)| *id)
        .collect()
}

/// Every kind with the formats it can become, in menu order.
pub fn all() -> impl Iterator<Item = (Kind, &'static [&'static str])> {
    KINDS.iter().map(|(kind, _, outs)| (*kind, *outs))
}

/// Most segments the wheel draws, so labels stay readable.
pub const MAX_SEGMENTS: usize = 8;

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
    fn word_files_offer_pdf() {
        assert!(formats_for(&v(&["/tmp/a.docx"]), all_shown).contains(&"pdf"));
        assert!(formats_for(&v(&["/tmp/a.doc"]), all_shown).contains(&"docx"));
    }

    #[test]
    fn tools_are_offered_when_every_file_takes_them() {
        let one = ["tool:compress", "tool:half", "tool:clean", "tool:rotate"];
        assert_eq!(tools_for(&v(&["/tmp/a.JPG"])), one);
        let two = ["tool:compress", "tool:half", "tool:clean", "tool:rotate", "tool:onepdf"];
        assert_eq!(tools_for(&v(&["/tmp/a.JPG", "/tmp/b.heic"])), two);
        // BMP has no metadata to remove and nothing to compress it with.
        assert_eq!(tools_for(&v(&["/tmp/a.png", "/tmp/b.bmp"])), ["tool:half", "tool:rotate", "tool:onepdf"]);
        assert!(tools_for(&v(&["/tmp/a.gif"])).is_empty());
        assert_eq!(tools_for(&v(&["/tmp/a.gif", "/tmp/b.jpg"])), ["tool:onepdf"]);
        assert!(tools_for(&v(&["/tmp/a.png", "/tmp/b.pdf"])).is_empty());
        // Merge only once there is something to merge.
        assert_eq!(tools_for(&v(&["/tmp/a.pdf"])), ["tool:split", "tool:smaller"]);
        assert_eq!(tools_for(&v(&["/tmp/a.pdf", "/tmp/b.PDF"])), ["tool:merge", "tool:split", "tool:smaller"]);
        assert!(tools_for(&v(&["/tmp/Makefile"])).is_empty());
        assert!(tool_takes("tool:half", "jpeg") && !tool_takes("tool:half", "pdf"));
        assert!(TOOLS.len() <= MAX_SEGMENTS && TOOLS.iter().all(|(id, _)| is_tool(id)));
    }

    #[test]
    fn no_kind_offers_more_than_the_wheel_draws() {
        for (_, _, outs) in KINDS {
            assert!(outs.len() <= MAX_SEGMENTS);
        }
    }
}
