//! Dump the canonical squarified example (areas 6,6,4,3,2,2,1) as an SVG, for
//! eyeballing the layout.
//!
//! Run:  cargo run -p treemap --example canonical_svg > /tmp/treemap.svg
//! then open /tmp/treemap.svg in a browser or image viewer.
//!
//! Note: the raw strings use `r##"..."##` because the SVG hex colours contain
//! the sequence `"#`, which would close a plain `r#"..."#` string early.

use treemap::{squarify, Rect};

fn main() {
    let (width, height) = (600.0, 400.0);
    let bounds = Rect::new(0.0, 0.0, width, height);
    let weights = [6u64, 6, 4, 3, 2, 2, 1];
    let rects = squarify(&weights, bounds);

    // A muted, readable dark palette (the project's aesthetic direction).
    let palette = [
        "#e06c75", "#61afef", "#98c379", "#e5c07b", "#c678dd", "#56b6c2", "#abb2bf",
    ];

    let mut svg = String::new();
    svg.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">"##
    ));
    svg.push_str(r##"<rect width="100%" height="100%" fill="#282c34"/>"##);
    for (i, r) in rects.iter().enumerate() {
        let color = palette[i % palette.len()];
        svg.push_str(&format!(
            r##"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="{color}" stroke="#1b1f23" stroke-width="2"/>"##,
            r.x, r.y, r.w, r.h
        ));
        svg.push_str(&format!(
            r##"<text x="{:.2}" y="{:.2}" fill="#1b1f23" font-family="monospace" font-size="18" font-weight="bold">{}</text>"##,
            r.x + 8.0,
            r.y + 24.0,
            weights[i]
        ));
    }
    svg.push_str("</svg>");

    println!("{svg}");
}
