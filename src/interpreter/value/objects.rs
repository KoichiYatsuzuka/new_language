// value/objects.rs — モジュール・Python 相互運用・ファイル I/O 型: PyObjHandle / NamespaceData / ModuleState / FileOpenModeRust / ByteModeRust / FileData。

use {
    std::collections::HashMap, std::rc::Rc,
};
use super::*;


// ---------------------------------------------------------------------------
// Module / Python interop types
// ---------------------------------------------------------------------------

/// CPython のオブジェクトへの参照 **1 つ**（pandas_import_plan.md の D-5・タスク 1-1）。
///
/// ⚠⚠ **写し 1 つが CPython の参照 1 つを持つ。** `Clone` で参照を増やし、`Drop` で減らす（実装は `py_interop`）。
///   CPython のコードは参照の数で振る舞いを変える（pandas の連鎖代入の警告・numpy の `resize` / 一時配列の再利用）ので、
///   Arrow の束縛・欄・要素・VM のスタックの値がそれぞれ 1 つずつ持ち、CPython で同じ形を書いたときと同じ数にする。
///   以前は `Arc<PyObjHandle>` で 1 つの参照を共有していたため、束縛がいくつあっても CPython からは 1 つに見え、
///   変数への `df["c"] = ..` が連鎖代入と誤判定されていた。
/// ⚠ GIL を持たずに写す・捨てるときは、増減を `py_interop` の待ち行列に積み、CPython へ入る入口で反映する
///   （CPython が数を見られるのは入口の中だけなので、見える数は正確になる）。
///
/// ⚠ **`Value::PyObject` の variant 自体は評価コアビルドにも残す**（評価コア切り出し #2）。
/// 消すと `Value` を網羅 `match` している 100 箇所超に `#[cfg]` を配る必要があり、
/// variant を足したときに止まる 2 段強制（`language-dev-principles` §2）を崩してしまう。
/// ⇒ **中身だけ差し替える。** 評価コアでは `Infallible` なので**構築できない**＝
/// `Value::PyObject` を作る経路（`py_interop`）が `native` 限定であることと整合する。
#[cfg(feature = "native")]
pub struct PyObjHandle {
    /// 持っている参照（所有権つき）。読むときは `py_interop` の `bind` で借りる（数を変えない）。
    pub(crate) ptr: std::ptr::NonNull<pyo3::ffi::PyObject>,
}

// ⚠ CPython のオブジェクトはスレッドをまたいで持てる（`Py<T>` も `Send + Sync`）。増減は GIL の下か待ち行列で行う。
#[cfg(feature = "native")]
unsafe impl Send for PyObjHandle {}
#[cfg(feature = "native")]
unsafe impl Sync for PyObjHandle {}

/// 評価コアビルド（`native` 無効）の `PyObjHandle`。構築不能。
#[cfg(not(feature = "native"))]
#[derive(Clone)]
pub struct PyObjHandle {
    pub inner: std::convert::Infallible,
}

impl PyObjHandle {
    /// 指している CPython のオブジェクトの番地。等しさ（`values_eq`）とハッシュはこれで決める（同一性）。
    ///
    /// ⚠ 写しごとに包みが別物になった（D-5）ので、包みの番地ではなくオブジェクトの番地を見る。これで同じ物を 2 回
    ///   受け取っても同じ値になる（以前は `Arc` の番地で、受け取るたびに別のキーだった）。
    pub(crate) fn identity(&self) -> *const () {
        #[cfg(feature = "native")]
        {
            self.ptr.as_ptr() as *const ()
        }
        #[cfg(not(feature = "native"))]
        match self.inner {}
    }
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
    /// **今の値をモジュールの大域から読むメンバー**（フェーズ10 10-11）: `(大域の添字, 名前の集合)`。
    ///
    /// ⚠ `members` は import が終わった時点の**写し**。モジュールの関数はモジュールの大域を書き換える
    ///   （名前空間の分離・2026-09-26）ので、`mut count` を `bump()` で増やしても `c.count` は 0 のまま
    ///   だった（実測）。`mut` で宣言された名前だけここに載せ、読むときはモジュールの大域を引く
    ///   （`Interpreter::namespace_member`）。変わらない名前（関数・`let` / `const`）は写しのまま読む。
    /// ⚠ Arrow のモジュールと `import[py]` のモジュール（[`NsLive`]）以外（ネイティブ・組み込み・メタ）は `None`。
    pub live: Option<NsLive>,
}

