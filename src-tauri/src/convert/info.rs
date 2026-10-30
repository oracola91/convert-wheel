//! Info: what a file says about itself, and for sound files and PDFs the tags that can
//! be changed. A picture's size, camera, date and place come from its EXIF (read by
//! ImageMagick), a sound's or video's tags and when and where it was recorded from
//! ffprobe, and a PDF's title, author and dates from mutool. Sound tags are written by
//! ffmpeg as a copy, nothing encoded again; a PDF's by mutool. Pictures and videos are
//! only read: nothing here writes EXIF, and ffmpeg would drop a phone video's own keys
//! (where it was recorded, the phone) when it copies one.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use serde::Serialize;

use super::{magick, read_head, run, sniff_image, tool, Mutool, FFMPEG_FORMATS, IMAGE_LIMITS, QUICK};
use crate::formats::{self, Kind};

/// What the Info window shows: what the file is, what it says (a label and a value
/// each, to read) and the tags that can be changed, with their values now.
pub struct Info {
    pub what: &'static str,
    pub facts: Vec<(String, String)>,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub value: String,
}

/// The tags Info changes in a sound file, by ffmpeg's names.
const SOUND: &[(&str, &str)] =
    &[("title", "Title"), ("artist", "Artist"), ("album", "Album"), ("date", "Year"), ("genre", "Genre"), ("comment", "Comment")];
/// And in a PDF, by the names in its Info dictionary.
const PDF: &[(&str, &str)] = &[("Title", "Title"), ("Author", "Author"), ("Subject", "Subject"), ("Keywords", "Keywords")];
/// The most characters a tag takes from the window.
const MAX_VALUE: usize = 1000;

/// Reads a PDF's Info for the window into the JSON file named second. getMetaData
/// decodes the PDF's own text encodings. MuPDF before 1.20 or so opens a document with
/// `new Document`, a PDF to change with `new PDFDocument`.
const PDF_READ: &str = r#"var doc = Document.openDocument ? Document.openDocument(scriptArgs[0]) : new Document(scriptArgs[0]);
var keys = ["Title", "Author", "Subject", "Keywords", "Creator", "Producer", "CreationDate", "ModDate"];
var out = {};
for (var i = 0; i < keys.length; i++) out[keys[i]] = doc.getMetaData("info:" + keys[i]) || "";
var b = new Buffer();
b.write(JSON.stringify(out));
b.save(scriptArgs[1]);
"#;

/// Saves the PDF (first) as the file named second with the tags that follow, as name
/// and value pairs. Older MuPDF has no setMetaData: there the Info dictionary is set
/// directly.
pub const PDF_WRITE: &str = r#"var doc = Document.openDocument ? Document.openDocument(scriptArgs[0]) : new PDFDocument(scriptArgs[0]);
for (var i = 2; i + 1 < scriptArgs.length; i += 2) {
  if (doc.setMetaData) {
    doc.setMetaData("info:" + scriptArgs[i], scriptArgs[i + 1]);
  } else {
    var trailer = doc.getTrailer();
    var info = trailer.get("Info");
    if (!info || info.isNull()) {
      info = doc.addObject(doc.newDictionary());
      trailer.put("Info", info);
    }
    info.put(scriptArgs[i], scriptArgs[i + 1]);
  }
}
doc.save(scriptArgs[1]);
"#;

pub fn read(src: &Path) -> Result<Info, String> {
    let ext = formats::extension(src).unwrap_or_default();
    match formats::kind_of(&ext) {
        Some(Kind::Image) => picture(src),
        Some(Kind::Audio) => media(src, false),
        Some(Kind::Video) => media(src, true),
        Some(Kind::Pdf) => pdf(src),
        _ => Err(format!(".{ext} files have no Info")),
    }
}

/// The window's changes: only tags it offered for this file, each once, as one line
/// of at most MAX_VALUE characters, and at least one.
pub fn check(changes: &[(String, String)], fields: &[Field]) -> Result<(), String> {
    if fields.is_empty() {
        return Err("this file's details can only be read".into());
    }
    if changes.is_empty() {
        return Err("nothing is changed yet: edit a tag first".into());
    }
    for (n, (key, value)) in changes.iter().enumerate() {
        let Some(field) = fields.iter().find(|f| f.key == key) else {
            return Err(format!("{key} isn't a tag this window changes"));
        };
        if changes[..n].iter().any(|(k, _)| k == key) {
            return Err(format!("{} is changed twice", field.label));
        }
        if value.chars().count() > MAX_VALUE || value.chars().any(char::is_control) {
            return Err(format!("{} must be one line of at most {MAX_VALUE} characters", field.label));
        }
    }
    Ok(())
}

