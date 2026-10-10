//! PaletteForge_H の egui 画面。
//!
//! シード色を触ると 8x8 のプレビューと検証結果が即座に更新される。
//! 「書き出す」で `Default/*.palette` を書き、色設定ウィンドウでパレットを
//! 切り替え直せば反映される（新規ファイルのときだけ再起動が要る）。

use std::sync::Arc;

use aviutl2_eframe::{eframe, egui, AviUtl2EframeHandle};
use parking_lot::RwLock;

use crate::color::to_hex;
use crate::line_edit::{drag_value, hex_edit, line_edit};
use crate::storage::{self, Entry, Ledger};
use crate::theme::{
    build_palette, seed_clashes, validate, Appearance, Palette, Report, Seeds, SEMANTIC,
    STEP_NAMES,
};

pub type SharedState = Arc<RwLock<AppState>>;

const CELL: f32 = 21.0;
const ROW_LABEL_W: f32 = 86.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// 固定の既定テーマ。Core.Light / Core.Dark
    Core,
    /// キャラごと。Chara.<名前> Light / Dark
    Chara,
}

pub struct AppState {
    pub target: Target,
    pub name: String,
    pub seeds: Seeds,
    pub ledger: Ledger,
    pub status: String,
    pub show_semantic: bool,
}

impl Default for AppState {
    fn default() -> Self {
        let ledger = storage::load_ledger();
        let seeds = ledger
            .fixed
            .core
            .as_ref()
            .map(|e| e.to_seeds())
            .unwrap_or_default();
        Self {
            target: Target::Core,
            name: String::new(),
            seeds,
            ledger,
            status: "台帳を読み込みました".to_string(),
            show_semantic: false,
        }
    }
}

impl AppState {
    fn file_names(&self) -> Option<[String; 2]> {
        let (label, name) = match self.target {
            Target::Core => ("Core", String::new()),
            Target::Chara => ("Chara", storage::check_name(&self.name).ok()?),
        };
        Some([
            storage::palette_file_name(label, &name, Appearance::Light),
            storage::palette_file_name(label, &name, Appearance::Dark),
        ])
    }

    /// 2 枚を書いて台帳も更新する。
    fn export(&mut self) {
        let names = match self.file_names() {
            Some(n) => n,
            None => {
                self.status = "名前が不正（'.' は使えない・空にできない）".to_string();
                return;
            }
        };
        let (mut applied, mut written) = (0, 0);
        let mut errors: Vec<String> = Vec::new();

        for (fname, mode) in names.iter().zip([Appearance::Light, Appearance::Dark]) {
            let pal = build_palette(&self.seeds, mode);
            let pname = fname.trim_end_matches(".palette");
            // 既存パレットなら編集 API がその場で反映してくれる（ファイル保存もホスト側）。
            // 新規は API では作れないので、ファイルを書いて再起動してもらう
            match crate::edit_ops::apply_palette(pname, &pal) {
                Ok(()) => applied += 1,
                Err(api_err) => match storage::write_palette(fname, &pal) {
                    Ok(_) => {
                        written += 1;
                        tracing::debug!("{pname}: API 更新は不可（{api_err}）。ファイルを書いた");
                    }
                    Err(e) => errors.push(format!("{fname}: {e:#}")),
                },
            }
        }

        if !errors.is_empty() {
            self.status = format!("書き出し失敗: {}", errors.join(" / "));
            return;
        }

        match self.target {
            Target::Core => {
                self.ledger.fixed.core = Some(Entry::from_seeds("", &self.seeds));
            }
            Target::Chara => {
                let name = storage::check_name(&self.name).unwrap_or_default();
                self.ledger.upsert(Entry::from_seeds(&name, &self.seeds));
            }
        }
        if let Err(e) = storage::save_ledger(&self.ledger) {
            self.status = format!("パレットは書けたが台帳の保存に失敗: {e:#}");
            return;
        }

        self.status = if written == 0 {
            format!("{applied} 枚を反映しました（色設定で開き直すと見えます）")
        } else if applied == 0 {
            format!("{written} 枚を新規作成しました。反映には AviUtl2 の再起動が要ります")
        } else {
            format!("{applied} 枚を反映、{written} 枚を新規作成（新規分は再起動が要ります）")
        };
    }

