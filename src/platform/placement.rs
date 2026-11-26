//! Where the popup goes within a monitor's work area. All values are physical
//! pixels, and the popup must be no larger than the area.

use windows::Win32::Foundation::{POINT, RECT};

pub fn centered(work: RECT, width: i32, height: i32) -> POINT {
    POINT {
        x: work.left + (work.right - work.left - width) / 2,
        y: work.top + (work.bottom - work.top - height) / 2,
    }
}

/// Below `anchor`, left edges aligned. Flips above it near the bottom, and
/// moves to one side if neither fits, so the anchor stays visible.
pub fn beside(anchor: RECT, work: RECT, width: i32, height: i32, gap: i32) -> POINT {
    let horizontal = |x: i32| x.clamp(work.left, work.right - width);
    let vertical = |y: i32| y.clamp(work.top, work.bottom - height);

    let below = anchor.bottom + gap;
    if below + height <= work.bottom {
        return POINT {
            x: horizontal(anchor.left),
            y: vertical(below),
        };
    }
    let above = anchor.top - gap - height;
    if above >= work.top {
        return POINT {
            x: horizontal(anchor.left),
            y: vertical(above),
        };
    }

    let right = anchor.right + gap;
    let left = anchor.left - gap - width;
    let x = if right + width <= work.right {
        right
    } else if left >= work.left {
        left
    } else {
        anchor.left
    };
    POINT {
        x: horizontal(x),
        y: vertical(anchor.top),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second monitor, left of and above the primary one.
    const WORK: RECT = RECT {
        left: -2000,
        top: -100,
        right: 0,
        bottom: 1000,
    };

    fn caret(x: i32, y: i32) -> RECT {
        RECT {
            left: x,
            top: y,
            right: x + 2,
            bottom: y + 20,
        }
    }

    fn place(anchor: RECT, height: i32) -> (i32, i32) {
        let point = beside(anchor, WORK, 700, height, 8);
        (point.x, point.y)
    }

    #[test]
    fn centers_within_the_work_area() {
        let point = centered(WORK, 700, 400);
        assert_eq!((point.x, point.y), (-1350, 250));
    }

    #[test]
    fn opens_below_the_caret() {
        assert_eq!(place(caret(-1500, 100), 400), (-1500, 128));
    }

    #[test]
    fn flips_above_near_the_bottom() {
        assert_eq!(place(caret(-1500, 900), 400), (-1500, 492));
    }

    #[test]
    fn stays_on_the_monitor_at_the_edges() {
        assert_eq!(place(caret(-300, 100), 400), (-700, 128));
        assert_eq!(place(caret(-2100, 100), 400), (-2000, 128));
        // A caret above the work area, in a partly hidden window.
        assert_eq!(place(caret(-1500, -300), 400), (-1500, -100));
    }

    #[test]
    fn moves_aside_when_neither_above_nor_below_fits() {
        // Right of the caret, then left when the right is too narrow.
        assert_eq!(place(caret(-1500, 450), 800), (-1490, 200));
        assert_eq!(place(caret(-400, 450), 800), (-1108, 200));
    }
}
