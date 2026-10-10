//! PaletteForge_H の egui 画面（描くだけ。状態と操作は `model.rs`）。
//!
//! シード色を触ると 8x8 のプレビューと検証結果が更新される（計算し直すのは色が変わったときだけ）。
//! 「パレットに書き出す」で `Default/*.palette` を書き、色設定ウィンドウでパレットを
//! 切り替え直せば反映される（新規ファイルのときだけ再起動が要る）。
//! 本体の API を使わないので、テスト（1 フレームの測定）でも組み込む。

use std::time::Instant;

use aviutl2_eframe::egui;

use crate::color::to_hex;
use crate::line_edit::{drag_value, hex_edit, line_edit};
use crate::model::{Action, Backend, Model, Target};
use crate::theme::{Palette, Report, SEMANTIC, STEP_NAMES};

const CELL: f32 = 21.0;
const ROW_LABEL_W: f32 = 86.0;
/// 台帳の 1 行の高さ（`show_rows` で見えている行だけを描くため、全行をこの高さにそろえる）
const LEDGER_ROW_H: f32 = 18.0;

const RED: egui::Color32 = egui::Color32::from_rgb(0xfa, 0x52, 0x52);
const ORANGE: egui::Color32 = egui::Color32::from_rgb(0xfa, 0xb0, 0x05);
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x40, 0xc0, 0x57);

/// 色 1 行。ピッカーと hex 直接入力の両方から編集できる（hex は確定してから使う。`line_edit::hex_edit`）。
/// 狭いパネルでは折り返す
fn color_row(ui: &mut egui::Ui, label: &str, rgb: &mut [u8; 3]) {
    ui.horizontal_wrapped(|ui| {
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
                    // 文字はマウスが乗ったときだけ作る（64 マス x 2 枚を毎フレーム作らない）
                    resp.on_hover_ui(|ui| {
                        ui.label(format!("{} / {}\n#{}", row.role, STEP_NAMES[ci], to_hex(sw.rgb)));
                    });
                }
            });
        }
    });
}

fn report_block(ui: &mut egui::Ui, title: &str, rep: &Report) {
    if rep.issues.is_empty() {
        ui.colored_label(GREEN, format!("{title}: 検証を通過"));
    } else {
        ui.colored_label(RED, format!("{title}: 未通過 {} 件", rep.issues.len()));
        for i in rep.issues.iter().take(8) {
            ui.small(i);
        }
    }
}

/// 左の台帳。選ぶと色を読み込む
fn ledger_panel(ui: &mut egui::Ui, model: &mut Model, now: Instant) {
    ui.heading("台帳");
    ui.small("選ぶと、その色を読み込みます");
    if model.dirty() {
        ui.colored_label(ORANGE, "● 書き出していない変更があります");
    }
    if let Some(a) = model.pending_action().filter(|a| a.is_switch()) {
        let label = match a {
            Action::LoadCore => "Core".to_string(),
            Action::LoadChara(n) => n.clone(),
            _ => "新しいキャラ".to_string(),
        };
        ui.colored_label(ORANGE, format!("もう一度押すと、変更を捨てて {label} を開きます"));
    }
    ui.separator();
    // 一覧が長くても押せるように、一覧より上に置く
    if ui.button("新しいキャラ").clicked() {
        model.click_new(now);
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new("固定").small());
    if ui
        .add_sized(
            [ui.available_width(), LEDGER_ROW_H],
            egui::Button::selectable(matches!(model.origin(), Some((Target::Core, _))), "Core"),
        )
        .clicked()
    {
        model.click_core(now);
    }
    ui.add_space(4.0);
    let total = model.ledger().characters.len();
    ui.label(egui::RichText::new(format!("キャラ（{total} 件）")).small());
    if total == 0 {
        ui.small("（未登録）");
        return;
    }
    // 絞り込み。打つたびに入力中の文字で絞る。確定は Enter・ほかをクリック、入力中の Esc は入力前の文字に戻す
    let (_, typing, _) = line_edit(ui, "ledger_filter", &mut model.filter_input, |t| {
        t.desired_width(f32::INFINITY).hint_text("名前で絞り込む")
    });
    model.filter = typing;
    let mut clicked: Option<String> = None;
    // 選ばれて見えるのは、色を読み込んだ（書き出した）先。上の「書き出し先」の切り替えでは変わらない
    let selected_name = match model.origin() {
        Some((Target::Chara, n)) => Some(n.to_string()),
        _ => None,
    };
    let visible = model.visible().to_vec();
    if visible.is_empty() {
        ui.small("（合うキャラがありません）");
        return;
    }
    // パネル全体が 1 つのスクロール（`draw`）の中にあるので、行は場所だけ取り、見えている行だけを描く
    // （v0.3.0 は一覧だけを `show_rows` にしていたので、パネルを低くすると上の見出しごと切れてスクロールできなかった）
    let characters = &model.ledger().characters;
    let width = ui.available_width();
    let spacing = ui.spacing().item_spacing.y;
    let clip = ui.clip_rect();
    let top = ui.cursor().top();
    let row_step = LEDGER_ROW_H + spacing;
    let first = (((clip.top() - top) / row_step).floor().max(0.0) as usize).min(visible.len());
    let last = ((((clip.bottom() - top) / row_step).ceil().max(0.0)) as usize + 1).min(visible.len());
    if first > 0 {
        ui.allocate_space(egui::vec2(width, first as f32 * row_step - spacing));
    }
    for &i in &visible[first..last.max(first)] {
        let e = &characters[i];
        let selected = selected_name.as_deref() == Some(e.name.as_str());
        let resp = ui.add_sized([width, LEDGER_ROW_H], egui::Button::selectable(selected, e.name.as_str()).truncate());
        if resp.clicked() {
            clicked = Some(e.name.clone());
        }
    }
    let rest = visible.len() - last.max(first);
    if rest > 0 {
        ui.allocate_space(egui::vec2(width, rest as f32 * row_step - spacing));
    }
    if let Some(name) = clicked {
        model.click_chara(&name, now);
    }
}

