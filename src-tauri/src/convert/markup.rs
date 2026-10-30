//! Mark Up: what the window drew on a picture (pen lines, arrows, boxes and words) and
//! the parts it blacked out, as shapes in the picture's own pixels, upright. The page
//! sends only these; here they are checked and turned into ImageMagick's drawing
//! steps, in colours from a fixed list, so nothing the page sends can name a file or
//! reach a command. Blacked-out parts are drawn last, so nothing lands on top of them,
//! and a picture with any of them loses its metadata too: the small preview a camera
//! keeps in the EXIF would otherwise still show what was covered.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The colours the window offers, by the names it sends.
const COLOURS: &[(&str, &str)] = &[
    ("red", "#e5322d"),
    ("yellow", "#f5c400"),
    ("blue", "#1d6fe5"),
    ("green", "#21a148"),
    ("black", "#000000"),
    ("white", "#ffffff"),
];

const MAX_SHAPES: usize = 500;
const MAX_POINTS: usize = 4000;
const MAX_WORDS: usize = 200;

/// One thing drawn, in pixels of the upright picture.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Shape {
    Pen { colour: String, width: u32, points: Vec<[i64; 2]> },
    Arrow { colour: String, width: u32, from: [i64; 2], to: [i64; 2] },
    Box { colour: String, width: u32, from: [i64; 2], to: [i64; 2] },
    /// Words, their top left corner at `at`, `size` pixels tall.
    Text { colour: String, size: u32, at: [i64; 2], text: String },
    /// A part blacked out.
    Redact { from: [i64; 2], to: [i64; 2] },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Markup {
    pub shapes: Vec<Shape>,
}

impl Markup {
    /// Something drawn, every shape inside a picture `width` x `height`, in a colour
    /// the window offers, of a sane size.
    pub fn check(&self, width: u32, height: u32) -> Result<(), String> {
        if self.shapes.is_empty() {
            return Err("nothing is drawn yet".into());
        }
        if self.shapes.len() > MAX_SHAPES {
            return Err(format!("at most {MAX_SHAPES} marks"));
        }
        let inside = |[x, y]: [i64; 2]| (0..=i64::from(width)).contains(&x) && (0..=i64::from(height)).contains(&y);
        let line = |colour: &str, w: u32| hex(colour).is_some() && (1..=200).contains(&w);
        let fine = self.shapes.iter().all(|shape| match shape {
            Shape::Pen { colour, width, points } => {
                line(colour, *width) && (1..=MAX_POINTS).contains(&points.len()) && points.iter().all(|p| inside(*p))
            }
            Shape::Arrow { colour, width, from, to } | Shape::Box { colour, width, from, to } => {
                line(colour, *width) && inside(*from) && inside(*to)
            }
            Shape::Text { colour, size, at, text } => {
                hex(colour).is_some()
                    && (8..=1000).contains(size)
                    && inside(*at)
                    && !text.trim().is_empty()
                    && text.chars().count() <= MAX_WORDS
                    && !text.chars().any(char::is_control)
            }
            Shape::Redact { from, to } => inside(*from) && inside(*to),
        });
        if fine {
            Ok(())
        } else {
            Err("a mark is outside the picture or not one the window makes".into())
        }
    }

    /// Whether any part is blacked out.
    pub fn redacts(&self) -> bool {
        self.shapes.iter().any(|s| matches!(s, Shape::Redact { .. }))
    }
}

fn hex(colour: &str) -> Option<&'static str> {
    COLOURS.iter().find(|(name, _)| *name == colour).map(|(_, hex)| *hex)
}

/// ImageMagick's steps for the shapes (checked first): the drawn ones in the order
/// they were drawn, then the blacked-out parts on top of everything. Words are drawn in
/// `font`; without one, only marks without words can be drawn.
pub fn draw_args(markup: &Markup, font: Option<&Path>) -> Result<Vec<String>, String> {
    let has_words = markup.shapes.iter().any(|s| matches!(s, Shape::Text { .. }));
    let font = match (has_words, font) {
        (true, None) => return Err("no font was found to write the words with".into()),
        (_, font) => font.map(|f| f.to_string_lossy().into_owned()),
    };
    let mut args: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));
    let point = |[x, y]: [i64; 2]| format!("{x},{y}");
    for shape in &markup.shapes {
        match shape {
            Shape::Pen { colour, width, points } => {
                let colour = hex(colour).unwrap_or("#000000");
                let (w, r) = (width.to_string(), f64::from(*width) / 2.0);
                let draw = match points.as_slice() {
                    // A dot: a click without a stroke.
                    [[x, y]] => format!("circle {x},{y} {},{y}", *x as f64 + r),
                    many => {
                        let line = many.iter().map(|p| point(*p)).collect::<Vec<_>>().join(" ");
                        format!("stroke-linecap round stroke-linejoin round polyline {line}")
                    }
                };
                let fill = if points.len() == 1 { colour } else { "none" };
                push(&["-fill", fill, "-stroke", colour, "-strokewidth", &w, "-draw", &draw]);
            }
            Shape::Arrow { colour, width, from, to } => {
                let colour = hex(colour).unwrap_or("#000000");
                let (shaft_end, head) = arrow_head(*from, *to, *width);
                let shaft = format!("line {} {}", point(*from), shaft_end);
                push(&["-fill", "none", "-stroke", colour, "-strokewidth", &width.to_string(), "-draw", &shaft]);
                push(&["-fill", colour, "-stroke", colour, "-strokewidth", "1", "-draw", &format!("polygon {head}")]);
            }
            Shape::Box { colour, width, from, to } => {
                let colour = hex(colour).unwrap_or("#000000");
                let rect = rectangle(*from, *to);
                push(&["-fill", "none", "-stroke", colour, "-strokewidth", &width.to_string(), "-draw", &rect]);
            }
            Shape::Text { colour, size, at, text } => {
                let colour = hex(colour).unwrap_or("#000000");
                let offset = format!("+{}+{}", at[0], at[1]);
                let font = font.as_deref().unwrap_or_default();
                push(&["-font", font, "-fill", colour, "-stroke", "none", "-pointsize", &size.to_string(), "-gravity", "NorthWest"]);
                push(&["-annotate", &offset, &annotate_text(text), "+gravity"]);
            }
            Shape::Redact { .. } => {}
        }
    }
    for shape in &markup.shapes {
        if let Shape::Redact { from, to } = shape {
            push(&["-fill", "#000000", "-stroke", "none", "-draw", &rectangle(*from, *to)]);
        }
    }
    Ok(args)
}

