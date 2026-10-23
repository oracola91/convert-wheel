//! Wheel geometry and the formats on show, shared by the trigger and the drop target.

use std::f64::consts::TAU;
use std::sync::Mutex;

/// Window size in points. src/wheel.js draws the rings with the same numbers.
pub const SIZE: f64 = 340.0;
pub const R_IN: f64 = 62.0;
pub const R_OUT: f64 = 160.0;

/// Formats on the wheel right now, in drawing order.
#[derive(Default)]
pub struct Shown(pub Mutex<Vec<&'static str>>);

/// Segment under a point given relative to the centre, with y pointing up. Segment 0
/// starts at 12 o'clock and they go clockwise, as in the SVG. The hole in the middle
/// and the space outside the ring mean "no format".
pub fn segment_at(dx: f64, dy: f64, n: usize) -> Option<usize> {
    let r = dx.hypot(dy);
    if n == 0 || r < R_IN || r > R_OUT + 10.0 {
        return None;
    }
    let mut a = dx.atan2(dy);
    if a < 0.0 {
        a += TAU;
    }
    Some(((a / TAU) * n as f64) as usize % n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_go_clockwise_from_the_top() {
        let r = (R_IN + R_OUT) / 2.0;
        assert_eq!(segment_at(0.0, r, 4), Some(0)); // up
        assert_eq!(segment_at(r, 0.0, 4), Some(1)); // right
        assert_eq!(segment_at(0.0, -r, 4), Some(2)); // down
        assert_eq!(segment_at(-r, 0.0, 4), Some(3)); // left
    }

    #[test]
    fn middle_and_outside_are_no_format() {
        assert_eq!(segment_at(0.0, 0.0, 4), None);
        assert_eq!(segment_at(R_OUT + 20.0, 0.0, 4), None);
        assert_eq!(segment_at(0.0, 100.0, 0), None);
    }
}
