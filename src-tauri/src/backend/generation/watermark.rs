//! The visible AI label on generated images (the old
//! `app/images/watermark.py`): a faint diagonal "Maskette" tiled across the
//! image and "Maskette AI generated" in the bottom-left corner.
//!
//! The hosted APIs are asked not to add their own watermark, so this label
//! is the one that says the image is AI-generated. Only the public copy
//! under `generated/` carries it; the clean copy that later jobs attach as a
//! reference does not, so labels do not pile up across rounds.
//!
//! The text is drawn with DejaVu Sans Bold, embedded in the binary through
//! the `dejavu` crate (the fonts' own licence lets them be bundled), so the
//! result does not depend on the fonts of the machine. The old backend
//! looked for system fonts and fell back to a bitmap font. Its corner logo
//! was turned off and did not move.

use std::sync::LazyLock;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use image::{Rgb, RgbImage};

pub const CORNER_TEXT: &str = "Maskette AI generated";
pub const TILED_TEXT: &str = "Maskette";

// Tiled diagonal text.
const TILED_FONT_MIN_SIZE: u32 = 18;
const TILED_FONT_SHORT_SIDE_DIVISOR: u32 = 24;
const TILED_TILE_MIN_SIZE: u32 = 180;
const TILED_TILE_SHORT_SIDE_DIVISOR: u32 = 3;
/// Clockwise, as the old `rotate(-32)`.
const TILED_ROTATION_DEGREES: f32 = 32.0;
const TILED_SHADOW_ALPHA: f32 = 16.0;

// Corner text.
const CORNER_MARGIN_RATIO: f32 = 0.035;
const CORNER_MARGIN_MIN: u32 = 12;
const CORNER_FONT_RATIO: f32 = 0.028;
const CORNER_FONT_MIN_SIZE: u32 = 14;
const CORNER_TEXT_ALPHA: f32 = 238.0;
const CORNER_SHADOW_ALPHA: f32 = 170.0;
const CORNER_SHADOW_OFFSET_RATIO: f32 = 0.08;

static FONT: LazyLock<FontRef<'static>> = LazyLock::new(|| {
    FontRef::try_from_slice(dejavu::sans::bold()).expect("the embedded font parses")
});

/// Text as coverage (0..1), laid out from the top of the ascender like
/// Pillow's `draw.text`.
struct TextMask {
    width: u32,
    height: u32,
    coverage: Vec<f32>,
}

impl TextMask {
    /// `size` is the em size in pixels, like Pillow's font size.
    fn new(text: &str, size: u32) -> Self {
        let font = &*FONT;
        let em = size as f32;
        let scale = PxScale::from(em * font.height_unscaled() / font.units_per_em().unwrap_or(1.0));
        let scaled = font.as_scaled(scale);
        let mut glyphs = Vec::new();
        let mut caret = 0.0_f32;
        let mut previous = None;
        for character in text.chars() {
            let id = font.glyph_id(character);
            if let Some(previous) = previous {
                caret += scaled.kern(previous, id);
            }
            glyphs.push(id.with_scale_and_position(scale, point(caret, scaled.ascent())));
            caret += scaled.h_advance(id);
            previous = Some(id);
        }
        let width = to_pixels(caret) + 2;
        let height = to_pixels(scaled.ascent() - scaled.descent()) + 2;
        let mut coverage = vec![0.0; (width * height) as usize];
        for glyph in glyphs {
            let Some(outlined) = font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outlined.px_bounds();
            outlined.draw(|x, y, value| {
                let (px, py) = (bounds.min.x + x as f32, bounds.min.y + y as f32);
                if px >= 0.0 && py >= 0.0 {
                    let (px, py) = (px as u32, py as u32);
                    if px < width && py < height {
                        let cell = &mut coverage[(py * width + px) as usize];
                        *cell = (*cell + value).min(1.0);
                    }
                }
            });
        }
        Self {
            width,
            height,
            coverage,
        }
    }

