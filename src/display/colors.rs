/// RGB color tuple.
pub type Rgb = (u8, u8, u8);

// Standard display colors
pub const COLOR_GREEN: Rgb = (0x00, 0xFF, 0x00);
pub const COLOR_RED: Rgb = (0xFF, 0x66, 0x44);
pub const COLOR_ORANGE: Rgb = (0xFF, 0x63, 0x19);
pub const COLOR_BLACK: Rgb = (0x00, 0x00, 0x00);

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
