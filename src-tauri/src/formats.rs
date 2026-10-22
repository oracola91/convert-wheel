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
];

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

/// Formats every dragged file can be turned into, minus the format they already have
/// when they all share one.
pub fn formats_for(paths: &[String]) -> Vec<&'static str> {
    let exts: Vec<String> = paths.iter().filter_map(|p| extension(Path::new(p))).collect();
    if exts.is_empty() || exts.len() != paths.len() {
        return Vec::new();
    }
    let mut common: Vec<&'static str> = outputs_for(&exts[0]).to_vec();
    for ext in &exts[1..] {
        let outs = outputs_for(ext);
        common.retain(|f| outs.contains(f));
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

    #[test]
    fn image_drops_its_own_format() {
        let f = formats_for(&v(&["/tmp/a.JPEG"]));
        assert!(!f.contains(&"jpg"));
        assert!(f.contains(&"png"));
    }

    #[test]
    fn mixed_kinds_share_only_common_formats() {
        // a video and an audio file only share mp3
        assert_eq!(formats_for(&v(&["/tmp/a.mov", "/tmp/b.wav"])), vec!["mp3"]);
        // office files of any kind can all become a PDF
        assert_eq!(formats_for(&v(&["/tmp/a.docx", "/tmp/b.xlsx", "/tmp/c.pptx"])), vec!["pdf"]);
    }

    #[test]
    fn unknown_or_missing_extension_gives_nothing() {
        assert!(formats_for(&v(&["/tmp/a.xyz"])).is_empty());
        assert!(formats_for(&v(&["/tmp/Makefile"])).is_empty());
        assert!(formats_for(&v(&["/tmp/a.png", "/tmp/Makefile"])).is_empty());
    }

    #[test]
    fn mixed_image_formats_keep_all_outputs() {
        let f = formats_for(&v(&["/tmp/a.png", "/tmp/b.jpg"]));
        assert!(f.contains(&"png") && f.contains(&"jpg"));
    }

    #[test]
    fn word_files_offer_pdf() {
        assert!(formats_for(&v(&["/tmp/a.docx"])).contains(&"pdf"));
        assert!(formats_for(&v(&["/tmp/a.doc"])).contains(&"docx"));
    }

    #[test]
    fn no_kind_offers_more_than_the_wheel_draws() {
        for (_, _, outs) in KINDS {
            assert!(outs.len() <= MAX_SEGMENTS);
        }
    }
}
