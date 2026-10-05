use regex::Regex;

use crate::models::{Alert, DisplaySnapshot, Train};

use super::colors::{self, COLOR_BLACK, COLOR_GREEN, COLOR_RED};
use super::fonts::{self, MtaFont};
use super::framebuffer::{FrameBuffer, DISPLAY_WIDTH};

/// Character spacing for the MTA font (kerning of -1px, matching Python).
const CHAR_SPACING: i32 = -1;

/// Width of a route icon (circle/diamond) in pixels.
const ICON_WIDTH: i32 = 14;
/// Sentinel value for "no train data" in the minutes field.
const EMPTY_TRAIN_SENTINEL: i32 = 999;
/// Y offset of the top train row (shifted up to align with V1 sign).
const TOP_ROW_Y_ADJUST: i32 = -4;
/// Y offset where the bottom train row starts.
const BOTTOM_ROW_Y: i32 = 16;
/// Gap between route icon and destination text in a train row.
const ICON_TEXT_GAP: i32 = 3;
/// Right margin before the arrival time text.
const TIME_RIGHT_MARGIN: i32 = 5;
/// Y position of the scrolling alert row.
const ALERT_ROW_Y: i32 = 15;

/// Gap before an icon (text → icon) in alerts.
const TEXT_TO_ICON_GAP: i32 = 5;
/// Gap after an icon (icon → text) in alerts.
const ICON_TO_TEXT_GAP: i32 = 2;
/// Gap between consecutive icons in alerts.
const ICON_ICON_GAP: i32 = 1;

/// Pure rendering engine for the subway sign display.
///
/// All methods are side-effect free — same inputs produce same output.
/// No I/O, no threading, no shared mutable state.
pub struct Renderer {
    /// Track width of last rendered alert for scroll completion.
    last_alert_width: i32,
    /// Cached alert rendering: (text, affected_routes_key) → pre-rendered pixels.
    alert_cache: Option<AlertCacheEntry>,
    /// Regex for MTA symbology tokens in alert text: route bullets like
    /// `[A]`, `[6X]`, `[SIR]` and pictograms like `[airplane icon]`.
    route_pattern: Regex,
    /// Clock face for one whole second (its text changes once a second, the
    /// render loop asks 60 times).
    clock_cache: Option<(i64, FrameBuffer)>,
}

struct AlertCacheEntry {
    text: String,
    routes_key: String,
    /// Pre-rendered alert as a small framebuffer (variable width x 17 height).
    buffer: FrameBuffer,
}

impl Renderer {
    /// Create a new renderer.
    pub fn new() -> Self {
        // Ensure font is loaded at init time
        let _ = fonts::get_font();

        Renderer {
            last_alert_width: 0,
            alert_cache: None,
            clock_cache: None,
            // Lazy `[A-Z]+?` so a trailing X on a letter route is the express
            // marker (`[FX]` = F express) while `[SIR]` still matches whole.
            route_pattern: Regex::new(r"\[(?:(\d+|[A-Z]+?)([xX])?|([a-z ]+) icon)\]").unwrap(),
        }
    }

    /// Render a complete frame.
    ///
    /// This is the main entry point called at 60fps.
    pub fn render_frame(
        &mut self,
        snapshot: &DisplaySnapshot,
        cycle_index: usize,
        flash_state: bool,
        alert_scroll_offset: f32,
        show_alert: bool,
        current_alert: Option<&Alert>,
    ) -> FrameBuffer {
        let mut fb = FrameBuffer::new();

        // Top row: next arriving train (any direction)
        let first_train = snapshot.get_first_train();
        self.render_train_row(&mut fb, first_train, 0, 1, flash_state);

        // Bottom row: cycling train OR scrolling alert
        if show_alert {
            if let Some(alert) = current_alert {
                self.render_alert_row(&mut fb, alert, alert_scroll_offset);
            }
        } else {
            let cycling = snapshot.get_cycling_trains(6);
            let idx = cycle_index.min(cycling.len().saturating_sub(1));
            self.render_train_row(&mut fb, &cycling[idx], BOTTOM_ROW_Y, idx + 2, false);
        }

        fb
    }

