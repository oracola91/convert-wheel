//! Wheel geometry and state, shared by the trigger and the drop target.

use std::f64::consts::TAU;
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

/// Window size in points. src/wheel.js draws the rings with the same numbers.
pub const SIZE: f64 = 340.0;
pub const R_IN: f64 = 62.0;
pub const R_OUT: f64 = 160.0;

/// A poisoned lock still holds valid data here, so carry on with it.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Which wheel opens: Shift while dragging gives the formats, Option+Shift (Alt+Shift
/// on Windows and Linux) the tools, Control+Shift the actions. While it is open the
/// keys can change it; see KeyFollower.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Which {
    #[default]
    Formats,
    Tools,
    Actions,
}

impl Which {
    /// The keys held with Shift: Option (Alt) or Control. Control wins when both are.
    pub fn from_keys(option: bool, control: bool) -> Self {
        match (option, control) {
            (_, true) => Which::Actions,
            (true, false) => Which::Tools,
            (false, false) => Which::Formats,
        }
    }
}

/// Follows the keys while a wheel is open, so one drag can move between the wheels:
/// Option (Alt) gives the tools, Control the actions, Shift alone the formats again.
/// With none of them held the wheel stays as it is, so the keys can be let go while
/// the file travels to a segment. A change counts once it has held for `settle` polls,
/// which also stops two keys let go a moment apart from passing through another wheel.
#[derive(Default)]
pub struct KeyFollower {
    wanted: Option<Which>,
    polls: u8,
}

impl KeyFollower {
    /// One poll of the keys while wheel `current` is open: the wheel to switch to,
    /// once, when the keys have asked for another for long enough.
    pub fn poll(&mut self, current: Which, settle: u8, shift: bool, option: bool, control: bool) -> Option<Which> {
        let asked = if option || control {
            Some(Which::from_keys(option, control))
        } else {
            shift.then_some(Which::Formats)
        };
        match asked {
            Some(which) if which != current => {
                self.polls = if self.wanted == Some(which) { self.polls.saturating_add(1) } else { 1 };
                self.wanted = Some(which);
                if self.polls >= settle {
                    *self = KeyFollower::default();
                    return Some(which);
                }
            }
            _ => *self = KeyFollower::default(),
        }
        None
    }
}

#[derive(Default)]
pub struct WheelState {
    /// Formats, or tools ("tool:half"), on the wheel right now, in drawing order.
    pub formats: Mutex<Vec<&'static str>>,
    /// The wheel that is open, or was last.
    pub which: Mutex<Which>,
    /// The dragged files, once known: a switch to another wheel needs them again.
    pub paths: Mutex<Vec<String>>,
    phase: Mutex<Phase>,
}

#[derive(Default)]
struct Phase {
    /// Bumped each time the wheel opens, so a late close can't hide a newer wheel.
    generation: u64,
    /// Open and not yet closing.
    open: bool,
    /// Opened before the dragged files were known (Windows, Linux).
    pending: bool,
}

impl WheelState {
    /// Marks a new wheel open and returns its generation.
    pub fn open(&self, pending: bool) -> u64 {
        let mut p = lock(&self.phase);
        p.generation += 1;
        p.open = true;
        p.pending = pending;
        p.generation
    }

    /// Marks the open wheel as closing and returns its generation. With `expected`,
    /// only that wheel: a timer started for an earlier wheel then closes nothing.
    /// Checking and closing happen under one lock, so a wheel that opens in between
    /// can't be caught by an old timer.
    pub fn close(&self, expected: Option<u64>) -> Option<u64> {
        let mut p = lock(&self.phase);
        if !p.open || expected.is_some_and(|g| g != p.generation) {
            return None;
        }
        p.open = false;
        Some(p.generation)
    }

    pub fn generation(&self) -> u64 {
        lock(&self.phase).generation
    }

    /// A wheel is open and not closing.
    pub fn is_open(&self) -> bool {
        lock(&self.phase).open
    }

    /// No newer wheel has opened since `generation`.
    pub fn is_current(&self, generation: u64) -> bool {
        lock(&self.phase).generation == generation
    }

    /// The dragged files reached the wheel.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn files_known(&self) {
        lock(&self.phase).pending = false;
    }

    /// Wheel `generation` is still open and still waiting for its files.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn still_waiting(&self, generation: u64) -> bool {
        let p = lock(&self.phase);
        p.open && p.pending && p.generation == generation
    }
}

/// What is under the cursor, as the page needs it to light things up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Hover {
    pub segment: Option<usize>,
    pub hub: bool,
}

