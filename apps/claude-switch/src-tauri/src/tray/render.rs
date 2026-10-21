//! Draws the menu bar strip, black on clear for macOS to tint as a template
//! image: two pixels to the point, 18 points tall, the height the status item
//! shows an icon at.

use std::ptr;

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSBezierPath, NSBitmapImageRep, NSColor,
    NSCompositingOperation, NSDeviceRGBColorSpace, NSFont, NSFontAttributeName, NSFontWeightBold,
    NSFontWeightSemibold, NSForegroundColorAttributeName, NSGraphicsContext, NSImage,
};
use objc2_foundation::{
    NSAttributedString, NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString,
};

use super::strip::{Group, Strip};
use crate::app_kind::AppKind;

/// Pixels to the point.
const SCALE: f64 = 2.0;
/// Points; the status item scales whatever it is given to this height.
const HEIGHT: f64 = 18.0;
/// Side of the square an app's mark is drawn in.
const MARK: f64 = 16.0;
const MARK_GAP: f64 = 4.0;
const GROUP_GAP: f64 = 11.0;
/// A profile's two values are stacked in the smaller size, a lone value is
/// drawn in the larger one.
const PAIR_FONT: f64 = 9.0;
const SINGLE_FONT: f64 = 12.0;
/// Between the bottom of the upper value's digits and the top of the lower's.
const LINE_GAP: f64 = 2.0;

const CLAUDE_MARK: &[u8] = include_bytes!("../../icons/tray/claude.png");
const OPENAI_MARK: &[u8] = include_bytes!("../../icons/tray/openai.png");
/// Gemini's sparkle, which Antigravity profiles are marked with too: no mark
/// of Antigravity's own is published under a licence that allows it.
const GEMINI_MARK: &[u8] = include_bytes!("../../icons/tray/gemini.png");

/// claude-switch's own mark, shown while no profile has usage to show: the
/// outline of the app icon's pixel art, a cell per character.
const APP_MARK: [&str; 16] = [
    "..#....#........",
    ".#.#..#.#.......",
    ".#.#..#.#.......",
    ".#.####.#.......",
    "#........#......",
    "#.#....#.#......",
    "#.#....###...#..",
    "#......#.#..#.#.",
    ".#.....#.#..#.#.",
    "..######.####.#.",
    "......#........#",
    "......#.#....#.#",
    "......#.#....#.#",
    "......#........#",
    ".......#......#.",
    "........######..",
];

