//! naz, soft: a colour of naz with its strength capped, for the dark theme's
//! second palette (`appearance.dark_palette = "naz-soft"`, `docs/specs/look.md`).
//! The hue stays; the OKLCH chroma is at most 0.095 (0.14 for the reds, which
//! would turn to pink) and the lightness at most 0.90. The neutrals are a
//! table. `web/src/reader.css` has the same values, made by the same rule:
//! change one, change the other. The port of `SoftPalette.swift`.

/// The neutrals of naz and what each becomes.
const NEUTRALS: &[(u32, u32)] =
    &[(0x32_3232, 0x2F_2F2D), (0x3A_3A35, 0x38_3835), (0x28_2826, 0x27_2725), (0xF5_F5F0, 0xE2_E2DD), (0x4A_4A4A, 0x47_4744)];
/// The reds keep more of their strength.
const REDS: &[u32] = &[0xFF_5070, 0xFF_7088];

/// [rgb] (`0xRRGGBB`) as naz, soft has it.
#[must_use]
pub fn soften(rgb: u32) -> u32 {
    if let Some((_, soft)) = NEUTRALS.iter().find(|(naz, _)| *naz == rgb) {
        return *soft;
    }
    let (l, a, b) = oklab(rgb);
    let chroma = a.hypot(b);
    let cap = if REDS.contains(&rgb) { 0.14 } else { 0.095 };
    let scale = if chroma > cap { cap / chroma } else { 1.0 };
    srgb(l.min(0.90), a * scale, b * scale)
}

fn oklab(rgb: u32) -> (f64, f64, f64) {
    let linear = |shift: u32| {
        let c = f64::from((rgb >> shift) & 0xFF) / 255.0;
        if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (linear(16), linear(8), linear(0));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    (
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    )
}

fn srgb(l: f64, a: f64, b: f64) -> u32 {
    let l3 = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m3 = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s3 = (l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
    let byte = |x: f64| {
        let c = if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * x.max(0.0).powf(1.0 / 2.4) - 0.055 };
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to 0..=255 first")]
        let value = (255.0 * c).round().clamp(0.0, 255.0) as u32;
        value
    };
    let r = byte(4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3);
    let g = byte(-1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3);
    let blue = byte(-0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701_0 * s3);
    r << 16 | g << 8 | blue
}

#[cfg(test)]
mod tests {
    use super::soften;

    /// The values are those of `:root[data-palette="soft"]` in `web/src/reader.css`.
    #[test]
    fn softens_naz_as_the_reader_does() {
        assert_eq!(soften(0xFF_B040), 0xE9_B87D); // accent
        assert_eq!(soften(0xFF_FF88), 0xE3_E499); // heading 2
        assert_eq!(soften(0x60_D8FF), 0x7D_D5F4); // heading 3
        assert_eq!(soften(0x20_FFA0), 0xA3_ECBF); // green
        assert_eq!(soften(0xFF_5070), 0xE1_717E); // red keeps more strength
    }

    #[test]
    fn neutrals_are_a_table() {
        assert_eq!(soften(0x32_3232), 0x2F_2F2D);
        assert_eq!(soften(0xF5_F5F0), 0xE2_E2DD);
    }

    #[test]
    fn a_grey_stays_as_it_is() {
        assert_eq!(soften(0xA8_A8A0), 0xA8_A8A0);
    }
}
