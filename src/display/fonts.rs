use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use super::colors::Rgb;

/// Font height in pixels (from the JSON font definition).
pub const FONT_HEIGHT: usize = 16;

/// Embedded font JSON (compiled into the binary).
const FONT_JSON: &str = include_str!("../../assets/fonts/mta-sign.json");

/// Embedded route icon metadata JSON.
const ICON_METADATA_JSON: &str =
    include_str!("../../assets/icons/route_icon_metadata.json");

/// Route icon metadata from JSON.
#[derive(Debug, Deserialize)]
struct IconMeta {
    width: usize,
    height: usize,
    baseline_offset: i32,
    color: String,
}

/// A rendered route icon bitmap.
#[derive(Debug, Clone)]
pub struct RouteIcon {
    /// Pixel data: row-major, each pixel is (r, g, b, a).
    pub pixels: Vec<Vec<(u8, u8, u8, u8)>>,
    pub width: usize,
    pub height: usize,
    pub baseline_offset: i32,
    pub color: Rgb,
}

/// Character bitmap: one Vec<bool> per row, LSB-first decoded.
pub type CharBitmap = Vec<Vec<bool>>;

/// The MTA bitmap font with character glyphs and route icons.
pub struct MtaFont {
    /// Character glyphs: char → row data (raw integers from JSON).
    chars: HashMap<char, Vec<u64>>,
    /// Pre-generated italic glyphs (midpoint-shifted).
    italic_chars: HashMap<char, Vec<u64>>,
    /// Route icon bitmaps.
    route_icons: HashMap<String, RouteIcon>,
}

static MTA_FONT: OnceLock<MtaFont> = OnceLock::new();

/// Get the global MTA font instance (loaded once).
pub fn get_font() -> &'static MtaFont {
    MTA_FONT.get_or_init(MtaFont::load)
}

impl MtaFont {
    fn load() -> Self {
        // Parse font JSON as generic map
        let font_data: HashMap<String, serde_json::Value> =
            serde_json::from_str(FONT_JSON).expect("embedded font JSON is valid");

        // Extract character glyphs (numeric keys = ASCII codes)
        let mut chars = HashMap::new();
        for (key, value) in &font_data {
            if let Ok(code) = key.parse::<u32>() {
                if let Some(ch) = char::from_u32(code) {
                    if let Some(rows) = value.as_array() {
                        let row_vals: Vec<u64> = rows
                            .iter()
                            .filter_map(|v| v.as_u64())
                            .collect();
                        chars.insert(ch, row_vals);
                    }
                }
            }
        }

        // Generate italic versions
        let italic_chars = Self::generate_italic(&chars);

        // Load route icons
        let route_icons = Self::load_route_icons(&font_data);

        MtaFont {
            chars,
            italic_chars,
            route_icons,
        }
    }

    /// Generate italic font via simple midpoint shift.
    /// Top half shifts 1px right, bottom half stays at baseline.
    fn generate_italic(chars: &HashMap<char, Vec<u64>>) -> HashMap<char, Vec<u64>> {
        let midpoint = FONT_HEIGHT / 2;
        let italic_shift: u32 = 1;
        let italic_padding: u32 = 1;

        let mut italic = HashMap::new();
        for (&ch, rows) in chars {
            let italic_rows: Vec<u64> = rows
                .iter()
                .enumerate()
                .map(|(y, &row_val)| {
                    let shift = if y < midpoint {
                        italic_shift + italic_padding
                    } else {
                        italic_padding
                    };
                    row_val << shift
                })
                .collect();
            italic.insert(ch, italic_rows);
        }
        italic
    }