    fn load_entry(&mut self, entry: &Entry, target: Target) {
        self.seeds = entry.to_seeds();
        self.target = target;
        self.name = entry.name.clone();
        self.status = format!(
            "{} を読み込みました",
            if entry.name.is_empty() {
                "Core"
            } else {
                &entry.name
            }
        );
    }

    fn remove_current(&mut self) {
        if self.target != Target::Chara {
            self.status = "Core は固定なので消せません（--restore で復活する側です）".to_string();
            return;
        }
        let name = match storage::check_name(&self.name) {
            Ok(n) => n,
            Err(e) => {
                self.status = format!("名前が不正: {e}");
                return;
            }
        };
        let mut removed = 0;
        for mode in [Appearance::Light, Appearance::Dark] {
            let f = storage::palette_file_name("Chara", &name, mode);
            match storage::remove_palette(&f) {
                Ok(true) => removed += 1,
                Ok(false) => {}
                Err(e) => {
                    self.status = format!("削除失敗: {e:#}");
                    return;
                }
            }
        }
        let had = self.ledger.remove(&name);
        if let Err(e) = storage::save_ledger(&self.ledger) {
            self.status = format!("削除したが台帳の保存に失敗: {e:#}");
            return;
        }
        self.status = format!("{name} を削除しました（定義 {} / ファイル {removed} 枚）",
            if had { "あり" } else { "なし" });
    }
}

pub struct PaletteForgeApp {
    state: SharedState,
    _handle: AviUtl2EframeHandle,
}

impl PaletteForgeApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        handle: AviUtl2EframeHandle,
        state: SharedState,
    ) -> Self {
        cc.egui_ctx.all_styles_mut(|style| {
            style.visuals = aviutl2_eframe::aviutl2_visuals();
        });
        cc.egui_ctx.set_fonts(aviutl2_eframe::aviutl2_fonts());
        Self {
            state,
            _handle: handle,
        }
    }
}

/// 色 1 行。ピッカーと hex 直接入力の両方から編集できる（hex は確定してから使う。`line_edit::hex_edit`）
fn color_row(ui: &mut egui::Ui, label: &str, rgb: &mut [u8; 3]) {
    ui.horizontal(|ui| {
        ui.add_sized([104.0, 18.0], egui::Label::new(label));
        egui::color_picker::color_edit_button_srgb(ui, rgb);
        hex_edit(ui, label, rgb);
        // R/G/B を数値でも入れられるようにする
        for (i, name) in ["R", "G", "B"].iter().enumerate() {
            let mut v = rgb[i] as i32;
            drag_value(ui, &mut v, |d| d.range(0..=255).speed(1.0).prefix(*name));
            rgb[i] = v.clamp(0, 255) as u8;
        }
    });
}

fn swatch_grid(ui: &mut egui::Ui, pal: &Palette, title: &str) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(title).strong());
        ui.horizontal(|ui| {
            ui.add_space(ROW_LABEL_W);
            for (i, name) in STEP_NAMES.iter().enumerate() {
                ui.add_sized(
                    [CELL, 14.0],
                    egui::Label::new(egui::RichText::new(format!("{}", i + 1)).small()),
                )
                .on_hover_text(*name);
            }
        });
        for row in pal.rows.iter() {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [ROW_LABEL_W, CELL],
                    egui::Label::new(egui::RichText::new(&row.role).small()),
                );
                for (ci, sw) in row.colors.iter().enumerate() {
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(CELL, CELL), egui::Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        egui::CornerRadius::same(2),
                        egui::Color32::from_rgb(sw.rgb[0], sw.rgb[1], sw.rgb[2]),
                    );
                    resp.on_hover_text(format!(
                        "{} / {}\n#{}",
                        row.role,
                        STEP_NAMES[ci],
                        to_hex(sw.rgb)
                    ));
                }
            });
        }
    });
}

fn report_block(ui: &mut egui::Ui, title: &str, rep: &Report) {
    if rep.issues.is_empty() {
        ui.colored_label(
            egui::Color32::from_rgb(0x40, 0xc0, 0x57),
            format!("{title}: 検証を通過"),
        );
    } else {
        ui.colored_label(
            egui::Color32::from_rgb(0xfa, 0x52, 0x52),
            format!("{title}: 未通過 {} 件", rep.issues.len()),
        );
        for i in rep.issues.iter().take(8) {
            ui.small(i);
        }
    }
}