/// A drawn image: straight-alpha RGBA, row-major.
pub struct Rendered {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The strip drawn, or `None` when it has nothing to show.
pub fn strip(strip: &Strip) -> Option<Rendered> {
    if strip.groups.is_empty() {
        return None;
    }
    // AppKit hands back autoreleased objects; drain them here.
    autoreleasepool(|_| draw_strip(strip))
}

/// claude-switch's mark, a cell to two pixels, with a cell's margin above and
/// below to make up the 18 points.
pub fn app_mark() -> Rendered {
    let width = APP_MARK[0].len() * 2;
    let height = (APP_MARK.len() + 2) * 2;
    let mut rgba = vec![0; width * height * 4];
    for (row, cells) in APP_MARK.iter().enumerate() {
        for (column, cell) in cells.chars().enumerate() {
            if cell != '#' {
                continue;
            }
            for y in (row + 1) * 2..(row + 2) * 2 {
                for x in column * 2..column * 2 + 2 {
                    rgba[(y * width + x) * 4 + 3] = 255;
                }
            }
        }
    }
    Rendered {
        rgba,
        width: width as u32,
        height: height as u32,
    }
}

/// The Bars style: up to four meters as bars in an 18-point square, each a
/// faint track, its fill, and a fainter remainder past it.
pub fn bars(fractions: &[f64]) -> Option<Rendered> {
    if fractions.is_empty() {
        return None;
    }
    autoreleasepool(|_| draw(HEIGHT, || draw_bars(fractions)))
}

/// claude-switch's mark and name, drawn in place of usage while the screen
/// is shared or recorded.
pub fn concealed() -> Option<Rendered> {
    autoreleasepool(|_| {
        let font = NSFont::boldSystemFontOfSize(SINGLE_FONT);
        let label = attributed("claude-switch", &font);
        let width = MARK + MARK_GAP + label.size().width;
        draw(width, || {
            NSColor::blackColor().set();
            let top = (HEIGHT - MARK) / 2.0 + MARK;
            for (row, cells) in APP_MARK.iter().enumerate() {
                for (column, cell) in cells.chars().enumerate() {
                    if cell == '#' {
                        NSBezierPath::fillRect(NSRect::new(
                            NSPoint::new(column as f64, top - (row as f64) - 1.0),
                            NSSize::new(1.0, 1.0),
                        ));
                    }
                }
            }
            let baseline = (HEIGHT - font.capHeight()) / 2.0;
            let height = label.size().height;
            label.drawAtPoint(NSPoint::new(
                MARK + MARK_GAP,
                baseline - (height - font.ascender()),
            ));
        })
    })
}

fn draw_bars(fractions: &[f64]) {
    let side = HEIGHT;
    let count = fractions.len().clamp(1, 4);
    let pad = (side * 0.08).round().max(1.0);
    let gap = (side * 0.03).round().max(1.0);
    let track_width = side - 2.0 * pad;
    let rows = count.max(2) as f64;
    let track_height = ((side - 2.0 * pad - (rows - 1.0) * gap) / rows)
        .floor()
        .max(1.0);
    let radius = (track_height / 3.0).floor().max(1.0);
    let total = count as f64 * track_height + (count as f64 - 1.0) * gap;
    let top_offset = pad + ((side - 2.0 * pad - total) / 2.0).floor();
    for (index, fraction) in fractions.iter().take(count).enumerate() {
        // From the top, as the glyph is laid out; AppKit counts from the bottom.
        let top = top_offset + index as f64 * (track_height + gap) + 1.0;
        let y = side - top - track_height;
        let rect = |x: f64, width: f64| {
            NSRect::new(NSPoint::new(pad + x, y), NSSize::new(width, track_height))
        };
        fill_rounded(rect(0.0, track_width), radius, 0.16);
        let fill = bar_fill(track_width, *fraction);
        if fill.fill_width > 0.0 {
            fill_rounded(rect(0.0, fill.fill_width), radius, 1.0);
            if let Some(divider) = fill.divider_x {
                fill_rounded(rect(divider, fill.remainder_width), radius, 0.24);
            }
        }
    }
}

fn fill_rounded(rect: NSRect, radius: f64, alpha: f64) {
    NSColor::colorWithWhite_alpha(0.0, alpha).set();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius).fill();
}

/// A bar's fill and remainder across a track `track_width` points long.
#[derive(Debug, Clone, Copy, PartialEq)]
struct BarFill {
    fill_width: f64,
    remainder_width: f64,
    /// Where the remainder starts, when one is drawn.
    divider_x: Option<f64>,
}

/// Fills between 70% and full are drawn in steps of 15% left, so a nearly
/// full bar still shows a tail rather than reading as full.
fn visual_fraction(fraction: f64) -> f64 {
    if !fraction.is_finite() {
        return 0.0;
    }
    let clamped = fraction.clamp(0.0, 1.0);
    if clamped > 0.7 && clamped < 1.0 {
        let quantized = (((1.0 - clamped) / 0.15).ceil() * 0.15).min(1.0);
        return (1.0 - quantized).max(0.0);
    }
    clamped
}

/// The fill of a bar `fraction` full, keeping a visible remainder of at least
/// four points (or a fifth of the track) until it is truly full.
fn bar_fill(track_width: f64, fraction: f64) -> BarFill {
    let empty = BarFill {
        fill_width: 0.0,
        remainder_width: 0.0,
        divider_x: None,
    };
    if !fraction.is_finite() || fraction <= 0.0 {
        return empty;
    }
    let visual = visual_fraction(fraction);
    if visual >= 1.0 {
        return BarFill {
            fill_width: track_width,
            ..empty
        };
    }
    let min_visible = (track_width * 0.2).round().max(4.0);
    let max_fill = (track_width - min_visible).max(1.0);
    let fill_width = (track_width * visual).round().min(max_fill).max(1.0);
    let remainder_width = (track_width - fill_width)
        .max(min_visible)
        .min(track_width - 1.0);
    BarFill {
        fill_width,
        remainder_width,
        divider_x: Some(track_width - remainder_width),
    }
}

