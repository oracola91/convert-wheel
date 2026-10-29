//! Roughly how big a file would come out, for the middle of the wheel while a segment
//! is under the cursor. Nothing is converted for it: sound is worked out from its
//! length and the bit rate the conversion uses, a picture by converting pieces of it
//! and scaling up, and a video that is only copied into another container stays the
//! size it is. Everything else (video that is encoded again, documents, PDFs) has no
//! estimate.

use super::*;

/// The side of each of the nine square pieces, three across and three down, that are
/// cut from a picture and joined to measure a format. At full size, so they have the
/// photo's own detail; a shrunk copy of the whole would have more detail per pixel and
/// come out too big. Spread over the picture, because one piece from the middle, where
/// a photo's subject usually is, came out up to a quarter too big.
const PIECE: u32 = 256;

/// How long one measuring tool may take. An estimate that slow is not worth waiting for.
const LIMIT: Duration = Duration::from_secs(15);

/// Bit rates of the MP3 encoder's quality settings (`-q:a`), in kbit/s: what
/// `audio_args_at` says they give.
const MP3_VBR_KBPS: &[(&str, f64)] = &[("5", 130.0), ("2", 190.0), ("0", 245.0)];

/// FLAC packs everyday sound to about this share of the plain 16-bit samples.
const FLAC_SHARE: f64 = 0.6;

/// Estimated sizes in bytes of `src` made into each of `targets` (formats from the
/// wheel, or the Tools wheel's Compress for sound) at `quality`, for those that can be
/// told. The rest are left out.
pub fn sizes(src: &Path, targets: &[&str], quality: Quality) -> Vec<(String, u64)> {
    let Some(ext) = formats::extension(src) else {
        return Vec::new();
    };
    match formats::kind_of(&ext) {
        Some(Kind::Image) => pictures(src, &ext, targets, quality.pictures),
        Some(Kind::Audio | Kind::Video) => media(src, &ext, targets, quality.audio),
        _ => Vec::new(),
    }
}

/// How long a file plays, and its first sound track's sample rate and channels.
struct Sound {
    seconds: f64,
    rate: Option<u32>,
    channels: Option<u32>,
}

fn media(src: &Path, ext: &str, targets: &[&str], level: Level) -> Vec<(String, u64)> {
    let own = formats::canonical(ext);
    let video = formats::kind_of(ext) == Some(Kind::Video);
    let Ok(source) = fs::metadata(src).map(|m| m.len()) else {
        return Vec::new();
    };
    // Asked for once, by the first target that needs it.
    let mut sound: Option<Option<Sound>> = None;
    let mut found = Vec::new();
    for &target in targets {
        let bytes = match target {
            // MP4, MOV and M4V are copied into each other, and anything into MKV.
            "mp4" | "mov" if video && matches!(own, "mp4" | "mov" | "m4v") => Some(source),
            "mkv" if video => Some(source),
            "tool:compress" if !video && own != "flac" => sound_compress_codec(own)
                .and_then(|codec| sound_bytes(codec, sound.get_or_insert_with(|| sound_of(src)).as_ref()?)),
            t if formats::outputs_for(ext).contains(&t) && (!video || t == "mp3") => {
                let codec = audio_args_at(t, level);
                sound.get_or_insert_with(|| sound_of(src)).as_ref().and_then(|s| sound_bytes(&codec, s))
            }
            _ => None,
        };
        if let Some(bytes) = bytes {
            found.push((target.to_string(), bytes));
        }
    }
    found
}

/// The size of `sound` encoded with `codec` (ffmpeg's arguments): its length at the
/// codec's bit rate. None without a sound track or when the rate can't be told.
fn sound_bytes(codec: &[&str], sound: &Sound) -> Option<u64> {
    let after = |flag: &str| codec.windows(2).find(|w| w[0] == flag).map(|w| w[1]);
    let channels = f64::from(sound.channels?);
    let samples = || Some(f64::from(sound.rate?) * channels * 16.0);
    let bits_per_second = match after("-c:a")? {
        "pcm_s16le" | "pcm_s16be" => samples()?,
        "flac" => samples()? * FLAC_SHARE,
        _ => match (after("-b:a"), after("-q:a")) {
            (Some(rate), _) => rate.strip_suffix('k')?.parse::<f64>().ok()? * 1000.0,
            (None, Some(q)) => MP3_VBR_KBPS.iter().find(|(level, _)| *level == q)?.1 * 1000.0,
            (None, None) => return None,
        },
    };
    Some((bits_per_second * sound.seconds / 8.0) as u64)
}

