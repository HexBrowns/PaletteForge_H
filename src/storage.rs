//! `.palette` の書き出しと、台帳（作ったキャラのシード色）の読み書き。
//!
//! 台帳の場所は `ledger_path`。開発環境（`AI/palettes/palettes.json` がある）ではそれを使い、`AI/tools/build_palettes.py` と
//! **互換**の形で同じファイルを読む。利用者の環境では `Plugin/PaletteForge_H/palettes.json`（データフォルダに `AI` を作らない）。
//! 読めない台帳は `.broken-{秒}` へ退避し、上書きしない（ルール au2-plugin-checklist「設定ファイルの下位互換」）。
//!
//! 書くときは、その直前に読み直して自分の 1 件だけを足す・消す（`update_ledger_at`）。起動中に CLI が足したキャラを消さない。
//! この版が知らない項目は `extra` に持って、そのまま書き戻す。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::color::{parse_hex, to_hex};
use crate::theme::{Appearance, Palette, Seeds, SEMANTIC};

/// ラベル名に使えない文字。`.` はラベル区切りなので 1 個までしか使えず、
/// 可変部分（キャラ名）には入れられない。
pub const BAD_CHARS: &str = "\\/:*?\"<>|.";

pub fn app_root() -> PathBuf {
    aviutl2::config::app_data_path()
}

pub fn palette_dir() -> PathBuf {
    app_root().join("Default")
}

/// 台帳の場所。開発環境の `AI/palettes/palettes.json` があればそれ、無ければ `Plugin/PaletteForge_H/palettes.json`
pub fn ledger_path() -> PathBuf {
    let dev = app_root().join("AI").join("palettes").join("palettes.json");
    if dev.is_file() {
        return dev;
    }
    app_root().join("Plugin").join("PaletteForge_H").join("palettes.json")
}

/// 台帳が読めず、退避もできなかった。その間は保存しない（読めなかったファイルを今の中身で上書きしない）
static SAVE_BLOCKED: AtomicBool = AtomicBool::new(false);

pub fn check_name(name: &str) -> Result<String> {
    let t = name.trim();
    if t.is_empty() {
        bail!("名前が空");
    }
    if let Some(ch) = t.chars().find(|c| BAD_CHARS.contains(*c)) {
        if ch == '.' {
            bail!("'.' はラベルの区切りなので名前には使えない");
        }
        bail!("名前に使えない文字 '{ch}' が入っている");
    }
    if t.chars().count() > 40 {
        bail!("名前が長すぎる（40 文字まで）");
    }
    Ok(t.to_string())
}

/// `Chara.<名前> Light.palette` のようなファイル名。
pub fn palette_file_name(label: &str, name: &str, appearance: Appearance) -> String {
    if name.is_empty() {
        format!("{label}.{}.palette", appearance.suffix())
    } else {
        format!("{label}.{name} {}.palette", appearance.suffix())
    }
}

/// 行優先で `index=rrggbb` を 64 行。UTF-8 BOM 無し / CRLF。
pub fn palette_body(pal: &Palette) -> String {
    let mut s = String::with_capacity(64 * 12);
    for (i, rgb) in pal.flat().iter().enumerate() {
        s.push_str(&format!("{i}={}\r\n", to_hex(*rgb)));
    }
    s
}

pub fn write_palette(file_name: &str, pal: &Palette) -> Result<PathBuf> {
    let dir = palette_dir();
    if !dir.is_dir() {
        bail!("Default フォルダが見つからない: {}", dir.display());
    }
    let path = dir.join(file_name);
    std::fs::write(&path, palette_body(pal))
        .with_context(|| format!("書き出し失敗: {}", path.display()))?;
    Ok(path)
}

pub fn palette_exists(file_name: &str) -> bool {
    palette_dir().join(file_name).is_file()
}

pub fn remove_palette(file_name: &str) -> Result<bool> {
    let path = palette_dir().join(file_name);
    if !path.is_file() {
        return Ok(false);
    }
    std::fs::remove_file(&path).with_context(|| format!("削除失敗: {}", path.display()))?;
    Ok(true)
}

// ---------------------------------------------------------------- 台帳