    /// Render the clock face the retired MTA LED countdown clocks show:
    /// the date in green on the top row ("Tue, April 23, 2024") and the
    /// time in red on the bottom row ("6:05:39 PM"), both upright and centred.
    pub fn render_clock<Tz: chrono::TimeZone>(&mut self, now: &chrono::DateTime<Tz>) -> FrameBuffer
    where
        Tz::Offset: std::fmt::Display,
    {
        let second = now.timestamp();
        if self.clock_cache.as_ref().is_none_or(|(s, _)| *s != second) {
            let mut fb = FrameBuffer::new();
            let (date, time) = Self::clock_text(now);
            self.draw_centered(&mut fb, &date, 0, COLOR_GREEN);
            self.draw_centered(&mut fb, &time, BOTTOM_ROW_Y, COLOR_RED);
            self.clock_cache = Some((second, fb));
        }
        self.clock_cache.as_ref().unwrap().1.clone()
    }

    /// The clock face's two lines, formatted as the retired MTA clocks do.
    fn clock_text<Tz: chrono::TimeZone>(now: &chrono::DateTime<Tz>) -> (String, String)
    where
        Tz::Offset: std::fmt::Display,
    {
        (
            now.format("%a, %B %-d, %Y").to_string(),
            now.format("%-I:%M:%S %p").to_string(),
        )
    }

    /// Draw upright text horizontally centred in a train row.
    fn draw_centered(&self, fb: &mut FrameBuffer, text: &str, row_y: i32, color: colors::Rgb) {
        let width = fonts::get_font().measure_text(text, CHAR_SPACING, false) as i32;
        let x = (DISPLAY_WIDTH as i32 - width) / 2;
        fb.draw_text(text, x, row_y + TOP_ROW_Y_ADJUST + 4, color, false, CHAR_SPACING);
    }

    /// Render a single train row at the given y_offset.
    fn render_train_row(
        &self,
        fb: &mut FrameBuffer,
        train: &Train,
        y_offset: i32,
        train_number: usize,
        flash_state: bool,
    ) {
        let font = fonts::get_font();

        let y = y_offset + TOP_ROW_Y_ADJUST;

        // Determine colors based on arrival state
        let is_arriving = train.minutes == 0;
        let (time_color, text_color) = if is_arriving && flash_state {
            (COLOR_BLACK, COLOR_RED) // Flash to black
        } else if is_arriving {
            (COLOR_RED, COLOR_RED)
        } else {
            (COLOR_GREEN, COLOR_GREEN)
        };

        // 1. Train number (e.g., "1.", "2.")
        let num_text = format!("{}.", train_number);
        let num_width = fb.draw_text(&num_text, -2, y + 4, text_color, false, CHAR_SPACING);

        // 2. Route icon
        let icon_x = num_width as i32;
        if !train.route.is_empty() {
            self.render_route_icon(fb, &train.route, train.is_express, icon_x, y + 4);
        }

        // 3. Destination text
        let station_x = icon_x + ICON_WIDTH + ICON_TEXT_GAP;

        // 4. Arrival time (right-aligned)
        let time_text = if train.minutes < EMPTY_TRAIN_SENTINEL {
            format!("{}min", train.minutes)
        } else {
            "---min".to_string()
        };
        let time_width = font.measure_text(&time_text, CHAR_SPACING, false) as i32;
        let time_x = DISPLAY_WIDTH as i32 - time_width;

        // Truncate destination to fit between icon and time
        let available_width = (time_x - station_x - TIME_RIGHT_MARGIN).max(0) as usize;
        let dest_text = self.truncate_text(font, &train.destination, available_width);
        fb.draw_text(&dest_text, station_x, y + 4, text_color, false, CHAR_SPACING);

        // Draw time
        fb.draw_text(&time_text, time_x, y + 4, time_color, false, CHAR_SPACING);
    }