/// ffmpeg's arguments to copy a sound file (`own`, its format) with its tags changed:
/// every stream, cover art too, and every other tag as it was. Ogg and Opus keep their
/// tags on the sound stream, the others on the file.
pub fn sound_args(own: &str, changes: &[(String, String)]) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = ["-map", "0", "-c", "copy", "-map_metadata", "0"].map(String::from).to_vec();
    let on = if matches!(own, "ogg" | "opus") { "-metadata:s:a:0" } else { "-metadata" };
    for (key, value) in changes {
        let (key, _) = SOUND.iter().find(|(k, _)| k == key).ok_or_else(|| format!("{key} isn't a sound tag"))?;
        args.extend([on.to_string(), format!("{key}={value}")]);
    }
    Ok(args)
}

/// mutool's arguments after the PDF for PDF_WRITE: the result's name, then the tags.
pub fn pdf_args(out_name: &str, changes: &[(String, String)]) -> Result<Vec<String>, String> {
    let mut args = vec![out_name.to_string()];
    for (key, value) in changes {
        let (key, _) = PDF.iter().find(|(k, _)| k == key).ok_or_else(|| format!("{key} isn't a PDF tag"))?;
        args.extend([key.to_string(), value.clone()]);
    }
    Ok(args)
}

fn work() -> Result<tempfile::TempDir, String> {
    tempfile::tempdir().map_err(|e| format!("work folder: {e}"))
}

fn picture(src: &Path) -> Result<Info, String> {
    let coder = sniff_image(&read_head(src)?).ok_or("this is not an image format the wheel can read")?;
    let work = work()?;
    let answer = work.path().join("info.txt");
    let mut cmd = Command::new(magick()?);
    cmd.args(IMAGE_LIMITS)
        .arg(format!("{coder}:{}[0]", src.display()))
        .args(["-auto-orient", "-format", "size=%w %h\n%[EXIF:*]"])
        .arg(format!("info:{}", answer.display()));
    run(cmd, QUICK)?;
    let text = String::from_utf8_lossy(&fs::read(&answer).map_err(|e| format!("work file: {e}"))?).into_owned();
    Ok(Info { what: "picture", facts: picture_facts(&text), fields: Vec::new() })
}

/// The size and the EXIF tags worth showing, from ImageMagick's lines ("size=4032 3024",
/// "exif:Make=Apple", ...). Where the picture was taken is always said, also when the
/// file doesn't hold it.
fn picture_facts(text: &str) -> Vec<(String, String)> {
    let tags: HashMap<&str, &str> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().strip_prefix("exif:").unwrap_or(k.trim()), v.trim()))
        .collect();
    let get = |key: &str| tags.get(key).copied().filter(|v| !v.is_empty());
    let mut facts = Vec::new();
    if let Some((w, h)) = get("size").and_then(|s| s.split_once(' ')) {
        facts.push(("Size".to_string(), format!("{w} × {h} pixels")));
    }
    if let Some(taken) = get("DateTimeOriginal").or_else(|| get("DateTime")) {
        facts.push(("Taken".to_string(), exif_date(taken)));
    }
    let camera = match (get("Make"), get("Model")) {
        (Some(make), Some(model)) if model.starts_with(make) => Some(model.to_string()),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (one, other) => one.or(other).map(str::to_string),
    };
    if let Some(camera) = camera {
        facts.push(("Camera".to_string(), camera));
    }
    for (key, label) in [("LensModel", "Lens"), ("Software", "Software"), ("Artist", "Artist"), ("Copyright", "Copyright"), ("ImageDescription", "Description")] {
        if let Some(value) = get(key) {
            facts.push((label.to_string(), value.to_string()));
        }
    }
    let place = gps(get("GPSLatitude"), get("GPSLatitudeRef"), get("GPSLongitude"), get("GPSLongitudeRef"));
    facts.push(("Location".to_string(), place.unwrap_or_else(|| "none in the file".to_string())));
    facts
}

/// EXIF's "2024:05:01 12:34:56" as "2024-05-01 12:34:56".
fn exif_date(text: &str) -> String {
    let b = text.as_bytes();
    if b.len() >= 10 && b[4] == b':' && b[7] == b':' {
        format!("{}-{}-{}", &text[..4], &text[5..7], &text[8..])
    } else {
        text.to_string()
    }
}

