//! Draws the menu bar strip, black on clear for macOS to tint as a template
//! image: two pixels to the point, 18 points tall, the height the status item
//! shows an icon at.

use std::ptr;

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSBitmapImageRep, NSColor, NSCompositingOperation,
    NSDeviceRGBColorSpace, NSFont, NSFontAttributeName, NSFontWeightBold, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSGraphicsContext, NSImage,
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
    let black = NSColor::blackColor();
    let keys = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let values: [&AnyObject; 2] = [font.as_ref(), black.as_ref()];
    let attributes = NSDictionary::from_slices(&keys, &values);
    let lines: Vec<_> = group
        .values
        .iter()
        .map(|value| {
            // SAFETY: the attributes map AppKit's own keys to a font and a
            // colour, the types those keys take.
            let line = unsafe {
                NSAttributedString::initWithString_attributes(
                    NSAttributedString::alloc(),
                    &NSString::from_str(value),
                    Some(&attributes),
                )
            };
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
        for pixel in row[..width * 4].chunks_exact(4) {
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
        assert!(one.rgba.chunks_exact(4).any(|pixel| pixel[3] > 0));
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
        bytes.extend(drawn.rgba.chunks_exact(4).map(|pixel| pixel[3]));
        std::fs::write(path, bytes).unwrap();
    }
}
