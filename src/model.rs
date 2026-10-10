//! 画面の状態と操作（書き出し・削除・切り替え・確かめ）。
//!
//! 本体の API とファイルは `Backend` 越しに触るので、テストでも組み込む（`ui.rs` は本体の API を使うので組み込まない）。
//! 重い派生値（2 枚のパレットと検証、絞り込んだ一覧）は、もとが変わったときだけ計算し直して覚える
//! （ルール au2-rs-plugin「ウィンドウの重さ」）。

use std::time::{Duration, Instant};

use crate::storage::{self, Entry, Ledger};
use crate::theme::{build_palette, seed_clashes, validate_issues, Appearance, Palette, Report, Seeds};

/// 2 段階の確かめで、1 回目を押してから 2 回目を受け付ける時間
pub const CONFIRM_FOR: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// 固定の既定テーマ。Core.Light / Core.Dark
    Core,
    /// キャラごと。Chara.<名前> Light / Dark
    Chara,
}

/// 編集 API で反映できなかった理由
#[derive(Debug, Clone)]
pub enum ApplyError {
    /// 本体の登録が終わっていない
    NotReady,
    /// 編集に入れなかった（本体 2.1.13 からは、ドラッグ中・出力中などに失敗する）
    Busy,
    /// 本体がそのパレットを受け付けなかった（本体に無いパレット。起動後に作ったファイルは再起動まで本体に無い）
    Refused,
    Other(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::NotReady => write!(f, "編集 API の準備ができていない"),
            ApplyError::Busy => write!(f, "本体が編集を受け付けなかった（操作中・出力中など）"),
            ApplyError::Refused => write!(f, "本体がパレットを受け付けなかった（本体に無いパレット）"),
            ApplyError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// 本体の API とファイル。実物は `ui.rs`、テストは偽物を使う
pub trait Backend {
    /// 既存のパレットを編集 API で更新する
    fn apply_palette(&mut self, name: &str, pal: &Palette) -> Result<(), ApplyError>;
    fn palette_exists(&self, file_name: &str) -> bool;
    fn write_palette(&mut self, file_name: &str, pal: &Palette) -> anyhow::Result<()>;
    /// 消したら true、無かったら false
    fn remove_palette(&mut self, file_name: &str) -> anyhow::Result<bool>;
    /// 台帳を読み直して `change` を当てて書く。戻り値は書いた台帳
    fn update_ledger(&mut self, change: &mut dyn FnMut(&mut Ledger)) -> anyhow::Result<Ledger>;
}

/// 2 段階の確かめが要る操作
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    LoadCore,
    LoadChara(String),
    NewChara,
    /// 既にある書き出し先へ上書きする
    Export,
    Delete,
}

impl Action {
    /// 表示を切り替える（書き出していない色を捨てる）操作か
    pub fn is_switch(&self) -> bool {
        matches!(self, Action::LoadCore | Action::LoadChara(_) | Action::NewChara)
    }
}

/// 1 回目を押した後の待ち。時間が過ぎるか、色・書き出し先・名前が変わったら取り消す
struct Pending {
    action: Action,
    until: Instant,
    seeds: Seeds,
    target: Target,
    name: String,
    prev_status: String,
}

/// 色から計算したもの。色が変わったときだけ作り直す
pub struct Preview {
    seeds: Seeds,
    pub light: Palette,
    pub dark: Palette,
    pub rep_light: Report,
    pub rep_dark: Report,
    pub clashes: Vec<String>,
}

impl Preview {
    fn build(seeds: &Seeds) -> Self {
        let light = build_palette(seeds, Appearance::Light);
        let dark = build_palette(seeds, Appearance::Dark);
        Self {
            rep_light: validate_issues(&light),
            rep_dark: validate_issues(&dark),
            clashes: seed_clashes(seeds),
            seeds: seeds.clone(),
            light,
            dark,
        }
    }
}

pub struct Model {
    pub target: Target,
    pub name: String,
    pub seeds: Seeds,
    pub status: String,
    pub show_semantic: bool,
    /// 台帳の絞り込みに使う文字（入力中の文字）
    pub filter: String,
    /// 絞り込みの欄の確定した文字（入力中の Esc はここへ戻す）
    pub filter_input: String,
    ledger: Ledger,
    /// 台帳を書き換えるたびに増やす（絞り込みの覚えを捨てる目印）
    ledger_gen: u64,
    /// 最後に読み込んだ・書き出した色。今の色と違えば未書き出し
    baseline: Seeds,
    /// 削除した後など、画面にしか無い色
    unsaved: bool,
    /// 最後に読み込んだ・書き出した先。ここと違う既存の先へ書き出すときは確かめる
    origin: Option<(Target, String)>,
    pending: Option<Pending>,
    preview: Option<Preview>,
    /// （絞り込みの文字, 台帳の世代, 合ったキャラの番号）
    filtered: Option<(String, u64, Vec<usize>)>,
}

impl Model {
    /// 起動時。台帳の Core を読み込んだ状態で始める
    pub fn new(ledger: Ledger) -> Self {
        let seeds = ledger.fixed.core.as_ref().map(|e| e.to_seeds()).unwrap_or_default();
        Self {
            target: Target::Core,
            name: String::new(),
            baseline: seeds.clone(),
            seeds,
            status: "台帳を読み込みました".to_string(),
            show_semantic: false,
            filter: String::new(),
            filter_input: String::new(),
            ledger,
            ledger_gen: 0,
            unsaved: false,
            origin: Some((Target::Core, String::new())),
            pending: None,
            preview: None,
            filtered: None,
        }
    }

    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    fn set_ledger(&mut self, ledger: Ledger) {
        self.ledger = ledger;
        self.ledger_gen += 1;
    }

