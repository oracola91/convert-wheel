//! SVG out: a picture traced into flat-coloured shapes by vtracer (visioncortex,
//! MIT or Apache-2.0, pure Rust, no file or image reading of its own).
//!
//! ImageMagick reads the picture as it does for every other format (the real format
//! from its first bytes, the usual limits), turns it upright, makes it at most
//! MAX_SIDE pixels on its longer side and writes plain pixels (PAM). The tracing then
//! runs in a process of its own, this same program started with `--trace`: a picture
//! that takes too long is stopped with the usual time limit and Stop Converting, and
//! whatever the tracing does with memory ends with that process.

use std::fs;
use std::path::Path;

use vtracer::{ColorImage, Config, Preset};

/// The longer side a picture is traced at. Tracing time and the SVG's size grow with
/// the pixels; past this, a photo's SVG only gets bigger, not better.
pub const MAX_SIDE: u32 = 2000;

/// Most pixels the tracing process takes, whatever the file says: MAX_SIDE squared.
const MAX_PIXELS: usize = (MAX_SIDE as usize) * (MAX_SIDE as usize);

/// What kind of picture it is, for the tracing settings: photos (JPEG, HEIC, AVIF)
/// get coarser layers and more speck filtering, so a photo doesn't become a
/// million tiny shapes; drawings, logos and screenshots keep their flat colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Photo,
    Flat,
}

impl Style {
    /// From ImageMagick's name for the picture's real format (convert.rs sniff_image).
    pub fn of_coder(coder: &str) -> Style {
        if matches!(coder, "jpeg" | "heic" | "avif") { Style::Photo } else { Style::Flat }
    }

    fn key(self) -> &'static str {
        match self {
            Style::Photo => "photo",
            Style::Flat => "flat",
        }
    }

    fn from_key(key: &str) -> Option<Style> {
        [Style::Photo, Style::Flat].into_iter().find(|s| s.key() == key)
    }
}

/// The arguments for the tracing process after `--trace`.
pub fn trace_args(pam: &Path, out: &Path, style: Style, level: crate::settings::Level) -> Vec<String> {
    vec![
        pam.display().to_string(),
        out.display().to_string(),
        style.key().to_string(),
        level.pick("smaller", "balanced", "best").to_string(),
    ]
}

/// vtracer's settings for a style at a quality level. Balanced is vtracer's own preset
/// for the style; Smaller drops more small specks and merges more colours, Best keeps
/// more of both. Coordinates are written with two decimals at most (one for Smaller).
pub fn config(style: Style, level: &str) -> Option<Config> {
    let mut config = match style {
        Style::Photo => Config::from_preset(Preset::Photo),
        Style::Flat => Config::from_preset(Preset::Poster),
    };
    match level {
        "smaller" => {
            config.filter_speckle *= 2;
            config.layer_difference += 16;
            config.color_precision -= 1;
            config.path_precision = Some(1);
        }
        "balanced" => {}
        "best" => {
            config.filter_speckle = (config.filter_speckle / 2).max(2);
            config.layer_difference = (config.layer_difference / 2).max(8);
        }
        _ => return None,
    }
    Some(config)
}