    /// Load route icon bitmaps from font data + metadata.
    fn load_route_icons(font_data: &HashMap<String, serde_json::Value>) -> HashMap<String, RouteIcon> {
        let metadata: HashMap<String, IconMeta> =
            serde_json::from_str(ICON_METADATA_JSON).expect("embedded icon metadata is valid");

        let mut icons = HashMap::new();

        for (name, meta) in &metadata {
            let Some(rows_value) = font_data.get(name) else {
                continue;
            };
            let Some(rows) = rows_value.as_array() else {
                continue;
            };

            let row_vals: Vec<u64> = rows.iter().filter_map(|v| v.as_u64()).collect();
            if row_vals.len() != meta.height {
                continue;
            }

            let color = crate::display::colors::hex_to_rgb(&meta.color);

            // Decode MSB-first: bit (width-1) = leftmost pixel
            let mut pixels = Vec::with_capacity(meta.height);
            for &row_val in &row_vals {
                let mut row = Vec::with_capacity(meta.width);
                for x in 0..meta.width {
                    let bit = meta.width - 1 - x;
                    if row_val & (1 << bit) != 0 {
                        row.push((color.0, color.1, color.2, 255));
                    } else {
                        row.push((0, 0, 0, 0));
                    }
                }
                pixels.push(row);
            }

            icons.insert(
                name.clone(),
                RouteIcon {
                    pixels,
                    width: meta.width,
                    height: meta.height,
                    baseline_offset: meta.baseline_offset,
                    color,
                },
            );
        }

        icons
    }

    /// Get the bitmap for a character, decoded LSB-first.
    ///
    /// Returns None if the character is not in the font.
    pub fn get_char_bitmap(&self, ch: char, italic: bool) -> Option<CharBitmap> {
        let source = if italic { &self.italic_chars } else { &self.chars };
        let rows = source.get(&ch).or_else(|| self.chars.get(&ch))?;

        let mut bitmap = Vec::with_capacity(rows.len());
        for &row_val in rows {
            // LSB-first: bit 0 = leftmost pixel
            let max_bits = if row_val > 0 {
                64 - row_val.leading_zeros() as usize
            } else {
                1
            };
            let mut bits = Vec::with_capacity(max_bits);
            for i in 0..max_bits {
                bits.push(row_val & (1 << i) != 0);
            }
            bitmap.push(bits);
        }

        Some(bitmap)
    }

    /// Get the width of a character in pixels.
    pub fn get_char_width(&self, ch: char, italic: bool) -> usize {
        if ch == ' ' {
            return 4;
        }
        match self.get_char_bitmap(ch, italic) {
            Some(bitmap) => bitmap.iter().map(|row| row.len()).max().unwrap_or(4),
            None => 4,
        }
    }

    /// Get left padding (empty columns before first lit pixel).
    pub fn get_char_left_padding(&self, ch: char, italic: bool) -> usize {
        let bitmap = match self.get_char_bitmap(ch, italic) {
            Some(b) => b,
            None => return 0,
        };

        let mut leftmost = usize::MAX;
        for row in &bitmap {
            for (col, &pixel) in row.iter().enumerate() {
                if pixel {
                    leftmost = leftmost.min(col);
                    break;
                }
            }
        }

        if leftmost == usize::MAX {
            0
        } else {
            leftmost
        }
    }

    /// Measure the total width of a text string.
    pub fn measure_text(&self, text: &str, spacing: i32, italic: bool) -> usize {
        if text.is_empty() {
            return 0;
        }
        let mut total: i32 = 0;
        for ch in text.chars() {
            total += self.get_char_width(ch, italic) as i32;
        }
        let char_count = text.chars().count();
        if char_count > 1 {
            total += spacing * (char_count as i32 - 1);
        }
        total.max(0) as usize
    }

    /// Get a route icon by route ID and express status.
    ///
    /// Returns the DIAMOND variant for express, CIRCLE for local.
    /// Falls back to CIRCLE if DIAMOND isn't available.
    pub fn get_route_icon(&self, route: &str, is_express: bool) -> Option<&RouteIcon> {
        let shape = if is_express { "DIAMOND" } else { "CIRCLE" };
        let name = format!("ROUTE_{}_{}", route, shape);

        self.route_icons.get(&name).or_else(|| {
            if is_express {
                let fallback = format!("ROUTE_{}_CIRCLE", route);
                self.route_icons.get(&fallback)
            } else {
                None
            }
        })
    }

    /// Check if a character exists in the font.
    pub fn has_char(&self, ch: char) -> bool {
        self.chars.contains_key(&ch)
    }

