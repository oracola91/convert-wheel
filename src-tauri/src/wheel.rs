//! Wheel geometry and state, shared by the trigger and the drop target.

use std::f64::consts::TAU;
use std::sync::{Mutex, MutexGuard};

use serde::Serialize;

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

#[cfg(test)]
mod tests {
    use super::*;

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
