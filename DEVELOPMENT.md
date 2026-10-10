# PaletteForge_H

シード 3 色から、役割ベースの 64 色パレットを作る AviUtl2 プラグイン。

AviUtl2 のパレットは 8x8 = 64 色で**行優先**なので、**1 行がそのまま 1 色相の 8 段スケール**になる。
この形に合わせて 8 色相 x 8 ステップを割り当てる。

| 行 | 内容 | 列 | 役割 |
|---|---|---|---|
| 1 | ニュートラル（メイン色相を微量だけ混ぜたグレー） | 1 | base — 最も薄い面 |
| 2 | メイン | 2 | subtle — カード、帯、字幕背景 |
| 3 | サブ | 3 | surface — パネル、囲み |
| 4 | アクセント | 4 | border — 罫線 |
| 5 | 成功 | 5 | strong-border — 強い境界、影 |
| 6 | 注意 | 6 | **solid — ソリッド塗り（主役）** |
| 7 | 警告 | 7 | text-low — 副次テキスト |
| 8 | 情報 | 8 | text-high — 高コントラストテキスト |

Light / Dark の 2 枚を組で作る。**同じ位置に同じ役割の色がある**ことが運用上の要になる。

## 使い方

「表示」メニュー →「PaletteForge_H」（ウィンドウ配置パネルからも選択可）。

1. 左の台帳で **Core** かキャラを選ぶ（色を読み込む）。新しく作るなら **新しいキャラ** を押して名前を入れる
2. メイン / サブ / アクセントを決める（色ピッカーか hex 直接入力）
3. 右の 8x8 プレビューと検証結果を見ながら調整する
4. 「パレットに書き出す」

書き出し先は `Default/Core.Light.palette` / `Default/Chara.<名前> Light.palette` など。
上の「書き出し先」の Core / キャラは**書き出す先だけ**を変え、色は読み込まない（読み込むのは左の台帳）。

v0.3.0 から、取り返しのつかない操作は 2 回押しで確かめる（4 秒以内にもう一度）。
キャラの削除、読み込んだのとは別の既存の名前（台帳かファイルにある）への書き出し、書き出していない色を捨てる切り替え。
コードは `src/model.rs`（状態と操作。`Backend` 越しに本体とファイルを触るのでテストで回せる）、`src/view.rs`（描画）、
`src/ui.rs`（本体の API をつなぐ・見えていない間は描かない）に分けている。

**反映のしかたは 2 通りある。**

| 状況 | どうなるか |
|---|---|
| **既にあるパレットへの書き出し** | `set_palette_info()` でその場で反映される。**再起動は要らない**（色設定でパレットを開き直すと見える） |
| **新しいパレットの作成** | API では作れないので `.palette` を書く。**AviUtl2 の再起動が 1 回要る** |

ステータス欄がどちらだったかを枚数付きで教えてくれる。ログにも `PaletteForge_H: 書き出し …` を 1 行出す（失敗は WARN）。

編集 API の失敗は 2 つに分ける（`src/edit_ops.rs`）。`call_edit_section` そのものの失敗は本体が編集を受け付けなかった
（本体 2.1.13 からはドラッグ中・出力中に失敗する）ので、ファイルも台帳も書かずに「もう一度押す」と出す。
中の `set_palette_info` の失敗は本体にそのパレットが無いので、ファイルを書く（既にファイルがあれば「本体に読み込まれていない。再起動が要る」）。

ホストは起動時に一度だけ `Default/*.palette` を読み、以後**ファイルを書き換えても読み直さない**。
だから新規作成には再起動が要る。一方 `set_palette_info()` はホスト内部のパレットを直接更新し、
ファイルの保存もホスト側でやってくれるので即座に効く。

**キャラを増やすときの流れ**: 名前を決めて一度書き出す（ファイルができる）→ 再起動 →
以後はシードを何度でも調整でき、そのつど即反映される。