    /// Render a scrolling alert in the bottom row.
    fn render_alert_row(
        &mut self,
        fb: &mut FrameBuffer,
        alert: &Alert,
        scroll_offset: f32,
    ) {
        let routes_key = Self::routes_key(&alert.affected_routes);

        // Check cache
        let need_render = match &self.alert_cache {
            Some(cached) => cached.text != alert.text || cached.routes_key != routes_key,
            None => true,
        };

        if need_render {
            let alert_buf = self.render_alert_with_icons(&alert.text);
            self.last_alert_width = alert_buf.width() as i32;
            self.alert_cache = Some(AlertCacheEntry {
                text: alert.text.clone(),
                routes_key,
                buffer: alert_buf,
            });
        }

        let alert_buf = &self.alert_cache.as_ref().unwrap().buffer;

        // Scroll: text starts off-screen right, moves left
        let x_pos = DISPLAY_WIDTH as i32 - scroll_offset as i32;

        // Only render if still visible (y=15 to fit 17px tall alert in bottom half)
        if x_pos > -(alert_buf.width() as i32) {
            self.blit_framebuffer(fb, alert_buf, x_pos, ALERT_ROW_Y);
        }
    }

    /// Get total scroll distance needed for current alert to fully cross the screen.
    pub fn get_scroll_complete_distance(&self) -> i32 {
        DISPLAY_WIDTH as i32 + self.last_alert_width + 10
    }

    /// Render alert text with inline route icons for `[route]` patterns.
    fn render_alert_with_icons(
        &self,
        text: &str,
    ) -> FrameBuffer {
        let font = fonts::get_font();
        let alert_color = colors::COLOR_ORANGE;

        let matches: Vec<_> = self.route_pattern.find_iter(text).collect();

        if matches.is_empty() {
            // No route patterns — render as simple italic text
            let width = font.measure_text(text, CHAR_SPACING, true).max(1);
            let mut buf = FrameBuffer::with_size(width, 17);
            buf.draw_text(text, 0, 1, alert_color, true, CHAR_SPACING);
            return buf;
        }

        // Parse into parts: text segments and resolved icons. A token whose
        // glyph doesn't exist is kept as its text rather than dropped.
        let mut parts: Vec<AlertPart> = Vec::new();
        let mut last_end = 0;

        for m in self.route_pattern.captures_iter(text) {
            let full = m.get(0).unwrap();
            if full.start() > last_end {
                parts.push(AlertPart::Text(text[last_end..full.start()].to_string()));
            }

            parts.push(match Self::resolve_token(font, &m) {
                Some(icon) => AlertPart::Icon(icon),
                None => AlertPart::Text(full.as_str().trim_matches(['[', ']']).to_string()),
            });

            last_end = full.end();
        }

        if last_end < text.len() {
            parts.push(AlertPart::Text(text[last_end..].to_string()));
        }

        // Measure total width with context-aware spacing
        let rendered: Vec<RenderedPart> = parts
            .into_iter()
            .map(|p| match p {
                AlertPart::Text(t) => {
                    let w = font.measure_text(&t, CHAR_SPACING, true);
                    RenderedPart::Text(t, w)
                }
                AlertPart::Icon(icon) => RenderedPart::Icon(icon),
            })
            .collect();

        let total_width = Self::measure_alert_parts(&rendered);

        // Render into buffer (17px tall to accommodate diamond icon offset)
        let mut buf = FrameBuffer::with_size(total_width.max(1), 17);
        let mut x_pos: i32 = 0;

        for (i, part) in rendered.iter().enumerate() {
            let prev_type = if i > 0 { rendered[i - 1].part_type() } else { PartType::None };
            let cur_type = part.part_type();

            // Context-aware gap
            x_pos += match (prev_type, cur_type) {
                (PartType::Icon, PartType::Text) => ICON_TO_TEXT_GAP,
                (PartType::Icon, PartType::Icon) => ICON_ICON_GAP,
                (PartType::Text, PartType::Icon) => TEXT_TO_ICON_GAP,
                _ => 0,
            };

            match part {
                RenderedPart::Text(t, _w) => {
                    let drawn = buf.draw_text(t, x_pos, 1, alert_color, true, CHAR_SPACING);
                    x_pos += drawn as i32;
                }
                RenderedPart::Icon(icon) => {
                    buf.blit_icon(icon, x_pos, 1 - icon.baseline_offset);
                    x_pos += icon.width as i32;
                }
            }
        }

        buf
    }

