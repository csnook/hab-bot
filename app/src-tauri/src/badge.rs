//! The tray icon's picture: a coloured disc with the count in white, drawn
//! at run time as raw pixels.
//!
//! Why drawn rather than a set of icon files: a StatusNotifierItem can carry
//! its icon as pixel data (`IconPixmap`, ARGB32), so the badge needs no file
//! on disk, no icon theme path and no PNG encoder, and works the same on
//! Plasma and under the GNOME AppIndicator extension. Several sizes are sent
//! and the host picks the nearest. Nothing here has been looked at on a real
//! panel; the tests check the pixels, not how they look.
//!
//! With nothing to count the icon is a plain ring, with no number.

use crate::tray_model::{Badge, Tone};

/// The sizes offered to the panel.
pub const SIZES: [u32; 4] = [22, 32, 48, 64];

/// The biggest number drawn; more show as this.
pub const MAX_SHOWN: usize = 99;

const RED: [u8; 3] = [0xd9, 0x2d, 0x3a];
const BLUE: [u8; 3] = [0x1f, 0x6f, 0xeb];
const RING: [u8; 3] = [0x7a, 0x86, 0x99];
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];

/// 5 columns by 7 rows, one string per row, `#` is ink.
const DIGITS: [[&str; 7]; 10] = [
    [
        " ### ", "#   #", "#  ##", "# # #", "##  #", "#   #", " ### ",
    ],
    [
        "  #  ", " ##  ", "  #  ", "  #  ", "  #  ", "  #  ", " ### ",
    ],
    [
        " ### ", "#   #", "    #", "   # ", "  #  ", " #   ", "#####",
    ],
    [
        " ### ", "#   #", "    #", "  ## ", "    #", "#   #", " ### ",
    ],
    [
        "   # ", "  ## ", " # # ", "#  # ", "#####", "   # ", "   # ",
    ],
    [
        "#####", "#    ", "#### ", "    #", "    #", "#   #", " ### ",
    ],
    [
        " ### ", "#   #", "#    ", "#### ", "#   #", "#   #", " ### ",
    ],
    [
        "#####", "    #", "   # ", "  #  ", "  #  ", "  #  ", "  #  ",
    ],
    [
        " ### ", "#   #", "#   #", " ### ", "#   #", "#   #", " ### ",
    ],
    [
        " ### ", "#   #", "#   #", " ####", "    #", "#   #", " ### ",
    ],
];

fn ink(digit: usize, col: usize, row: usize) -> bool {
    DIGITS[digit][row].as_bytes()[col] == b'#'
}

/// The digits to draw for `count`, most significant first.
pub fn digits(count: usize) -> Vec<usize> {
    let n = count.min(MAX_SHOWN);
    if n >= 10 {
        vec![n / 10, n % 10]
    } else {
        vec![n]
    }
}

