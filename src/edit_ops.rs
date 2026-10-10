//! 編集 API 経由でパレットを更新する。
//!
//! **なぜ API を使うのか。** ホストは起動時に一度だけ `Default/*.palette` を読み、
//! 以後ファイルを書き換えても読み直さない（実測）。一方
//! `EDIT_SECTION::set_palette_info()`（SDK 2026/8/9 追加）はホスト内部のパレットを
//! 直接更新し、**ファイルの保存もホストがやってくれる**ので、その場で反映される。
//!
//! ただし SDK のコメントにあるとおり **「対象が見つからない場合は失敗します」**。
//! つまり**既存パレットの更新専用で、新規作成はできない**。新規のときはファイルを
//! 書いて再起動してもらうしかない（PaletteHistory も同じ二段構えになっている）。

use aviutl2::generic::{EditSectionError, GlobalEditHandle, PaletteColor, PaletteInfo};

use crate::model::ApplyError;
use crate::theme::Palette;

pub static EDIT_HANDLE: GlobalEditHandle = GlobalEditHandle::new();

/// 既存パレットを編集 API で更新する。存在しないパレットには失敗する。
///
/// 失敗の理由を分けて返す。`call_edit_section` そのものの失敗は本体が編集を受け付けなかった
/// （本体 2.1.13 からはドラッグ中・出力中などに失敗する）ので `Busy`、中の `set_palette_info` の失敗は
/// 本体にそのパレットが無い（SDK「対象が見つからない場合は失敗します」）ので `Refused`。
pub fn apply_palette(name: &str, pal: &Palette) -> Result<(), ApplyError> {
    if !EDIT_HANDLE.is_ready() {
        return Err(ApplyError::NotReady);
    }
    let flat = pal.flat();
    if flat.len() != PaletteInfo::PALETTE_NUM {
        return Err(ApplyError::Other(format!(
            "色数が {} で、パレットの {} と合わない",
            flat.len(),
            PaletteInfo::PALETTE_NUM
        )));
    }
    let mut info = PaletteInfo {
        colors: [PaletteColor {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        }; PaletteInfo::PALETTE_NUM],
    };
    for (slot, rgb) in info.colors.iter_mut().zip(flat.iter()) {
        // SDK が「a は常に 255」と定めているのでアルファは触らない
        *slot = PaletteColor {
            r: rgb[0],
            g: rgb[1],
            b: rgb[2],
            a: 255,
        };
    }

    let name = name.to_string();
    let pname = name.clone();
    match EDIT_HANDLE.call_edit_section(move |edit| edit.set_palette_info(&name, &info)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(EditSectionError::ApiCallFailed)) => {
            tracing::debug!("PaletteForge_H: set_palette_info({pname}) を本体が受け付けなかった");
            Err(ApplyError::Refused)
        }
        Ok(Err(e)) => Err(ApplyError::Other(e.to_string())),
        Err(e) => {
            tracing::warn!("PaletteForge_H: call_edit_section 失敗（{pname}）: {e}");
            Err(ApplyError::Busy)
        }
    }
}