    /// Render a route icon at (x, y) with baseline offset.
    fn render_route_icon(
        &self,
        fb: &mut FrameBuffer,
        route: &str,
        is_express: bool,
        x: i32,
        y: i32,
    ) {
        if let Some(icon) = Self::lookup_icon(fonts::get_font(), route, is_express) {
            fb.blit_icon(icon, x, y - icon.baseline_offset);
        }
    }

    /// Resolve one symbology token match to its glyph. A route is express
    /// only when the token says so (`[6X]`); a plain `[6]` is the local bullet.
    fn resolve_token(font: &'static MtaFont, m: &regex::Captures) -> Option<&'static fonts::RouteIcon> {
        match (m.get(1), m.get(3)) {
            (Some(route), _) => Self::lookup_icon(font, route.as_str(), m.get(2).is_some()),
            (None, Some(name)) => font.get_icon(&Self::pictogram_key(name.as_str())),
            (None, None) => None,
        }
    }

    /// Font key for a pictogram token: `shuttle bus` -> `ICON_SHUTTLE_BUS`.
    fn pictogram_key(name: &str) -> String {
        format!("ICON_{}", name.trim().replace(' ', "_").to_uppercase())
    }

    /// Look up a route icon with express fallback to local variant.
    fn lookup_icon<'a>(
        font: &'a MtaFont,
        route: &str,
        is_express: bool,
    ) -> Option<&'a fonts::RouteIcon> {
        font.get_route_icon(route, is_express)
            .or_else(|| font.get_route_icon(route, false))
    }

    /// Truncate text to fit within max_width pixels.
    fn truncate_text(&self, font: &MtaFont, text: &str, max_width: usize) -> String {
        if font.measure_text(text, CHAR_SPACING, false) <= max_width {
            return text.to_string();
        }

        let char_count = text.chars().count();
        let mut lo: usize = 0;
        let mut hi: usize = char_count;
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let candidate: String = text.chars().take(mid).collect();
            if font.measure_text(&candidate, CHAR_SPACING, false) <= max_width {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        text.chars().take(lo).collect()
    }

    /// Blit one framebuffer onto another at (x, y). Non-black pixels overwrite.
    fn blit_framebuffer(&self, dst: &mut FrameBuffer, src: &FrameBuffer, x: i32, y: i32) {
        for sy in 0..src.height() {
            for sx in 0..src.width() {
                let px = src.get_pixel(sx, sy);
                if px != (0, 0, 0) {
                    dst.set_pixel(x + sx as i32, y + sy as i32, px);
                }
            }
        }
    }

    /// Build a stable string key from a set of routes (for cache comparison).
    fn routes_key(routes: &std::collections::HashSet<String>) -> String {
        let mut sorted: Vec<&str> = routes.iter().map(|s| s.as_str()).collect();
        sorted.sort();
        sorted.join(",")
    }

    /// Measure total width of rendered alert parts with context-aware spacing.
    fn measure_alert_parts(parts: &[RenderedPart]) -> usize {
        let mut total: i32 = 0;
        for (i, part) in parts.iter().enumerate() {
            let prev_type = if i > 0 { parts[i - 1].part_type() } else { PartType::None };
            let cur_type = part.part_type();

            total += match (prev_type, cur_type) {
                (PartType::Icon, PartType::Text) => ICON_TO_TEXT_GAP,
                (PartType::Icon, PartType::Icon) => ICON_ICON_GAP,
                (PartType::Text, PartType::Icon) => TEXT_TO_ICON_GAP,
                _ => 0,
            };

            total += part.width() as i32;
        }
        total.max(0) as usize
    }
}

// -- Internal types for alert rendering --

enum AlertPart {
    Text(String),
    Icon(&'static fonts::RouteIcon),
}

enum RenderedPart {
    /// (text, measured width)
    Text(String, usize),
    Icon(&'static fonts::RouteIcon),
}

#[derive(Clone, Copy, PartialEq)]
enum PartType {
    None,
    Text,
    Icon,
}

impl RenderedPart {
    fn part_type(&self) -> PartType {
        match self {
            RenderedPart::Text(..) => PartType::Text,
            RenderedPart::Icon(..) => PartType::Icon,
        }
    }

