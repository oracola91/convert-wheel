//! Wheel geometry and state, shared by the trigger and the drop target.

use std::f64::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Mutex;

use serde::Serialize;

/// Window size in points. src/wheel.js draws the rings with the same numbers.
pub const SIZE: f64 = 340.0;
pub const R_IN: f64 = 62.0;
pub const R_OUT: f64 = 160.0;

#[derive(Default)]
pub struct WheelState {
    /// Formats on the wheel right now, in drawing order.
    pub formats: Mutex<Vec<&'static str>>,
    /// Bumped each time the wheel opens, so a late close can't hide a newer wheel.
    pub generation: AtomicU64,
    /// Open and not yet closing.
    pub open: AtomicBool,
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
}
