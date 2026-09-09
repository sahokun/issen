/// Parses a `#rrggbb` or `rrggbb` hex string into RGB (0-255).
pub fn parse_hex(input: &str) -> Option<(u8, u8, u8)> {
    let s = input.trim().trim_start_matches('#');
    // Check ASCII, not just byte length, before slicing by byte index — a string
    // mixing in non-ASCII characters can happen to have a byte length of 6 while its
    // byte boundaries don't align with char boundaries, which panics on slicing.
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

pub fn to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Converts RGB (0-255) to HSL (hue 0-360 degrees, saturation/lightness 0-100%).
pub fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let l = (max + min) / 2.0;

    if delta == 0.0 {
        return (0.0, 0.0, l * 100.0);
    }

    let s = if l < 0.5 {
        delta / (max + min)
    } else {
        delta / (2.0 - max - min)
    };

    let h = if max == r {
        ((g - b) / delta) % 6.0
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    let h = h * 60.0;
    let h = if h < 0.0 { h + 360.0 } else { h };

    (h, s * 100.0, l * 100.0)
}

/// Converts RGB (0-255) to HSV (hue 0-360 degrees, saturation/value 0-1).
/// Used by the color picker tool's saturation/value square and hue bar,
/// which need HSV rather than HSL: an HSV square keeps a single degenerate
/// edge (value=0 is uniformly black), while an HSL square would have two
/// (lightness=0 *and* lightness=1 both collapse to a single color),
/// wasting half the square. See `tools/mod.rs::ToolsWindow`'s `hue`/`sat`/
/// `val` fields for why hue/saturation are kept as persistent state rather
/// than recomputed from RGB on every drag step.
pub fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let v = max;
    let s = if max == 0.0 { 0.0 } else { delta / max };

    if delta == 0.0 {
        return (0.0, s, v);
    }

    let h = if max == r {
        ((g - b) / delta) % 6.0
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    let h = h * 60.0;
    let h = if h < 0.0 { h + 360.0 } else { h };

    (h, s, v)
}

/// Converts HSV (hue 0-360 degrees, saturation/value 0-1) to RGB (0-255).
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0);
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);

    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_with_and_without_hash() {
        assert_eq!(parse_hex("#F2A93B"), Some((0xF2, 0xA9, 0x3B)));
        assert_eq!(parse_hex("F2A93B"), Some((0xF2, 0xA9, 0x3B)));
        assert_eq!(parse_hex("nope"), None);
    }

    #[test]
    fn does_not_panic_on_non_ascii_input_with_matching_byte_length() {
        // "aé345" is 1+2+1+1+1 = 6 bytes but not 6 chars; byte-index slicing here
        // would previously land mid-codepoint and panic.
        assert_eq!(parse_hex("aé345"), None);
    }

    #[test]
    fn formats_hex_uppercase() {
        assert_eq!(to_hex(0xF2, 0xA9, 0x3B), "#F2A93B");
    }

    #[test]
    fn pure_red_hsl() {
        let (h, s, l) = rgb_to_hsl(255, 0, 0);
        assert!((h - 0.0).abs() < 0.01);
        assert!((s - 100.0).abs() < 0.01);
        assert!((l - 50.0).abs() < 0.01);
    }

    #[test]
    fn rgb_hsv_roundtrip_is_close() {
        let (r, g, b) = (0xF2, 0xA9, 0x3B);
        let (h, s, v) = rgb_to_hsv(r, g, b);
        let (r2, g2, b2) = hsv_to_rgb(h, s, v);
        assert!((r as i32 - r2 as i32).abs() <= 1);
        assert!((g as i32 - g2 as i32).abs() <= 1);
        assert!((b as i32 - b2 as i32).abs() <= 1);
    }

    #[test]
    fn pure_red_hsv() {
        let (h, s, v) = rgb_to_hsv(255, 0, 0);
        assert!((h - 0.0).abs() < 0.01);
        assert!((s - 1.0).abs() < 0.01);
        assert!((v - 1.0).abs() < 0.01);
    }

    #[test]
    fn hsv_black_has_zero_value() {
        assert_eq!(hsv_to_rgb(0.0, 0.0, 0.0), (0, 0, 0));
        assert_eq!(rgb_to_hsv(0, 0, 0), (0.0, 0.0, 0.0));
    }

    #[test]
    fn hsv_white_has_zero_saturation() {
        assert_eq!(hsv_to_rgb(120.0, 0.0, 1.0), (255, 255, 255));
    }
}