    fn width(&self) -> usize {
        match self {
            RenderedPart::Text(_, w) => *w,
            RenderedPart::Icon(icon) => icon.width,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Direction, DisplaySnapshot, Train};
    use std::collections::HashSet;

    fn make_train(route: &str, dest: &str, minutes: i32, is_express: bool) -> Train {
        Train {
            route: route.into(),
            destination: dest.into(),
            minutes,
            is_express,
            arrival_timestamp: 0.0,
            direction: Direction::Uptown,
            stop_id: "127N".into(),
        }
    }

    #[test]
    fn test_render_frame_basic() {
        let mut renderer = Renderer::new();
        let snapshot = DisplaySnapshot {
            trains: vec![
                make_train("1", "Van Cortlandt Park", 2, false),
                make_train("2", "Wakefield", 5, true),
                make_train("3", "Harlem", 8, false),
            ],
            alerts: Vec::new(),
            fetched_at: 1000.0,
        };

        let fb = renderer.render_frame(&snapshot, 0, false, 0.0, false, None);
        assert_eq!(fb.width(), 192);
        assert_eq!(fb.height(), 32);

        // Should have drawn something (not all black)
        let mut has_pixels = false;
        for y in 0..32 {
            for x in 0..192 {
                if fb.get_pixel(x, y) != (0, 0, 0) {
                    has_pixels = true;
                    break;
                }
            }
            if has_pixels {
                break;
            }
        }
        assert!(has_pixels, "frame should have some non-black pixels");
    }

    #[test]
    fn test_render_frame_empty_snapshot() {
        let mut renderer = Renderer::new();
        let snapshot = DisplaySnapshot::empty();
        let fb = renderer.render_frame(&snapshot, 0, false, 0.0, false, None);
        assert_eq!(fb.width(), 192);
        assert_eq!(fb.height(), 32);
    }

    #[test]
    fn test_render_frame_flash_state() {
        let mut renderer = Renderer::new();
        let snapshot = DisplaySnapshot {
            trains: vec![make_train("1", "Van Cortlandt", 0, false)], // arriving!
            alerts: Vec::new(),
            fetched_at: 1000.0,
        };

        // Flash on — time should be black (invisible)
        let fb_on = renderer.render_frame(&snapshot, 0, true, 0.0, false, None);
        // Flash off — time should be red
        let fb_off = renderer.render_frame(&snapshot, 0, false, 0.0, false, None);

        // The two frames should differ (flash state changes pixel colors)
        let mut differs = false;
        for y in 0..32 {
            for x in 0..192 {
                if fb_on.get_pixel(x, y) != fb_off.get_pixel(x, y) {
                    differs = true;
                    break;
                }
            }
            if differs {
                break;
            }
        }
        assert!(differs, "flash on/off frames should differ for arriving train");
    }

    #[test]
    fn test_render_alert_with_icons() {
        let renderer = Renderer::new();

        let buf = renderer.render_alert_with_icons(
            "Delays on [1] [2] trains due to signal problems",
        );

        assert!(buf.width() > 0);
        assert_eq!(buf.height(), 17);
    }

    #[test]
    fn test_render_alert_no_icons() {
        let renderer = Renderer::new();

        let buf = renderer.render_alert_with_icons("Service change in effect");

        assert!(buf.width() > 0);
        assert_eq!(buf.height(), 17);
    }

    #[test]
    fn test_truncate_text() {
        let renderer = Renderer::new();
        let font = fonts::get_font();

        let text = "Van Cortlandt Park-242 St";
        let truncated = renderer.truncate_text(font, text, 80);
        assert!(
            font.measure_text(&truncated, CHAR_SPACING, false) <= 80,
            "truncated text should fit within 80px"
        );

        // Short text should not be truncated
        let short = "42 St";
        assert_eq!(renderer.truncate_text(font, short, 200), short);
    }

    #[test]
    fn test_scroll_complete_distance() {
        let mut renderer = Renderer::new();
        let mut routes = HashSet::new();
        routes.insert("A".into());

        let alert = Alert {
            text: "Service suspended on [A] train".into(),
            affected_routes: routes,
            priority: 1,
            alert_id: "test".into(),
            active_periods: Vec::new(),
        };

        let snapshot = DisplaySnapshot {
            trains: vec![make_train("1", "Test", 5, false)],
            alerts: vec![alert.clone()],
            fetched_at: 0.0,
        };

        // Render a frame with alert to populate last_alert_width
        renderer.render_frame(&snapshot, 0, false, 0.0, true, Some(&alert));

        let dist = renderer.get_scroll_complete_distance();
        assert!(dist > 192, "scroll distance should exceed screen width");
    }

    #[test]
    fn test_render_frame_with_alert_scroll() {
        let mut renderer = Renderer::new();
        let mut routes = HashSet::new();
        routes.insert("1".into());

        let alert = Alert {
            text: "Delays on [1] trains".into(),
            affected_routes: routes,
            priority: 1,
            alert_id: "test".into(),
            active_periods: Vec::new(),
        };

        let snapshot = DisplaySnapshot {
            trains: vec![make_train("1", "Test", 5, false)],
            alerts: vec![alert.clone()],
            fetched_at: 0.0,
        };

        // Render at different scroll positions
        let fb1 = renderer.render_frame(&snapshot, 0, false, 0.0, true, Some(&alert));
        let fb2 = renderer.render_frame(&snapshot, 0, false, 50.0, true, Some(&alert));

        // The bottom halves should differ (alert scrolled)
        let mut differs = false;
        for y in 16..32 {
            for x in 0..192 {
                if fb1.get_pixel(x, y) != fb2.get_pixel(x, y) {
                    differs = true;
                    break;
                }
            }
            if differs {
                break;
            }
        }
        assert!(differs, "different scroll offsets should produce different frames");
    }

    #[test]
    #[ignore]
    fn test_render_ppm_output() {
        use std::io::Write;

        let mut renderer = Renderer::new();
        let snapshot = DisplaySnapshot {
            trains: vec![
                make_train("1", "Van Cortlandt Park", 2, false),
                make_train("A", "Far Rockaway", 5, true),
                make_train("7", "Flushing", 8, false),
            ],
            alerts: Vec::new(),
            fetched_at: 1000.0,
        };

        let fb = renderer.render_frame(&snapshot, 0, false, 0.0, false, None);

        // Write at 4x scale for visibility
        let scale = 4usize;
        let w = fb.width() * scale;
        let h = fb.height() * scale;
        let mut pixels = vec![0u8; w * h * 3];

        for y in 0..fb.height() {
            for x in 0..fb.width() {
                let px = fb.get_pixel(x, y);
                for sy in 0..scale {
                    for sx in 0..scale {
                        let idx = ((y * scale + sy) * w + x * scale + sx) * 3;
                        pixels[idx] = px.0;
                        pixels[idx + 1] = px.1;
                        pixels[idx + 2] = px.2;
                    }
                }
            }
        }

        let path = "/tmp/mta_renderer_frame.ppm";
        let mut f = std::fs::File::create(path).unwrap();
        write!(f, "P6\n{} {}\n255\n", w, h).unwrap();
        f.write_all(&pixels).unwrap();
        println!("Rendered frame written to {path}");
        println!("Open with: open {path}");
    }

    /// The glyph a single token resolves to, if any.
    fn resolve(token: &str) -> Option<&'static fonts::RouteIcon> {
        let renderer = Renderer::new();
        let caps = renderer.route_pattern.captures(token).expect("token should match");
        Renderer::resolve_token(fonts::get_font(), &caps)
    }