fn attributed(text: &str, font: &NSFont) -> Retained<NSAttributedString> {
    let black = NSColor::blackColor();
    let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let values: [&AnyObject; 2] = [font.as_ref(), black.as_ref()];
    let attributes = NSDictionary::from_slices(&keys, &values);
    // SAFETY: the attributes map AppKit's own keys to a font and a colour,
    // the types those keys take.
    unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &NSString::from_str(text),
            Some(&attributes),
        )
    }
}

/// A group laid out: its mark and its values, ready to draw.
struct Laid {
    mark: Retained<NSImage>,
    font: Retained<NSFont>,
    lines: Vec<(Retained<NSAttributedString>, f64)>,
    text_width: f64,
}

impl Laid {
    fn width(&self) -> f64 {
        MARK + MARK_GAP + self.text_width
    }
}

fn draw_strip(strip: &Strip) -> Option<Rendered> {
    let groups = strip
        .groups
        .iter()
        .map(lay_out)
        .collect::<Option<Vec<_>>>()?;
    let width = groups.iter().map(Laid::width).sum::<f64>() + GROUP_GAP * (groups.len() - 1) as f64;

    draw(width, || {
        let mut x = 0.0;
        for group in &groups {
            group.mark.drawInRect_fromRect_operation_fraction(
                NSRect::new(
                    NSPoint::new(x, (HEIGHT - MARK) / 2.0),
                    NSSize::new(MARK, MARK),
                ),
                NSRect::ZERO,
                NSCompositingOperation::SourceOver,
                1.0,
            );
            let text_x = x + MARK + MARK_GAP;
            let cap = group.font.capHeight();
            let ascender = group.font.ascender();
            // Baselines that centre the digits' cap height, stacked or alone.
            let baselines: Vec<f64> = if group.lines.len() == 1 {
                vec![(HEIGHT - cap) / 2.0]
            } else {
                let bottom = (HEIGHT - (2.0 * cap + LINE_GAP)) / 2.0;
                vec![bottom + cap + LINE_GAP, bottom]
            };
            for ((line, line_width), baseline) in group.lines.iter().zip(baselines) {
                // Right-aligned, so a stacked pair lines up on its % signs.
                // `drawAtPoint` takes the line's lower-left corner, which sits
                // the line's height less its ascender below the baseline.
                let height = line.size().height;
                line.drawAtPoint(NSPoint::new(
                    text_x + group.text_width - line_width,
                    baseline - (height - ascender),
                ));
            }
            x += group.width() + GROUP_GAP;
        }
    })
}

fn lay_out(group: &Group) -> Option<Laid> {
    let bytes = match group.app {
        AppKind::Claude => CLAUDE_MARK,
        AppKind::Codex => OPENAI_MARK,
        AppKind::Gemini | AppKind::Antigravity => GEMINI_MARK,
    };
    let mark = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))?;
    let (size, weight) = if group.values.len() == 1 {
        // SAFETY: AppKit's font weight constants are plain CGFloat statics.
        (SINGLE_FONT, unsafe { NSFontWeightBold })
    } else {
        (PAIR_FONT, unsafe { NSFontWeightSemibold })
    };
    // Figures of equal width, so a value doesn't change width as its digits do.
    let font = NSFont::monospacedDigitSystemFontOfSize_weight(size, weight);
    let lines: Vec<_> = group
        .values
        .iter()
        .map(|value| {
            let line = attributed(value, &font);
            let width = line.size().width;
            (line, width)
        })
        .collect();
    let text_width = lines.iter().map(|(_, width)| *width).fold(0.0, f64::max);
    Some(Laid {
        mark,
        font,
        lines,
        text_width,
    })
}