impl Hover {
    pub const NONE: Hover = Hover { segment: None, hub: false };
}

/// Hover state for a point given relative to the centre, with y pointing up. Segment 0
/// starts at 12 o'clock and they go clockwise, as in the SVG. The hub in the middle
/// means "cancel"; the corners outside the ring mean nothing.
pub fn hover_at(dx: f64, dy: f64, n: usize) -> Hover {
    let r = dx.hypot(dy);
    if r < R_IN {
        return Hover { segment: None, hub: true };
    }
    if n == 0 || r > R_OUT + 10.0 {
        return Hover::NONE;
    }
    let mut a = dx.atan2(dy);
    if a < 0.0 {
        a += TAU;
    }
    Hover { segment: Some(((a / TAU) * n as f64) as usize % n), hub: false }
}

/// A word on a segment, with its icon if it has one: its box in page points from the
/// window's top left, as src/wheel.js measures it. On macOS the glass gets a soft
/// frosted patch there, so the word reads over anything while the rest stays clear.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct WordBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl WordBox {
    /// Inside the window and no bigger than a segment; the page sends nothing else.
    pub fn valid(&self) -> bool {
        let inside = |v: f64| v.is_finite() && (0.0..=SIZE).contains(&v);
        inside(self.x) && inside(self.y) && inside(self.x + self.width) && inside(self.y + self.height)
            && self.width > 0.0 && self.height > 0.0 && self.width <= R_OUT && self.height <= R_OUT
    }
}

/// How far the full-strength patch reaches past the word, how far it then fades out,
/// and how strongly the frosted material shows in it. Points, and 0 to 1.
const PATCH_PAD: f64 = 4.0;
const PATCH_FADE: f64 = 10.0;
const PATCH: f64 = 0.92;

/// How strongly the frosted material shows at `(x, y)`, page points from the top left:
/// `blur` over the whole disc, and the patch strength around each word.
pub fn glass_alpha(x: f64, y: f64, blur: f64, words: &[WordBox]) -> f64 {
    let c = SIZE / 2.0;
    // A smooth edge about one Retina pixel wide.
    let disc = ((R_OUT - (x - c).hypot(y - c)) * 2.0 + 0.5).clamp(0.0, 1.0);
    if disc == 0.0 {
        return 0.0;
    }
    let patch = words.iter().map(|w| PATCH * patch_cover(x, y, w)).fold(0.0, f64::max);
    disc * (1.0 - (1.0 - blur.clamp(0.0, 1.0)) * (1.0 - patch))
}