fn editor_panel(ui: &mut egui::Ui, model: &mut Model, backend: &mut dyn Backend, now: Instant) {
    // 書き出し先だけを変える（色は読み込まない。読み込むのは左の台帳）
    ui.horizontal_wrapped(|ui| {
        ui.label("書き出し先")
            .on_hover_text("書き出す先だけを切り替えます。色は読み込みません（読み込むのは左の台帳で選んだとき）");
        ui.selectable_value(&mut model.target, Target::Core, "Core");
        ui.selectable_value(&mut model.target, Target::Chara, "キャラ");
        if model.target == Target::Chara {
            ui.label("名前");
            // 名前は書き出し・削除で使う。打っている途中では書き出し先を変えない
            line_edit(ui, "chara_name", &mut model.name, |t| t.desired_width(140.0))
                .0
                .on_hover_text("Enter かほかをクリックで確定、Esc で元に戻す");
        }
    });
    if let Some(names) = model.file_names() {
        ui.small(format!("ファイル: {} / {}", names[0], names[1]));
    } else {
        ui.colored_label(RED, "名前が不正です（'.' は使えません）");
    }

    ui.separator();
    let seeds = &mut model.seeds;
    color_row(ui, "メイン", &mut seeds.main);
    color_row(ui, "サブ", &mut seeds.sub);
    color_row(ui, "アクセント", &mut seeds.accent);
    ui.horizontal_wrapped(|ui| {
        ui.add_sized([104.0, 18.0], egui::Label::new("ニュートラル彩度"));
        // スライダーの数値欄は Esc で戻せない（egui 0.36）ので、数値は別の欄に出す（`drag_value`）
        ui.add(egui::Slider::new(&mut model.seeds.neutral_bias, 0.0..=0.015).show_value(false));
        drag_value(ui, &mut model.seeds.neutral_bias, |d| {
            d.range(0.0..=0.015).speed(0.0001).fixed_decimals(3)
        });
    });

    let show = model.show_semantic;
    if ui.selectable_label(show, "セマンティック 4 色を編集").clicked() {
        model.show_semantic = !show;
    }
    if model.show_semantic {
        for (i, (key, _)) in SEMANTIC.iter().enumerate() {
            color_row(ui, key, &mut model.seeds.semantic[i]);
        }
    }

    // ここから下は色から計算した値（色が変わったときだけ作り直す）
    let preview = model.preview();
    for msg in &preview.clashes {
        ui.colored_label(ORANGE, msg);
    }
    ui.separator();
    swatch_grid(ui, &preview.light, "Light");
    ui.add_space(8.0);
    swatch_grid(ui, &preview.dark, "Dark");
    ui.separator();
    report_block(ui, "Light", &preview.rep_light);
    report_block(ui, "Dark", &preview.rep_dark);

    ui.separator();
    let pending = model.pending_action().cloned();
    ui.horizontal_wrapped(|ui| {
        let export_label = if pending == Some(Action::Export) {
            egui::RichText::new("もう一度押すと上書き").color(RED)
        } else {
            egui::RichText::new("パレットに書き出す")
        };
        if ui.button(export_label).clicked() {
            model.export(backend, now);
        }
        if model.target == Target::Chara {
            let delete_label = if pending == Some(Action::Delete) {
                egui::RichText::new("もう一度押すと削除").color(RED)
            } else {
                egui::RichText::new("このキャラを削除")
            };
            if ui.button(delete_label).clicked() {
                model.press_delete(backend, now);
            }
        }
        if model.dirty() {
            ui.colored_label(ORANGE, "● 未書き出し");
        }
    });
    ui.add(egui::Label::new(egui::RichText::new(model.status.as_str()).small()).wrap());
}

