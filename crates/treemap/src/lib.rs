//! Squarified treemap layout.
//!
//! Turns a list of weights plus a target rectangle into one sub-rectangle per
//! weight, using the squarified algorithm (Bruls, Huizing & van Wijk, 2000).
//! Squarified layout keeps each cell as close to a square as possible, which
//! makes areas easy to compare by eye and leaves room for labels — unlike
//! slice-and-dice, which degenerates into thin slivers.
//!
//! This crate is pure geometry: it has no dependency on the scanner or the UI.
//! The caller maps its own items to weights, calls [`squarify`], and zips the
//! returned rectangles back onto its items (output order matches input order).

mod squarify;

pub use squarify::squarify;

/// An axis-aligned rectangle in treemap space.
///
/// Coordinates are `f64` for layout precision; the renderer casts to `f32` at
/// the boundary. `x`/`y` is the top-left corner, with `y` increasing downward
/// (screen convention).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    #[inline]
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }

    #[inline]
    pub fn area(&self) -> f64 {
        self.w * self.h
    }

    /// Length of the shorter side — the side a squarified row is laid along.
    #[inline]
    pub fn shorter_side(&self) -> f64 {
        self.w.min(self.h)
    }

    /// Longer-side / shorter-side ratio. `1.0` is a perfect square; larger is
    /// more elongated. A zero-area rectangle reports [`f64::INFINITY`].
    pub fn aspect_ratio(&self) -> f64 {
        let (w, h) = (self.w.abs(), self.h.abs());
        if w == 0.0 || h == 0.0 {
            f64::INFINITY
        } else {
            (w / h).max(h / w)
        }
    }

    /// Shrink by `d` on every side (for padding or a directory header strip),
    /// clamped so width/height never go negative.
    pub fn inset(&self, d: f64) -> Rect {
        Rect::new(
            self.x + d,
            self.y + d,
            (self.w - 2.0 * d).max(0.0),
            (self.h - 2.0 * d).max(0.0),
        )
    }
}
