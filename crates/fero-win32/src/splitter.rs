//! A draggable bar that divides an area into two panes.
//!
//! [`Splitter`] only does the arithmetic: where the bar is, whether a mouse
//! position is on it, and where it goes while being dragged. The window that
//! owns the panes forwards its mouse messages and lays out the panes with
//! [`Splitter::split`]. Keeping it free of Win32 calls means it can be unit
//! tested on any platform.

/// Which way the bar runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// A vertical bar with one pane on the left and one on the right.
    Vertical,
    /// A horizontal bar with one pane above and one below.
    Horizontal,
}

/// A rectangle in client coordinates; `right` and `bottom` are exclusive, as
/// in Win32's `RECT`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub left: i32,
    /// Top edge.
    pub top: i32,
    /// One past the right edge.
    pub right: i32,
    /// One past the bottom edge.
    pub bottom: i32,
}

impl Rect {
    /// A rectangle from its position and size.
    pub fn new(left: i32, top: i32, width: i32, height: i32) -> Self {
        Rect {
            left,
            top,
            right: left + width,
            bottom: top + height,
        }
    }

    /// Width, never negative.
    pub fn width(&self) -> i32 {
        (self.right - self.left).max(0)
    }

    /// Height, never negative.
    pub fn height(&self) -> i32 {
        (self.bottom - self.top).max(0)
    }

    /// True if the point lies inside.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// The position of a splitter bar and the state of a drag.
///
/// All sizes are in physical pixels. When the window moves to a monitor with
/// a different DPI, call [`rescale`](Self::rescale).
#[derive(Debug, Clone)]
pub struct Splitter {
    orientation: Orientation,
    /// Size of the first pane (left or top), before clamping to the area.
    position: i32,
    /// Thickness of the bar.
    thickness: i32,
    /// Smallest size either pane may be dragged to.
    min_pane: i32,
    /// While dragging: where in the bar the mouse grabbed it.
    grab: Option<i32>,
}

impl Splitter {
    /// A splitter whose first pane is `position` pixels wide (or high).
    pub fn new(orientation: Orientation, position: i32, thickness: i32, min_pane: i32) -> Self {
        Splitter {
            orientation,
            position,
            thickness,
            min_pane,
            grab: None,
        }
    }

    /// Which way the bar runs.
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    /// Size of the first pane as last set, in pixels.
    pub fn position(&self) -> i32 {
        self.position
    }

    /// True while the bar is being dragged.
    pub fn is_dragging(&self) -> bool {
        self.grab.is_some()
    }

    /// Split `area` into the first pane, the bar and the second pane.
    ///
    /// The first pane keeps its size when the area grows or shrinks, as far
    /// as the minimum size of the second pane allows.
    pub fn split(&self, area: Rect) -> (Rect, Rect, Rect) {
        let first = self.clamped(area);
        match self.orientation {
            Orientation::Vertical => {
                let bar = area.left + first;
                (
                    Rect { right: bar, ..area },
                    Rect {
                        left: bar,
                        right: bar + self.thickness,
                        ..area
                    },
                    Rect {
                        left: (bar + self.thickness).min(area.right),
                        ..area
                    },
                )
            }
            Orientation::Horizontal => {
                let bar = area.top + first;
                (
                    Rect {
                        bottom: bar,
                        ..area
                    },
                    Rect {
                        top: bar,
                        bottom: bar + self.thickness,
                        ..area
                    },
                    Rect {
                        top: (bar + self.thickness).min(area.bottom),
                        ..area
                    },
                )
            }
        }
    }

    /// True if the point is on the bar.
    pub fn hit(&self, area: Rect, x: i32, y: i32) -> bool {
        self.split(area).1.contains(x, y)
    }

    /// Start dragging if the point is on the bar. Returns true if it was, so
    /// the caller should capture the mouse.
    pub fn begin_drag(&mut self, area: Rect, x: i32, y: i32) -> bool {
        let (_, bar, _) = self.split(area);
        if !bar.contains(x, y) {
            return false;
        }
        self.grab = Some(match self.orientation {
            Orientation::Vertical => x - bar.left,
            Orientation::Horizontal => y - bar.top,
        });
        true
    }