    /// 最後に読み込んだ・書き出した先（左の台帳で選ばれて見えるもの）
    pub fn origin(&self) -> Option<(Target, &str)> {
        self.origin.as_ref().map(|(t, n)| (*t, n.as_str()))
    }

    /// 書き出していない色があるか
    pub fn dirty(&self) -> bool {
        self.unsaved || self.seeds != self.baseline
    }

    /// 2 回目を待っている操作
    pub fn pending_action(&self) -> Option<&Action> {
        self.pending.as_ref().map(|p| &p.action)
    }

    /// 2 回目を待っている残り時間（再描画の予約に使う）
    pub fn pending_left(&self, now: Instant) -> Option<Duration> {
        self.pending.as_ref().map(|p| p.until.saturating_duration_since(now))
    }

    /// 毎フレームの最初に呼ぶ。時間が過ぎたか、色・書き出し先・名前が変わった待ちを取り消す
    pub fn tick(&mut self, now: Instant) {
        let stale = match &self.pending {
            Some(p) => now >= p.until || p.seeds != self.seeds || p.target != self.target || p.name != self.name,
            None => false,
        };
        if stale {
            let p = self.pending.take().unwrap();
            self.status = p.prev_status;
        }
    }

    /// 2 回目なら true を返して待ちを消す。1 回目（または別の操作）なら待ちを置いて `msg` を出し、false
    fn confirmed(&mut self, action: Action, now: Instant, msg: String) -> bool {
        if let Some(p) = &self.pending {
            if p.action == action && now < p.until {
                self.pending = None;
                return true;
            }
        }
        let prev_status = match self.pending.take() {
            // 別の操作の待ちを置き換えるときは、その前の文に戻せるようにする
            Some(p) => p.prev_status,
            None => self.status.clone(),
        };
        self.pending = Some(Pending {
            action,
            until: now + CONFIRM_FOR,
            seeds: self.seeds.clone(),
            target: self.target,
            name: self.name.clone(),
            prev_status,
        });
        self.status = msg;
        false
    }

    /// 書き出していない色があるなら、切り替えの前に確かめる
    fn may_switch(&mut self, action: Action, label: &str, now: Instant) -> bool {
        if !self.dirty() {
            self.pending = None;
            return true;
        }
        self.confirmed(
            action,
            now,
            format!("書き出していない色の変更があります。もう一度押すと、変更を捨てて{label}を開きます"),
        )
    }

