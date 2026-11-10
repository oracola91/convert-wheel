//! Info: what a file says about itself, and for sound files and PDFs the tags that can
//! be changed. A picture's size, camera, date and place come from its EXIF (read by
//! ImageMagick), a sound's or video's tags and when and where it was recorded from
//! ffprobe, and a PDF's title, author and dates from mutool. Sound tags are written by
//! ffmpeg as a copy, nothing encoded again; a PDF's by mutool. Pictures and videos are
//! only read: nothing here writes EXIF, and ffmpeg would drop a phone video's own keys
//! (where it was recorded, the phone) when it copies one. So a picture's or video's
//! date taken and location are written by ExifTool instead (council 2026-11-10: one
//! more converter, installed on demand), into a copy, and read back before it counts.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use serde::Serialize;

use super::{magick, quiet, read_head, run, sniff_image, tool, Mutool, FFMPEG_FORMATS, IMAGE_LIMITS, QUICK};
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
    /// How a value is written, shown in the empty box ("" for free text).
    pub hint: &'static str,
}

/// The tags Info changes in a sound file, by ffmpeg's names.
const SOUND: &[(&str, &str)] =
    &[("title", "Title"), ("artist", "Artist"), ("album", "Album"), ("date", "Year"), ("genre", "Genre"), ("comment", "Comment")];
/// And in a PDF, by the names in its Info dictionary.
const PDF: &[(&str, &str)] = &[("Title", "Title"), ("Author", "Author"), ("Subject", "Subject"), ("Keywords", "Keywords")];
/// The most characters a tag takes from the window.
const MAX_VALUE: usize = 1000;

/// Reads a PDF's Info for the window into the JSON file named second, and whether this
/// MuPDF can change it ("write"). getMetaData decodes the PDF's own text encodings.
/// Older MuPDF (Ubuntu 22.04's 1.19) opens a document with `new Document` and has no
/// setMetaData: there the details are only read.
const PDF_READ: &str = r#"var doc = Document.openDocument ? Document.openDocument(scriptArgs[0]) : new Document(scriptArgs[0]);
var keys = ["Title", "Author", "Subject", "Keywords", "Creator", "Producer", "CreationDate", "ModDate"];
var out = {};
for (var i = 0; i < keys.length; i++) out[keys[i]] = doc.getMetaData("info:" + keys[i]) || "";
out.write = Document.openDocument && typeof doc.setMetaData === "function" ? "yes" : "";
var b = new Buffer();
b.write(JSON.stringify(out));
b.save(scriptArgs[1]);
"#;

