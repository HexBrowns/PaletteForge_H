//! 確定してから使う入力欄（1 行の文字・数値。ルール au2-rs-plugin「入力の確定と取り消し」）。
//!
//! `ui` はテストでは組み込まない（本体の API を使う）ので、入力欄だけをここに分けてテストする。

use aviutl2_eframe::egui;

use crate::color::{parse_hex, to_hex};

/// 確定してから使う 1 行の入力欄（ルール au2-rs-plugin「入力の確定と取り消し」）。
///
/// 打っている間の文字は欄の中（egui の一時領域）だけに持ち、`value` を書き換えるのは
/// Enter・ほかをクリック・Tab でフォーカスが外れたときだけ。入力中の Esc は、入力を始める前の文字に戻す。
/// 戻り値は（欄の Response, 入力中の文字, 確定して `value` が変わったか）
pub fn line_edit(
    ui: &mut egui::Ui,
    id_salt: &str,
    value: &mut String,
    edit: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
) -> (egui::Response, String, bool) {
    // ウィンドウの中で一意な名前から作る（親の Ui の自動の ID は、前に出す部品の数で変わる）
    let id = egui::Id::new(("line_edit", id_salt));
    let key = id.with("pending");
    // 入力中の Esc は取り消し。egui はこのフレームの始めに Esc でフォーカスを外しているので、
    // 「フォーカスが無い + このフレームの Esc」なら、入力中だった文字を描く前に捨てる
    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().memory(|m| m.has_focus(id)) {
        ui.ctx().data_mut(|d| d.remove_temp::<String>(key));
    }
    // 前のフレームまで入力中だった文字。無ければ今の値を出す
    let mut buf = ui
        .ctx()
        .data(|d| d.get_temp::<String>(key))
        .unwrap_or_else(|| value.clone());
    let resp = ui.add(edit(egui::TextEdit::singleline(&mut buf).id(id)));
    let shown = buf.clone();
    let committed = finish_line_edit(ui.ctx(), key, resp.has_focus(), buf, value);
    (resp, shown, committed)
}

/// `line_edit` の後半。フォーカスがある間は入力中の文字を覚え、外れたら確定するか取り消す
fn finish_line_edit(
    ctx: &egui::Context,
    key: egui::Id,
    focused: bool,
    buf: String,
    value: &mut String,
) -> bool {
    if focused {
        ctx.data_mut(|d| d.insert_temp(key, buf));
        return false;
    }
    // 入力中だった文字が残っていれば確定する（Esc で取り消したときは、描く前に捨ててある）
    let was_editing = ctx.data_mut(|d| d.remove_temp::<String>(key)).is_some();
    if !was_editing || buf == *value {
        return false;
    }
    *value = buf;
    true
}

/// 数値の欄（ルール au2-rs-plugin「入力の確定と取り消し」）。`update_while_editing(false)` を付けて出す。
/// 打っている途中の値は使わず、Enter かほかをクリックしてフォーカスが外れたときに確定する。入力中の Esc は入力前の値に戻す。
/// ドラッグで変えた値はその場で使う。
///
/// egui 0.36 の `DragValue` は `update_while_editing(false)` でも、Esc の次のフレームで打った文字を値にしてしまう
/// （フォーカスを失ったとみなす期間が 2 フレームあり、2 フレーム目には Esc が押されていないため）。
/// Esc のフレームの値を覚えておき、次のフレームで戻す（参照実装 LayerSilenceCut_H の `setting_value`）
pub fn drag_value<T>(
    ui: &mut egui::Ui,
    value: &mut T,
    build: impl FnOnce(egui::DragValue<'_>) -> egui::DragValue<'_>,
) -> egui::Response
where
    T: egui::emath::Numeric + Default + Send + Sync,
{
    let resp = ui.add(build(egui::DragValue::new(value)).update_while_editing(false));
    let key = resp.id.with("escaped");
    let pass = ui.ctx().cumulative_pass_nr();
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ui.ctx().data_mut(|d| d.insert_temp(key, (pass, *value)));
    } else if let Some((at, before)) = ui.ctx().data_mut(|d| d.remove_temp::<(u64, T)>(key)) {
        if pass == at + 1 {
            *value = before;
        }
    }
    resp
}

