use super::colors::Rgb;
use super::fonts::{CharBitmap, RouteIcon, FONT_HEIGHT};

/// Display dimensions.
pub const DISPLAY_WIDTH: usize = 192;
pub const DISPLAY_HEIGHT: usize = 32;

/// A 192x32 RGB framebuffer for the LED matrix display.
///
/// Stores pixels as a flat `Vec<u8>` in row-major order (R, G, B per pixel).
/// Total size: 192 * 32 * 3 = 18,432 bytes.
pub struct FrameBuffer {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
}

impl FrameBuffer {
    /// Create a new framebuffer filled with black.
    pub fn new() -> Self {
        Self::with_size(DISPLAY_WIDTH, DISPLAY_HEIGHT)
    }

    /// Create a framebuffer with custom dimensions (for testing).
    pub fn with_size(width: usize, height: usize) -> Self {
        FrameBuffer {
            pixels: vec![0u8; width * height * 3],
            width,
            height,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Clear the entire framebuffer to black.
    pub fn clear(&mut self) {
        self.pixels.fill(0);
    }

    /// Set a single pixel. Out-of-bounds coordinates are silently ignored.
    #[inline]
    pub fn set_pixel(&mut self, x: i32, y: i32, color: Rgb) {
        if x >= 0 && y >= 0 {
            let x = x as usize;
            let y = y as usize;
            if x < self.width && y < self.height {
                let idx = (y * self.width + x) * 3;
                self.pixels[idx] = color.0;
                self.pixels[idx + 1] = color.1;
                self.pixels[idx + 2] = color.2;
            }
        }
    }

    /// Get the color of a pixel. Returns black for out-of-bounds.
    pub fn get_pixel(&self, x: usize, y: usize) -> Rgb {
        if x < self.width && y < self.height {
            let idx = (y * self.width + x) * 3;
            (self.pixels[idx], self.pixels[idx + 1], self.pixels[idx + 2])
        } else {
            (0, 0, 0)
        }
    }

    /// Draw a character bitmap at (x, y) with the given color.
    ///
    /// The bitmap is from `MtaFont::get_char_bitmap()` — LSB-first decoded
    /// where each row is a `Vec<bool>` of lit pixels.
    pub fn blit_char(&mut self, bitmap: &CharBitmap, x: i32, y: i32, color: Rgb) {
        for (row_idx, row) in bitmap.iter().enumerate() {
            let py = y + row_idx as i32;
            for (col_idx, &lit) in row.iter().enumerate() {
                if lit {
                    self.set_pixel(x + col_idx as i32, py, color);
                }
            }
        }
    }

    /// Draw a route icon at (x, y) with alpha compositing.
    ///
    /// Icons use 1-bit alpha: pixels with a > 0 overwrite the destination.
    pub fn blit_icon(&mut self, icon: &RouteIcon, x: i32, y: i32) {
        for (row_idx, row) in icon.pixels.iter().enumerate() {
            let py = y + row_idx as i32;
            for (col_idx, &(r, g, b, a)) in row.iter().enumerate() {
                if a > 0 {
                    self.set_pixel(x + col_idx as i32, py, (r, g, b));
                }
            }
        }
    }

    /// Draw text string at (x, y) with the given color.
    ///
    /// Uses the global MTA font. Returns the total width drawn in pixels.
    pub fn draw_text(
        &mut self,
        text: &str,
        x: i32,
        y: i32,
        color: Rgb,
        italic: bool,
        spacing: i32,
    ) -> usize {
        let font = super::fonts::get_font();
        let mut x_offset: i32 = 0;

        for ch in text.chars() {
            if let Some(bitmap) = font.get_char_bitmap(ch, italic) {
                self.blit_char(&bitmap, x + x_offset, y, color);
            }
            x_offset += font.get_char_width(ch, italic) as i32 + spacing;
        }

        x_offset.max(0) as usize
    }

    /// Get the raw pixel buffer for passing to the LED matrix driver.
    pub fn raw_pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_framebuffer_is_black() {
        let fb = FrameBuffer::new();
        assert_eq!(fb.width(), DISPLAY_WIDTH);
        assert_eq!(fb.height(), DISPLAY_HEIGHT);
        assert_eq!(fb.pixels.len(), DISPLAY_WIDTH * DISPLAY_HEIGHT * 3);
        assert!(fb.pixels.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_set_get_pixel() {
        let mut fb = FrameBuffer::with_size(10, 10);
        fb.set_pixel(5, 3, (255, 128, 64));
        assert_eq!(fb.get_pixel(5, 3), (255, 128, 64));
        assert_eq!(fb.get_pixel(0, 0), (0, 0, 0));
    }

    #[test]
    fn test_out_of_bounds_ignored() {
        let mut fb = FrameBuffer::with_size(10, 10);
        // Should not panic
        fb.set_pixel(-1, 5, (255, 0, 0));
        fb.set_pixel(5, -1, (255, 0, 0));
        fb.set_pixel(10, 5, (255, 0, 0));
        fb.set_pixel(5, 10, (255, 0, 0));
        // All pixels should still be black
        assert_eq!(fb.get_pixel(0, 0), (0, 0, 0));
        assert_eq!(fb.get_pixel(9, 9), (0, 0, 0));
    }

    #[test]
    fn test_clear() {
        let mut fb = FrameBuffer::with_size(10, 10);
        fb.set_pixel(5, 5, (255, 0, 0));
        assert_ne!(fb.get_pixel(5, 5), (0, 0, 0));
        fb.clear();
        assert_eq!(fb.get_pixel(5, 5), (0, 0, 0));
    }

    #[test]
    fn test_draw_text() {
        let mut fb = FrameBuffer::new();
        let width = fb.draw_text("A", 0, 0, (0, 255, 0), false, -1);
        assert!(width > 0, "draw_text should return non-zero width");
        // Some pixels should be green
        let mut found_green = false;
        for y in 0..FONT_HEIGHT {
            for x in 0..width {
                if fb.get_pixel(x, y) == (0, 255, 0) {
                    found_green = true;
                    break;
                }
            }
        }
        assert!(found_green, "should have drawn some green pixels");
    }

    #[test]
    fn test_blit_icon() {
        let mut fb = FrameBuffer::new();
        let font = super::super::fonts::get_font();
        let icon = font.get_route_icon("1", false).expect("route 1 icon");
        fb.blit_icon(icon, 10, 5);

        // Check that some pixels were drawn with the icon's color
        let mut found_icon_pixel = false;
        for y in 5..(5 + icon.height) {
            for x in 10..(10 + icon.width) {
                let px = fb.get_pixel(x, y);
                if px != (0, 0, 0) {
                    found_icon_pixel = true;
                    break;
                }
            }
        }
        assert!(found_icon_pixel, "icon should have drawn some pixels");
    }

    #[test]
    fn test_raw_pixels_size() {
        let fb = FrameBuffer::new();
        assert_eq!(fb.raw_pixels().len(), DISPLAY_WIDTH * DISPLAY_HEIGHT * 3);
    }
}
