//! 役割ベースのパレット生成と検証。
//!
//! `AI/tools/make_theme_palette.py` の移植で、階段・重み・検証ルールは同じもの。
//! 仕様は `AI/docs/aviutl2-palette-spec.md` の §2〜§5。

use crate::color::{
    contrast_ratio, fit_gamut, lab_lightness, oklch_to_rgb8, parse_hex, rgb8_to_oklch, to_hex,
};

/// 明るい背景向けの L 階段。仕様書 §2 の暫定値を V1 が通るよう調整したもの。
pub const LADDER_LIGHT: [f64; 8] = [0.985, 0.932, 0.878, 0.805, 0.695, 0.600, 0.492, 0.300];
/// 暗い背景向け。
pub const LADDER_DARK: [f64; 8] = [0.170, 0.235, 0.298, 0.372, 0.462, 0.620, 0.752, 0.930];
/// step6(solid) を頂点とする山型。Open Color v1.9.1 の実測から。
pub const WEIGHT: [f64; 8] = [0.72, 0.78, 0.85, 0.92, 0.98, 1.00, 0.98, 0.88];
/// ニュートラル彩度（ニュートラルの行の OKLCH の C）の上限。画面の範囲と読み込みの検査で同じ値を使う。
/// v0.3.0 までは画面だけ 0.015 で、上限でも灰色からほとんど離れなかった（issues/20261012_neutral_bias_range_too_small.md）
pub const NEUTRAL_BIAS_MAX: f64 = 0.05;

pub const STEP_NAMES: [&str; 8] = [
    "base",
    "subtle",
    "surface",
    "border",
    "strong-border",
    "solid",
    "text-low",
    "text-high",
];

/// 意味が値を決める 4 色。既定は Open Color の 6 番。
pub const SEMANTIC: [(&str, &str); 4] = [
    ("success", "40c057"),
    ("warning", "fab005"),
    ("danger", "fa5252"),
    ("info", "228be6"),
];

