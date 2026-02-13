/// RGB color tuple.
pub type Rgb = (u8, u8, u8);

// MTA Official Route Colors (RGB)
// IRT Broadway-Seventh Avenue Line
pub const COLOR_123: Rgb = (0xFF, 0x66, 0x44);
// IRT Lexington Avenue Line
pub const COLOR_456: Rgb = (0x00, 0xFF, 0x00);
// IRT Flushing Line
pub const COLOR_7: Rgb = (0xCC, 0x55, 0x88);
// IRT 42nd Street Shuttle
pub const COLOR_GS: Rgb = (0x99, 0x88, 0x88);
// IND Eighth Avenue Line
pub const COLOR_ACE: Rgb = (0x28, 0x50, 0xAD);
// IND Sixth Avenue Line
pub const COLOR_BDFM: Rgb = (0xFF, 0x63, 0x19);
// IND Crosstown Line
pub const COLOR_G: Rgb = (0x6C, 0xBE, 0x45);
// BMT Canarsie Line
pub const COLOR_L: Rgb = (0xA7, 0xA9, 0xAC);
// BMT Jamaica Line
pub const COLOR_JZ: Rgb = (0x99, 0x66, 0x33);
// BMT Broadway Line
pub const COLOR_NQRW: Rgb = (0xFC, 0xCC, 0x0A);

// Standard display colors
pub const COLOR_GREEN: Rgb = (0x00, 0xFF, 0x00);
pub const COLOR_RED: Rgb = (0xFF, 0x66, 0x44);
pub const COLOR_ORANGE: Rgb = (0xFF, 0x63, 0x19);
pub const COLOR_BLACK: Rgb = (0x00, 0x00, 0x00);

/// Get the RGB color for a route.
pub fn route_color(route: &str) -> Rgb {
    match route {
        "1" | "2" | "3" => COLOR_123,
        "4" | "5" | "6" => COLOR_456,
        "7" => COLOR_7,
        "GS" => COLOR_GS,
        "A" | "C" | "E" => COLOR_ACE,
        "B" | "D" | "F" | "M" => COLOR_BDFM,
        "G" => COLOR_G,
        "L" => COLOR_L,
        "J" | "Z" => COLOR_JZ,
        "N" | "Q" | "R" | "W" => COLOR_NQRW,
        _ => COLOR_GREEN,
    }
}

/// Routes that can run express service.
pub fn is_express_capable(route: &str) -> bool {
    matches!(route, "2" | "3" | "4" | "5" | "6" | "7" | "A" | "D" | "E")
}

/// Convert a hex color string (e.g., "#FF6644") to RGB.
pub fn hex_to_rgb(hex: &str) -> Rgb {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return COLOR_GREEN;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_colors() {
        assert_eq!(route_color("1"), COLOR_123);
        assert_eq!(route_color("A"), COLOR_ACE);
        assert_eq!(route_color("N"), COLOR_NQRW);
        assert_eq!(route_color("L"), COLOR_L);
        assert_eq!(route_color("X"), COLOR_GREEN); // fallback
    }

    #[test]
    fn test_express_capable() {
        assert!(is_express_capable("2"));
        assert!(is_express_capable("A"));
        assert!(!is_express_capable("1"));
        assert!(!is_express_capable("N"));
    }

    #[test]
    fn test_hex_to_rgb() {
        assert_eq!(hex_to_rgb("#FF6644"), (0xFF, 0x66, 0x44));
        assert_eq!(hex_to_rgb("00FF00"), (0, 255, 0));
        assert_eq!(hex_to_rgb("#000000"), (0, 0, 0));
    }
}