impl eframe::App for PaletteForgeApp {
    // aviutl2-eframe はホストのウィンドウ内に描くので、update ではなく ui を実装する
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut state = self.state.write();

        egui::Panel::left("ledger")
            .default_size(180.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.heading("台帳");
                ui.separator();
                ui.label(egui::RichText::new("固定").small());
                if ui
                    .selectable_label(state.target == Target::Core, "Core")
                    .clicked()
                {
                    let core = state.ledger.fixed.core.clone();
                    match core {
                        Some(e) => state.load_entry(&e, Target::Core),
                        None => {
                            state.target = Target::Core;
                            state.name.clear();
                        }
                    }
                }
                ui.add_space(6.0);
                ui.label(egui::RichText::new("キャラ").small());
                let entries = state.ledger.characters.clone();
                if entries.is_empty() {
                    ui.small("（未登録）");
                }
                for e in entries.iter() {
                    let selected = state.target == Target::Chara && state.name == e.name;
                    if ui.selectable_label(selected, &e.name).clicked() {
                        state.load_entry(e, Target::Chara);
                    }
                }
                ui.add_space(6.0);
                if ui.button("新しいキャラ").clicked() {
                    state.target = Target::Chara;
                    state.name = String::new();
                    state.seeds = Seeds::default();
                    state.status = "名前とシード色を決めて書き出してください".to_string();
                }
            });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut state.target, Target::Core, "Core（固定）");
                    ui.selectable_value(&mut state.target, Target::Chara, "キャラ");
                    if state.target == Target::Chara {
                        ui.label("名前");
                        // 名前は書き出し・削除で使う。打っている途中では書き出し先を変えない
                        line_edit(ui, "chara_name", &mut state.name, |t| t.desired_width(140.0))
                            .0
                            .on_hover_text("Enter かほかをクリックで確定、Esc で元に戻す");
                    }
                });
                if let Some(names) = state.file_names() {
                    ui.small(format!("書き出し先: {} / {}", names[0], names[1]));
                } else {
                    ui.colored_label(
                        egui::Color32::from_rgb(0xfa, 0x52, 0x52),
                        "名前が不正です（'.' は使えません）",
                    );
                }

                ui.separator();
                let seeds = &mut state.seeds;
                color_row(ui, "メイン", &mut seeds.main);
                color_row(ui, "サブ", &mut seeds.sub);
                color_row(ui, "アクセント", &mut seeds.accent);
                ui.horizontal(|ui| {
                    ui.add_sized([104.0, 18.0], egui::Label::new("ニュートラル彩度"));
                    // スライダーの数値欄は Esc で戻せない（egui 0.36）ので、数値は別の欄に出す（`drag_value`）
                    ui.add(
                        egui::Slider::new(&mut state.seeds.neutral_bias, 0.0..=0.015)
                            .show_value(false),
                    );
                    drag_value(ui, &mut state.seeds.neutral_bias, |d| {
                        d.range(0.0..=0.015).speed(0.0001).fixed_decimals(3)
                    });
                });

                let show = state.show_semantic;
                if ui
                    .selectable_label(show, "セマンティック 4 色を編集")
                    .clicked()
                {
                    state.show_semantic = !show;
                }
                if state.show_semantic {
                    for (i, (key, _)) in SEMANTIC.iter().enumerate() {
                        color_row(ui, key, &mut state.seeds.semantic[i]);
                    }
                }

                for msg in seed_clashes(&state.seeds) {
                    ui.colored_label(egui::Color32::from_rgb(0xfa, 0xb0, 0x05), msg);
                }

                ui.separator();
                let light = build_palette(&state.seeds, Appearance::Light);
                let dark = build_palette(&state.seeds, Appearance::Dark);
                let rep_l = validate(&light);
                let rep_d = validate(&dark);
                swatch_grid(ui, &light, "Light");
                ui.add_space(8.0);
                swatch_grid(ui, &dark, "Dark");

                ui.separator();
                report_block(ui, "Light", &rep_l);
                report_block(ui, "Dark", &rep_d);

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("パレットに書き出す").clicked() {
                        state.export();
                    }
                    if state.target == Target::Chara && ui.button("このキャラを削除").clicked() {
                        state.remove_current();
                    }
                });
                ui.small(state.status.clone());
            });
        });
    }
}
