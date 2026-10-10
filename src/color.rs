//! sRGB / OKLCH の変換とガマットマッピング。
//!
//! `AI/tools/make_theme_palette.py` の移植。**数値は Python 版と一致させること**
//! （下の `tests` が既知値で照合している）。係数は Björn Ottosson の OKLab。

/// sRGB のガンマを外して線形にする。
pub fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// 線形値に sRGB のガンマを掛ける。
pub fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn cbrt(x: f64) -> f64 {
    if x >= 0.0 {
        x.powf(1.0 / 3.0)
    } else {
        -(-x).powf(1.0 / 3.0)
    }
}

fn linear_to_oklab(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    let (l, m, s) = (cbrt(l), cbrt(m), cbrt(s));
    (
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    )
}

fn oklab_to_linear(bl: f64, a: f64, b: f64) -> (f64, f64, f64) {
    let l = (bl + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (bl - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (bl - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    (
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    )
}

/// 8bit sRGB を OKLCH (L, C, H[deg]) にする。
pub fn rgb8_to_oklch(rgb: [u8; 3]) -> (f64, f64, f64) {
    let [r, g, b] = rgb.map(|v| srgb_to_linear(v as f64 / 255.0));
    let (l, a, bb) = linear_to_oklab(r, g, b);
    let c = (a * a + bb * bb).sqrt();
    let h = bb.atan2(a).to_degrees().rem_euclid(360.0);
    (l, c, h)
}

fn oklch_to_linear(l: f64, c: f64, h: f64) -> (f64, f64, f64) {
    let rad = h.to_radians();
    oklab_to_linear(l, c * rad.cos(), c * rad.sin())
}

/// その OKLCH が sRGB に収まるか。
pub fn in_gamut(l: f64, c: f64, h: f64) -> bool {
    let (r, g, b) = oklch_to_linear(l, c, h);
    const EPS: f64 = 1e-6;
    [r, g, b].iter().all(|v| *v >= -EPS && *v <= 1.0 + EPS)
}

/// L と H を保ったまま、sRGB に収まる最大の C を二分探索する。
pub fn fit_gamut(l: f64, c: f64, h: f64) -> f64 {
    if in_gamut(l, c, h) {
        return c;
    }
    let (mut lo, mut hi) = (0.0, c);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if in_gamut(l, mid, h) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// OKLCH を 8bit sRGB へ。ガマット外はクランプされるので、先に `fit_gamut` を通すこと。
pub fn oklch_to_rgb8(l: f64, c: f64, h: f64) -> [u8; 3] {
    let (r, g, b) = oklch_to_linear(l, c, h);
    [r, g, b].map(|v| (linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u8)
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let t = s.trim().trim_start_matches('#').to_ascii_lowercase();
    let t = match t.len() {
        3 => t.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => t,
        _ => return None,
    };
    if !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&t[0..2], 16).ok()?,
        u8::from_str_radix(&t[2..4], 16).ok()?,
        u8::from_str_radix(&t[4..6], 16).ok()?,
    ])
}

pub fn to_hex(rgb: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// WCAG の相対輝度。
pub fn relative_luminance(rgb: [u8; 3]) -> f64 {
    let [r, g, b] = rgb.map(|v| srgb_to_linear(v as f64 / 255.0));
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG のコントラスト比（1.0〜21.0）。
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (mut la, mut lb) = (relative_luminance(a), relative_luminance(b));
    if la < lb {
        std::mem::swap(&mut la, &mut lb);
    }
    (la + 0.05) / (lb + 0.05)
}

/// CIE L*（0-100）。隣接ステップの ΔL* 判定に使う。
pub fn lab_lightness(rgb: [u8; 3]) -> f64 {
    let y = relative_luminance(rgb);
    let d = 6.0 / 29.0;
    let f = if y > d * d * d {
        y.powf(1.0 / 3.0)
    } else {
        y / (3.0 * d * d) + 4.0 / 29.0
    };
    116.0 * f - 16.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_and_black() {
        let (l, c, _) = rgb8_to_oklch([255, 255, 255]);
        assert!((l - 1.0).abs() < 1e-3, "white L={l}");
        assert!(c < 1e-3, "white C={c}");
        let (l, _, _) = rgb8_to_oklch([0, 0, 0]);
        assert!(l.abs() < 1e-6, "black L={l}");
    }

    #[test]
    fn roundtrip_keeps_hex() {
        // Python 版の自己検査と同じ標本
        for hex in ["7950f2", "fab005", "228be6", "40c057", "c92a2a", "0b7285"] {
            let rgb = parse_hex(hex).unwrap();
            let (l, c, h) = rgb8_to_oklch(rgb);
            assert_eq!(to_hex(oklch_to_rgb8(l, c, h)), hex, "roundtrip {hex}");
        }
    }

    #[test]
    fn fit_gamut_never_escapes_srgb() {
        let mut hue = 0.0;
        while hue < 360.0 {
            for l in [0.15, 0.3, 0.5, 0.7, 0.9, 0.985] {
                let c = fit_gamut(l, 0.4, hue);
                assert!(in_gamut(l, c, hue), "hue={hue} L={l} C={c}");
            }
            hue += 15.0;
        }
    }

    #[test]
    fn measurement_known_values() {
        assert!((lab_lightness([255, 255, 255]) - 100.0).abs() < 1e-6);
        assert!(lab_lightness([0, 0, 0]).abs() < 1e-6);
        assert!((contrast_ratio([255, 255, 255], [0, 0, 0]) - 21.0).abs() < 1e-6);
    }

    #[test]
    fn parse_hex_forms() {
        assert_eq!(parse_hex("#FFF"), Some([255, 255, 255]));
        assert_eq!(parse_hex("7950f2"), Some([0x79, 0x50, 0xf2]));
        assert_eq!(parse_hex("zzzzzz"), None);
        assert_eq!(parse_hex("12345"), None);
    }
}