    pub fn preview(&mut self) -> &Preview {
        let stale = self.preview.as_ref().map(|p| p.seeds != self.seeds).unwrap_or(true);
        if stale {
            self.preview = Some(Preview::build(&self.seeds));
        }
        self.preview.as_ref().unwrap()
    }

    /// 絞り込みに合うキャラの番号（台帳の並び）。文字か台帳が変わったときだけ数え直す
    pub fn visible(&mut self) -> &[usize] {
        let stale = match &self.filtered {
            Some((f, g, _)) => f != &self.filter || *g != self.ledger_gen,
            None => true,
        };
        if stale {
            let needle = self.filter.trim().to_lowercase();
            let idx: Vec<usize> = self
                .ledger
                .characters
                .iter()
                .enumerate()
                .filter(|(_, e)| needle.is_empty() || e.name.to_lowercase().contains(&needle))
                .map(|(i, _)| i)
                .collect();
            self.filtered = Some((self.filter.clone(), self.ledger_gen, idx));
        }
        &self.filtered.as_ref().unwrap().2
    }

    pub fn file_names(&self) -> Option<[String; 2]> {
        let (label, name) = match self.target {
            Target::Core => ("Core", String::new()),
            Target::Chara => ("Chara", storage::check_name(&self.name).ok()?),
        };
        Some([
            storage::palette_file_name(label, &name, Appearance::Light),
            storage::palette_file_name(label, &name, Appearance::Dark),
        ])
    }

    fn load(&mut self, entry: Option<Entry>, target: Target, name: String) {
        if let Some(e) = &entry {
            self.seeds = e.to_seeds();
        }
        self.target = target;
        self.name = name.clone();
        self.baseline = self.seeds.clone();
        self.unsaved = false;
        self.origin = Some((target, name.clone()));
        self.status = match (&entry, target) {
            (Some(_), Target::Core) => "Core を読み込みました".to_string(),
            (Some(_), Target::Chara) => format!("{name} を読み込みました"),
            (None, _) => "台帳に Core がありません。今の色のまま Core に書き出せます".to_string(),
        };
    }

    /// 左の台帳の「Core」
    pub fn click_core(&mut self, now: Instant) {
        if !self.may_switch(Action::LoadCore, "Core", now) {
            return;
        }
        let core = self.ledger.fixed.core.clone();
        self.load(core, Target::Core, String::new());
    }

    /// 左の台帳のキャラ
    pub fn click_chara(&mut self, name: &str, now: Instant) {
        if !self.may_switch(Action::LoadChara(name.to_string()), name, now) {
            return;
        }
        match self.ledger.find(name).cloned() {
            Some(e) => self.load(Some(e), Target::Chara, name.to_string()),
            None => self.status = format!("{name} は台帳にありません"),
        }
    }

    /// 「新しいキャラ」
    pub fn click_new(&mut self, now: Instant) {
        if !self.may_switch(Action::NewChara, "新しいキャラ", now) {
            return;
        }
        self.target = Target::Chara;
        self.name = String::new();
        self.seeds = Seeds::default();
        self.baseline = self.seeds.clone();
        self.unsaved = false;
        self.origin = None;
        self.status = "名前とシード色を決めて書き出してください".to_string();
    }

    /// 書き出し先が既にあるか（台帳かファイル）
    fn target_exists(&self, backend: &dyn Backend, name: &str, files: &[String; 2]) -> bool {
        let in_ledger = match self.target {
            Target::Core => self.ledger.fixed.core.is_some(),
            Target::Chara => self.ledger.find(name).is_some(),
        };
        in_ledger || files.iter().any(|f| backend.palette_exists(f))
    }