/// "51/1, 30/1, 1234/100" and its N/S, E/W as degrees: "51.50343, -0.12780".
fn gps(lat: Option<&str>, lat_ref: Option<&str>, lon: Option<&str>, lon_ref: Option<&str>) -> Option<String> {
    fn rational(text: &str) -> Option<f64> {
        let (n, d) = text.trim().split_once('/')?;
        let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
        (d != 0.0).then_some(n / d)
    }
    fn degrees(dms: &str) -> Option<f64> {
        let parts: Vec<f64> = dms.split(',').map(rational).collect::<Option<_>>()?;
        let [d, m, s] = parts.as_slice() else { return None };
        Some(d + m / 60.0 + s / 3600.0)
    }
    let (mut lat, mut lon) = (degrees(lat?)?, degrees(lon?)?);
    if lat_ref == Some("S") {
        lat = -lat;
    }
    if lon_ref == Some("W") {
        lon = -lon;
    }
    Some(format!("{lat:.5}, {lon:.5}"))
}

fn media(src: &Path, video: bool) -> Result<Info, String> {
    let work = work()?;
    let answer = work.path().join("tags.json");
    let mut cmd = Command::new(tool("ffprobe")?);
    cmd.args(["-v", "error", "-protocol_whitelist", "file", "-format_whitelist", FFMPEG_FORMATS])
        .args(["-show_entries", "format_tags:stream_tags", "-of", "json", "-o"])
        .arg(format!("file:{}", answer.display()))
        .arg(format!("file:{}", src.display()));
    run(cmd, QUICK)?;
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&answer).map_err(|e| format!("work file: {e}"))?)
        .map_err(|e| format!("ffprobe's answer: {e}"))?;
    let tags = media_tags(&json);
    let get = |key: &str| tags.get(key).filter(|v| !v.is_empty()).cloned();
    let mut facts = Vec::new();
    // A video's tags are shown, not changed.
    let shown: &[(&str, &str)] = if video { SOUND } else { &[] };
    let more = [("album_artist", "Album artist"), ("track", "Track"), ("composer", "Composer")];
    for (key, label) in shown.iter().chain(&more) {
        if let Some(value) = get(key) {
            facts.push((label.to_string(), value));
        }
    }
    if let Some(when) = get("com.apple.quicktime.creationdate").or_else(|| get("creation_time")) {
        facts.push(("Recorded".to_string(), when.replacen('T', " ", 1).trim_end_matches(".000000Z").to_string()));
    }
    let device = get("com.apple.quicktime.model").map(|model| match get("com.apple.quicktime.make") {
        Some(make) if !model.starts_with(&make) => format!("{make} {model}"),
        _ => model,
    });
    if let Some(device) = device {
        facts.push(("Recorded with".to_string(), device));
    }
    if video {
        let place = get("com.apple.quicktime.location.iso6709").or_else(|| get("location")).and_then(|l| iso6709(&l));
        facts.push(("Location".to_string(), place.unwrap_or_else(|| "none in the file".to_string())));
    }
    let fields = if video {
        Vec::new()
    } else {
        SOUND.iter().map(|&(key, label)| Field { key, label, value: get(key).unwrap_or_default() }).collect()
    };
    Ok(Info { what: if video { "video" } else { "sound" }, facts, fields })
}

/// ffprobe's tags with their names in lower case: the file's own, then for any not
/// there its streams' (Ogg and Opus keep theirs on the sound stream).
fn media_tags(json: &serde_json::Value) -> HashMap<String, String> {
    let mut tags = HashMap::new();
    let format = json.pointer("/format/tags").into_iter();
    let streams = json.get("streams").and_then(|s| s.as_array()).into_iter().flatten().filter_map(|s| s.get("tags"));
    for group in format.chain(streams).filter_map(|t| t.as_object()) {
        for (key, value) in group {
            if let Some(value) = value.as_str() {
                tags.entry(key.to_lowercase()).or_insert_with(|| value.trim().to_string());
            }
        }
    }
    tags
}

/// ISO 6709 "+51.5074-000.1278+010.000/" as "51.50740, -0.12780".
fn iso6709(text: &str) -> Option<String> {
    let starts: Vec<usize> = text.char_indices().filter(|&(_, c)| c == '+' || c == '-').map(|(i, _)| i).collect();
    let (&a, &b) = (starts.first()?, starts.get(1)?);
    let end = starts.get(2).copied().unwrap_or_else(|| text.find('/').unwrap_or(text.len()));
    let lat: f64 = text[a..b].parse().ok()?;
    let lon: f64 = text[b..end].parse().ok()?;
    Some(format!("{lat:.5}, {lon:.5}"))
}