/// 1 フレーム分を描く
pub fn draw(ui: &mut egui::Ui, model: &mut Model, backend: &mut dyn Backend, now: Instant) {
    model.tick(now);
    if let Some(left) = model.pending_left(now) {
        // 確かめの待ちが切れたら表示を戻す
        ui.ctx().request_repaint_after(left);
    }

    egui::Panel::left("ledger")
        .default_size(180.0)
        .resizable(true)
        .show(ui, |ui| {
            // 見出し・Core・絞り込みも含めてスクロールする（パネルを低くしても下まで届く）
            egui::ScrollArea::vertical().id_salt("ledger_scroll").auto_shrink([false, false]).show(ui, |ui| ledger_panel(ui, model, now));
        });

    // 中央パネルは最後に追加する（ルール au2-rs-plugin「egui レイアウト」）
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| editor_panel(ui, model, backend, now));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ApplyError;
    use crate::storage::{Entry, Ledger};
    use crate::theme::Seeds;

    struct NullBackend;

    impl Backend for NullBackend {
        fn apply_palette(&mut self, _: &str, _: &Palette) -> Result<(), ApplyError> {
            Ok(())
        }
        fn palette_exists(&self, _: &str) -> bool {
            false
        }
        fn write_palette(&mut self, _: &str, _: &Palette) -> anyhow::Result<()> {
            Ok(())
        }
        fn remove_palette(&mut self, _: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        fn update_ledger(&mut self, change: &mut dyn FnMut(&mut Ledger)) -> anyhow::Result<Ledger> {
            let mut l = Ledger::default();
            change(&mut l);
            Ok(l)
        }
    }

    fn key(k: egui::Key) -> egui::Event {
        egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// 1 フレーム回す。テクスチャの差分を片付けてから捨てる（デバッグビルドの debug_assert）
    fn frame(ctx: &egui::Context, m: &mut Model, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| draw(ui, m, &mut NullBackend, Instant::now()));
        out.textures_delta.clear();
    }

    fn model(n: usize) -> Model {
        let mut ledger = Ledger::default();
        for i in 0..n {
            ledger.upsert(Entry::from_seeds(&format!("キャラ{i:04}"), &Seeds::default()));
        }
        Model::new(ledger)
    }

    /// 打つたびに絞り、Esc で入力前の文字に戻す
    #[test]
    fn filter_narrows_while_typing_and_escape_reverts() {
        let ctx = egui::Context::default();
        let mut m = model(300);
        frame(&ctx, &mut m, vec![]);
        ctx.memory_mut(|mem| mem.request_focus(egui::Id::new(("line_edit", "ledger_filter"))));
        frame(&ctx, &mut m, vec![egui::Event::Text("0299".into())]);
        assert_eq!(m.filter, "0299");
        assert_eq!(m.visible(), &[299]);
        assert!(m.filter_input.is_empty(), "打っている途中では確定しない");
        frame(&ctx, &mut m, vec![key(egui::Key::Escape)]);
        assert!(m.filter.is_empty(), "Esc で入力前の文字に戻す");
        assert_eq!(m.visible().len(), 300);
        frame(&ctx, &mut m, vec![]);
        assert!(m.filter.is_empty());
        // Enter で確定すると、フォーカスが外れても絞ったまま
        ctx.memory_mut(|mem| mem.request_focus(egui::Id::new(("line_edit", "ledger_filter"))));
        frame(&ctx, &mut m, vec![egui::Event::Text("001".into())]);
        frame(&ctx, &mut m, vec![key(egui::Key::Enter)]);
        frame(&ctx, &mut m, vec![]);
        assert_eq!(m.filter_input, "001");
        // キャラ0001 と キャラ0010〜0019
        assert_eq!(m.visible().len(), 11);
    }
}
