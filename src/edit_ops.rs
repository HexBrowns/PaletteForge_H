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

use aviutl2::generic::{GlobalEditHandle, PaletteColor, PaletteInfo};

use crate::theme::Palette;

pub static EDIT_HANDLE: GlobalEditHandle = GlobalEditHandle::new();

/// 既存パレットを編集 API で更新する。存在しないパレットには失敗する。
pub fn apply_palette(name: &str, pal: &Palette) -> Result<(), String> {
    if !EDIT_HANDLE.is_ready() {
        return Err("編集 API の準備ができていない".to_string());
    }
    let flat = pal.flat();
    if flat.len() != PaletteInfo::PALETTE_NUM {
        return Err(format!(
            "色数が {} で、パレットの {} と合わない",
            flat.len(),
            PaletteInfo::PALETTE_NUM
        ));
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
    EDIT_HANDLE
        .call_edit_section(move |edit| {
            edit.set_palette_info(&name, &info)
                .map_err(|e| e.to_string())
        })
        .map_err(|e| e.to_string())
        .and_then(|r| r)
}