/// An icon at `size` pixels square as ARGB32, four bytes a pixel in the
/// order alpha, red, green, blue, which is what `IconPixmap` carries.
pub fn pixels(badge: Badge, size: u32) -> Vec<u8> {
    const SUB: u32 = 4;
    let s = size as f32;
    let (cx, cy) = (s / 2.0, s / 2.0);
    let radius = s / 2.0 - s * 0.03;
    let empty = badge.count == 0;
    let tone = match badge.tone {
        Tone::Red => RED,
        Tone::Blue => BLUE,
    };
    let digits = digits(badge.count);

    // Where the digits go: centred, as large as fits in the disc.
    let cols = digits.len() * 6 - 1;
    let cell = (s * 0.74 / cols as f32).min(s * 0.6 / 7.0);
    let (gw, gh) = (cols as f32 * cell, 7.0 * cell);
    let (gx0, gy0) = (cx - gw / 2.0, cy - gh / 2.0);
    let ring_width = s * 0.14;

    let glyph_at = |x: f32, y: f32| -> bool {
        if empty || x < gx0 || y < gy0 {
            return false;
        }
        let (col, row) = (((x - gx0) / cell) as usize, ((y - gy0) / cell) as usize);
        if col >= cols || row >= 7 || col % 6 == 5 {
            return false;
        }
        ink(digits[col / 6], col % 6, row)
    };

    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut inside, mut r, mut g, mut b) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..SUB {
                for sx in 0..SUB {
                    let x = px as f32 + (sx as f32 + 0.5) / SUB as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SUB as f32;
                    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                    if d > radius {
                        continue;
                    }
                    let colour = if empty {
                        // A ring, with the middle left clear.
                        if d < radius - ring_width {
                            continue;
                        }
                        RING
                    } else if glyph_at(x, y) {
                        WHITE
                    } else {
                        tone
                    };
                    inside += 1;
                    r += colour[0] as u32;
                    g += colour[1] as u32;
                    b += colour[2] as u32;
                }
            }
            if inside == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let alpha = (inside * 255 / (SUB * SUB)) as u8;
                out.extend_from_slice(&[
                    alpha,
                    (r / inside) as u8,
                    (g / inside) as u8,
                    (b / inside) as u8,
                ]);
            }
        }
    }
    out
}

/// Every size, as `IconPixmap` wants them: width, height, ARGB32 bytes.
pub fn pixmaps(badge: Badge) -> Vec<(i32, i32, Vec<u8>)> {
    SIZES
        .iter()
        .map(|&s| (s as i32, s as i32, pixels(badge, s)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn badge(count: usize, tone: Tone) -> Badge {
        Badge { count, tone }
    }

    fn at(px: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }

    #[test]
    fn the_buffer_is_size_squared_argb() {
        for size in SIZES {
            assert_eq!(
                pixels(badge(3, Tone::Blue), size).len(),
                (size * size * 4) as usize
            );
        }
    }

    #[test]
    fn corners_are_clear_and_the_disc_is_the_tone() {
        let size = 48;
        let red = pixels(badge(2, Tone::Red), size);
        assert_eq!(at(&red, size, 0, 0)[0], 0);
        // Near the edge of the disc, away from the digit.
        let p = at(&red, size, 24, 4);
        assert_eq!(p, [255, RED[0], RED[1], RED[2]]);
        let blue = pixels(badge(2, Tone::Blue), size);
        assert_eq!(at(&blue, size, 24, 4), [255, BLUE[0], BLUE[1], BLUE[2]]);
    }

    #[test]
    fn the_number_is_drawn_in_white_and_changes_with_the_count() {
        let size = 48;
        let one = pixels(badge(1, Tone::Blue), size);
        assert!(one.chunks(4).any(|p| p[0] == 255 && p[1..] == WHITE));
        assert_ne!(one, pixels(badge(7, Tone::Blue), size));
        assert_ne!(one, pixels(badge(11, Tone::Blue), size));
    }

    #[test]
    fn nothing_to_count_is_a_ring_with_no_number() {
        let size = 48;
        let p = pixels(badge(0, Tone::Blue), size);
        // Clear in the middle, inked at the rim, never white.
        assert_eq!(at(&p, size, 24, 24)[0], 0);
        assert_eq!(at(&p, size, 24, 3)[0], 255);
        assert!(!p.chunks(4).any(|q| q[0] == 255 && q[1..] == WHITE));
    }

    #[test]
    fn big_counts_show_as_99() {
        assert_eq!(digits(7), [7]);
        assert_eq!(digits(42), [4, 2]);
        assert_eq!(digits(100), [9, 9]);
        assert_eq!(
            pixels(badge(100, Tone::Red), 32),
            pixels(badge(99, Tone::Red), 32)
        );
    }

    #[test]
    fn every_digit_has_ink_in_a_5_by_7_cell() {
        for (d, rows) in DIGITS.iter().enumerate() {
            assert!(rows.iter().all(|r| r.len() == 5), "digit {d}");
            assert!(rows.iter().any(|r| r.contains('#')), "digit {d}");
        }
    }
}
