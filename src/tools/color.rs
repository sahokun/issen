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

/// Converts HSL (hue 0-360 degrees, saturation/lightness 0-100%) to RGB (0-255). Used
/// when editing the color tool's HSL fields directly
/// (`tools/mod.rs::show_color_picker`).
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0);
    let s = (s / 100.0).clamp(0.0, 1.0);
    let l = (l / 100.0).clamp(0.0, 1.0);

    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }

    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

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
    fn rgb_hsl_roundtrip_is_close() {
        let (r, g, b) = (0xF2, 0xA9, 0x3B);
        let (h, s, l) = rgb_to_hsl(r, g, b);
        let (r2, g2, b2) = hsl_to_rgb(h, s, l);
        assert!((r as i32 - r2 as i32).abs() <= 1);
        assert!((g as i32 - g2 as i32).abs() <= 1);
        assert!((b as i32 - b2 as i32).abs() <= 1);
    }

    #[test]
    fn holding_hue_and_saturation_survives_round_trip_through_white() {
        // The color tool's HSL fields hold h/s/l as editable state and don't recompute
        // h/s from `self.color` on an L-only change (see
        // `tools/mod.rs::ToolsState::hsl`). This test checks the `hsl_to_rgb`
        // assumption that design relies on: holding red's h/s while round-tripping L
        // through 100 (white) and back to 50 returns to red, as long as h/s aren't
        // recomputed from the fully-desaturated white in between.
        let (h, s, _l) = rgb_to_hsl(255, 0, 0);
        assert_eq!(hsl_to_rgb(h, s, 100.0), (255, 255, 255));
        let back = hsl_to_rgb(h, s, 50.0);
        assert!((back.0 as i32 - 255).abs() <= 1);
        assert!((back.1 as i32).abs() <= 1);
        assert!((back.2 as i32).abs() <= 1);
    }

    #[test]
    fn pure_red_hsl() {
        let (h, s, l) = rgb_to_hsl(255, 0, 0);
        assert!((h - 0.0).abs() < 0.01);
        assert!((s - 100.0).abs() < 0.01);
        assert!((l - 50.0).abs() < 0.01);
    }
}