    /// 「パレットに書き出す」。2 枚を書いて台帳も更新する
    pub fn export(&mut self, backend: &mut dyn Backend, now: Instant) {
        let names = match self.file_names() {
            Some(n) => n,
            None => {
                self.pending = None;
                self.status = "名前が不正（'.' は使えない・空にできない）".to_string();
                return;
            }
        };
        let name = match self.target {
            Target::Core => String::new(),
            Target::Chara => storage::check_name(&self.name).unwrap_or_default(),
        };
        let shown = if name.is_empty() { "Core".to_string() } else { name.clone() };
        let identity = (self.target, name.clone());

        // 読み込んだ先と違う、既にある先へ書くときは確かめる（同じ名前のキャラを黙って上書きしない）
        if self.origin.as_ref() != Some(&identity) && self.target_exists(backend, &name, &names) {
            let msg = format!("{shown} は既にあります。もう一度押すと、今の色で上書きします");
            if !self.confirmed(Action::Export, now, msg) {
                return;
            }
        } else {
            self.pending = None;
        }

        let (mut applied, mut created, mut rewritten, mut busy) = (0, 0, 0, 0);
        let mut errors: Vec<String> = Vec::new();
        let preview = Preview::build(&self.seeds);
        for (fname, pal) in names.iter().zip([&preview.light, &preview.dark]) {
            let pname = fname.trim_end_matches(".palette");
            // 既存パレットなら編集 API がその場で反映してくれる（ファイル保存もホスト側）。
            // 新規は API では作れないので、ファイルを書いて再起動してもらう
            match backend.apply_palette(pname, pal) {
                Ok(()) => applied += 1,
                Err(ApplyError::Busy) | Err(ApplyError::NotReady) => {
                    tracing::warn!("PaletteForge_H: {pname} を反映できなかった（本体が操作中など）");
                    busy += 1;
                }
                Err(ApplyError::Refused) => {
                    let existed = backend.palette_exists(fname);
                    if existed {
                        // ファイルはあるのに本体が受け付けない。起動後に作ったパレットは再起動まで本体に無い
                        tracing::warn!("PaletteForge_H: {pname} はファイルがあるのに本体が受け付けなかった。ファイルを書き直す");
                    }
                    match backend.write_palette(fname, pal) {
                        Ok(()) if existed => rewritten += 1,
                        Ok(()) => created += 1,
                        Err(e) => errors.push(format!("{fname}: {e:#}")),
                    }
                }
                Err(e @ ApplyError::Other(_)) => errors.push(format!("{pname}: {e}")),
            }
        }

        if !errors.is_empty() || busy > 0 {
            let mut parts = Vec::new();
            if applied > 0 {
                parts.push(format!("{applied} 枚は反映しました"));
            }
            if busy > 0 {
                parts.push(format!(
                    "{busy} 枚は本体が操作中（ドラッグ中・出力中など）で書けませんでした。操作を終えてからもう一度押してください"
                ));
            }
            if !errors.is_empty() {
                parts.push(format!("書き出し失敗: {}", errors.join(" / ")));
            }
            self.status = parts.join("。");
            tracing::warn!(
                "PaletteForge_H: 書き出し {shown}: 反映 {applied} / 新規 {created} / ファイル更新 {rewritten} / 操作中で不可 {busy} / 失敗 {}{}",
                errors.len(),
                if errors.is_empty() { String::new() } else { format!("（{}）", errors.join(" / ")) }
            );
            return;
        }

        let entry = Entry::from_seeds(&name, &self.seeds);
        let target = self.target;
        let mut entry = Some(entry);
        let result = backend.update_ledger(&mut |l: &mut Ledger| {
            if let Some(e) = entry.take() {
                match target {
                    Target::Core => l.set_core(e),
                    Target::Chara => l.upsert(e),
                }
            }
        });
        match result {
            Ok(l) => self.set_ledger(l),
            Err(e) => {
                tracing::warn!("PaletteForge_H: 書き出し {shown}: パレットは書けたが台帳の保存に失敗: {e:#}");
                self.status = format!("パレットは書けたが台帳の保存に失敗: {e:#}");
                return;
            }
        }
        self.name = name;
        self.baseline = self.seeds.clone();
        self.unsaved = false;
        self.origin = Some(identity);

        let mut parts = Vec::new();
        if applied > 0 {
            parts.push(format!("{applied} 枚を反映しました（色設定で開き直すと見えます）"));
        }
        if created > 0 {
            parts.push(format!("{created} 枚を新規作成しました。反映には AviUtl2 の再起動が要ります"));
        }
        if rewritten > 0 {
            parts.push(format!(
                "{rewritten} 枚は本体に読み込まれていない（起動後に作ったパレットは再起動まで本体に出ない）ので、ファイルを書き直しました。反映には AviUtl2 の再起動が要ります"
            ));
        }
        self.status = parts.join("。");
        tracing::info!(
            "PaletteForge_H: 書き出し {shown}: 反映 {applied} / 新規 {created} / ファイル更新 {rewritten}"
        );
    }