fn default_bias() -> f64 {
    0.008
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub main: String,
    pub sub: String,
    pub accent: String,
    #[serde(default = "default_bias")]
    pub neutral_bias: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<BTreeMap<String, String>>,
    /// この版が知らない項目（新しい版や CLI が足したもの）。そのまま書き戻す
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Entry {
    pub fn from_seeds(name: &str, seeds: &Seeds) -> Self {
        let mut semantic = BTreeMap::new();
        for (i, (key, default_hex)) in SEMANTIC.iter().enumerate() {
            let hex = to_hex(seeds.semantic[i]);
            if &hex != default_hex {
                semantic.insert(key.to_string(), hex);
            }
        }
        Self {
            name: name.to_string(),
            main: to_hex(seeds.main),
            sub: to_hex(seeds.sub),
            accent: to_hex(seeds.accent),
            neutral_bias: seeds.neutral_bias,
            semantic: if semantic.is_empty() {
                None
            } else {
                Some(semantic)
            },
            extra: serde_json::Map::new(),
        }
    }

    /// 色だけを `new` に替える。この版が知らない項目は残す
    fn replace_keeping_extra(&mut self, new: Entry) {
        let extra = std::mem::take(&mut self.extra);
        *self = new;
        for (k, v) in extra {
            self.extra.entry(k).or_insert(v);
        }
    }

    /// 壊れた hex は既定値で補う（台帳を手で編集されても落ちないように）。
    pub fn to_seeds(&self) -> Seeds {
        let mut seeds = Seeds::default();
        if let Some(c) = parse_hex(&self.main) {
            seeds.main = c;
        }
        if let Some(c) = parse_hex(&self.sub) {
            seeds.sub = c;
        }
        if let Some(c) = parse_hex(&self.accent) {
            seeds.accent = c;
        }
        if self.neutral_bias.is_finite() && (0.0..=0.05).contains(&self.neutral_bias) {
            seeds.neutral_bias = self.neutral_bias;
        }
        if let Some(over) = &self.semantic {
            for (i, (key, _)) in SEMANTIC.iter().enumerate() {
                if let Some(c) = over.get(*key).and_then(|h| parse_hex(h)) {
                    seeds.semantic[i] = c;
                }
            }
        }
        seeds
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FixedSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core: Option<Entry>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub fixed: FixedSet,
    #[serde(default)]
    pub characters: Vec<Entry>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Ledger {
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.characters.iter().find(|c| c.name == name)
    }

    /// 同じ名前があれば色を替え（知らない項目は残す）、無ければ末尾に足す
    pub fn upsert(&mut self, entry: Entry) {
        match self.characters.iter_mut().find(|c| c.name == entry.name) {
            Some(slot) => slot.replace_keeping_extra(entry),
            None => self.characters.push(entry),
        }
    }

    /// Core の色を替える（知らない項目は残す）
    pub fn set_core(&mut self, entry: Entry) {
        match self.fixed.core.as_mut() {
            Some(slot) => slot.replace_keeping_extra(entry),
            None => self.fixed.core = Some(entry),
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.characters.len();
        self.characters.retain(|c| c.name != name);
        self.characters.len() != before
    }
}

pub fn load_ledger() -> Ledger {
    load_ledger_at(&ledger_path())
}

/// 読めない（解析の失敗・UTF-8 でない・I/O エラー）ときは `.broken-{秒}` へ退避して空で始める。
/// 退避できなければ、その起動の間は保存しない
pub fn load_ledger_at(path: &Path) -> Ledger {
    let parsed = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Ledger>(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ledger::default(),
        Err(e) => Err(e.to_string()),
    };
    match parsed {
        Ok(l) => l,
        Err(e) => {
            match backup_broken(path) {
                Ok(dest) => tracing::warn!("台帳を読めなかったので {} へ退避し、空で始めます: {e}", dest.display()),
                Err(be) => {
                    SAVE_BLOCKED.store(true, Ordering::Release);
                    tracing::warn!("台帳を読めず、退避もできないので、この起動の間は保存しません: {e} / {be}");
                }
            }
            Ledger::default()
        }
    }
}

/// 台帳を読み直して `change` を当て、書き戻す。戻り値は書いた台帳（CLI が起動中に足した分も入っている）
pub fn update_ledger(change: impl FnOnce(&mut Ledger)) -> Result<Ledger> {
    update_ledger_at(&ledger_path(), change)
}

/// 書く直前にファイルを読み直し、`change`（自分の 1 件の追加・削除）だけを当てて書く。
/// 起動時に読んだ台帳をそのまま書くと、起動中に CLI（`build_palettes.py`）が変えた分が消える。
/// 読み直せない（壊れている・I/O エラー）ときは書かない（上書きすると、その中身が消える）
pub fn update_ledger_at(path: &Path, change: impl FnOnce(&mut Ledger)) -> Result<Ledger> {
    if SAVE_BLOCKED.load(Ordering::Acquire) {
        bail!("台帳を読めなかったので、この起動の間は保存しません（{}）", path.display());
    }
    let mut ledger = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Ledger>(text.trim_start_matches('\u{feff}'))
            .with_context(|| format!("台帳を読み直せない（中身が壊れている）ので保存しません: {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ledger::default(),
        Err(e) => {
            return Err(e).with_context(|| format!("台帳を読み直せないので保存しません: {}", path.display()));
        }
    };
    change(&mut ledger);
    save_ledger_at(path, &ledger)?;
    Ok(ledger)
}

/// 一時ファイルに書いてから置き換える
pub fn save_ledger_at(path: &Path, ledger: &Ledger) -> Result<()> {
    if SAVE_BLOCKED.load(Ordering::Acquire) {
        bail!("台帳を読めなかったので、この起動の間は保存しません（{}）", path.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("フォルダを作れない: {}", dir.display()))?;
    }
    let mut text = serde_json::to_string_pretty(ledger)?;
    text.push('\n');
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text).with_context(|| format!("台帳の保存に失敗: {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("台帳の保存に失敗: {}", path.display()))?;
    Ok(())
}

/// 読めなかった台帳を `{名前}.broken-{秒}` へ移す
fn backup_broken(path: &Path) -> std::io::Result<PathBuf> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".broken-{secs}"));
    let dest = PathBuf::from(name);
    std::fs::rename(path, &dest)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_roundtrip_and_broken_file_is_moved_aside() {
        let dir = std::env::temp_dir().join(format!("palette_forge_h_ledger_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("palettes.json");
        // 無ければ空
        assert!(load_ledger_at(&path).characters.is_empty());
        let mut l = Ledger::default();
        l.upsert(Entry::from_seeds("さくら", &Seeds::default()));
        save_ledger_at(&path, &l).unwrap();
        assert!(!dir.join("palettes.json.tmp").exists());
        assert_eq!(load_ledger_at(&path).characters.len(), 1);
        // 知らない項目（新しい版が足したもの）があっても読める
        std::fs::write(&path, r#"{"characters":[{"name":"a","main":"ff0000","sub":"00ff00","accent":"0000ff","future":1}],"version":2}"#).unwrap();
        assert_eq!(load_ledger_at(&path).characters[0].name, "a");
        // 壊れた台帳は退避され、中身が残る
        std::fs::write(&path, "{ broken").unwrap();
        assert!(load_ledger_at(&path).characters.is_empty());
        assert!(!path.exists());
        let moved: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".broken-"))
            .collect();
        assert_eq!(moved.len(), 1);
        assert_eq!(std::fs::read_to_string(moved[0].path()).unwrap(), "{ broken");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_ledger(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("palette_forge_h_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("palettes.json");
        (dir, path)
    }

    /// 起動時に読んだ後で CLI が足したキャラと、この版が知らない項目を、保存で消さない
    #[test]
    fn update_keeps_cli_changes_and_unknown_fields() {
        let (dir, path) = temp_ledger("update");
        std::fs::write(
            &path,
            r#"{"fixed":{"core":{"main":"7950f2","sub":"12b886","accent":"fd7e14","core_note":"x"},"websafe":true},
"characters":[{"name":"a","main":"ff0000","sub":"00ff00","accent":"0000ff","tags":["主役"]}],"version":2}"#,
        )
        .unwrap();
        // プラグインが起動時に読む
        let startup = load_ledger_at(&path);
        assert_eq!(startup.characters.len(), 1);
        // 起動中に CLI が b を足す
        let mut cli: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        cli["characters"].as_array_mut().unwrap().push(serde_json::json!({
            "name": "b", "main": "111111", "sub": "222222", "accent": "333333"
        }));
        std::fs::write(&path, serde_json::to_string(&cli).unwrap()).unwrap();
        // プラグインが a を直して保存し、新しいキャラ c も足す
        let mut seeds = Seeds::default();
        seeds.main = [1, 2, 3];
        update_ledger_at(&path, |l| l.upsert(Entry::from_seeds("a", &seeds))).unwrap();
        let after = update_ledger_at(&path, |l| l.upsert(Entry::from_seeds("c", &Seeds::default()))).unwrap();
        let names: Vec<&str> = after.characters.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"], "CLI が足した b が残る");
        // 書いたファイルに、知らない項目がそのまま残る
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["version"], 2);
        assert_eq!(v["fixed"]["websafe"], true);
        assert_eq!(v["fixed"]["core"]["core_note"], "x");
        assert_eq!(v["characters"][0]["tags"][0], "主役");
        assert_eq!(v["characters"][0]["main"], "010203");
        // Core を直しても、Core の知らない項目は残る
        update_ledger_at(&path, |l| l.set_core(Entry::from_seeds("", &seeds))).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["fixed"]["core"]["core_note"], "x");
        assert_eq!(v["fixed"]["core"]["main"], "010203");
        // 削除も 1 件だけ
        let after = update_ledger_at(&path, |l| {
            l.remove("a");
        })
        .unwrap();
        assert_eq!(after.characters.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 保存の直前に読み直せない（壊れている）ときは書かない
    #[test]
    fn update_does_not_overwrite_broken_file() {
        let (dir, path) = temp_ledger("update_broken");
        std::fs::write(&path, "{ broken").unwrap();
        assert!(update_ledger_at(&path, |l| l.upsert(Entry::from_seeds("a", &Seeds::default()))).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        // 無ければ作る
        let fresh = dir.join("new.json");
        assert_eq!(update_ledger_at(&fresh, |l| l.upsert(Entry::from_seeds("a", &Seeds::default()))).unwrap().characters.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
    use crate::theme::build_palette;

    #[test]
    fn body_has_64_rows_and_crlf() {
        let pal = build_palette(&Seeds::default(), Appearance::Light);
        let body = palette_body(&pal);
        assert_eq!(body.matches("\r\n").count(), 64);
        assert!(body.starts_with("0="));
        assert!(body.contains("\r\n63="));
        assert!(!body.contains('\u{feff}'), "BOM を含んではいけない");
    }

    #[test]
    fn names_are_checked() {
        assert!(check_name("さくら").is_ok());
        assert!(check_name("a.b").is_err());
        assert!(check_name("  ").is_err());
        assert!(check_name(&"あ".repeat(41)).is_err());
    }

    #[test]
    fn file_names_follow_label_rule() {
        assert_eq!(
            palette_file_name("Chara", "さくら", Appearance::Light),
            "Chara.さくら Light.palette"
        );
        assert_eq!(
            palette_file_name("Core", "", Appearance::Dark),
            "Core.Dark.palette"
        );
        // ラベルの '.' はちょうど 1 個
        assert_eq!(
            palette_file_name("Chara", "さくら", Appearance::Light)
                .matches('.')
                .count(),
            2 // ラベル区切り 1 個 + 拡張子
        );
    }

    #[test]
    fn entry_roundtrip_keeps_seeds() {
        let mut seeds = Seeds::default();
        seeds.semantic[1] = parse_hex("f76707").unwrap();
        let entry = Entry::from_seeds("テスト", &seeds);
        assert!(entry.semantic.as_ref().unwrap().contains_key("warning"));
        let back = entry.to_seeds();
        assert_eq!(back.main, seeds.main);
        assert_eq!(back.semantic[1], seeds.semantic[1]);
    }
}
