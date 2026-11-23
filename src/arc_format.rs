//! arc_format.rs — コンパイル済みモジュール（`.arc`）の**形式**: 定数と読み取り。
//!
//! # 形式（version 0）
//!
//! ソースだけを埋め込む。
//!
//! ```text
//! [4 bytes]  magic    : b"TLC\x00"
//! [4 bytes]  version  : u32 LE  (0)
//! [4 bytes]  name_len : u32 LE
//! [name_len] name     : UTF-8 module name
//! [4 bytes]  src_len  : u32 LE
//! [src_len]  source   : UTF-8 source text
//! ```
//!
//! # 形式（version 1）
//!
//! version 0 に、ネイティブにコンパイルした共有ライブラリ（DLL/SO/dylib）を足したもの。
//!
//! ```text
//! （version 0 と同じ 5 項目・version は 1）
//! [4 bytes]  n_fns    : u32 LE  (number of natively compiled functions)
//! for each fn:
//!   [4 bytes]       fn_name_len : u32 LE
//!   [fn_name_len]   fn_name     : UTF-8
//!   [4 bytes]       n_params    : u32 LE
//! [4 bytes]  dll_len  : u32 LE
//! [dll_len]  dll_bytes: raw shared-library bytes
//! ```
//!
//! # 形式（version 3）
//!
//! version 1 に、DLL の生成コードが前提にするコールバックの表の大きさ（使う欄の最大の番号 + 1）を足したもの
//! （known_bugs.md #29）。表は末尾に欄を足して伸びるので、読む exe の表がこれより短ければ、DLL は表の外を引いて落ちる。
//! 読み取り（[`parse`]）が [`CALLBACK_TABLE_LEN`] と照らして誤りにする（CLI も VS Code 拡張も同じ）。
//!
//! ```text
//! （version 0 と同じ 5 項目・version は 3）
//! [4 bytes]  n_callbacks : u32 LE  (callback-table fields the DLL needs)
//! （version 1 の n_fns 以降と同じ）
//! ```
//!
//! ⚠ version 1 の `.arc` は大きさを持たないので照らさない（version 3 より前の exe が作ったもので、表はその後
//!   伸びる一方なので、新しい exe が読む分には足りている）。version 3 より前の exe は version 3 を
//!   `unsupported .arc version 3` で読まない（落ちずに誤りになる）。
//! ⚠ version 2 は使わない: 取り除いた inkwell の JIT のころ、LLVM のビットコードを持つ version 2 があった
//!   （コミット `a8035d95` で削除済み）。その残りを新しい形として読み違えないよう、番号を飛ばす（今も読まない）。
//!
//! # なぜ `partial_compiler` の外にあるか
//!
//! import の処理（`parser/imports/ar_modules.rs`）は `.arc` から**埋め込みソース**を取り出して型を読む。
//! VS Code 拡張（wasm）でも CLI と同じ import の処理を使うので、形式の読み取りを LLVM・clang・DLL を扱う
//! `partial_compiler/module_compiler.rs`（書き出しと、実行時の DLL の登録）から切り出した
//! （editor_import_resolution_plan.md 3-2）。ファイルは `crate::import_fs` で読む。

use std::path::Path;

pub const MAGIC: &[u8; 4] = b"TLC\x00";
pub const VERSION_V0: u32 = 0;
pub const VERSION_V1: u32 = 1;
pub const VERSION_V3: u32 = 3;

/// この arrow のコールバックの表の欄の数（`interpreter::native_api::ArCallbacks` の欄の数）。version 3 の `.arc` の
/// DLL がこれより大きい表を前提にすれば、読み取りが誤りにする（known_bugs.md #29）。
/// ⚠ 表に欄を足したらここも足す（`native_api/mod.rs` のコンパイル時の `assert!` が、ずれるとビルドを止める）。
/// 形式の読み取りに置くのは、VS Code 拡張（wasm・`ArCallbacks` を持たない）も CLI と同じ誤りを出すため
/// （`compare_wasm_frontend`）。
pub const CALLBACK_TABLE_LEN: u32 = 88;

/// `.arc` の中身。
pub struct ArcFile {
    pub module_name: String,
    pub source: String,
    /// version 1 のネイティブ部（関数の表 `(名前, 引数の数)` と DLL のバイト列）。version 0 は `None`。
    pub native: Option<(Vec<(String, usize)>, Vec<u8>)>,
}

/// `.arc` を読む（`crate::import_fs` 経由）。
pub fn read(path: &Path) -> std::io::Result<ArcFile> {
    let data = crate::import_fs::read(path)?;
    parse(&data).map_err(|msg| std::io::Error::new(std::io::ErrorKind::InvalidData, msg))
}

/// `.arc` のバイト列を読む。
pub fn parse(data: &[u8]) -> Result<ArcFile, String> {
    let mut pos = 0;

    if data.len() < 4 || &data[..4] != MAGIC {
        return Err("not a valid .arc file (bad magic)".into());
    }
    pos += 4;

    let version = read_u32(data, &mut pos)?;
    // version 2 は飛ばした番号（モジュールの doc）。表に無い番号は読まない
    if !matches!(version, VERSION_V0 | VERSION_V1 | VERSION_V3) {
        return Err(format!("unsupported .arc version {version}"));
    }

    let module_name = read_string(data, &mut pos)?;
    let source = read_string(data, &mut pos)?;

    if version == VERSION_V0 {
        return Ok(ArcFile { module_name, source, native: None });
    }

    // version 3 だけがコールバックの表の大きさを持つ（version 1 は照らさない・モジュールの doc）
    if version == VERSION_V3 {
        let need = read_u32(data, &mut pos)?;
        if need > CALLBACK_TABLE_LEN {
            return Err(format!(
                "it was compiled by a newer arrow (its native code needs {need} callbacks, this arrow has \
                 {CALLBACK_TABLE_LEN}); recompile it with `arrow --compile`"
            ));
        }
    }

    // version 1 / 3: parse fn export table
    let n_fns = read_u32(data, &mut pos)? as usize;
    let mut exports = Vec::with_capacity(n_fns);
    for _ in 0..n_fns {
        let fn_name = read_string(data, &mut pos)?;
        let n_params = read_u32(data, &mut pos)? as usize;
        exports.push((fn_name, n_params));
    }

    let payload = read_len_prefixed(data, &mut pos)?.to_vec();

    Ok(ArcFile { module_name, source, native: Some((exports, payload)) })
}

/// バイト列の現在位置から u32 をリトルエンディアンで読み取り、位置を4バイト進める。
fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32, String> {
    if data.len() < *pos + 4 {
        return Err("unexpected end of .arc data".into());
    }
    let v = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    Ok(v)
}

/// バイト列の現在位置から `len` バイトのスライスを返し、位置を進める。
fn read_bytes<'a>(data: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8], String> {
    if data.len() < *pos + len {
        return Err("unexpected end of .arc data".into());
    }
    let slice = &data[*pos..*pos + len];
    *pos += len;
    Ok(slice)
}

/// `[u32 LE length][bytes]` を読み取りスライスを返す。
fn read_len_prefixed<'a>(data: &'a [u8], pos: &mut usize) -> Result<&'a [u8], String> {
    let len = read_u32(data, pos)? as usize;
    read_bytes(data, pos, len)
}

/// `[u32 LE length][UTF-8 bytes]` を読み取り String を返す。
fn read_string(data: &[u8], pos: &mut usize) -> Result<String, String> {
    let bytes = read_len_prefixed(data, pos)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| "invalid UTF-8 in .arc data".to_string())
}
