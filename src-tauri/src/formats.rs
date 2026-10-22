//! Which formats the wheel offers for a dragged file.
//!
//! Proof step: this only picks the labels. The converter that uses them comes next.

use std::path::Path;

const IMAGE_IN: &[&str] = &["jpg", "jpeg", "png", "heic", "heif", "webp", "tiff", "tif", "bmp", "gif", "avif"];
const IMAGE_OUT: &[&str] = &["jpg", "png", "heic", "webp", "tiff", "gif", "avif", "pdf"];

const AUDIO_IN: &[&str] = &["mp3", "wav", "m4a", "aac", "flac", "aiff", "aif", "ogg", "opus"];
const AUDIO_OUT: &[&str] = &["mp3", "m4a", "wav", "flac", "aiff", "ogg"];

const VIDEO_IN: &[&str] = &["mp4", "mov", "m4v", "mkv", "webm", "avi"];
const VIDEO_OUT: &[&str] = &["mp4", "mov", "webm", "gif", "mp3"];

const DOC_IN: &[&str] = &["md", "markdown", "docx", "odt", "html", "htm", "txt", "rtf", "epub"];
const DOC_OUT: &[&str] = &["docx", "html", "md", "txt", "odt", "epub"];

/// Most segments the wheel draws, so labels stay readable.
pub const MAX_SEGMENTS: usize = 8;

fn extension(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

fn outputs_for(ext: &str) -> &'static [&'static str] {
    if IMAGE_IN.contains(&ext) {
        IMAGE_OUT
    } else if AUDIO_IN.contains(&ext) {
        AUDIO_OUT
    } else if VIDEO_IN.contains(&ext) {
        VIDEO_OUT
    } else if DOC_IN.contains(&ext) {
        DOC_OUT
    } else {
        &[]
    }
}

/// Same name for the same format, so "jpeg" in and "jpg" out don't both show.
fn canonical(ext: &str) -> &str {
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
    let exts: Vec<String> = paths.iter().filter_map(|p| extension(p)).collect();
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
}