/// A font for the words, with Latin, Greek and Cyrillic letters, from those each system
/// comes with: ImageMagick has none of its own to fall back on everywhere.
pub fn font() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let candidates: Vec<PathBuf> = [
        "/Library/Fonts/Arial Unicode.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
    ]
    .map(PathBuf::from)
    .to_vec();
    #[cfg(windows)]
    let candidates: Vec<PathBuf> = {
        let fonts = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
        ["arial.ttf", "segoeui.ttf"].map(|f| fonts.join(f)).to_vec()
    };
    #[cfg(target_os = "linux")]
    let candidates: Vec<PathBuf> = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    ]
    .map(PathBuf::from)
    .to_vec();
    candidates.into_iter().find(|f| f.is_file())
}

fn rectangle([x0, y0]: [i64; 2], [x1, y1]: [i64; 2]) -> String {
    format!("rectangle {},{} {},{}", x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1))
}

/// Where an arrow's line stops (under its head, so a thick line doesn't poke out of the
/// tip) and the three corners of its head, at `to`.
fn arrow_head(from: [i64; 2], to: [i64; 2], width: u32) -> (String, String) {
    let (fx, fy, tx, ty) = (from[0] as f64, from[1] as f64, to[0] as f64, to[1] as f64);
    let angle = (ty - fy).atan2(tx - fx);
    let length = (f64::from(width) * 4.0).max(12.0);
    let spread = 25f64.to_radians();
    let corner = |a: f64| format!("{:.0},{:.0}", tx - length * a.cos(), ty - length * a.sin());
    let back = length * spread.cos() * 0.8;
    let shaft_end = format!("{:.0},{:.0}", tx - back * angle.cos(), ty - back * angle.sin());
    (shaft_end, format!("{tx:.0},{ty:.0} {} {}", corner(angle - spread), corner(angle + spread)))
}

/// Words as ImageMagick takes them in -annotate, read literally: it would otherwise
/// treat % as the start of a property, a backslash as an escape, and a leading @ as
/// the name of a file to read the words from.
fn annotate_text(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('%', "%%");
    match escaped.strip_prefix('@') {
        Some(rest) => format!("\\@{rest}"),
        None => escaped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pen(points: Vec<[i64; 2]>) -> Shape {
        Shape::Pen { colour: "red".into(), width: 4, points }
    }

    #[test]
    fn marks_must_be_inside_and_from_the_window() {
        let ok = Markup { shapes: vec![pen(vec![[0, 0], [100, 50]]), Shape::Redact { from: [10, 10], to: [20, 20] }] };
        assert!(ok.check(100, 50).is_ok());
        assert!(ok.redacts());
        assert!(Markup { shapes: vec![] }.check(100, 50).is_err());
        assert!(Markup { shapes: vec![pen(vec![[0, 0], [101, 50]])] }.check(100, 50).is_err());
        let odd = Shape::Box { colour: "#ff00ff".into(), width: 4, from: [0, 0], to: [5, 5] };
        assert!(Markup { shapes: vec![odd] }.check(100, 50).is_err());
        let control = Shape::Text { colour: "red".into(), size: 20, at: [0, 0], text: "a\nb".into() };
        assert!(Markup { shapes: vec![control] }.check(100, 50).is_err());
    }

    #[test]
    fn words_are_read_literally() {
        assert_eq!(annotate_text("50% off"), "50%% off");
        assert_eq!(annotate_text("@/etc/passwd"), "\\@/etc/passwd");
        assert_eq!(annotate_text("a\\n"), "a\\\\n");
        assert_eq!(annotate_text("me@home"), "me@home");
    }

    #[test]
    fn blacked_out_parts_are_drawn_last() {
        let markup = Markup {
            shapes: vec![
                Shape::Redact { from: [5, 5], to: [1, 1] },
                Shape::Text { colour: "blue".into(), size: 20, at: [2, 3], text: "hi".into() },
            ],
        };
        assert!(draw_args(&markup, None).is_err(), "words need a font");
        let args = draw_args(&markup, Some(Path::new("/fonts/a.ttf"))).unwrap();
        let text = args.iter().position(|a| a == "-annotate").unwrap();
        let black = args.iter().position(|a| a == "rectangle 1,1 5,5").unwrap();
        assert!(text < black, "{args:?}");
        assert_eq!(args[text + 1], "+2+3");
    }
}