    fn icon(name: &str) -> &'static fonts::RouteIcon {
        fonts::get_font().get_icon(name).unwrap_or_else(|| panic!("{} not loaded", name))
    }

    #[test]
    fn test_tokens_resolve_to_the_right_glyph() {
        let cases = [
            ("[airplane icon]", "ICON_AIRPLANE"),
            ("[shuttle bus icon]", "ICON_SHUTTLE_BUS"),
            ("[accessibility icon]", "ICON_ACCESSIBILITY"),
            ("[SIR]", "ROUTE_SIR_CIRCLE"),
            ("[H]", "ROUTE_H_CIRCLE"),
            ("[FX]", "ROUTE_F_DIAMOND"),
            ("[6X]", "ROUTE_6_DIAMOND"),
            // Plain bullets are local, even on express-capable lines.
            ("[A]", "ROUTE_A_CIRCLE"),
            ("[6]", "ROUTE_6_CIRCLE"),
        ];
        for (token, name) in cases {
            let got = resolve(token).unwrap_or_else(|| panic!("{} did not resolve", token));
            assert!(std::ptr::eq(got, icon(name)), "{} should draw {}", token, name);
        }
    }

    #[test]
    fn test_unknown_token_renders_as_text() {
        assert!(resolve("[ZZ]").is_none());
        assert!(resolve("[mystery icon]").is_none());

        let renderer = Renderer::new();
        let font = fonts::get_font();
        let buf = renderer.render_alert_with_icons("[ZZ]");
        assert_eq!(buf.width(), font.measure_text("ZZ", CHAR_SPACING, true));
    }