    /// Get all available route icon names.
    pub fn icon_names(&self) -> Vec<&String> {
        self.route_icons.keys().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_loads() {
        let font = get_font();
        assert!(font.has_char('A'));
        assert!(font.has_char('0'));
        assert!(font.has_char(' '));
        assert!(!font.has_char('\u{FFFF}')); // unlikely to exist
    }

    #[test]
    fn test_char_bitmap_a() {
        let font = get_font();
        let bitmap = font.get_char_bitmap('A', false).expect("A should exist");
        assert_eq!(bitmap.len(), FONT_HEIGHT, "A should have {FONT_HEIGHT} rows");
        // A's first row is 0 (empty), so no lit pixels
        assert!(bitmap[0].iter().all(|&p| !p), "first row of A should be blank");
    }

    #[test]
    fn test_char_width() {
        let font = get_font();
        assert!(font.get_char_width('A', false) > 0);
        assert_eq!(font.get_char_width(' ', false), 4);
    }

    #[test]
    fn test_measure_text() {
        let font = get_font();
        let w = font.measure_text("A", -1, false);
        assert!(w > 0);

        let w2 = font.measure_text("AB", -1, false);
        // Two chars with spacing=-1 should be roughly: width_A + width_B - 1
        let wa = font.get_char_width('A', false);
        let wb = font.get_char_width('B', false);
        assert_eq!(w2, wa + wb - 1);

        assert_eq!(font.measure_text("", -1, false), 0);
    }

    #[test]
    fn test_italic_generation() {
        let font = get_font();
        let regular = font.get_char_bitmap('A', false).unwrap();
        let italic = font.get_char_bitmap('A', true).unwrap();
        assert_eq!(regular.len(), italic.len(), "same number of rows");
        // Italic top half should be wider (shifted right)
        let reg_top_width = regular[1].len(); // row 1 (row 0 is blank for A)
        let ital_top_width = italic[1].len();
        assert!(
            ital_top_width >= reg_top_width,
            "italic top should be at least as wide"
        );
    }

    #[test]
    fn test_lsb_first_encoding() {
        // Verify LSB-first: for value 224 = 0b11100000,
        // bits 5,6,7 are set → pixels at x=5,6,7 should be lit
        let font = get_font();
        let bitmap = font.get_char_bitmap('A', false).unwrap();
        // Row 1 of 'A' has value 224
        // 224 = 0b11100000 → bits 5,6,7 set
        // LSB-first means bit 0 = x=0, bit 5 = x=5, etc.
        let row1 = &bitmap[1];
        assert!(!row1[0], "bit 0 should be off");
        assert!(!row1[4], "bit 4 should be off");
        assert!(row1[5], "bit 5 should be on");
        assert!(row1[6], "bit 6 should be on");
        assert!(row1[7], "bit 7 should be on");
    }

    #[test]
    fn test_route_icons_loaded() {
        let font = get_font();
        let names = font.icon_names();
        assert!(!names.is_empty(), "should have route icons");
        // Check specific icons
        assert!(
            font.get_route_icon("1", false).is_some(),
            "should have route 1 circle"
        );
        assert!(
            font.get_route_icon("A", false).is_some(),
            "should have route A circle"
        );
    }

    #[test]
    fn test_route_icon_dimensions() {
        let font = get_font();
        let icon = font.get_route_icon("1", false).unwrap();
        assert_eq!(icon.width, 14);
        assert_eq!(icon.height, 13);
        assert_eq!(icon.pixels.len(), 13);
        assert_eq!(icon.pixels[0].len(), 14);
    }

    #[test]
    fn test_msb_first_icon_encoding() {
        let font = get_font();
        let icon = font.get_route_icon("1", false).unwrap();
        // ROUTE_1_CIRCLE first row value is 480 = 0b0000111100000
        // For 14-bit width MSB-first: bit 13 = leftmost
        // 480 = 0b111100000 → bits 5,6,7,8 set
        // MSB-first with width=14: pixel x = (width-1) - bit_position
        // So bits 5,6,7,8 → pixels at x=5,6,7,8 from right → x=8,7,6,5 from left
        // Actually: for x in 0..14, pixel lit if bit (13-x) is set in 480
        // 480 = 0b0000000111100000
        // bit 13-0 = 480>>13 & 1 = 0, ..., bit 13-5 = 480>>8 & 1 = 1, etc.
        let row0 = &icon.pixels[0];
        // 480 in binary is 111100000 (9 bits)
        // In 14-bit MSB: bits 5,6,7,8 are set
        // x=0: bit 13 → 0, x=1: bit 12 → 0, ..., x=5: bit 8 → 1, x=6: bit 7 → 1,
        // x=7: bit 6 → 1, x=8: bit 5 → 1, x=9: bit 4 → 0, ...
        assert_eq!(row0[0].3, 0, "pixel 0 should be transparent");
        assert_eq!(row0[5].3, 255, "pixel 5 should be opaque");
        assert_eq!(row0[6].3, 255, "pixel 6 should be opaque");
        assert_eq!(row0[7].3, 255, "pixel 7 should be opaque");
        assert_eq!(row0[8].3, 255, "pixel 8 should be opaque");
        assert_eq!(row0[9].3, 0, "pixel 9 should be transparent");
    }

    #[test]
    fn test_express_icon_fallback() {
        let font = get_font();
        // Route 1 has no DIAMOND variant → should fall back to CIRCLE
        assert!(font.get_route_icon("1", true).is_some());
        // Route 4 has DIAMOND → should return it
        let diamond = font.get_route_icon("4", true).unwrap();
        assert_eq!(diamond.width, 14);
    }

    #[test]
    fn test_space_width() {
        let font = get_font();
        assert_eq!(font.get_char_width(' ', false), 4);
        assert_eq!(font.get_char_width(' ', true), 4);
    }

    #[test]
    fn test_left_padding() {
        let font = get_font();
        let padding = font.get_char_left_padding('A', false);
        // 'A' widest rows (e.g., 1548=0b11000001100) have leftmost pixel at col 2.
        // get_char_left_padding returns the minimum across all rows.
        assert_eq!(padding, 2, "A should have 2px left padding (from widest rows)");
    }

    /// Helper: render text to a flat RGB pixel buffer, returns (width, height, pixels).
    fn render_text_to_pixels(
        text: &str,
        color: Rgb,
        italic: bool,
        scale: usize,
    ) -> (usize, usize, Vec<u8>) {
        let font = get_font();
        let spacing: i32 = -1;
        let width = font.measure_text(text, spacing, italic).max(1);
        let height = FONT_HEIGHT;

        let mut pixels = vec![0u8; width * height * 3 * scale * scale];

        let mut x_offset: i32 = 0;
        for ch in text.chars() {
            if let Some(bitmap) = font.get_char_bitmap(ch, italic) {
                for (y, row) in bitmap.iter().enumerate() {
                    for (x, &lit) in row.iter().enumerate() {
                        if lit {
                            let px = x_offset as usize + x;
                            if px < width {
                                for sy in 0..scale {
                                    for sx in 0..scale {
                                        let idx = ((y * scale + sy) * width * scale + px * scale + sx) * 3;
                                        if idx + 2 < pixels.len() {
                                            pixels[idx] = color.0;
                                            pixels[idx + 1] = color.1;
                                            pixels[idx + 2] = color.2;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            x_offset += font.get_char_width(ch, italic) as i32 + spacing;
        }

        (width * scale, height * scale, pixels)
    }

    /// Write a PPM (P6) image file.
    fn write_ppm(path: &str, width: usize, height: usize, pixels: &[u8]) {
        use std::io::Write;
        let mut f = std::fs::File::create(path).unwrap();
        write!(f, "P6\n{} {}\n255\n", width, height).unwrap();
        f.write_all(pixels).unwrap();
    }

    /// Render a route icon to a flat RGB pixel buffer, returns (width, height, pixels).
    fn render_icon_to_pixels(icon: &RouteIcon, scale: usize) -> (usize, usize, Vec<u8>) {
        let w = icon.width * scale;
        let h = icon.height * scale;
        let mut pixels = vec![0u8; w * h * 3];

        for (y, row) in icon.pixels.iter().enumerate() {
            for (x, &(r, g, b, a)) in row.iter().enumerate() {
                if a > 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let idx = ((y * scale + sy) * w + x * scale + sx) * 3;
                            if idx + 2 < pixels.len() {
                                pixels[idx] = r;
                                pixels[idx + 1] = g;
                                pixels[idx + 2] = b;
                            }
                        }
                    }
                }
            }
        }

        (w, h, pixels)
    }

    #[test]
    fn test_render_sample_ppm() {
        let scale = 4; // 4x scale for visibility

        // 1. Render "Times Sq-42 St" in green
        let (w, h, px) =
            render_text_to_pixels("Times Sq-42 St", (0x00, 0xFF, 0x00), false, scale);
        write_ppm("/tmp/mta_font_station.ppm", w, h, &px);

        // 2. Render "2 min" in green
        let (w, h, px) =
            render_text_to_pixels("2 min", (0x00, 0xFF, 0x00), false, scale);
        write_ppm("/tmp/mta_font_minutes.ppm", w, h, &px);

        // 3. Render italic "Downtown" in green
        let (w, h, px) =
            render_text_to_pixels("Downtown", (0x00, 0xFF, 0x00), true, scale);
        write_ppm("/tmp/mta_font_italic.ppm", w, h, &px);

        // 4. Render a composite with route icons for 1,2,3,A,N
        let font = get_font();
        let routes = ["1", "2", "3", "A", "N", "7", "L"];
        let icon_gap = 2 * scale;
        let total_w: usize = routes.iter().map(|r| {
            font.get_route_icon(r, false).map_or(0, |i| i.width * scale)
        }).sum::<usize>() + icon_gap * (routes.len() - 1);
        let total_h = 13 * scale;
        let mut composite = vec![0u8; total_w * total_h * 3];

        let mut x_off = 0;
        for route in &routes {
            if let Some(icon) = font.get_route_icon(route, false) {
                let (iw, ih, ipx) = render_icon_to_pixels(icon, scale);
                // Blit into composite
                for y in 0..ih.min(total_h) {
                    for x in 0..iw {
                        let src = (y * iw + x) * 3;
                        let dst_x = x_off + x;
                        if dst_x < total_w && src + 2 < ipx.len() {
                            let dst = (y * total_w + dst_x) * 3;
                            if dst + 2 < composite.len() {
                                composite[dst] = ipx[src];
                                composite[dst + 1] = ipx[src + 1];
                                composite[dst + 2] = ipx[src + 2];
                            }
                        }
                    }
                }
                x_off += iw + icon_gap;
            }
        }
        write_ppm("/tmp/mta_font_icons.ppm", total_w, total_h, &composite);

        // 5. Render express diamond icons for 2, 4, A
        let express_routes = ["2", "4", "A", "D", "7"];
        let total_w: usize = express_routes.iter().map(|r| {
            font.get_route_icon(r, true).map_or(0, |i| i.width * scale)
        }).sum::<usize>() + icon_gap * (express_routes.len() - 1);
        let mut composite_exp = vec![0u8; total_w * total_h * 3];

        x_off = 0;
        for route in &express_routes {
            if let Some(icon) = font.get_route_icon(route, true) {
                let (iw, ih, ipx) = render_icon_to_pixels(icon, scale);
                for y in 0..ih.min(total_h) {
                    for x in 0..iw {
                        let src = (y * iw + x) * 3;
                        let dst_x = x_off + x;
                        if dst_x < total_w && src + 2 < ipx.len() {
                            let dst = (y * total_w + dst_x) * 3;
                            if dst + 2 < composite_exp.len() {
                                composite_exp[dst] = ipx[src];
                                composite_exp[dst + 1] = ipx[src + 1];
                                composite_exp[dst + 2] = ipx[src + 2];
                            }
                        }
                    }
                }
                x_off += iw + icon_gap;
            }
        }
        write_ppm("/tmp/mta_font_express_icons.ppm", total_w, total_h, &composite_exp);

        println!("PPM files written to /tmp/mta_font_*.ppm");
        println!("Open with: open /tmp/mta_font_station.ppm");
    }
}
