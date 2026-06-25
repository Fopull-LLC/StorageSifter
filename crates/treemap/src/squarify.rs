//! The squarified treemap algorithm.
//!
//! Children are laid out in rows along the shorter side of the remaining
//! rectangle. We greedily extend the current row while doing so keeps the
//! worst (largest) aspect ratio in that row from getting worse; when the next
//! item would make it worse, the row is fixed in place, its strip is carved off
//! the remaining rectangle, and a new row begins. This is the standard approach
//! from Bruls, Huizing & van Wijk (2000).

use crate::Rect;

/// Lay out `weights` within `bounds`, returning one [`Rect`] per weight in the
/// **same order as the input** (so callers can zip results back onto their
/// items).
///
/// Behaviour at the edges:
/// - A weight of `0` yields a zero-area rectangle and does not affect the layout
///   of the others.
/// - If every weight is `0`, or `bounds` has no area, every rectangle is
///   zero-area.
/// - Areas are faithfully proportional to weights: the returned rectangles tile
///   `bounds` exactly (subject to floating-point rounding). Culling tiny cells
///   is the renderer's job, not the layout's — areas here stay truthful.
pub fn squarify(weights: &[u64], bounds: Rect) -> Vec<Rect> {
    let n = weights.len();
    // Default every cell to a zero-area rect at the origin of `bounds`.
    let mut out = vec![Rect::new(bounds.x, bounds.y, 0.0, 0.0); n];

    let total: u128 = weights.iter().map(|&w| w as u128).sum();
    let area = bounds.area();
    if total == 0 || area <= 0.0 {
        return out;
    }

    // Positive-weight items, largest first (ties broken by original index so the
    // layout is deterministic).
    let mut order: Vec<usize> = (0..n).filter(|&i| weights[i] > 0).collect();
    order.sort_by(|&a, &b| weights[b].cmp(&weights[a]).then(a.cmp(&b)));

    // Scale weights to areas so the whole set tiles `bounds` exactly.
    let scale = area / total as f64;
    let areas: Vec<f64> = weights.iter().map(|&w| w as f64 * scale).collect();

    let mut remaining = bounds;
    let mut idx = 0;
    while idx < order.len() {
        // The row is laid along the shorter side of the remaining rectangle.
        let side = remaining.shorter_side();

        let mut row: Vec<usize> = vec![order[idx]];
        idx += 1;
        while idx < order.len() {
            let candidate = order[idx];
            // Extend the row only while it does not worsen the aspect ratio.
            if row_worst(&row, &areas, side, Some(candidate)) <= row_worst(&row, &areas, side, None)
            {
                row.push(candidate);
                idx += 1;
            } else {
                break;
            }
        }

        layout_row(&row, &areas, side, &mut remaining, &mut out);
    }

    out
}

/// Worst (largest) aspect ratio of the cells in `row`, optionally including one
/// `extra` candidate, when laid along a side of length `side`. Lower is better.
fn row_worst(row: &[usize], areas: &[f64], side: f64, extra: Option<usize>) -> f64 {
    let mut sum = 0.0;
    let mut rmin = f64::INFINITY;
    let mut rmax: f64 = 0.0;
    let mut include = |a: f64| {
        sum += a;
        rmin = rmin.min(a);
        rmax = rmax.max(a);
    };
    for &i in row {
        include(areas[i]);
    }
    if let Some(e) = extra {
        include(areas[e]);
    }

    if sum <= 0.0 || side <= 0.0 || rmin <= 0.0 {
        return f64::INFINITY;
    }
    let s2 = sum * sum;
    let side2 = side * side;
    // max( side^2 * rmax / s^2 , s^2 / (side^2 * rmin) )
    (side2 * rmax / s2).max(s2 / (side2 * rmin))
}

