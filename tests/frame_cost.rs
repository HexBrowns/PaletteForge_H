//! ウィンドウの 1 フレームの時間の目安（台帳 0 / 300 / 1000 / 3000 件、ガマットの外へ出やすい種の色 ff0000 / 00ff00 / 0000ff）。
//! `view::draw` を画面なしの egui で回す。モデル（色からの計算）は別に、色を変えたフレームと変えていないフレームで測る。
//! 時間は機械で変わるので、既定では走らせない:
//!   cargo test --release --test frame_cost -- --ignored --nocapture

use std::time::Instant;

use aviutl2_eframe::egui;
use palette_forge_h::color::parse_hex;
use palette_forge_h::model::{ApplyError, Backend, Model};
use palette_forge_h::storage::{Entry, Ledger};
use palette_forge_h::theme::{Palette, Seeds};
use palette_forge_h::view;

/// 何もしない本体（測るのは描画だけ）
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

fn worst_seeds() -> Seeds {
    let mut s = Seeds::default();
    s.main = parse_hex("ff0000").unwrap();
    s.sub = parse_hex("00ff00").unwrap();
    s.accent = parse_hex("0000ff").unwrap();
    s
}

fn model(n: usize) -> Model {
    let mut ledger = Ledger::default();
    ledger.set_core(Entry::from_seeds("", &worst_seeds()));
    for i in 0..n {
        ledger.upsert(Entry::from_seeds(&format!("キャラ{i:04}"), &worst_seeds()));
    }
    let mut m = Model::new(ledger);
    m.show_semantic = true;
    if n > 0 {
        m.click_chara("キャラ0000", Instant::now());
    }
    m
}

fn run(ctx: &egui::Context, m: &mut Model) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| view::draw(ui, m, &mut NullBackend, Instant::now()));
    // テクスチャの差分を片付けてから捨てる（デバッグビルドの debug_assert）
    out.textures_delta.clear();
}

#[test]
#[ignore]
fn frame_cost_by_ledger_size() {
    for n in [0usize, 300, 1000, 3000] {
        let mut m = model(n);
        let ctx = egui::Context::default();
        for _ in 0..3 {
            run(&ctx, &mut m);
        }
        let frames = 20;
        let started = Instant::now();
        for _ in 0..frames {
            run(&ctx, &mut m);
        }
        let still = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        // 色を変え続けたとき（ピッカーのドラッグ。毎フレーム 2 枚の生成と検証が走る）
        let started = Instant::now();
        for i in 0..frames {
            m.seeds.main[1] = i as u8;
            run(&ctx, &mut m);
        }
        let dragging = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        // モデルだけ: 変えていないとき（覚えた値を返す）と、変えたとき（作り直す）
        let started = Instant::now();
        let mut sink = 0;
        for _ in 0..frames {
            sink += m.preview().rep_light.issues.len() + m.visible().len();
        }
        let model_still = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        let started = Instant::now();
        for i in 0..frames {
            m.seeds.main[2] = i as u8;
            sink += m.preview().rep_light.issues.len();
        }
        let model_changed = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!(
            "[INFO] 台帳 {n} 件: 1 フレーム {still:.2} ms / 色を変え続けて {dragging:.2} ms（モデルだけ: 変えないとき {model_still:.4} ms / 変えたとき {model_changed:.2} ms）({sink})"
        );
    }
}