**これが CLI との差**。`build_palettes.py` はファイルしか書けないので、必ず再起動が要る。

## 検証

書き出す前に、仕様書 §5 の 6 ルールを毎回チェックしている。

| | 内容 |
|---|---|
| V1 | 隣接ステップの ΔL* が 5 以上（段差が見分けられるか） |
| V2 | text-high と base / subtle が 7:1 以上 |
| V3 | text-low と base / subtle が 4.5:1 以上 |
| V4 | 64 色に完全重複が無い |
| V5 | ガマットマッピングで C が半分以上削られた色を報告 |
| V6 | solid の上に乗せる文字色（白か黒か）を判定 |

**V4 が落ちるときは、たいてい 2 つの行に同じシードを使っている**（例: アクセントに
Open Color の yellow6 を選ぶと警告色と衝突する）。その場合はシード欄の下に
どの行同士が衝突しているかが出るので、片方を変えるか「セマンティック 4 色を編集」で差し替える。

## CLI 版と台帳を共有する

同じロジックの CLI があり、**台帳 `AI/palettes/palettes.json` は共通**（この開発環境に台帳があるときだけ使う。無い環境では `Plugin\PaletteForge_H\palettes.json` に置き、データフォルダに `AI` を作らない。v0.2.0 で 2026-10-10 に PaletteForge から改名）。

```bash
python AI/tools/build_palettes.py --list
python AI/tools/build_palettes.py --restore        # 固定パレットの欠けを復活
python AI/tools/build_palettes.py --add "名前" --main "#..." --sub "#..." --accent "#..."
```

プラグインで作ったキャラは `--list` に出るし、その逆も同じ。
プラグインは保存の直前に台帳を読み直し、自分の 1 件だけを足す・消す（`storage::update_ledger_at`）ので、起動中に CLI が変えた分は消えない。
知らない項目は `#[serde(flatten)]` の `extra` に持って書き戻す。
Web セーフや Open Color の参照パレットは CLI 側にある。

## 命名の制約

**AviUtl2 のパレット名は `.` がちょうど 1 個のときだけ「ラベル.名前」に分割**され、
UI でサブメニューに畳まれる。2 個以上あると分割されずフルネームが平らに並ぶ（実機で確認）。
そのため第 2 区切りは半角スペースにしてあり、**キャラ名に `.` は使えない**。

## ビルド

```powershell
python AI/tools/au2_build.py PaletteForge_H             # au2 release → 本番（C:\ProgramData\aviutl2）へ配置
python AI/tools/au2_build.py PaletteForge_H --no-deploy # 配置しない（本番との違いだけ出す）
```

ビルドと同梱物は `aviutl2.toml`（[aviutl2-cli](https://github.com/sevenc-nanashi/aviutl2-cli)）が正本。このフォルダで `au2 release` だけを実行すると `release/` に au2pkg ができる。

`cargo test --lib`（`aviutl2.toml` の prebuild）→ `cargo build --release` → `Plugin\PaletteForge_H\PaletteForge_H.aux2`。
**AviUtl2 が起動中は配置の前に止まる**（ホストが掴んでいて置き換えられない）。

テストは色変換の往復・ガマットマッピング・検証ルールに加えて、
**Python 版（`AI/tools/make_theme_palette.py`）と出力が 1 色も違わないこと**を確認している。

1 フレームの時間（台帳 0 / 300 / 1000 / 3000 件、種の色 ff0000 / 00ff00 / 0000ff）は `cargo test --release --test frame_cost -- --ignored --nocapture` で測る。
直す前と後の数値は `issues/20261010_ledger_and_preview_per_frame_cost.md`。

## 背景

ホストにはパレットを**書き込む API が無い**（読み取り 3 種のみ）。そのため
`.palette` ファイルを直接書く方式を採っている。この判断の根拠と実測は
`AI/docs/20260904_palette_host_investigation.md` にある。

色設計の考え方そのものは `AI/docs/aviutl2-palette-spec.md`。