/// Saves the PDF (first) as the file named second with the tags that follow, as name
/// and value pairs. Only run where PDF_READ found setMetaData.
pub const PDF_WRITE: &str = r#"var doc = Document.openDocument(scriptArgs[0]);
for (var i = 2; i + 1 < scriptArgs.length; i += 2) doc.setMetaData("info:" + scriptArgs[i], scriptArgs[i + 1]);
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
        if key == "taken" && parse_date(value).is_none() {
            return Err(format!("{} must be a date like 2024-05-01 12:34:56", field.label));
        }
        if key == "location" {
            parse_place(value).map_err(|e| format!("Location: {e}"))?;
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

/// The pictures and videos whose date and place ExifTool writes.
const EXIF_PICTURES: &[&str] = &["jpg", "jpeg", "png", "heic", "heif", "webp", "tif", "tiff", "avif"];
const EXIF_VIDEOS: &[&str] = &["mp4", "mov", "m4v"];
/// ExifTool 12.24 fixed a flaw in reading files (CVE-2021-22204); older ones aren't run.
const EXIFTOOL_AT_LEAST: (u32, u32) = (12, 24);
const PLACE_FORMAT: &str = "write it as latitude, longitude, like 51.50343, -0.12800";

/// The date and the place, as the window offers them: the file's own values (from
/// `facts`, under `date_fact` and "Location"), in the form the boxes take.
fn date_place_fields(label: &'static str, date_fact: &str, facts: &[(String, String)]) -> Vec<Field> {
    let fact = |name: &str| facts.iter().find(|(l, _)| l == name).map(|(_, v)| v.clone());
    let date = fact(date_fact).map(|d| d.chars().take(19).collect::<String>()).filter(|d| parse_date(d).is_some());
    let place = fact("Location").filter(|p| matches!(parse_place(p), Ok(Some(_))));
    vec![
        Field { key: "taken", label, value: date.unwrap_or_default(), hint: "2024-05-01 12:34:56" },
        Field { key: "location", label: "Location", value: place.unwrap_or_default(), hint: "51.50343, -0.12800 (empty: none)" },
    ]
}

/// "2024-05-01 12:34:56" (also with ':' in the date, a 'T' between, or no seconds) as
/// EXIF's "2024:05:01 12:34:56"; None for anything else or a day that doesn't exist.
fn parse_date(text: &str) -> Option<String> {
    let (date, time) = text.trim().split_once([' ', 'T'])?;
    let d: Vec<u32> = date.split(['-', ':']).map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let t: Vec<u32> = time.trim().split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let [y, mo, da] = d[..] else { return None };
    let (h, mi, se) = match t[..] {
        [h, m] => (h, m, 0),
        [h, m, s] => (h, m, s),
        _ => return None,
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match mo {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    ((1900..=2100).contains(&y) && (1..=days).contains(&da) && h < 24 && mi < 60 && se < 60)
        .then(|| format!("{y:04}:{mo:02}:{da:02} {h:02}:{mi:02}:{se:02}"))
}

/// "51.50343, -0.12800" (a comma, spaces or both between) as latitude and longitude;
/// empty means no location at all.
fn parse_place(text: &str) -> Result<Option<(f64, f64)>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let numbers: Vec<f64> = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<f64>().map_err(|_| PLACE_FORMAT.to_string()))
        .collect::<Result<_, _>>()?;
    let [lat, lon] = numbers[..] else { return Err(PLACE_FORMAT.into()) };
    if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
        return Err("latitude goes from -90 to 90, longitude from -180 to 180".into());
    }
    Ok(Some((lat, lon)))
}

/// ExifTool's tag arguments for the window's changes. Pictures: the EXIF date taken
/// and GPS (with XMP's copy of the place when it goes). Videos: QuickTime's date (in
/// UTC, from local time) and Apple's own keys, which phones and Photos read, plus the
/// common user-data place.
fn exif_args(video: bool, changes: &[(String, String)]) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = Vec::new();
    if video {
        args.extend(["-api", "QuickTimeUTC=1"].map(String::from));
    }
    for (key, value) in changes {
        match key.as_str() {
            "taken" => {
                let d = parse_date(value).ok_or("the date must look like 2024-05-01 12:34:56")?;
                let tags: &[&str] = if video { &["QuickTime:CreateDate", "Keys:CreationDate"] } else { &["DateTimeOriginal", "CreateDate"] };
                args.extend(tags.iter().map(|t| format!("-{t}={d}")));
            }
            "location" => match parse_place(value)? {
                None if video => args.extend(["-Keys:GPSCoordinates=", "-UserData:GPSCoordinates=", "-ItemList:GPSCoordinates="].map(String::from)),
                None => args.extend(["-gps:all=", "-XMP-exif:GPSLatitude=", "-XMP-exif:GPSLongitude=", "-XMP-exif:GPSAltitude="].map(String::from)),
                Some((lat, lon)) if video => {
                    args.extend([format!("-Keys:GPSCoordinates={lat:.6}, {lon:.6}"), format!("-UserData:GPSCoordinates={lat:.6}, {lon:.6}")]);
                }
                Some((lat, lon)) => args.extend([
                    format!("-GPSLatitude={:.6}", lat.abs()),
                    format!("-GPSLatitudeRef={}", if lat < 0.0 { "S" } else { "N" }),
                    format!("-GPSLongitude={:.6}", lon.abs()),
                    format!("-GPSLongitudeRef={}", if lon < 0.0 { "W" } else { "E" }),
                ]),
            },
            other => return Err(format!("{other} isn't a tag ExifTool changes here")),
        }
    }
    Ok(args)
}