/// 色の hex 入力欄。打っている途中の文字では色を変えない（3 桁の省略形が途中で当たって色が跳ぶ）。
/// Enter・ほかをクリックで確定し、読めない文字なら元の色のまま。入力中の Esc で元に戻す
pub fn hex_edit(ui: &mut egui::Ui, label: &str, rgb: &mut [u8; 3]) {
    let mut hex = to_hex(*rgb);
    let (resp, typing, committed) = line_edit(ui, &format!("hex_{label}"), &mut hex, |t| {
        t.desired_width(76.0).char_limit(7).hint_text("rrggbb")
    });
    if committed {
        if let Some(c) = parse_hex(&hex) {
            *rgb = c;
        }
    }
    if resp.has_focus() && parse_hex(&typing).is_none() {
        ui.colored_label(egui::Color32::from_rgb(0xfa, 0x52, 0x52), "?")
            .on_hover_text("6 桁（または 3 桁）の 16 進数で入れてください。このまま確定すると元の色に戻ります");
    }
}

#[cfg(test)]
mod tests {
    /// テストの 1 フレームの出力を捨てる。テクスチャの差分を片付けずに捨てると、デバッグビルドで epaint の debug_assert
    /// （Dropped TexturesDelta with N unapplied deltas）が落ちる（au2 release の prebuild の cargo test はデバッグビルド）
    fn discard_frame(mut out: egui::FullOutput) {
        out.textures_delta.clear();
    }

    use super::*;

    fn key(k: egui::Key) -> egui::Event {
        egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn run(ctx: &egui::Context, events: Vec<egui::Event>, f: impl FnMut(&mut egui::Ui)) {
        let raw = egui::RawInput {
            events,
            ..Default::default()
        };
        discard_frame(ctx.run_ui(raw, f));
    }

    /// 1 フレーム回す。戻り値は（確定したか, 欄に出ている文字）
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, value: &mut String) -> (bool, String) {
        let mut out = (false, String::new());
        run(ctx, events, |ui| {
            let (_, shown, committed) = line_edit(ui, "t", value, |t| t);
            out = (committed, shown);
        });
        out
    }

    fn focus(ctx: &egui::Context, salt: &str) {
        let id = egui::Id::new(("line_edit", salt));
        ctx.memory_mut(|m| m.request_focus(id));
    }

    /// フォーカスを当てて "X" を打つところまで
    fn start(ctx: &egui::Context, value: &mut String) {
        frame(ctx, vec![], value);
        focus(ctx, "t");
        let (committed, shown) = frame(ctx, vec![egui::Event::Text("X".into())], value);
        assert!(!committed);
        assert_eq!(shown, "oldX");
        assert_eq!(value, "old", "打っている途中では値を変えない");
    }

    #[test]
    fn enter_commits() {
        let ctx = egui::Context::default();
        let mut value = "old".to_string();
        start(&ctx, &mut value);
        assert!(frame(&ctx, vec![key(egui::Key::Enter)], &mut value).0);
        assert_eq!(value, "oldX");
        // 次のフレームで二度確定しない
        assert!(!frame(&ctx, vec![], &mut value).0);
    }

    #[test]
    fn escape_reverts() {
        let ctx = egui::Context::default();
        let mut value = "old".to_string();
        start(&ctx, &mut value);
        let (committed, shown) = frame(&ctx, vec![key(egui::Key::Escape)], &mut value);
        assert!(!committed);
        assert_eq!(value, "old");
        assert_eq!(shown, "old", "欄の文字も入力前に戻る");
        assert!(!frame(&ctx, vec![], &mut value).0);
        assert_eq!(value, "old");
    }

    #[test]
    fn losing_focus_commits() {
        let ctx = egui::Context::default();
        let mut value = "old".to_string();
        start(&ctx, &mut value);
        // ほかをクリックしたのと同じく、フォーカスを外す
        ctx.memory_mut(|m| m.stop_text_input());
        assert!(frame(&ctx, vec![], &mut value).0);
        assert_eq!(value, "oldX");
    }