/// Runs `paint` in a bitmap context `width` points wide and [`HEIGHT`] tall,
/// in points, and returns what it drew as a black template: only the alpha
/// is kept.
fn draw(width: f64, paint: impl FnOnce()) -> Option<Rendered> {
    let pixels_wide = (width * SCALE).ceil() as isize;
    let pixels_high = (HEIGHT * SCALE) as isize;
    if pixels_wide <= 0 {
        return None;
    }
    // SAFETY: a null `planes` has AppKit allocate the pixel storage, and the
    // rest describes an ordinary 8-bit RGBA bitmap of that many pixels.
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            ptr::null_mut(),
            pixels_wide,
            pixels_high,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }?;
    // Drawing is in points; the bitmap holds two pixels to each.
    bitmap.setSize(NSSize::new(
        pixels_wide as f64 / SCALE,
        pixels_high as f64 / SCALE,
    ));
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)?;

    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    paint();
    context.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();

    let row_bytes = usize::try_from(bitmap.bytesPerRow()).ok()?;
    let data = bitmap.bitmapData();
    if data.is_null() {
        return None;
    }
    let (width, height) = (pixels_wide as usize, pixels_high as usize);
    // SAFETY: the bitmap owns `bytesPerRow` bytes for each of its rows, and is
    // kept alive by `bitmap` until after the copy.
    let rows = unsafe { std::slice::from_raw_parts(data, row_bytes * height) };
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in rows.chunks_exact(row_bytes).take(height) {
        for pixel in row[..width * 4].as_chunks::<4>().0 {
            rgba.extend_from_slice(&[0, 0, 0, pixel[3]]);
        }
    }
    Some(Rendered {
        rgba,
        width: width as u32,
        height: height as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(app: AppKind, values: &[&str]) -> Group {
        Group {
            app,
            values: values.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn draws_nothing_for_an_empty_strip() {
        assert!(strip(&Strip { groups: Vec::new() }).is_none());
    }

    #[test]
    fn draws_eighteen_points_tall_and_as_wide_as_its_groups() {
        let one = strip(&Strip {
            groups: vec![group(AppKind::Claude, &["18%", "24%"])],
        })
        .unwrap();
        let two = strip(&Strip {
            groups: vec![
                group(AppKind::Claude, &["18%", "24%"]),
                group(AppKind::Codex, &["2%"]),
            ],
        })
        .unwrap();
        assert_eq!(one.height, 36);
        assert_eq!(one.rgba.len(), (one.width * one.height * 4) as usize);
        assert!(two.width > one.width + (GROUP_GAP * SCALE) as u32);
        assert!(one.rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
    }

    #[test]
    fn nearly_full_bars_keep_a_visible_tail() {
        assert_eq!(visual_fraction(0.5), 0.5);
        assert!((visual_fraction(0.97) - 0.85).abs() < 1e-9);
        assert!((visual_fraction(0.8) - 0.7).abs() < 1e-9);
        assert_eq!(visual_fraction(1.0), 1.0);
        let fill = bar_fill(16.0, 0.97);
        assert!(fill.remainder_width >= 4.0);
        assert_eq!(fill.divider_x, Some(16.0 - fill.remainder_width));
    }

    #[test]
    fn an_empty_or_full_bar_has_no_remainder() {
        assert_eq!(bar_fill(16.0, 0.0).fill_width, 0.0);
        let full = bar_fill(16.0, 1.2);
        assert_eq!((full.fill_width, full.divider_x), (16.0, None));
    }

    #[test]
    fn bars_and_the_concealed_mark_are_eighteen_points_tall() {
        let drawn = bars(&[0.2, 0.9]).unwrap();
        assert_eq!((drawn.width, drawn.height), (36, 36));
        assert!(drawn
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] == 255));
        assert!(bars(&[]).is_none());
        let concealed = concealed().unwrap();
        assert_eq!(concealed.height, 36);
        assert!(concealed.width > 36);
    }

    #[test]
    fn app_mark_is_eighteen_points_tall() {
        let mark = app_mark();
        assert_eq!((mark.width, mark.height), (32, 36));
        assert_eq!(mark.rgba.len(), 32 * 36 * 4);
    }

    /// Writes the strip's alpha to `$CLAUDE_SWITCH_TRAY_DUMP` (width, height,
    /// then a byte per pixel), to look at while changing the layout.
    #[test]
    #[ignore = "writes a file; opt in via CLAUDE_SWITCH_TRAY_DUMP"]
    fn dump_strip() {
        let Ok(path) = std::env::var("CLAUDE_SWITCH_TRAY_DUMP") else {
            return;
        };
        let drawn = strip(&Strip {
            groups: vec![
                group(AppKind::Claude, &["18%", "24%"]),
                group(AppKind::Claude, &["6%", "2%"]),
                group(AppKind::Codex, &["2%"]),
            ],
        })
        .unwrap();
        let mut bytes = format!("{} {}\n", drawn.width, drawn.height).into_bytes();
        bytes.extend(drawn.rgba.as_chunks::<4>().0.iter().map(|pixel| pixel[3]));
        std::fs::write(path, bytes).unwrap();
    }
}