/// Writes the window's date and place into a copy (`out`) with ExifTool, then reads
/// the copy back: a copy that doesn't say what was asked is deleted and the job fails,
/// so a wrong copy never ends up next to the original.
pub fn write_exif(src: &Path, out: &Path, video: bool, changes: &[(String, String)]) -> Result<(), String> {
    let exiftool = tool("exiftool")?;
    let version = answer(Command::new(&exiftool).arg("-ver"))?;
    if !version_at_least(&version, EXIFTOOL_AT_LEAST) {
        return Err(format!("ExifTool {} is too old: 12.24 or newer is needed (Settings > Update Converters…)", version.trim()));
    }
    // ExifTool reads anything starting with "-" as an option; full paths never do.
    if !src.is_absolute() || !out.is_absolute() {
        return Err("ExifTool is only given full paths".into());
    }
    let mut cmd = Command::new(&exiftool);
    // `-config ""` (it must come first): no config file, only what the app asks for.
    // `-o` writes a new file and leaves the original alone, with no backup next to it.
    cmd.args(["-config", ""]).args(exif_args(video, changes)?).arg("-o").arg(out).arg(src);
    run(cmd, QUICK)?;
    let back = read_back(&exiftool, out, video).and_then(|tags| verify(&tags, video, changes));
    if let Err(e) = back {
        let _ = fs::remove_file(out);
        return Err(e);
    }
    Ok(())
}

/// What ExifTool prints for `cmd`. The answers read here are short, so a pipe is fine.
fn answer(cmd: &mut Command) -> Result<String, String> {
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    quiet(cmd);
    let out = cmd.output().map_err(|e| format!("could not start ExifTool: {e}"))?;
    if !out.status.success() {
        return Err(format!("ExifTool couldn't read the copy ({})", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// "13.50" at least `want` (major, minor).
fn version_at_least(version: &str, want: (u32, u32)) -> bool {
    let mut parts = version.trim().split('.');
    let major = parts.next().and_then(|p| p.parse::<u32>().ok());
    let minor = parts.next().and_then(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse::<u32>().ok());
    matches!((major, minor), (Some(a), Some(b)) if (a, b) >= want)
}

/// The copy's date and place tags, by group and name ("GPS:GPSLatitude"), as text.
fn read_back(exiftool: &Path, out: &Path, video: bool) -> Result<HashMap<String, String>, String> {
    let mut cmd = Command::new(exiftool);
    cmd.args(["-config", "", "-j", "-a", "-G1", "-n"]);
    if video {
        cmd.args(["-api", "QuickTimeUTC=1", "-CreationDate", "-CreateDate", "-GPSCoordinates"]);
    } else {
        cmd.args(["-DateTimeOriginal", "-GPS*"]);
    }
    let json: serde_json::Value = serde_json::from_str(&answer(cmd.arg(out))?).map_err(|e| format!("ExifTool's answer: {e}"))?;
    let tags = json.get(0).and_then(|v| v.as_object()).ok_or("ExifTool read nothing back")?;
    Ok(tags
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), str::to_string)))
        .collect())
}