pub const ROW_COUNT: usize = 8;
pub const STEP_COUNT: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    pub fn ladder(self) -> &'static [f64; 8] {
        match self {
            Appearance::Light => &LADDER_LIGHT,
            Appearance::Dark => &LADDER_DARK,
        }
    }
    pub fn suffix(self) -> &'static str {
        match self {
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Swatch {
    pub rgb: [u8; 3],
    /// ガマットマッピング前に欲しかった C。V5 の判定に使う
    pub c_want: f64,
    pub c_fit: f64,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub role: String,
    pub seed: [u8; 3],
    pub colors: Vec<Swatch>,
}

#[derive(Clone, Debug)]
pub struct Palette {
    pub appearance: Appearance,
    pub rows: Vec<Row>,
}

impl Palette {
    /// 行優先で 64 色。AviUtl2 のパレットのスロット順そのもの。
    pub fn flat(&self) -> Vec<[u8; 3]> {
        self.rows
            .iter()
            .flat_map(|r| r.colors.iter().map(|c| c.rgb))
            .collect()
    }
}

/// 白・黒に近いほど彩度を保持できないぶんの減衰。
fn damp(l: f64) -> f64 {
    (std::f64::consts::PI * l).sin()
}

fn build_row(role: &str, seed: [u8; 3], ladder: &[f64; 8], neutral_bias: Option<f64>) -> Row {
    let (_, c_seed, hue) = rgb8_to_oklch(seed);
    let mut colors = Vec::with_capacity(STEP_COUNT);
    for (i, &l) in ladder.iter().enumerate() {
        let c_want = match neutral_bias {
            None => c_seed * damp(l) * WEIGHT[i],
            // 全 step でほぼ一定、両端でわずかに減衰
            Some(bias) => bias * (0.30 + 0.70 * damp(l)),
        };
        let c_fit = fit_gamut(l, c_want, hue);
        colors.push(Swatch {
            rgb: oklch_to_rgb8(l, c_fit, hue),
            c_want,
            c_fit,
        });
    }
    Row {
        role: role.to_string(),
        seed,
        colors,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seeds {
    pub main: [u8; 3],
    pub sub: [u8; 3],
    pub accent: [u8; 3],
    pub semantic: [[u8; 3]; 4],
    pub neutral_bias: f64,
}

impl Default for Seeds {
    fn default() -> Self {
        Self {
            main: parse_hex("7950f2").unwrap(),
            sub: parse_hex("12b886").unwrap(),
            accent: parse_hex("fd7e14").unwrap(),
            semantic: [
                parse_hex(SEMANTIC[0].1).unwrap(),
                parse_hex(SEMANTIC[1].1).unwrap(),
                parse_hex(SEMANTIC[2].1).unwrap(),
                parse_hex(SEMANTIC[3].1).unwrap(),
            ],
            neutral_bias: 0.008,
        }
    }
}

pub fn build_palette(seeds: &Seeds, appearance: Appearance) -> Palette {
    let ladder = appearance.ladder();
    let mut rows = Vec::with_capacity(ROW_COUNT);
    rows.push(build_row(
        "neutral",
        seeds.main,
        ladder,
        Some(seeds.neutral_bias),
    ));
    rows.push(build_row("main", seeds.main, ladder, None));
    rows.push(build_row("sub", seeds.sub, ladder, None));
    rows.push(build_row("accent", seeds.accent, ladder, None));
    for (i, (name, _)) in SEMANTIC.iter().enumerate() {
        rows.push(build_row(name, seeds.semantic[i], ladder, None));
    }
    Palette { appearance, rows }
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    /// 通らなかったルール
    pub issues: Vec<String>,
    /// 参考情報（V5 の削られ量、V6 の文字色）
    pub notes: Vec<String>,
    /// 行ごとの solid の上に乗せる文字色（true = 白）
    pub on_solid_white: [bool; ROW_COUNT],
}

/// 仕様書 §5 の V1〜V6。
pub fn validate(pal: &Palette) -> Report {
    validate_inner(pal, true)
}

/// 画面に出す分（通らなかったルールと solid の上の文字色）だけを調べる。参考情報（`notes`）の文字列は作らない
pub fn validate_issues(pal: &Palette) -> Report {
    validate_inner(pal, false)
}

fn validate_inner(pal: &Palette, with_notes: bool) -> Report {
    let mut rep = Report::default();
    let flat = pal.flat();

    for (ri, row) in pal.rows.iter().enumerate() {
        let c = &row.colors;
        // V1 隣接ステップの ΔL* が 5 以上
        for i in 0..c.len() - 1 {
            let d = (lab_lightness(c[i].rgb) - lab_lightness(c[i + 1].rgb)).abs();
            if d < 5.0 {
                rep.issues.push(format!(
                    "V1 {} step{}-{} の ΔL* が {:.1}（5 未満）",
                    row.role,
                    i + 1,
                    i + 2,
                    d
                ));
            }
        }
        // V2 text-high と base / subtle が 7:1 以上
        for bg in 0..2 {
            let cr = contrast_ratio(c[7].rgb, c[bg].rgb);
            if cr < 7.0 {
                rep.issues.push(format!(
                    "V2 {} text-high と {} のコントラストが {:.2}:1（7:1 未満）",
                    row.role, STEP_NAMES[bg], cr
                ));
            }
        }
        // V3 text-low と base / subtle が 4.5:1 以上
        for bg in 0..2 {
            let cr = contrast_ratio(c[6].rgb, c[bg].rgb);
            if cr < 4.5 {
                rep.issues.push(format!(
                    "V3 {} text-low と {} のコントラストが {:.2}:1（4.5:1 未満）",
                    row.role, STEP_NAMES[bg], cr
                ));
            }
        }
        // V5 ガマットマッピングで C が半分以上削られたもの
        for (i, sw) in c.iter().enumerate() {
            if with_notes && sw.c_want > 0.0 && sw.c_fit < sw.c_want * 0.5 {
                rep.notes.push(format!(
                    "V5 {} {} は C が {:.3}→{:.3}（{:.0}% 削られた）",
                    row.role,
                    STEP_NAMES[i],
                    sw.c_want,
                    sw.c_fit,
                    100.0 * (1.0 - sw.c_fit / sw.c_want)
                ));
            }
        }
        // V6 solid の上に乗る文字色
        let solid = c[5].rgb;
        let on_white = contrast_ratio(solid, [255, 255, 255]);
        let on_black = contrast_ratio(solid, [0, 0, 0]);
        let white = on_white >= on_black;
        if ri < ROW_COUNT {
            rep.on_solid_white[ri] = white;
        }
        if !with_notes {
            continue;
        }
        rep.notes.push(format!(
            "V6 {} solid #{} の上は {} 文字（白 {:.2}:1 / 黒 {:.2}:1）",
            row.role,
            to_hex(solid),
            if white { "白" } else { "黒" },
            on_white,
            on_black
        ));
    }

    // V4 64 色に完全重複が無いこと
    let mut dup: Vec<String> = Vec::new();
    for (i, a) in flat.iter().enumerate() {
        if flat[..i].contains(a) && !dup.contains(&to_hex(*a)) {
            dup.push(to_hex(*a));
        }
    }
    if !dup.is_empty() {
        rep.issues.push(format!(
            "V4 重複 {} 色: {}",
            dup.len(),
            dup.iter()
                .map(|d| format!("#{d}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    rep
}

/// 同じ色を 2 行に使っていないか。V4 の重複はほぼこれが原因になる。
pub fn seed_clashes(seeds: &Seeds) -> Vec<String> {
    let mut named: Vec<(&str, [u8; 3])> = vec![
        ("main", seeds.main),
        ("sub", seeds.sub),
        ("accent", seeds.accent),
    ];
    for (i, (name, _)) in SEMANTIC.iter().enumerate() {
        named.push((name, seeds.semantic[i]));
    }
    let mut out = Vec::new();
    for i in 0..named.len() {
        for j in i + 1..named.len() {
            if named[i].1 == named[j].1 {
                out.push(format!(
                    "#{} を {} と {} が共有している（その 8 色が丸ごと重複する）",
                    to_hex(named[i].1),
                    named[i].0,
                    named[j].0
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_seeds_pass_all_rules() {
        for mode in [Appearance::Light, Appearance::Dark] {
            let pal = build_palette(&Seeds::default(), mode);
            let rep = validate(&pal);
            assert!(
                rep.issues.is_empty(),
                "{:?} で検証が落ちた: {:?}",
                mode,
                rep.issues
            );
            assert_eq!(pal.flat().len(), 64);
        }
    }

    #[test]
    fn matches_python_output() {
        // make_theme_palette.py --dry-run（既定シード）の light 1〜2 行目
        let pal = build_palette(&Seeds::default(), Appearance::Light);
        let flat = pal.flat();
        let head: Vec<String> = flat[..16].iter().map(|c| to_hex(*c)).collect();
        assert_eq!(
            head,
            vec![
                "fafafc", "e8e8eb", "d6d6da", "bfbfc3", "9c9ca1", "807f85", "616065", "2d2d31",
                "faf9ff", "e7e5ff", "d4d1ff", "bcb5ff", "9b87ff", "815ef7", "6436d5", "310778",
            ]
        );
    }

    #[test]
    fn clashing_seed_is_detected() {
        let mut seeds = Seeds::default();
        seeds.accent = parse_hex(SEMANTIC[1].1).unwrap(); // warning と同じ
        assert!(!seed_clashes(&seeds).is_empty());
        let rep = validate(&build_palette(&seeds, Appearance::Light));
        assert!(rep.issues.iter().any(|i| i.starts_with("V4")));
    }

    #[test]
    fn issues_only_matches_full_validation() {
        let mut seeds = Seeds::default();
        seeds.accent = parse_hex(SEMANTIC[1].1).unwrap();
        for mode in [Appearance::Light, Appearance::Dark] {
            let pal = build_palette(&seeds, mode);
            let full = validate(&pal);
            let quick = validate_issues(&pal);
            assert_eq!(full.issues, quick.issues);
            assert_eq!(full.on_solid_white, quick.on_solid_white);
            assert!(quick.notes.is_empty());
            assert!(!full.notes.is_empty());
        }
    }

    #[test]
    fn broken_palette_fails_validation() {
        let mut pal = build_palette(&Seeds::default(), Appearance::Light);
        for row in pal.rows.iter_mut() {
            for sw in row.colors.iter_mut() {
                sw.rgb = [128, 128, 128];
            }
        }
        let rep = validate(&pal);
        assert!(rep.issues.iter().any(|i| i.starts_with("V1")));
        assert!(rep.issues.iter().any(|i| i.starts_with("V2")));
        assert!(rep.issues.iter().any(|i| i.starts_with("V4")));
    }

    /// 上限のニュートラル彩度で、ニュートラルの行が灰色から目に見えて離れる（v0.3.0 の 0.015 では最大 8 段しか動かなかった）
    #[test]
    fn neutral_bias_max_is_visibly_tinted() {
        let spread = |bias: f64| {
            let seeds = Seeds { neutral_bias: bias, ..Seeds::default() };
            let pal = build_palette(&seeds, Appearance::Light);
            pal.rows[0]
                .colors
                .iter()
                .map(|c| {
                    let (lo, hi) = (*c.rgb.iter().min().unwrap(), *c.rgb.iter().max().unwrap());
                    hi - lo
                })
                .max()
                .unwrap()
        };
        assert!(spread(0.0) <= 1, "0 は無彩色");
        assert!(spread(NEUTRAL_BIAS_MAX) >= 24, "上限で色味が見える（{}）", spread(NEUTRAL_BIAS_MAX));
        assert!(spread(0.015) < spread(NEUTRAL_BIAS_MAX));
    }
}