    fn at(&self, x: i64, y: i64) -> f32 {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return 0.0;
        }
        self.coverage[(y as u64 * u64::from(self.width) + x as u64) as usize]
    }

    /// Bilinear coverage at a fractional position.
    fn sample(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = self.at(x0, y0) * (1.0 - fx) + self.at(x0 + 1, y0) * fx;
        let bottom = self.at(x0, y0 + 1) * (1.0 - fx) + self.at(x0 + 1, y0 + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// The first and last rows with ink.
    fn ink_rows(&self) -> (u32, u32) {
        let rows_with_ink: Vec<u32> = (0..self.height)
            .filter(|y| (0..self.width).any(|x| self.coverage[(y * self.width + x) as usize] > 0.0))
            .collect();
        match (rows_with_ink.first(), rows_with_ink.last()) {
            (Some(first), Some(last)) => (*first, *last + 1),
            _ => (0, self.height),
        }
    }
}

fn to_pixels(value: f32) -> u32 {
    value.ceil().max(0.0) as u32
}

fn round(value: f32) -> u32 {
    value.round().max(0.0) as u32
}

/// Blends `colour` at `alpha` (0..255) times `coverage` over a pixel.
fn blend(pixel: &mut Rgb<u8>, colour: [u8; 3], alpha: f32, coverage: f32) {
    let weight = (alpha / 255.0 * coverage).clamp(0.0, 1.0);
    if weight <= 0.0 {
        return;
    }
    for (channel, target) in pixel.0.iter_mut().zip(colour) {
        let value = f32::from(*channel) * (1.0 - weight) + f32::from(target) * weight;
        *channel = value.round().clamp(0.0, 255.0) as u8;
    }
}

/// Puts the label on an image.
pub fn apply(image: &mut RgbImage) {
    if image.width() == 0 || image.height() == 0 {
        return;
    }
    draw_tiled(image);
    draw_corner(image);
}

fn draw_tiled(image: &mut RgbImage) {
    let (width, height) = image.dimensions();
    let short_side = width.min(height);
    let tile = TILED_TILE_MIN_SIZE.max(short_side / TILED_TILE_SHORT_SIDE_DIVISOR);
    let mask = TextMask::new(
        TILED_TEXT,
        TILED_FONT_MIN_SIZE.max(short_side / TILED_FONT_SHORT_SIDE_DIVISOR),
    );
    // The old version drew the text on a square layer every `tile` pixels
    // from -tile, rotated the layer about its centre and centred it on the
    // image. Each pixel here looks up where it falls on that layer.
    let (half_width, half_height, tile_f) = (width as f32 / 2.0, height as f32 / 2.0, tile as f32);
    let layer = f64::from(width).hypot(f64::from(height)) as u32 + tile * 2;
    let layer_centre = layer as f32 / 2.0;
    let (sin, cos) = TILED_ROTATION_DEGREES.to_radians().sin_cos();
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let (dx, dy) = (x as f32 + 0.5 - half_width, y as f32 + 0.5 - half_height);
        // Undo the clockwise turn.
        let lx = dx * cos + dy * sin + layer_centre;
        let ly = -dx * sin + dy * cos + layer_centre;
        // Text origins sit at -tile + k * tile; the shadow is one pixel off.
        let (sx, sy) = (lx - 1.0 + tile_f, ly - 1.0 + tile_f);
        let (cell_x, cell_y) = ((sx / tile_f).floor(), (sy / tile_f).floor());
        let mut coverage: f32 = 0.0;
        for column in [cell_x, cell_x - 1.0] {
            for row in [cell_y, cell_y - 1.0] {
                coverage =
                    coverage.max(mask.sample(sx - column * tile_f - 0.5, sy - row * tile_f - 0.5));
            }
        }
        blend(pixel, [0, 0, 0], TILED_SHADOW_ALPHA, coverage);
    }
}

fn draw_corner(image: &mut RgbImage) {
    let (width, height) = image.dimensions();
    let short_side = width.min(height).max(1) as f32;
    let margin = CORNER_MARGIN_MIN.max(round(short_side * CORNER_MARGIN_RATIO));
    let font_size = CORNER_FONT_MIN_SIZE.max(round(short_side * CORNER_FONT_RATIO));
    let shadow_offset = 1.max(round(font_size as f32 * CORNER_SHADOW_OFFSET_RATIO));
    let mask = TextMask::new(CORNER_TEXT, font_size);
    let (ink_top, ink_bottom) = mask.ink_rows();
    // The ink ends `margin` above the bottom edge.
    let top = margin.max(height.saturating_sub(margin + (ink_bottom - ink_top)));
    let origin_y = i64::from(top) - i64::from(ink_top);
    let origin_x = i64::from(margin);
    for (offset, colour, alpha) in [
        (i64::from(shadow_offset), [0, 0, 0], CORNER_SHADOW_ALPHA),
        (0, [255, 255, 255], CORNER_TEXT_ALPHA),
    ] {
        for my in 0..mask.height {
            for mx in 0..mask.width {
                let coverage = mask.coverage[(my * mask.width + mx) as usize];
                if coverage <= 0.0 {
                    continue;
                }
                let (x, y) = (
                    origin_x + offset + i64::from(mx),
                    origin_y + offset + i64::from(my),
                );
                if let (Ok(x), Ok(y)) = (u32::try_from(x), u32::try_from(y))
                    && x < width
                    && y < height
                {
                    blend(image.get_pixel_mut(x, y), colour, alpha, coverage);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_lands_in_the_corner_and_faintly_across() {
        let mut image = RgbImage::from_pixel(800, 1100, Rgb([128, 128, 128]));
        apply(&mut image);
        let changed = |x0: u32, y0: u32, x1: u32, y1: u32| {
            (y0..y1)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|&(x, y)| image.get_pixel(x, y).0 != [128, 128, 128])
                .count()
        };
        // White text and its shadow at the bottom left.
        let corner = changed(0, 1000, 400, 1100);
        assert!(corner > 500, "{corner}");
        assert!(
            (1000..1100)
                .flat_map(|y| (0..400).map(move |x| (x, y)))
                .any(|(x, y)| image.get_pixel(x, y).0[0] > 220)
        );
        // Nothing to the right of the corner text at its height.
        assert_eq!(
            (1060..1072)
                .flat_map(|y| (700..800).map(move |x| (x, y)))
                .filter(|&(x, y)| image.get_pixel(x, y).0[0] > 140)
                .count(),
            0
        );
        // The tiled text darkens a little all over, never much.
        let tiled = changed(0, 0, 800, 900);
        assert!(tiled > 5_000, "{tiled}");
        let darkest = (0..900)
            .flat_map(|y| (0..800).map(move |x| (x, y)))
            .map(|(x, y)| image.get_pixel(x, y).0[0])
            .min()
            .unwrap();
        assert!(darkest >= 118, "{darkest}");
    }

    #[test]
    fn tiny_images_are_labelled_without_panicking() {
        for (width, height) in [(1, 1), (5, 300), (300, 5)] {
            let mut image = RgbImage::new(width, height);
            apply(&mut image);
        }
    }
}