    /// Move the bar with the mouse. Returns true if it moved, so the caller
    /// should lay out the panes again.
    pub fn drag(&mut self, area: Rect, x: i32, y: i32) -> bool {
        let Some(grab) = self.grab else {
            return false;
        };
        let wanted = match self.orientation {
            Orientation::Vertical => x - area.left - grab,
            Orientation::Horizontal => y - area.top - grab,
        };
        let old = self.clamped(area);
        self.position = wanted;
        // Store the clamped value, so that dragging past the end and back
        // does not leave the bar stuck at the end.
        self.position = self.clamped(area);
        self.position != old
    }

    /// Stop dragging (on mouse button up or lost capture).
    pub fn end_drag(&mut self) {
        self.grab = None;
    }

    /// Scale the position and sizes from one DPI to another.
    pub fn rescale(&mut self, old_dpi: u32, new_dpi: u32) {
        let scale =
            |value: i32| (i64::from(value) * i64::from(new_dpi) / i64::from(old_dpi.max(1))) as i32;
        self.position = scale(self.position);
        self.thickness = scale(self.thickness).max(1);
        self.min_pane = scale(self.min_pane);
    }

    /// The first pane's size, kept between the minimum pane sizes.
    fn clamped(&self, area: Rect) -> i32 {
        let length = match self.orientation {
            Orientation::Vertical => area.width(),
            Orientation::Horizontal => area.height(),
        };
        let max = (length - self.thickness - self.min_pane).max(0);
        let min = self.min_pane.min(max);
        self.position.clamp(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::new(0, 0, 1000, 500)
    }

    #[test]
    fn splits_vertically() {
        let splitter = Splitter::new(Orientation::Vertical, 300, 6, 50);
        let (left, bar, right) = splitter.split(area());
        assert_eq!(left, Rect::new(0, 0, 300, 500));
        assert_eq!(bar, Rect::new(300, 0, 6, 500));
        assert_eq!(right, Rect::new(306, 0, 694, 500));
    }

    #[test]
    fn splits_horizontally_inside_an_offset_area() {
        let splitter = Splitter::new(Orientation::Horizontal, 100, 4, 20);
        let area = Rect::new(10, 40, 200, 300);
        let (top, bar, bottom) = splitter.split(area);
        assert_eq!(top, Rect::new(10, 40, 200, 100));
        assert_eq!(bar, Rect::new(10, 140, 200, 4));
        assert_eq!(bottom, Rect::new(10, 144, 200, 196));
    }

    #[test]
    fn keeps_the_second_pane_visible_when_the_area_shrinks() {
        let splitter = Splitter::new(Orientation::Vertical, 900, 6, 50);
        let (left, _, right) = splitter.split(Rect::new(0, 0, 400, 100));
        assert_eq!(left.width(), 400 - 6 - 50);
        assert_eq!(right.width(), 50);
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        let splitter = Splitter::new(Orientation::Vertical, 300, 6, 50);
        let (left, bar, right) = splitter.split(Rect::new(0, 0, 3, 3));
        assert_eq!(left.width(), 0);
        assert!(bar.width() <= 6);
        assert_eq!(right.width(), 0);
    }

    #[test]
    fn drags_with_the_grab_offset() {
        let mut splitter = Splitter::new(Orientation::Vertical, 300, 6, 50);
        assert!(!splitter.begin_drag(area(), 100, 10), "not on the bar");
        assert!(splitter.begin_drag(area(), 302, 10));
        assert!(splitter.drag(area(), 402, 10));
        assert_eq!(splitter.position(), 400);
        // Past the end it stops at the minimum size of the other pane.
        splitter.drag(area(), 5000, 10);
        assert_eq!(splitter.position(), 1000 - 6 - 50);
        // And comes straight back when the mouse does.
        splitter.drag(area(), 502, 10);
        assert_eq!(splitter.position(), 500);
        splitter.end_drag();
        assert!(!splitter.drag(area(), 700, 10), "no drag in progress");
    }

    #[test]
    fn rescales_for_a_new_dpi() {
        let mut splitter = Splitter::new(Orientation::Vertical, 300, 6, 50);
        splitter.rescale(96, 144);
        let (left, bar, _) = splitter.split(Rect::new(0, 0, 2000, 100));
        assert_eq!(left.width(), 450);
        assert_eq!(bar.width(), 9);
    }
}