    /// hex は打っている途中（3 桁の省略形が当たる "00f" など）では色を変えず、確定で変える
    #[test]
    fn hex_changes_color_only_on_commit() {
        let ctx = egui::Context::default();
        let mut rgb = [0x12, 0x34, 0x56];
        run(&ctx, vec![], |ui| hex_edit(ui, "c", &mut rgb));
        focus(&ctx, "hex_c");
        let mut events = vec![key(egui::Key::Backspace); 6];
        events.push(egui::Event::Text("00f".into()));
        run(&ctx, events, |ui| hex_edit(ui, "c", &mut rgb));
        assert_eq!(rgb, [0x12, 0x34, 0x56], "3 桁を打った途中で色を変えない");
        run(&ctx, vec![egui::Event::Text("f00".into())], |ui| hex_edit(ui, "c", &mut rgb));
        assert_eq!(rgb, [0x12, 0x34, 0x56]);
        run(&ctx, vec![key(egui::Key::Enter)], |ui| hex_edit(ui, "c", &mut rgb));
        assert_eq!(rgb, [0x00, 0xff, 0x00]);
    }

    #[test]
    fn hex_invalid_or_escape_keeps_color() {
        let ctx = egui::Context::default();
        let mut rgb = [0x12, 0x34, 0x56];
        run(&ctx, vec![], |ui| hex_edit(ui, "c", &mut rgb));
        focus(&ctx, "hex_c");
        run(&ctx, vec![egui::Event::Text("z".into())], |ui| hex_edit(ui, "c", &mut rgb));
        run(&ctx, vec![key(egui::Key::Enter)], |ui| hex_edit(ui, "c", &mut rgb));
        assert_eq!(rgb, [0x12, 0x34, 0x56], "読めない文字で確定したら元の色のまま");
        focus(&ctx, "hex_c");
        let mut events = vec![key(egui::Key::Backspace); 6];
        events.push(egui::Event::Text("ffffff".into()));
        run(&ctx, events, |ui| hex_edit(ui, "c", &mut rgb));
        run(&ctx, vec![key(egui::Key::Escape)], |ui| hex_edit(ui, "c", &mut rgb));
        assert_eq!(rgb, [0x12, 0x34, 0x56], "Esc で取り消す");
    }

    /// 数値の欄を 1 フレーム回す
    fn drag_frame(ctx: &egui::Context, events: Vec<egui::Event>, value: &mut i32, focus: bool) {
        run(ctx, events, |ui| {
            let resp = drag_value(ui, value, |d| d.range(0..=255));
            if focus {
                resp.request_focus();
            }
        });
    }

    /// フォーカスを当てて 5 を消し、42 を打つところまで
    fn drag_start(ctx: &egui::Context, value: &mut i32) {
        drag_frame(ctx, vec![], value, true);
        let mut events = vec![key(egui::Key::Backspace); 5];
        events.push(egui::Event::Text("42".into()));
        drag_frame(ctx, events, value, false);
        assert_eq!(*value, 5, "打っている途中では値を変えない");
    }

    #[test]
    fn drag_value_escape_reverts_for_following_frames() {
        let ctx = egui::Context::default();
        let mut value = 5;
        drag_start(&ctx, &mut value);
        drag_frame(&ctx, vec![key(egui::Key::Escape)], &mut value, false);
        assert_eq!(value, 5);
        // egui は Esc の次のフレームで打った文字を値にする。3 フレーム先まで元のままか
        for n in 1..=3 {
            drag_frame(&ctx, vec![], &mut value, false);
            assert_eq!(value, 5, "Esc の {n} フレーム後");
        }
    }

    #[test]
    fn drag_value_enter_commits() {
        let ctx = egui::Context::default();
        let mut value = 5;
        drag_start(&ctx, &mut value);
        drag_frame(&ctx, vec![key(egui::Key::Enter)], &mut value, false);
        assert_eq!(value, 42);
        for _ in 0..3 {
            drag_frame(&ctx, vec![], &mut value, false);
            assert_eq!(value, 42);
        }
    }
}