/// Place `row` as a strip along the shorter side of `remaining`, write each
/// cell into `out`, and shrink `remaining` by the strip's thickness.
fn layout_row(row: &[usize], areas: &[f64], side: f64, remaining: &mut Rect, out: &mut [Rect]) {
    let row_sum: f64 = row.iter().map(|&i| areas[i]).sum();
    if row_sum <= 0.0 || side <= 0.0 {
        return;
    }
    let thickness = row_sum / side;

    if remaining.w <= remaining.h {
        // Shorter side is the width: a horizontal strip across the top, cells
        // laid left-to-right, thickness consumed from the height.
        let mut x = remaining.x;
        let y = remaining.y;
        for &i in row {
            let cw = areas[i] / thickness;
            out[i] = Rect::new(x, y, cw, thickness);
            x += cw;
        }
        remaining.y += thickness;
        remaining.h -= thickness;
    } else {
        // Shorter side is the height: a vertical strip down the left, cells
        // laid top-to-bottom, thickness consumed from the width.
        let x = remaining.x;
        let mut y = remaining.y;
        for &i in row {
            let ch = areas[i] / thickness;
            out[i] = Rect::new(x, y, thickness, ch);
            y += ch;
        }
        remaining.x += thickness;
        remaining.w -= thickness;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-9;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-6
    }

    fn assert_rect(r: Rect, x: f64, y: f64, w: f64, h: f64) {
        assert!(
            (r.x - x).abs() <= EPS
                && (r.y - y).abs() <= EPS
                && (r.w - w).abs() <= EPS
                && (r.h - h).abs() <= EPS,
            "expected ({x},{y},{w},{h}), got {r:?}"
        );
    }

    #[test]
    fn single_item_fills_bounds() {
        let r = squarify(&[5], Rect::new(0.0, 0.0, 10.0, 8.0));
        assert_eq!(r.len(), 1);
        assert_rect(r[0], 0.0, 0.0, 10.0, 8.0);
    }

    #[test]
    fn two_equal_items_split_in_half() {
        // A 2x1 box splits into two unit squares, side by side.
        let r = squarify(&[1, 1], Rect::new(0.0, 0.0, 2.0, 1.0));
        assert_rect(r[0], 0.0, 0.0, 1.0, 1.0);
        assert_rect(r[1], 1.0, 0.0, 1.0, 1.0);
    }

    #[test]
    fn four_equal_items_form_a_grid() {
        // A 2x2 box splits into a 2x2 grid of unit squares.
        let r = squarify(&[1, 1, 1, 1], Rect::new(0.0, 0.0, 2.0, 2.0));
        assert_rect(r[0], 0.0, 0.0, 1.0, 1.0);
        assert_rect(r[1], 1.0, 0.0, 1.0, 1.0);
        assert_rect(r[2], 0.0, 1.0, 1.0, 1.0);
        assert_rect(r[3], 1.0, 1.0, 1.0, 1.0);
    }

    #[test]
    fn canonical_example_is_proportional_contained_and_squarish() {
        // The worked example from the paper: areas 6,6,4,3,2,2,1 in a 6x4 box.
        let bounds = Rect::new(0.0, 0.0, 6.0, 4.0);
        let weights = [6u64, 6, 4, 3, 2, 2, 1];
        let rects = squarify(&weights, bounds);

        let total_w: u64 = weights.iter().sum();
        let total_area = bounds.area();

        // Each cell's area is proportional to its weight.
        for (i, &w) in weights.iter().enumerate() {
            let expected = w as f64 / total_w as f64 * total_area;
            assert!(
                approx(rects[i].area(), expected),
                "cell {i}: area {} != expected {expected}",
                rects[i].area()
            );
        }

        // Every cell sits inside the bounds and the set tiles it exactly.
        let mut covered = 0.0;
        for r in &rects {
            assert!(r.x >= -1e-6 && r.y >= -1e-6, "cell escaped origin: {r:?}");
            assert!(r.x + r.w <= bounds.w + 1e-6, "cell overflows width: {r:?}");
            assert!(r.y + r.h <= bounds.h + 1e-6, "cell overflows height: {r:?}");
            covered += r.area();
        }
        assert!(
            approx(covered, total_area),
            "tiling gap: {covered} vs {total_area}"
        );

        // The point of "squarified": aspect ratios stay low. Slice-and-dice on
        // this example would produce far more elongated cells.
        for r in &rects {
            assert!(r.aspect_ratio() <= 4.0, "cell too elongated: {r:?}");
        }
    }

    #[test]
    fn zero_weights_are_handled() {
        // Nothing to lay out -> empty.
        assert!(squarify(&[], Rect::new(0.0, 0.0, 10.0, 10.0)).is_empty());

        // All zero -> all zero-area, but still one rect per input.
        let all_zero = squarify(&[0, 0, 0], Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(all_zero.len(), 3);
        assert!(all_zero.iter().all(|r| r.area() == 0.0));

        // Mixed: zero-weight items get zero area; the rest stay proportional.
        let mixed = squarify(&[0, 4, 0, 4], Rect::new(0.0, 0.0, 4.0, 2.0));
        assert_eq!(mixed[0].area(), 0.0);
        assert_eq!(mixed[2].area(), 0.0);
        assert!(approx(mixed[1].area(), 4.0));
        assert!(approx(mixed[3].area(), 4.0));
    }
}