/// Whether the copy says what the window asked for.
fn verify(tags: &HashMap<String, String>, video: bool, changes: &[(String, String)]) -> Result<(), String> {
    // Composite tags are ExifTool's own sums of the others (some versions list them
    // for "-GPS*"), not places stored in the file.
    let named = |name: &str| -> Vec<&String> {
        tags.iter().filter(|(k, _)| !k.starts_with("Composite:") && k.rsplit(':').next() == Some(name)).map(|(_, v)| v).collect()
    };
    let near = |(a, b): (f64, f64), (c, d): (f64, f64)| (a - c).abs() < 1e-4 && (b - d).abs() < 1e-4;
    for (key, value) in changes {
        match key.as_str() {
            "taken" => {
                let want = parse_date(value).ok_or("the date must look like 2024-05-01 12:34:56")?;
                let names: &[&str] = if video { &["CreationDate", "CreateDate"] } else { &["DateTimeOriginal"] };
                if !names.iter().flat_map(|n| named(n)).any(|v| v.starts_with(&want)) {
                    return Err("ExifTool didn't keep the new date, so no copy was made".into());
                }
            }
            "location" => {
                let places: Vec<Option<(f64, f64)>> = if video {
                    named("GPSCoordinates").into_iter().map(|v| first_two(v)).collect()
                } else {
                    let get = |k: &str| tags.get(k).and_then(|v| v.parse::<f64>().ok());
                    let sign = |k: &str, neg: &str| if tags.get(k).is_some_and(|r| r == neg) { -1.0 } else { 1.0 };
                    let exif = match (get("GPS:GPSLatitude"), get("GPS:GPSLongitude")) {
                        (Some(lat), Some(lon)) => vec![Some((lat * sign("GPS:GPSLatitudeRef", "S"), lon * sign("GPS:GPSLongitudeRef", "W")))],
                        _ => Vec::new(),
                    };
                    // Any other place left (XMP's) counts too, as a place still in the file.
                    let others = named("GPSLatitude").len() + named("GPSLongitude").len() - 2 * exif.len();
                    exif.into_iter().chain((0..others).map(|_| None)).collect()
                };
                let kept = match parse_place(value)? {
                    None => places.is_empty(),
                    Some(want) => !places.is_empty() && places.iter().all(|p| p.is_some_and(|p| near(p, want))),
                };
                if !kept {
                    return Err("ExifTool didn't keep the new location, so no copy was made".into());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// The first two numbers in "48.85 2.29 0" or "48.85, 2.29".
fn first_two(text: &str) -> Option<(f64, f64)> {
    let mut numbers = text.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).map(|p| p.parse::<f64>().ok());
    Some((numbers.next()??, numbers.next()??))
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
    let facts = picture_facts(&text);
    let ext = formats::extension(src).unwrap_or_default();
    let fields = if EXIF_PICTURES.contains(&ext.as_str()) { date_place_fields("Date taken", "Taken", &facts) } else { Vec::new() };
    Ok(Info { what: "picture", facts, fields })
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
    let ext = formats::extension(src).unwrap_or_default();
    let fields = if video {
        if EXIF_VIDEOS.contains(&ext.as_str()) { date_place_fields("Recorded", "Recorded", &facts) } else { Vec::new() }
    } else {
        SOUND.iter().map(|&(key, label)| Field { key, label, value: get(key).unwrap_or_default(), hint: "" }).collect()
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
    let fields = if get("write").is_some() {
        PDF.iter().map(|&(key, label)| Field { key, label, value: get(key).unwrap_or_default(), hint: "" }).collect()
    } else {
        // Shown, not changed, by an older MuPDF.
        for &(key, label) in PDF.iter().rev() {
            if let Some(value) = get(key) {
                facts.insert(1, (label.to_string(), value));
            }
        }
        Vec::new()
    };
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
    fn dates_and_places_are_read_as_written() {
        assert_eq!(parse_date("2024-05-01 12:34:56").as_deref(), Some("2024:05:01 12:34:56"));
        assert_eq!(parse_date("2024:05:01 12:34").as_deref(), Some("2024:05:01 12:34:00"));
        assert_eq!(parse_date("2024-05-01T08:00:00").as_deref(), Some("2024:05:01 08:00:00"));
        assert_eq!(parse_date("2023-02-29 10:00"), None);
        assert_eq!(parse_date("2024-02-29 10:00").as_deref(), Some("2024:02:29 10:00:00"));
        assert_eq!(parse_date("yesterday"), None);
        assert_eq!(parse_place("  "), Ok(None));
        assert_eq!(parse_place("51.50343, -0.128"), Ok(Some((51.50343, -0.128))));
        assert_eq!(parse_place("51.5 -0.1"), Ok(Some((51.5, -0.1))));
        assert!(parse_place("95, 10").is_err());
        assert!(parse_place("NaN, 1").is_err());
        assert!(parse_place("London").is_err());
        assert!(version_at_least("13.50\n", (12, 24)));
        assert!(version_at_least("12.24", (12, 24)));
        assert!(!version_at_least("12.23", (12, 24)));
        assert!(!version_at_least("", (12, 24)));
    }

    #[test]
    fn exiftool_gets_tags_not_a_shell_line() {
        let ch = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        assert_eq!(exif_args(false, &ch("taken", "2024-05-01 12:34")).unwrap(), ["-DateTimeOriginal=2024:05:01 12:34:00", "-CreateDate=2024:05:01 12:34:00"]);
        assert_eq!(
            exif_args(false, &ch("location", "-33.86, 151.2")).unwrap(),
            ["-GPSLatitude=33.860000", "-GPSLatitudeRef=S", "-GPSLongitude=151.200000", "-GPSLongitudeRef=E"]
        );
        let video = exif_args(true, &ch("location", "")).unwrap();
        assert_eq!(video[..2], ["-api", "QuickTimeUTC=1"]);
        assert!(video.iter().any(|a| a == "-Keys:GPSCoordinates="));
        assert!(exif_args(false, &ch("title", "x")).is_err());
        assert!(exif_args(false, &ch("taken", "-x")).is_err());
    }

    #[test]
    fn a_copy_counts_only_when_it_says_what_was_asked() {
        let tags = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        let ch = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        let photo = tags(&[("ExifIFD:DateTimeOriginal", "2024:05:01 12:34:56"), ("GPS:GPSLatitude", "48.85"), ("GPS:GPSLatitudeRef", "N"), ("GPS:GPSLongitude", "2.29"), ("GPS:GPSLongitudeRef", "W")]);
        assert!(verify(&photo, false, &ch("taken", "2024-05-01 12:34:56")).is_ok());
        assert!(verify(&photo, false, &ch("location", "48.85, -2.29")).is_ok());
        let mut with_sums = photo.clone();
        with_sums.insert("Composite:GPSLatitude".into(), "48.85".into());
        with_sums.insert("Composite:GPSLongitude".into(), "-2.29".into());
        assert!(verify(&with_sums, false, &ch("location", "48.85, -2.29")).is_ok());
        assert!(verify(&photo, false, &ch("location", "48.85, 2.29")).is_err());
        assert!(verify(&photo, false, &ch("location", "")).is_err());
        assert!(verify(&tags(&[]), false, &ch("location", "")).is_ok());
        // A place left in XMP is still a place.
        assert!(verify(&tags(&[("XMP-exif:GPSLatitude", "48.85")]), false, &ch("location", "")).is_err());
        let clip = tags(&[("Keys:GPSCoordinates", "51.50343 -0.128 0"), ("UserData:GPSCoordinates", "51.50343 -0.128 0"), ("Keys:CreationDate", "2024:05:01 12:34:56+00:00")]);
        assert!(verify(&clip, true, &ch("location", "51.50343, -0.128")).is_ok());
        assert!(verify(&clip, true, &ch("taken", "2024-05-01 12:34:56")).is_ok());
        assert!(verify(&clip, true, &ch("taken", "2024-05-02 12:34:56")).is_err());
    }

    #[test]
    fn only_offered_tags_change() {
        let fields: Vec<Field> = SOUND.iter().map(|&(key, label)| Field { key, label, value: String::new(), hint: "" }).collect();
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
