//! PaletteForge_H の egui ウィンドウ（本体とつなぐ部分）。
//!
//! 描くのは `view.rs`、状態と操作は `model.rs`。ここは本体の API とファイルを `Backend` として渡すのと、
//! 見えていない間は描かない判定だけを持つ（ルール au2-rs-plugin「ウィンドウの重さ」）。

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aviutl2_eframe::{eframe, egui, AviUtl2EframeHandle};
use parking_lot::RwLock;

use crate::model::{ApplyError, Backend, Model};
use crate::storage::{self, Ledger};
use crate::theme::Palette;

pub type SharedState = Arc<RwLock<Model>>;

/// 見えていない間、見えたかを確かめる間隔
const HIDDEN_CHECK: Duration = Duration::from_millis(500);

/// 自分のウィンドウ（`register` で入れる）。0 なら見えているとみなす
pub static WINDOW_HWND: AtomicIsize = AtomicIsize::new(0);

/// ウィンドウが見えているか（本体のパネルに出ていなければ偽）
fn window_visible() -> bool {
    let hwnd = WINDOW_HWND.load(Ordering::Acquire);
    if hwnd == 0 {
        return true;
    }
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(windows::Win32::Foundation::HWND(
            hwnd as _,
        ))
        .as_bool()
    }
}

pub fn new_state() -> SharedState {
    Arc::new(RwLock::new(Model::new(storage::load_ledger())))
}

/// 本体の編集 API と `Default` / 台帳のファイル
struct HostBackend;

impl Backend for HostBackend {
    fn apply_palette(&mut self, name: &str, pal: &Palette) -> Result<(), ApplyError> {
        crate::edit_ops::apply_palette(name, pal)
    }
    fn palette_exists(&self, file_name: &str) -> bool {
        storage::palette_exists(file_name)
    }
    fn write_palette(&mut self, file_name: &str, pal: &Palette) -> anyhow::Result<()> {
        storage::write_palette(file_name, pal).map(|_| ())
    }
    fn remove_palette(&mut self, file_name: &str) -> anyhow::Result<bool> {
        storage::remove_palette(file_name)
    }
    fn update_ledger(&mut self, change: &mut dyn FnMut(&mut Ledger)) -> anyhow::Result<Ledger> {
        storage::update_ledger(|l| change(l))
    }
}

pub struct PaletteForgeApp {
    state: SharedState,
    _handle: AviUtl2EframeHandle,
    /// 本体のフォント（窓が見えてから読む。`host_fonts`）
    host_fonts: crate::host_fonts::LazyFonts,
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
        // 本体のフォントは窓が見えてから ui で読む（ここで読むと、本体の起動がこの窓の初期化を待つ間ずっと止まる。`host_fonts`）
        let host_fonts = crate::host_fonts::LazyFonts::new(&handle);
        Self {
            state,
            _handle: handle,
            host_fonts,
        }
    }
}

impl eframe::App for PaletteForgeApp {
    // aviutl2-eframe はホストのウィンドウ内に描くので、update ではなく ui を実装する
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 本体のフォントは窓が見えてから読む。読んだフレームは描かない（新しいフォントは次のフレームから効く。次のフレームは頼んである）
        if self.host_fonts.load_if_visible(ui.ctx()) {
            return;
        }
        // 本体のパネルに出ていない間は描かない（eframe は隠れていても ui を呼び続ける）
        if !window_visible() {
            ui.ctx().request_repaint_after(HIDDEN_CHECK);
            return;
        }
        let mut state = self.state.write();
        crate::view::draw(ui, &mut state, &mut HostBackend, Instant::now());
    }
}