fn pdf(src: &Path) -> Result<Info, String> {
    let work = work()?;
    let mutool = Mutool { path: tool("mutool")?, src, deadline: Instant::now() + QUICK };
    let pages = mutool.page_count(work.path())?;
    fs::write(work.path().join("read.js"), PDF_READ).map_err(|e| format!("work file: {e}"))?;
    mutool.run_args(work.path(), &["run", "read.js"], &["info.json"])?;
    let read: HashMap<String, String> =
        serde_json::from_slice(&fs::read(work.path().join("info.json")).map_err(|e| format!("MuPDF didn't read the details: {e}"))?)
            .map_err(|e| format!("MuPDF's answer: {e}"))?;
    let get = |key: &str| read.get(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let mut facts = vec![("Pages".to_string(), pages.to_string())];
    for (key, label) in [("Creator", "Made with"), ("Producer", "PDF made by")] {
        if let Some(value) = get(key) {
            facts.push((label.to_string(), value));
        }
    }
    for (key, label) in [("CreationDate", "Created"), ("ModDate", "Changed")] {
        if let Some(value) = get(key) {
            facts.push((label.to_string(), pdf_date(&value)));
        }
    }
    let fields = PDF.iter().map(|&(key, label)| Field { key, label, value: get(key).unwrap_or_default() }).collect();
    Ok(Info { what: "PDF", facts, fields })
}

/// A PDF date, "D:20240101120000+02'00'", as "2024-01-01 12:00"; anything else as it is.
fn pdf_date(text: &str) -> String {
    let digits: String = text.trim_start_matches("D:").chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 12 {
        return text.to_string();
    }
    format!("{}-{}-{} {}:{}", &digits[..4], &digits[4..6], &digits[6..8], &digits[8..10], &digits[10..12])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_facts_say_what_and_where() {
        let text = "size=4032 3024\nexif:Make=Apple\nexif:Model=iPhone 15\nexif:DateTimeOriginal=2024:05:01 12:34:56\n\
                    exif:GPSLatitude=51/1, 30/1, 1234/100\nexif:GPSLatitudeRef=N\nexif:GPSLongitude=0/1, 7/1, 4080/100\nexif:GPSLongitudeRef=W\n";
        let facts = picture_facts(text);
        let fact = |label: &str| facts.iter().find(|(l, _)| l == label).map(|(_, v)| v.as_str());
        assert_eq!(fact("Size"), Some("4032 × 3024 pixels"));
        assert_eq!(fact("Camera"), Some("Apple iPhone 15"));
        assert_eq!(fact("Taken"), Some("2024-05-01 12:34:56"));
        assert_eq!(fact("Location"), Some("51.50343, -0.12800"));
        assert_eq!(picture_facts("size=10 10\n").last().map(|(_, v)| v.as_str()), Some("none in the file"));
        assert_eq!(iso6709("+51.5074-000.1278+010.000/").as_deref(), Some("51.50740, -0.12780"));
        assert_eq!(pdf_date("D:20240101120000+02'00'"), "2024-01-01 12:00");
    }

    #[test]
    fn only_offered_tags_change() {
        let fields: Vec<Field> = SOUND.iter().map(|&(key, label)| Field { key, label, value: String::new() }).collect();
        let change = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        assert!(check(&change("title", "Café — тест"), &fields).is_ok());
        assert!(check(&[], &fields).is_err());
        assert!(check(&change("encoder", "x"), &fields).is_err());
        assert!(check(&change("title", "two\nlines"), &fields).is_err());
        assert!(check(&change("title", &"x".repeat(1001)), &fields).is_err());
        let twice = [change("title", "a"), change("title", "b")].concat();
        assert!(check(&twice, &fields).is_err());
        assert!(check(&change("title", "a"), &[]).is_err());
        let args = sound_args("opus", &change("title", "-x")).unwrap();
        assert_eq!(args[6..], ["-metadata:s:a:0", "title=-x"]);
        assert!(sound_args("mp3", &change("../x", "y")).is_err());
        assert_eq!(pdf_args("result.pdf", &change("Author", "Me")).unwrap(), ["result.pdf", "Author", "Me"]);
    }
}