/// The length and sound of `src`, from ffprobe, locked down like ffmpeg.
fn sound_of(src: &Path) -> Option<Sound> {
    let work = tempfile::tempdir().ok()?;
    let answer = work.path().join("sound.txt");
    let mut cmd = Command::new(tool("ffprobe").ok()?);
    cmd.args(["-v", "error", "-protocol_whitelist", "file", "-format_whitelist", FFMPEG_FORMATS])
        .args(["-select_streams", "a:0", "-show_entries", "format=duration:stream=sample_rate,channels"])
        .args(["-of", "default=noprint_wrappers=1", "-o"])
        .arg(format!("file:{}", answer.display()))
        .arg(format!("file:{}", src.display()));
    run(cmd, LIMIT).ok()?;
    let text = fs::read_to_string(&answer).ok()?;
    let value = |key: &str| text.lines().find_map(|line| line.strip_prefix(key)?.strip_prefix('=')).map(str::trim);
    let seconds = value("duration")?.parse::<f64>().ok().filter(|s| s.is_finite() && *s > 0.0)?;
    Some(Sound {
        seconds,
        rate: value("sample_rate").and_then(|v| v.parse().ok()),
        channels: value("channels").and_then(|v| v.parse().ok()),
    })
}

/// Where the three pieces sit along a side `length` pixels long: in the middles of its
/// thirds. Each is PIECE long, or a third of the side when that is shorter, so on a
/// small picture the pieces meet and together are the whole side.
fn pieces_along(length: u32) -> (u32, [u32; 3]) {
    let piece = PIECE.min(length / 3).max(1);
    let at = |third: u32| {
        let middle = u64::from(length) * u64::from(2 * third + 1) / 6;
        let start = u32::try_from(middle).unwrap_or(u32::MAX).saturating_sub(piece / 2);
        start.min(length.saturating_sub(piece))
    };
    (piece, [at(0), at(1), at(2)])
}