/// 1 inside a pill around the word, fading smoothly to 0 over PATCH_FADE outside it.
fn patch_cover(x: f64, y: f64, w: &WordBox) -> f64 {
    let (hx, hy) = (w.width / 2.0 + PATCH_PAD, w.height / 2.0 + PATCH_PAD);
    let r = hx.min(hy);
    // How far the point lies outside the pill (negative inside).
    let qx = (x - (w.x + w.width / 2.0)).abs() - (hx - r);
    let qy = (y - (w.y + w.height / 2.0)).abs() - (hy - r);
    let outside = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
    let t = (outside / PATCH_FADE).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glass_is_frosted_behind_words_and_follows_the_slider_elsewhere() {
        let word = WordBox { x: 150.0, y: 60.0, width: 40.0, height: 12.0 };
        let c = SIZE / 2.0;
        // Clear wheel: nothing behind the hub, the patch behind the word, fading out.
        assert_eq!(glass_alpha(c, c, 0.0, &[word]), 0.0);
        assert!((glass_alpha(170.0, 66.0, 0.0, &[word]) - PATCH).abs() < 1e-9);
        assert!((glass_alpha(150.0 - PATCH_PAD, 66.0, 0.0, &[word]) - PATCH).abs() < 1e-9, "full strength to the pad");
        let halfway = glass_alpha(150.0 - PATCH_PAD - PATCH_FADE / 2.0, 66.0, 0.0, &[word]);
        assert!(halfway > 0.3 * PATCH && halfway < 0.7 * PATCH, "{halfway}");
        assert_eq!(glass_alpha(150.0 - PATCH_PAD - PATCH_FADE, 66.0, 0.0, &[word]), 0.0);
        // The slider's blur everywhere on the disc; a patch only adds to it.
        assert!((glass_alpha(c, c, 0.4, &[word]) - 0.4).abs() < 1e-9);
        assert!(glass_alpha(170.0, 66.0, 0.4, &[word]) > PATCH);
        assert!((glass_alpha(170.0, 66.0, 1.0, &[word]) - 1.0).abs() < 1e-9);
        // Nothing outside the disc, even at full blur.
        assert_eq!(glass_alpha(2.0, 2.0, 1.0, &[]), 0.0);
    }

    #[test]
    fn word_boxes_outside_the_window_or_too_big_are_refused() {
        let ok = WordBox { x: 10.0, y: 10.0, width: 60.0, height: 20.0 };
        assert!(ok.valid());
        assert!(!WordBox { x: -1.0, ..ok }.valid());
        assert!(!WordBox { x: SIZE - 10.0, ..ok }.valid());
        assert!(!WordBox { width: f64::NAN, ..ok }.valid());
        assert!(!WordBox { height: 0.0, ..ok }.valid());
        assert!(!WordBox { x: 0.0, width: R_OUT + 1.0, ..ok }.valid());
    }

    fn seg(dx: f64, dy: f64, n: usize) -> Option<usize> {
        hover_at(dx, dy, n).segment
    }

    #[test]
    fn segments_go_clockwise_from_the_top() {
        let r = (R_IN + R_OUT) / 2.0;
        assert_eq!(seg(0.0, r, 4), Some(0)); // up
        assert_eq!(seg(r, 0.0, 4), Some(1)); // right
        assert_eq!(seg(0.0, -r, 4), Some(2)); // down
        assert_eq!(seg(-r, 0.0, 4), Some(3)); // left
    }

    #[test]
    fn hub_and_outside_are_no_format() {
        assert_eq!(hover_at(0.0, 0.0, 4), Hover { segment: None, hub: true });
        assert_eq!(hover_at(R_OUT + 20.0, 0.0, 4), Hover::NONE);
        assert_eq!(hover_at(0.0, 100.0, 0), Hover::NONE);
    }

    #[test]
    fn an_old_timer_cannot_close_a_newer_wheel() {
        let state = WheelState::default();
        let first = state.open(false);
        let second = state.open(false);
        assert_eq!(state.close(Some(first)), None);
        assert_eq!(state.close(Some(second)), Some(second));
        // Closing twice does nothing.
        assert_eq!(state.close(None), None);
    }

    /// Polls `keys` (shift, option, control) one after another while `current` is open
    /// and returns what each poll asked to switch to.
    fn follow(current: Which, keys: &[(bool, bool, bool)]) -> Vec<Option<Which>> {
        let mut f = KeyFollower::default();
        keys.iter().map(|&(s, o, c)| f.poll(current, 2, s, o, c)).collect()
    }

    #[test]
    fn the_keys_move_between_the_wheels() {
        let (shift, option_shift, control) = ((true, false, false), (true, true, false), (false, false, true));
        // Option joins Shift: the tools, once it has held for two polls.
        assert_eq!(follow(Which::Formats, &[option_shift, option_shift]), [None, Some(Which::Tools)]);
        // Control, with or without Shift: the actions.
        assert_eq!(follow(Which::Tools, &[control, control]), [None, Some(Which::Actions)]);
        // Shift alone again: the formats.
        assert_eq!(follow(Which::Actions, &[shift, shift]), [None, Some(Which::Formats)]);
        // The keys the open wheel already has ask for nothing.
        assert_eq!(follow(Which::Tools, &[option_shift, option_shift, option_shift]), [None, None, None]);
    }

    #[test]
    fn letting_go_of_the_keys_keeps_the_wheel() {
        let none = (false, false, false);
        assert_eq!(follow(Which::Tools, &[none, none, none]), [None, None, None]);
        // Option let go a poll before Shift: Shift alone for one poll isn't enough.
        assert_eq!(follow(Which::Tools, &[(true, false, false), none, none]), [None, None, None]);
    }

    #[test]
    fn a_switch_happens_once() {
        let mut f = KeyFollower::default();
        let option_shift = |f: &mut KeyFollower, current| f.poll(current, 2, true, true, false);
        assert_eq!(option_shift(&mut f, Which::Formats), None);
        assert_eq!(option_shift(&mut f, Which::Formats), Some(Which::Tools));
        // The wheel is the tools now: holding on asks for nothing more.
        assert_eq!(option_shift(&mut f, Which::Tools), None);
    }

    #[test]
    fn a_pending_wheel_stops_waiting_once_files_arrive() {
        let state = WheelState::default();
        let g = state.open(true);
        assert!(state.still_waiting(g));
        state.files_known();
        assert!(!state.still_waiting(g));
    }
}