/// The tracing process: `--trace <pixels.pam> <out.svg> <photo|flat> <smaller|balanced|best>`.
/// Returns the exit code; a failure's reason goes to the error output, whose last
/// line becomes the notification's text.
pub fn trace_main(args: &[String]) -> i32 {
    let [pam, out, style, level] = args else {
        eprintln!("usage: convert-wheel --trace <pixels.pam> <out.svg> <photo|flat> <smaller|balanced|best>");
        return 2;
    };
    match trace_file(Path::new(pam), Path::new(out), style, level) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn trace_file(pam: &Path, out: &Path, style: &str, level: &str) -> Result<(), String> {
    let style = Style::from_key(style).ok_or("unknown tracing style")?;
    let config = config(style, level).ok_or("unknown quality level")?;
    let data = fs::read(pam).map_err(|e| format!("can't read the pixels: {e}"))?;
    let image = read_pam(&data).ok_or("the pixels to trace are not in a form this reads")?;
    let svg = trace(image, &config)?;
    fs::write(out, svg).map_err(|e| format!("can't save the SVG: {e}"))
}

/// The traced picture as an SVG document. vtracer gives it a width and a height only;
/// a viewBox as well lets other apps scale it to any size, as an SVG should.
pub fn trace(image: ColorImage, config: &Config) -> Result<String, String> {
    let (width, height) = (image.width, image.height);
    let pipeline = config.build().map_err(|e| format!("tracing settings: {e}"))?;
    let svg = pipeline.to_svg(&image).map_err(|e| format!("couldn't trace the picture: {e}"))?;
    let size = format!(r#"width="{width}" height="{height}""#);
    Ok(match svg.find(&size) {
        Some(at) if !svg.contains("viewBox") => {
            format!(r#"{}viewBox="0 0 {width} {height}" {}"#, &svg[..at], &svg[at..])
        }
        _ => svg,
    })
}

/// A binary PAM ("P7") as ImageMagick writes it, 8 bits a sample, grey or colour, with
/// or without transparency, as RGBA pixels. Anything else, or more than MAX_PIXELS, is
/// refused.
pub fn read_pam(data: &[u8]) -> Option<ColorImage> {
    let end = find(data, b"ENDHDR\n")?;
    let header = std::str::from_utf8(&data[..end]).ok()?;
    let mut lines = header.lines();
    if lines.next()?.trim() != "P7" {
        return None;
    }
    let (mut width, mut height, mut depth, mut max) = (0usize, 0usize, 0usize, 0usize);
    for line in lines {
        let mut words = line.split_whitespace();
        let (Some(key), value) = (words.next(), words.next()) else { continue };
        let number = || value.and_then(|v| v.parse::<usize>().ok());
        match key {
            "WIDTH" => width = number()?,
            "HEIGHT" => height = number()?,
            "DEPTH" => depth = number()?,
            "MAXVAL" => max = number()?,
            _ => {}
        }
    }
    let pixels = width.checked_mul(height)?;
    if pixels == 0 || pixels > MAX_PIXELS || max != 255 || !(1..=4).contains(&depth) {
        return None;
    }
    let body = data.get(end + b"ENDHDR\n".len()..)?.get(..pixels.checked_mul(depth)?)?;
    let mut rgba = Vec::with_capacity(pixels * 4);
    for p in body.chunks_exact(depth) {
        let pixel = match *p {
            [g] => [g, g, g, 255],
            [g, a] => [g, g, g, a],
            [r, g, b] => [r, g, b, 255],
            [r, g, b, a] => [r, g, b, a],
            _ => return None,
        };
        rgba.extend_from_slice(&pixel);
    }
    Some(ColorImage { pixels: rgba, width, height })
}

fn find(data: &[u8], what: &[u8]) -> Option<usize> {
    // The header is short; never look further than its first few hundred bytes.
    let head = &data[..data.len().min(1024)];
    head.windows(what.len()).position(|w| w == what)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pam(width: usize, height: usize, depth: usize, tuple: &str, body: &[u8]) -> Vec<u8> {
        let mut data =
            format!("P7\nWIDTH {width}\nHEIGHT {height}\nDEPTH {depth}\nMAXVAL 255\nTUPLTYPE {tuple}\nENDHDR\n").into_bytes();
        data.extend_from_slice(body);
        data
    }

    #[test]
    fn pam_reads_every_layout_as_rgba() {
        let rgba = read_pam(&pam(1, 1, 4, "RGB_ALPHA", &[1, 2, 3, 4])).unwrap();
        assert_eq!((rgba.width, rgba.height, rgba.pixels.clone()), (1, 1, vec![1, 2, 3, 4]));
        assert_eq!(read_pam(&pam(1, 1, 3, "RGB", &[1, 2, 3])).unwrap().pixels, [1, 2, 3, 255]);
        assert_eq!(read_pam(&pam(1, 1, 2, "GRAYSCALE_ALPHA", &[9, 7])).unwrap().pixels, [9, 9, 9, 7]);
        assert_eq!(read_pam(&pam(2, 1, 1, "GRAYSCALE", &[5, 6])).unwrap().pixels, [5, 5, 5, 255, 6, 6, 6, 255]);
    }

    #[test]
    fn pam_refuses_what_it_should_not_trace() {
        // too short, 16-bit, too many pixels, no pixels, not a PAM
        assert!(read_pam(&pam(2, 2, 4, "RGB_ALPHA", &[0; 15])).is_none());
        assert!(read_pam(b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 65535\nENDHDR\n\0\0\0\0\0\0\0\0").is_none());
        assert!(read_pam(&pam(MAX_SIDE as usize + 1, MAX_SIDE as usize, 4, "RGB_ALPHA", &[])).is_none());
        assert!(read_pam(&pam(0, 5, 4, "RGB_ALPHA", &[])).is_none());
        assert!(read_pam(b"P6\n1 1\n255\n\0\0\0").is_none());
    }

    #[test]
    fn styles_and_levels_give_settings() {
        assert_eq!(Style::of_coder("jpeg"), Style::Photo);
        assert_eq!(Style::of_coder("png"), Style::Flat);
        for style in [Style::Photo, Style::Flat] {
            assert_eq!(Style::from_key(style.key()), Some(style));
            let (smaller, balanced, best) =
                (config(style, "smaller").unwrap(), config(style, "balanced").unwrap(), config(style, "best").unwrap());
            assert!(smaller.filter_speckle > balanced.filter_speckle && best.filter_speckle <= balanced.filter_speckle);
            assert!(smaller.layer_difference > best.layer_difference);
            assert!(smaller.color_precision >= 1 && best.color_precision <= 8);
        }
        assert!(config(Style::Flat, "huge").is_none());
    }

    #[test]
    fn a_flat_picture_traces_into_its_colours() {
        // A red square on a white 40x40 picture.
        let (w, h) = (40, 40);
        let mut pixels = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let red = (10..30).contains(&x) && (10..30).contains(&y);
                pixels.extend_from_slice(if red { &[220, 20, 20, 255] } else { &[255, 255, 255, 255] });
            }
        }
        let svg = trace(ColorImage { pixels, width: w, height: h }, &config(Style::Flat, "balanced").unwrap()).unwrap();
        assert!(svg.contains("<svg") && svg.contains("<path"), "{svg}");
        assert!(svg.contains(r#"viewBox="0 0 40 40" width="40" height="40""#), "{svg}");
        assert!(svg.to_ascii_lowercase().contains("#dc1414"), "{svg}");
        assert!(!svg.contains("<script") && !svg.contains("href"));
    }
}