    /// 「このキャラを削除」。1 回目は確かめ、2 回目で消す
    pub fn press_delete(&mut self, backend: &mut dyn Backend, now: Instant) {
        if self.target != Target::Chara {
            self.pending = None;
            self.status = "Core は固定なので消せません（--restore で復活する側です）".to_string();
            return;
        }
        let name = match storage::check_name(&self.name) {
            Ok(n) => n,
            Err(e) => {
                self.pending = None;
                self.status = format!("名前が不正: {e}");
                return;
            }
        };
        let msg = format!(
            "もう一度押すと {name} を削除します（台帳と Default の 2 枚。ごみ箱には入りません）"
        );
        if !self.confirmed(Action::Delete, now, msg) {
            return;
        }
        let mut removed = 0;
        for mode in [Appearance::Light, Appearance::Dark] {
            let f = storage::palette_file_name("Chara", &name, mode);
            match backend.remove_palette(&f) {
                Ok(true) => removed += 1,
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!("PaletteForge_H: 削除 {name}: 失敗: {e:#}");
                    self.status = format!("削除失敗: {e:#}");
                    return;
                }
            }
        }
        let mut had = false;
        let result = backend.update_ledger(&mut |l: &mut Ledger| had = l.remove(&name));
        match result {
            Ok(l) => self.set_ledger(l),
            Err(e) => {
                tracing::warn!("PaletteForge_H: 削除 {name}: ファイル {removed} 枚を消したが台帳の保存に失敗: {e:#}");
                self.status = format!("削除したが台帳の保存に失敗: {e:#}");
                return;
            }
        }
        // 色は画面に残る。切り替えるときは確かめる
        self.unsaved = true;
        self.origin = None;
        tracing::info!(
            "PaletteForge_H: 削除 {name}: 台帳 {} / ファイル {removed} 枚",
            if had { "あり" } else { "なし" }
        );
        self.status = format!(
            "{name} を削除しました（台帳 {} / ファイル {removed} 枚）。画面の色はそのままなので、すぐ書き出せば戻せます。本体の色設定の一覧には再起動まで残ります",
            if had { "あり" } else { "なし" }
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    /// 本体とファイルの偽物。`known` は本体が読み込んでいるパレット、`files` は Default のファイル
    #[derive(Default)]
    struct Fake {
        known: BTreeSet<String>,
        files: BTreeMap<String, String>,
        ledger: Ledger,
        busy: bool,
        applied: Vec<String>,
    }

    impl Backend for Fake {
        fn apply_palette(&mut self, name: &str, _pal: &Palette) -> Result<(), ApplyError> {
            if self.busy {
                return Err(ApplyError::Busy);
            }
            if !self.known.contains(name) {
                return Err(ApplyError::Refused);
            }
            self.applied.push(name.to_string());
            Ok(())
        }
        fn palette_exists(&self, file_name: &str) -> bool {
            self.files.contains_key(file_name)
        }
        fn write_palette(&mut self, file_name: &str, pal: &Palette) -> anyhow::Result<()> {
            self.files.insert(file_name.to_string(), storage::palette_body(pal));
            Ok(())
        }
        fn remove_palette(&mut self, file_name: &str) -> anyhow::Result<bool> {
            Ok(self.files.remove(file_name).is_some())
        }
        fn update_ledger(&mut self, change: &mut dyn FnMut(&mut Ledger)) -> anyhow::Result<Ledger> {
            change(&mut self.ledger);
            Ok(self.ledger.clone())
        }
    }

    fn t0() -> Instant {
        Instant::now()
    }

    fn model_with(names: &[&str]) -> (Model, Fake) {
        let mut ledger = Ledger::default();
        ledger.set_core(Entry::from_seeds("", &Seeds::default()));
        for n in names {
            let mut s = Seeds::default();
            s.main = [n.len() as u8, 1, 2];
            ledger.upsert(Entry::from_seeds(n, &s));
        }
        let fake = Fake { ledger: ledger.clone(), ..Default::default() };
        (Model::new(ledger), fake)
    }

    #[test]
    fn delete_needs_two_presses_and_expires() {
        let (mut m, mut fake) = model_with(&["さくら"]);
        fake.files.insert("Chara.さくら Light.palette".into(), String::new());
        fake.files.insert("Chara.さくら Dark.palette".into(), String::new());
        let now = t0();
        m.click_chara("さくら", now);
        m.press_delete(&mut fake, now);
        assert_eq!(fake.files.len(), 2, "1 回目では消さない");
        assert_eq!(m.pending_action(), Some(&Action::Delete));
        // 時間が過ぎたら取り消し
        m.tick(now + CONFIRM_FOR);
        assert!(m.pending_action().is_none());
        assert!(m.status.contains("読み込みました"), "確かめの前の文に戻る: {}", m.status);
        m.press_delete(&mut fake, now + CONFIRM_FOR);
        assert_eq!(fake.files.len(), 2);
        m.press_delete(&mut fake, now + CONFIRM_FOR + Duration::from_secs(1));
        assert!(fake.files.is_empty());
        assert!(m.ledger().find("さくら").is_none());
        assert!(m.status.contains("すぐ書き出せば戻せます"));
        assert!(m.status.contains("再起動まで残ります"));
        // 色は画面に残り、切り替えるときは確かめる
        assert!(m.dirty());
        m.click_core(now + Duration::from_secs(10));
        assert_eq!(m.target, Target::Chara);
        // すぐ書き出せば戻る（ファイルは無くなり、本体には残っている）
        fake.known.insert("Chara.さくら Light".into());
        fake.known.insert("Chara.さくら Dark".into());
        m.export(&mut fake, now + Duration::from_secs(11));
        assert!(m.ledger().find("さくら").is_some());
        assert!(!m.dirty());
    }

    #[test]
    fn other_action_cancels_delete() {
        let (mut m, mut fake) = model_with(&["a", "b"]);
        fake.files.insert("Chara.a Light.palette".into(), String::new());
        let now = t0();
        m.click_chara("a", now);
        m.press_delete(&mut fake, now);
        // 色を変えたら取り消し
        m.seeds.main = [9, 9, 9];
        m.tick(now);
        assert!(m.pending_action().is_none());
        m.press_delete(&mut fake, now);
        assert!(fake.files.contains_key("Chara.a Light.palette"), "取り消した後の 1 回目では消さない");
        // ほかのボタン（書き出し）を押しても取り消し
        m.export(&mut fake, now);
        assert_ne!(m.pending_action(), Some(&Action::Delete));
    }

    #[test]
    fn switching_with_unsaved_colors_asks_once_more() {
        let (mut m, mut fake) = model_with(&["a", "b"]);
        let now = t0();
        m.click_chara("a", now);
        assert!(!m.dirty());
        m.seeds.sub = [1, 1, 1];
        assert!(m.dirty());
        m.click_chara("b", now);
        assert_eq!(m.name, "a", "1 回目では切り替えない");
        assert_eq!(m.seeds.sub, [1, 1, 1]);
        // 別の項目を押したら、その項目の 1 回目になる
        m.click_new(now);
        assert_eq!(m.name, "a");
        m.click_new(now + Duration::from_secs(1));
        assert!(m.name.is_empty());
        assert_eq!(m.seeds, Seeds::default());
        // 新しいキャラは、色を変えなければそのまま切り替わる
        m.click_chara("b", now + Duration::from_secs(2));
        assert_eq!(m.name, "b");
        // 書き出せば未書き出しは消える
        m.seeds.main = [3, 3, 3];
        fake.known.insert("Chara.b Light".into());
        fake.known.insert("Chara.b Dark".into());
        m.export(&mut fake, now + Duration::from_secs(3));
        assert!(!m.dirty());
        m.click_core(now + Duration::from_secs(4));
        assert_eq!(m.target, Target::Core);
    }

    #[test]
    fn exporting_new_chara_over_existing_name_asks() {
        let (mut m, mut fake) = model_with(&["さくら"]);
        let now = t0();
        m.click_new(now);
        m.name = "さくら".into();
        m.seeds.main = [5, 5, 5];
        m.export(&mut fake, now);
        assert_eq!(m.pending_action(), Some(&Action::Export));
        assert_ne!(fake.ledger.find("さくら").unwrap().main, "050505", "1 回目では上書きしない");
        assert!(fake.files.is_empty());
        m.export(&mut fake, now + Duration::from_secs(1));
        assert_eq!(fake.ledger.find("さくら").unwrap().main, "050505");
        // 読み込んだキャラへの書き出しは確かめない
        m.seeds.main = [6, 6, 6];
        fake.known.insert("Chara.さくら Light".into());
        fake.known.insert("Chara.さくら Dark".into());
        m.export(&mut fake, now + Duration::from_secs(2));
        assert_eq!(fake.ledger.find("さくら").unwrap().main, "060606");
        // 台帳に無くてもファイルがあれば確かめる
        m.click_new(now + Duration::from_secs(3));
        m.name = "ゆき".into();
        fake.files.insert("Chara.ゆき Dark.palette".into(), String::new());
        m.export(&mut fake, now + Duration::from_secs(4));
        assert_eq!(m.pending_action(), Some(&Action::Export));
        // 新しい名前なら確かめない
        m.name = "はな".into();
        m.tick(now + Duration::from_secs(4));
        m.export(&mut fake, now + Duration::from_secs(4));
        assert!(fake.ledger.find("はな").is_some());
        assert!(m.status.contains("2 枚を新規作成"), "{}", m.status);
    }

    #[test]
    fn busy_host_is_reported_and_ledger_untouched() {
        let (mut m, mut fake) = model_with(&["a"]);
        fake.known.insert("Chara.a Light".into());
        fake.known.insert("Chara.a Dark".into());
        let now = t0();
        m.click_chara("a", now);
        m.seeds.main = [7, 7, 7];
        fake.busy = true;
        m.export(&mut fake, now);
        assert!(m.status.contains("もう一度押して"), "{}", m.status);
        assert!(!m.status.contains("新規作成"));
        assert!(fake.files.is_empty(), "操作中はファイルも書かない");
        assert_ne!(fake.ledger.find("a").unwrap().main, "070707");
        assert!(m.dirty());
        fake.busy = false;
        m.export(&mut fake, now);
        assert!(m.status.starts_with("2 枚を反映"), "{}", m.status);
        assert_eq!(fake.ledger.find("a").unwrap().main, "070707");
    }

    #[test]
    fn file_exists_but_host_refuses_rewrites_and_says_so() {
        let (mut m, mut fake) = model_with(&[]);
        let now = t0();
        m.click_new(now);
        m.name = "z".into();
        m.export(&mut fake, now);
        assert!(m.status.contains("2 枚を新規作成"), "{}", m.status);
        // 再起動しないまま、もう一度書き出す（ファイルはあるが本体は知らない）
        m.seeds.main = [8, 8, 8];
        m.export(&mut fake, now + Duration::from_secs(1));
        assert!(m.status.contains("ファイルを書き直しました"), "{}", m.status);
        assert!(!m.status.contains("新規作成"));
    }

    #[test]
    fn preview_is_rebuilt_only_when_seeds_change() {
        let (mut m, _) = model_with(&[]);
        let a = m.preview().light.rows[1].colors[5].rgb;
        let p1 = m.preview() as *const Preview;
        let p2 = m.preview() as *const Preview;
        assert_eq!(p1, p2);
        m.seeds.main = [255, 0, 0];
        let b = m.preview().light.rows[1].colors[5].rgb;
        assert_ne!(a, b);
    }

    #[test]
    fn filter_matches_names_case_insensitively() {
        let (mut m, _) = model_with(&["Sakura", "yuki", "さくら"]);
        assert_eq!(m.visible().len(), 3);
        m.filter = "SAK".into();
        assert_eq!(m.visible(), &[0]);
        m.filter = "さ".into();
        assert_eq!(m.visible(), &[2]);
        m.filter = "  ".into();
        assert_eq!(m.visible().len(), 3);
    }
}
