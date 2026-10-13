//! PaletteForge_H — 役割ベースのカラーパレット生成プラグイン。
//!
//! シード 3 色から 8 色相 x 8 ステップ = 64 色を作り、`Default/*.palette` へ書き出す。
//! AviUtl2 のパレットは行優先なので、**1 行がそのまま 1 色相のスケール**になる。
//!
//! 反映は二段構え。**既存パレットは `EDIT_SECTION::set_palette_info()`（SDK 2026/8/9 追加）で
//! その場で更新できる**（ファイルの保存もホストがやる）。**新規パレットは API では作れない**ので、
//! そのときだけファイルを書いて再起動してもらう。詳しくは
//! `AI/docs/20260904_palette_host_investigation.md`。
//!
//! 同じロジックの CLI 版が `AI/tools/make_theme_palette.py` と
//! `AI/tools/build_palettes.py` にあり、**台帳ファイルは共通**。

pub mod color;
pub mod line_edit;
pub mod model;
pub mod storage;
pub mod theme;
pub mod view;
mod host_fonts;

#[cfg(not(test))]
pub mod edit_ops;
#[cfg(not(test))]
mod ui;

#[cfg(not(test))]
mod plugin {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use aviutl2::AnyResult;
    use aviutl2::raw_window_handle::HasWindowHandle;
    use parking_lot::Mutex;

    use crate::ui::{PaletteForgeApp, SharedState};

    fn init_logging() {
        let _ = aviutl2::tracing_subscriber::fmt()
            .with_max_level(if cfg!(debug_assertions) {
                aviutl2::tracing::Level::DEBUG
            } else {
                aviutl2::tracing::Level::INFO
            })
            .event_format(aviutl2::logger::AviUtl2Formatter)
            // 書き込み先を本体のログにする（無いと標準出力へ行き、本体のログに 1 行も残らない。v0.3.0 まで）
            .with_writer(aviutl2::logger::AviUtl2LogWriter)
            .try_init();
    }

    /// 初期化にかかった時間をログに 1 行出す（強化デバッグ。仕様 `AI/specifications/20261012_CurveEditor_H_キーフレーム速度_spec.md` の 7.0）。
    /// `register` の始めに作り、`register` を抜けるとき（途中の `return` も含む）に new と register の時間を出す
    struct InitTimeLog {
        new_time: std::time::Duration,
        register_start: std::time::Instant,
    }

    impl InitTimeLog {
        fn start(new_time: std::time::Duration) -> Self {
            Self { new_time, register_start: std::time::Instant::now() }
        }
    }

    impl Drop for InitTimeLog {
        fn drop(&mut self) {
            let register = self.register_start.elapsed();
            let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
            aviutl2::tracing::info!(
                "PaletteForge_H: 初期化 {:.1} ms（new {:.1} ms・register {:.1} ms）",
                ms(self.new_time + register),
                ms(self.new_time),
                ms(register)
            );
        }
    }

    #[aviutl2::plugin(GenericPlugin)]
    pub struct PaletteForgePlugin {
        window: Mutex<Option<aviutl2_eframe::EframeWindow>>,
        state: SharedState,
        window_created: AtomicBool,
        /// `new` にかかった時間（初期化の時間をログに出すため。`InitTimeLog`）
        init_new_time: std::time::Duration,
    }

    impl PaletteForgePlugin {
        fn ensure_window(&self) -> AnyResult<()> {
            if self.window_created.load(Ordering::Acquire) {
                return Ok(());
            }
            let mut guard = self.window.lock();
            if guard.is_some() {
                self.window_created.store(true, Ordering::Release);
                return Ok(());
            }
            let state = Arc::clone(&self.state);
            *guard = Some(aviutl2_eframe::EframeWindow::new(
                "PaletteForge_H",
                move |cc, handle| Ok(Box::new(PaletteForgeApp::new(cc, handle, state))),
            )?);
            self.window_created.store(true, Ordering::Release);
            Ok(())
        }
    }

    impl aviutl2::generic::GenericPlugin for PaletteForgePlugin {
        fn new(_info: aviutl2::AviUtl2Info) -> AnyResult<Self> {
            let init_t0 = std::time::Instant::now();
            init_logging();
            tracing::info!("PaletteForge_H v{} 初期化", env!("CARGO_PKG_VERSION"));
            Ok(Self {
                window: Mutex::new(None),
                state: crate::ui::new_state(),
                window_created: AtomicBool::new(false),
                init_new_time: init_t0.elapsed(),
            })
        }

        fn plugin_info(&self) -> aviutl2::generic::GenericPluginTable {
            aviutl2::generic::GenericPluginTable {
                name: "PaletteForge_H".to_string(),
                information: format!(
                    "PaletteForge_H v{version} - role based palette generator / by HexBrowns",
                    version = env!("CARGO_PKG_VERSION")
                ),
            }
        }

        fn register(&mut self, registry: &mut aviutl2::generic::HostAppHandle) {
            // 抜けるときに初期化の時間をログに出す（`InitTimeLog`）
            let _init_time = InitTimeLog::start(self.init_new_time);
            // set_palette_info で既存パレットをその場で更新するために要る
            crate::edit_ops::EDIT_HANDLE.init(registry.create_edit_handle());
            if let Err(e) = self.ensure_window() {
                tracing::error!("PaletteForge_H ウィンドウ初期化失敗: {e:#}");
            } else {
                let guard = self.window.lock();
                if let Some(window) = guard.as_ref() {
                    match window.handle() {
                        Ok(handle) => {
                            if let Err(e) = registry.register_window_client("PaletteForge_H", &handle)
                            {
                                tracing::error!("register_window_client 失敗: {e}");
                            }
                            // 見えていない間は描かない（`ui::window_visible`）
                            if let Ok(raw) = handle.window_handle() {
                                if let aviutl2::raw_window_handle::RawWindowHandle::Win32(h) = raw.as_raw() {
                                    crate::ui::WINDOW_HWND.store(h.hwnd.get(), Ordering::Release);
                                }
                            }
                        }
                        Err(e) => tracing::error!("PaletteForge_H ハンドル取得失敗: {e:#}"),
                    }
                }
            }
            // ウィンドウは表示メニューから開く（メニューから ShowWindow しても開かない。2026-10-10 にユーザーの指摘で外した）
        }
    }

    aviutl2::register_generic_plugin!(PaletteForgePlugin);
}
