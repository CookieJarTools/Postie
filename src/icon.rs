//! The Postie mark: a white envelope on an indigo tile, drawn from distance fields so it stays crisp
//! at every tray size without shipping bitmaps. The SVG is the same drawing for desktop entries.

const ACCENT: [f32; 3] = [0x5e as f32, 0x6a as f32, 0xd2 as f32];
const WHITE: [f32; 3] = [0xf7 as f32, 0xf8 as f32, 0xf8 as f32];
const DOT: [f32; 3] = [0xf2 as f32, 0xc9 as f32, 0x4c as f32];
const RING: [f32; 3] = [0x08 as f32, 0x09 as f32, 0x0a as f32];

pub const SVG: &str = include_str!("../assets/icon.svg");

fn rounded_box(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - hw + r;
    let qy = (py - cy).abs() - hh + r;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

fn segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (pax, pay, bax, bay) = (px - ax, py - ay, bx - ax, by - ay);
    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
    ((pax - bax * h).powi(2) + (pay - bay * h).powi(2)).sqrt()
}

fn coverage(distance: f32, size: f32) -> f32 {
    (0.5 - distance * size).clamp(0.0, 1.0)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Straight (not premultiplied) RGBA, `size` × `size`.
pub fn rgba(size: u32, unread: bool) -> Vec<u8> {
    let s = size as f32;
    let stroke = (3.4_f32 / 64.0).max(1.4 / s);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (u, v) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let tile = coverage(rounded_box(u, v, 0.5, 0.5, 29.0 / 64.0, 29.0 / 64.0, 14.0 / 64.0), s);
            let body = rounded_box(u, v, 0.5, 32.5 / 64.0, 16.0 / 64.0, 11.5 / 64.0, 2.5 / 64.0).abs();
            let flap = segment(u, v, 17.5 / 64.0, 23.5 / 64.0, 0.5, 34.5 / 64.0).min(segment(
                u,
                v,
                0.5,
                34.5 / 64.0,
                46.5 / 64.0,
                23.5 / 64.0,
            ));
            let line = coverage(body.min(flap) - stroke / 2.0, s);
            let mut color = mix(ACCENT, WHITE, line);
            let mut alpha = tile;
            if unread {
                let (dx, dy) = (u - 0.8, v - 0.2);
                let d = (dx * dx + dy * dy).sqrt();
                let ring = coverage(d - 0.2, s);
                let dot = coverage(d - 0.14, s);
                color = mix(mix(color, RING, ring), DOT, dot);
                alpha = alpha.max(ring);
            }
            out.extend([color[0] as u8, color[1] as u8, color[2] as u8, (alpha * 255.0) as u8]);
        }
    }
    out
}

/// ARGB32 in network byte order, as StatusNotifierItem wants.
pub fn argb(size: u32, unread: bool) -> Vec<u8> {
    rgba(size, unread).as_chunks::<4>().0.iter().flat_map(|p| [p[3], p[0], p[1], p[2]]).collect()
}
