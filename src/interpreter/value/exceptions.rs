// value/exceptions.rs — 例外トレースバック型: StackFrame / RaisedError。

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
// Exception / traceback types
// ---------------------------------------------------------------------------

/// エラーのトレースバックにおける1つのスタックフレーム（コールスタックの1階層）。
///
/// - `file`: ソースファイル名
/// - `line`: 行番号（1始まり）
/// - `col`: 列番号（1始まり）
/// - `fn_name`: raise または伝播が発生した関数名（`<module>` はトップレベル）
/// - `context`: `line` を中心とした最大5行のソースコンテキスト文字列。取得不可能な場合は空文字列
#[derive(Debug, Clone)]
pub struct StackFrame {
    pub file: String,
    pub line: usize,
    pub col: usize,
    /// raise（または伝播）が発生した関数名。トップレベルは `<module>`。
    pub fn_name: String,
    /// `line` を中心とした最大5行のソースコンテキスト。取得不可能な場合は空文字列。
    pub context: String,
}


/// コールスタックを遡って伝播中の言語レベル例外。
///
/// - `exception`: 例外インスタンス。ユーザー raise では常に `Value::Instance`
/// - `frames`: 例外が伝播するにつれて収集されたスタックフレームのリスト。
///   インデックス 0 が raise 発生地点（最内部）、末尾が `<module>` 到達直前の最外部フレーム。
#[derive(Debug, Clone)]
pub struct RaisedError {
    /// 例外インスタンス（ユーザー raise では常に `Value::Instance`）。
    pub exception: Value,
    /// 例外伝播中に収集されたフレーム: インデックス 0 = raise 地点（最内部）、
    /// 末尾 = `<module>` に到達する直前の最外部フレーム。
    pub frames: Vec<StackFrame>,
}