    #[test]
    fn test_express_diamonds_load() {
        // The diamonds are 15x15; they were once declared 14x13, so the loader
        // skipped them and, once loaded, would have cut off the west point.
        let diamond = fonts::get_font().get_route_icon("A", true).unwrap();
        assert_eq!(diamond.pixels.len(), 15);
        assert_eq!(diamond.width, 15, "a diamond with single-pixel tips is 15 wide");
        assert!(!std::ptr::eq(diamond, icon("ROUTE_A_CIRCLE")));
    }

    #[test]
    fn test_real_alert_headers_fully_drawable() {
        // Subway alert headers from the live MTA feed (2026-10-04). Every token
        // must resolve to a glyph and every character must have one.
        let renderer = Renderer::new();
        let font = fonts::get_font();
        let mut problems = Vec::new();
        for header in include_str!("../../tests/fixtures/alert_headers.txt").lines() {
            let text = crate::mta::client::clean_alert_text(header);
            for caps in renderer.route_pattern.captures_iter(&text) {
                if Renderer::resolve_token(font, &caps).is_none() {
                    problems.push(format!("token {}", &caps[0]));
                }
            }
            for ch in renderer.route_pattern.replace_all(&text, "").chars() {
                if ch != ' ' && font.get_char_bitmap(ch, false).is_none() {
                    problems.push(format!("char {:?}", ch));
                }
            }
        }
        problems.sort();
        problems.dedup();
        assert!(problems.is_empty(), "undrawable: {:?}", problems);
    }

    fn clock_at(y: i32, mo: u32, d: u32, h: u32, mi: u32, sec: u32) -> chrono::DateTime<chrono::FixedOffset> {
        use chrono::TimeZone;
        chrono::FixedOffset::west_opt(7 * 3600)
            .unwrap()
            .with_ymd_and_hms(y, mo, d, h, mi, sec)
            .unwrap()
    }

    #[test]
    fn test_clock_text_matches_mta_format() {
        // As photographed on a retired countdown clock at Spring St.
        let (date, time) = Renderer::clock_text(&clock_at(2024, 4, 23, 18, 5, 39));
        assert_eq!(date, "Tue, April 23, 2024");
        assert_eq!(time, "6:05:39 PM");

        let (_, morning) = Renderer::clock_text(&clock_at(2024, 4, 23, 0, 7, 3));
        assert_eq!(morning, "12:07:03 AM");
    }

    #[test]
    fn test_clock_face_layout() {
        let mut renderer = Renderer::new();
        // Longest month and weekday name must still fit the 192px width.
        let fb = renderer.render_clock(&clock_at(2026, 9, 30, 22, 58, 58));
        let lit = |y0: usize, y1: usize, color| {
            let xs: Vec<usize> = (0..fb.width())
                .filter(|&x| (y0..y1).any(|y| fb.get_pixel(x, y) == color))
                .collect();
            (*xs.first().unwrap(), *xs.last().unwrap())
        };
        let (dl, dr) = lit(0, 16, COLOR_GREEN);
        let (tl, tr) = lit(16, 32, COLOR_RED);
        assert!(dl > 0 && dr < fb.width() - 1, "date fits: {}..{}", dl, dr);
        // Centred to within a pixel or two of glyph padding.
        assert!((dl as i32 - (fb.width() - 1 - dr) as i32).abs() <= 3, "date centred: {}..{}", dl, dr);
        assert!((tl as i32 - (fb.width() - 1 - tr) as i32).abs() <= 3, "time centred: {}..{}", tl, tr);
    }
}