/// 名前空間のメンバーのうち、**読むたびに元から引く**もの（[`NamespaceData::live`]）。
#[derive(Debug, Clone)]
pub enum NsLive {
    /// Arrow のモジュールの `mut` の名前（フェーズ10 10-11）: `(大域の添字, 名前の集合)`。
    Globals(u32, Rc<std::collections::HashSet<String>>),
    /// `import[py]` のモジュール（pandas_import_plan.md の 1-4）: CPython のモジュールそのもの。どのメンバーも読むたびに
    /// `getattr` して Arrow の値にする（`py_getattr`・インスタンスの属性と同じ）。
    /// ⚠ 以前は import の時点の写し（`members`）を Python のオブジェクトのまま返していた。`sys.version` が Arrow の `str`
    ///   にならず（`let v: str = sys.version` が実行時の `TypeError`）、モジュールの関数が書き換えた大域も古いままだった。
    // 評価コアでは作らない（`py_interop` が `native` 限定・`Value::PyObject` と同じ）
    #[cfg_attr(not(feature = "native"), allow(dead_code))]
    Python(PyObjHandle),
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
    /// 非同期タスクへ送った複製（7-19）。使うとエラー（`async_mgr::ensure_attached`）。
    pub detached: bool,
    /// Python の形（`open(path, "w")`）で開いたときのモードの文字列（python_builtins_plan.md のタスク 5-4）。
    ///
    /// `Some` のファイルだけ `read()` / `write()` を CPython の意味にする（終わりの `read()` は `""`・`read(n)`・
    /// `write` は書いた数を返す・バイト列は `bytes`・読めない／書けないモードは誤り）。Arrow の形
    /// （`open(path, FileOpenMode.read, ..)`）で開いたファイルは `None` で、従来どおり。
    pub py_mode: Option<String>,
}


impl FileData {
    /// 非同期タスクへ送る、切り離した複製（7-19）。中身もハンドルも持たない（閉じた扱い）ので、破棄しても
    /// ディスクへは書かない。⚠ 同じファイルを 2 つのバッファで持つと、閉じたときに後の方が前の方を上書きする。
    pub fn detached_copy(&self) -> FileData {
        FileData {
            path: self.path.clone(),
            mode: self.mode.clone(),
            byte_mode: self.byte_mode.clone(),
            content: Vec::new(),
            pointer: 0,
            is_closed: true,
            file_handle: None,
            detached: true,
            py_mode: self.py_mode.clone(),
        }
    }

    /// バッファをディスクに書き戻す（閉じない・`flush()`・タスク 5-4）。
    /// 書き込みモード (`write` / `rewrite` / `make_and_write`) のみ実際に書き戻す。
    pub fn flush_to_disk(&mut self) {
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
    }

    /// バッファをディスクに書き戻してファイルハンドルを閉じる。
    /// 書き込みモード (`write` / `rewrite` / `make_and_write`) のみ実際に書き戻す。
    /// 既に close 済みの場合は何もしない。
    pub fn close(&mut self) {
        if self.is_closed {
            return;
        }
        self.is_closed = true;
        self.flush_to_disk();
        self.file_handle = None;
    }

    /// 読めるか。Python の形は CPython と同じく `r` か `+`、Arrow の形はどのモードでも読める（従来どおり）。
    pub fn readable(&self) -> bool {
        match &self.py_mode {
            Some(m) => m.contains('r') || m.contains('+'),
            None => true,
        }
    }

    /// 書けるか。Python の形は `w` / `a` / `x` か `+`、Arrow の形は `read` 以外（従来どおり）。
    pub fn writable(&self) -> bool {
        match &self.py_mode {
            Some(m) => m.contains(['w', 'a', 'x', '+']),
            None => self.mode != FileOpenModeRust::Read,
        }
    }
}


impl Drop for FileData {
    /// `FileData` がスコープを抜けるときに自動的に `close()` を呼び出す。
    /// 書き込みモードの場合、バッファをディスクに書き戻してからハンドルを解放する。
    fn drop(&mut self) {
        self.close();
    }
}