/// Each picture format converted from nine pieces spread over the picture, joined into
/// one, and scaled up by the picture's pixels. An icon is 256 pixels whatever the
/// picture, so its pieces are the estimate.
fn pictures(src: &Path, ext: &str, targets: &[&str], level: Level) -> Vec<(String, u64)> {
    let wanted: Vec<&str> = targets.iter().copied().filter(|t| formats::outputs_for(ext).contains(t)).collect();
    let (Some(coder), Ok(magick), Ok(work)) = (
        read_head(src).ok().and_then(|head| sniff_image(&head)),
        magick(),
        tempfile::tempdir(),
    ) else {
        return Vec::new();
    };
    let Some((width, height)) = (!wanted.is_empty()).then(|| pixel_size(src, coder)).flatten() else {
        return Vec::new();
    };
    let ((across, xs), (down, ys)) = (pieces_along(width), pieces_along(height));
    let piece = work.path().join("piece.miff");
    let mut cmd = Command::new(&magick);
    cmd.args(IMAGE_LIMITS).arg(format!("{coder}:{}[0]", src.display())).arg("-auto-orient");
    for y in ys {
        for x in xs {
            cmd.args(["(", "-clone", "0", "-crop", &format!("{across}x{down}+{x}+{y}"), "+repage", ")"]);
        }
    }
    // The nine pieces after the picture, joined three to a row and the rows stacked.
    for row in ["1-3", "4-6", "7-9"] {
        cmd.args(["(", "-clone", row, "+append", ")"]);
    }
    cmd.args(["-delete", "0-9", "-append", "+repage"]).arg(format!("miff:{}", piece.display()));
    if run(cmd, LIMIT).is_err() {
        return Vec::new();
    }
    let pixels = |w: u32, h: u32| f64::from(w) * f64::from(h);
    let scale = pixels(width, height) / (9.0 * pixels(across, down));
    wanted
        .into_iter()
        .filter_map(|target| {
            let out = work.path().join(format!("piece.{target}"));
            let mut cmd = Command::new(&magick);
            cmd.args(IMAGE_LIMITS).arg(format!("miff:{}", piece.display()));
            cmd.args(image_options(target, level));
            cmd.arg(format!("{}:{}", out_coder(target), out.display()));
            run(cmd, LIMIT).ok()?;
            let bytes = fs::metadata(&out).ok()?.len() as f64;
            let estimate = if target == "ico" { bytes } else { bytes * scale };
            Some((target.to_string(), estimate as u64))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(seconds: f64) -> Sound {
        Sound { seconds, rate: Some(44_100), channels: Some(2) }
    }

    #[test]
    fn sound_is_its_length_at_the_bit_rate() {
        // 10 s at 160 kbit/s: 200 KB.
        assert_eq!(sound_bytes(&audio_args_at("m4a", Level::Smaller), &sound(10.0)), Some(200_000));
        // WAV: 44.1 kHz, 2 channels, 16 bits.
        assert_eq!(sound_bytes(&audio_args_at("wav", Level::Balanced), &sound(1.0)), Some(176_400));
        // MP3 by its quality setting: Balanced is about 190 kbit/s.
        assert_eq!(sound_bytes(&audio_args_at("mp3", Level::Balanced), &sound(8.0)), Some(190_000));
        // Compress for an MP3: 96 kbit/s.
        assert_eq!(sound_bytes(sound_compress_codec("mp3").unwrap(), &sound(10.0)), Some(120_000));
        // A video without sound has nothing to measure.
        let silent = Sound { seconds: 10.0, rate: None, channels: None };
        assert_eq!(sound_bytes(&audio_args_at("mp3", Level::Balanced), &silent), None);
    }

    /// Needs ffmpeg. The estimates come near what the conversions make.
    #[test]
    fn sound_estimates_match_the_conversions() {
        let (Ok(ffmpeg), Ok(_)) = (tool("ffmpeg"), tool("ffprobe")) else {
            eprintln!("ffmpeg or ffprobe isn't installed: sound estimates not tested");
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let song = work.path().join("song.flac");
        let mut make = Command::new(ffmpeg);
        make.args(["-v", "error", "-f", "lavfi", "-i", "anoisesrc=color=pink:amplitude=0.2:duration=20", "-ac", "2", "-ar", "44100"]).arg(&song);
        assert!(make.status().unwrap().success());
        let found = sizes(&song, &["mp3", "wav", "m4a", "pdf", "tool:compress"], Quality::default());
        let names: Vec<&str> = found.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(names, ["mp3", "wav", "m4a"], "no PDF from sound, and no Compress for FLAC");
        // MP3's rate follows the sound (noise packs far below music's), so only the fixed
        // rates are compared.
        for (target, estimate) in found.into_iter().filter(|(t, _)| t != "mp3") {
            let made = convert(&song, &target, None, work.path()).unwrap();
            let real = fs::metadata(&made).unwrap().len() as f64;
            let ratio = estimate as f64 / real;
            assert!((0.85..=1.15).contains(&ratio), "{target}: estimated {estimate}, made {real}");
        }
    }

    /// Needs ImageMagick. The estimates come near what the conversions make.
    #[test]
    fn picture_estimates_match_the_conversions() {
        let Ok(magick) = magick() else {
            eprintln!("ImageMagick isn't installed: picture estimates not tested");
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let photo = work.path().join("photo.jpg");
        let mut make = Command::new(&magick);
        make.args(["-size", "1600x1200", "-seed", "7", "plasma:", "-quality", "92"]).arg(&photo);
        run(make, QUICK).unwrap();
        let targets = ["png", "jpg", "webp", "bmp", "ico", "docx"];
        let found = sizes(&photo, &targets, Quality::default());
        assert!(found.iter().all(|(t, _)| t != "docx"), "a picture can't become a document");
        assert!(found.len() >= 4, "{found:?}");
        for (target, estimate) in found {
            let made = convert(&photo, &target, None, work.path()).unwrap();
            let real = fs::metadata(&made).unwrap().len() as f64;
            let ratio = estimate as f64 / real;
            let range = if target == "bmp" { 0.99..=1.01 } else { 0.5..=2.0 };
            assert!(range.contains(&ratio), "{target}: estimated {estimate}, made {real}");
        }
        // Nothing to say about a file that isn't a picture.
        let text = work.path().join("notes.png");
        fs::write(&text, "not a picture").unwrap();
        assert!(sizes(&text, &targets, Quality::default()).is_empty());
    }

    #[test]
    fn pieces_spread_over_the_picture() {
        // A big side: PIECE-long pieces in the middles of its thirds.
        assert_eq!(pieces_along(3000), (256, [372, 1372, 2372]));
        // A small side: the pieces meet and are all of it.
        assert_eq!(pieces_along(300), (100, [0, 100, 200]));
        // Never past the end, however small.
        let (piece, at) = pieces_along(2);
        assert!(at.iter().all(|&x| x + piece <= 2), "{piece} {at:?}");
    }

    /// Needs ImageMagick. A photo whose middle is plain isn't taken for a plain photo.
    #[test]
    fn picture_estimates_look_past_the_middle() {
        let Ok(magick) = magick() else {
            eprintln!("ImageMagick isn't installed: picture estimates not tested");
            return;
        };
        let work = tempfile::tempdir().unwrap();
        let photo = work.path().join("photo.jpg");
        let mut make = Command::new(&magick);
        make.args(["-size", "1600x1200", "-seed", "7", "plasma:", "-fill", "gray50"])
            .args(["-draw", "rectangle 450,350 1150,850", "-quality", "92"])
            .arg(&photo);
        run(make, QUICK).unwrap();
        let found = sizes(&photo, &["png", "webp"], Quality::default());
        assert_eq!(found.len(), 2, "{found:?}");
        for (target, estimate) in found {
            let made = convert(&photo, &target, None, work.path()).unwrap();
            let real = fs::metadata(&made).unwrap().len() as f64;
            let ratio = estimate as f64 / real;
            assert!((0.8..=1.25).contains(&ratio), "{target}: estimated {estimate}, made {real}");
        }
    }
}
