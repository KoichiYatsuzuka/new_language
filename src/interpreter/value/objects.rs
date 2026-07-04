// value/objects.rs — モジュール・Python 相互運用・ファイル I/O 型: PyObjHandle / NamespaceData / ModuleState / FileOpenModeRust / ByteModeRust / FileData。

#[allow(unused_imports)]
use {
    std::cell::RefCell, std::collections::{HashMap, HashSet}, std::fmt,
    std::path::PathBuf, std::rc::Rc, std::sync::atomic::{AtomicU32, Ordering}, std::sync::Arc,
    indexmap::IndexMap,
    crate::ast::{Accessibility, Param, Stmt},
    crate::interpreter::async_mgr,
};
#[allow(unused_imports)]
use super::*;


// ---------------------------------------------------------------------------
// Module / Python interop types
// ---------------------------------------------------------------------------

/// PyO3 を通じて Python オブジェクトへの参照を保持するハンドル。
/// GIL を保持せずにオブジェクトを所有でき、ドロップ時に Python 側の参照カウントを自動減少させる。
pub struct PyObjHandle {
    pub inner: pyo3::Py<pyo3::PyAny>,
}


impl std::fmt::Debug for PyObjHandle {
    /// `PyObjHandle` のデバッグ表示。常に `"<PyObject>"` を出力する。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<PyObject>")
    }
}


/// モジュールまたは名前空間の実行時データ。
/// `import[py] mod as m` で `m` にバインドされる。
/// `m.ClassName()` のように `.` でメンバにアクセスする。
#[derive(Debug, Clone)]
pub struct NamespaceData {
    /// モジュール名（エラーメッセージに使用）
    pub name: String,
    /// メンバ名 → 値のマップ
    pub members: HashMap<String, Value>,
}


/// モジュールキャッシュのエントリ状態。
#[derive(Debug, Clone)]
pub enum ModuleState {
    /// 現在ロード中（循環 import 検出用）
    Loading,
    /// ロード済み
    Loaded(Rc<NamespaceData>),
}


// ---------------------------------------------------------------------------
// File I/O types
// ---------------------------------------------------------------------------

/// ファイルオープンモード（Rust 内部表現）。
/// tl 側の `FileOpenMode` 列挙型と整数値で対応する。
#[derive(Debug, Clone, PartialEq)]
pub enum FileOpenModeRust {
    /// 既存ファイルを読み書きモードで開く（内容保持）
    Write,
    /// ファイルを空の状態から読み書きモードで開く（内容破棄）
    Rewrite,
    /// 既存ファイルを読み取り専用で開く
    Read,
    /// 新規ファイルを作成して読み書きモードで開く（既存時はエラー）
    MakeAndWrite,
}


/// バイト認識モード（Rust 内部表現）。
/// tl 側の `ByteRecognizingMode` 列挙型と整数値で対応する。
#[derive(Debug, Clone, PartialEq)]
pub enum ByteModeRust {
    /// バイト列として扱う: read 系は `list[int]`、write 系は `list[int]` を受け取る
    Byte,
    /// UTF-8 テキストとして扱う: read 系は `str`、write 系は `str` を受け取る
    Text,
}


/// ファイルオブジェクトの実行時状態。`open()` 組み込み関数で生成される。
///
/// - `path`: ファイルパス文字列
/// - `mode`: オープンモード
/// - `byte_mode`: バイト/テキストモード
/// - `content`: ファイル内容のメモリバッファ（open 時に全読み込み）
/// - `pointer`: 現在の読み書き位置（バイトインデックス）
/// - `is_closed`: `close()` または Drop 時に `true` にセット
/// - `file_handle`: 排他ロック保持用のファイルハンドル（close 時に None にセット）
#[derive(Debug)]
pub struct FileData {
    pub path: String,
    pub mode: FileOpenModeRust,
    pub byte_mode: ByteModeRust,
    /// ファイル内容のメモリバッファ。読み書きはこのバッファに対して行い、close 時にディスクへ書き戻す。
    pub content: Vec<u8>,
    /// 現在の読み書き位置（バイトインデックス）。0 がファイル先頭、content.len() がEOF。
    pub pointer: usize,
    pub is_closed: bool,
    /// ファイルハンドル。書き込みモードでは排他ロックとして機能し、close 時に None にセット。
    pub file_handle: Option<std::fs::File>,
}


impl FileData {
    /// バッファをディスクに書き戻してファイルハンドルを閉じる。
    /// 書き込みモード (`write` / `rewrite` / `make_and_write`) のみ実際に書き戻す。
    /// 既に close 済みの場合は何もしない。
    pub fn close(&mut self) {
        if self.is_closed {
            return;
        }
        self.is_closed = true;
        if matches!(
            self.mode,
            FileOpenModeRust::Write | FileOpenModeRust::Rewrite | FileOpenModeRust::MakeAndWrite
        ) {
            if let Some(ref mut f) = self.file_handle {
                use std::io::{Seek, SeekFrom, Write};
                let _ = f.seek(SeekFrom::Start(0));
                let _ = f.write_all(&self.content);
                // ファイルサイズをバッファサイズに合わせてトリム（書き込みが元より短い場合）
                let _ = f.set_len(self.content.len() as u64);
                let _ = f.flush();
            }
        }
        self.file_handle = None;
    }
}


impl Drop for FileData {
    /// `FileData` がスコープを抜けるときに自動的に `close()` を呼び出す。
    /// 書き込みモードの場合、バッファをディスクに書き戻してからハンドルを解放する。
    fn drop(&mut self) {
        self.close();
    }
}
