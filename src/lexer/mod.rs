/// 字句解析器モジュール。
///
/// - `math`  — LaTeX-like 数式表記を Unicode に変換するユーティリティ
/// - `scan`  — `Lexer` 本体（インデント処理・トークン生成）
mod chars;
mod keyword;
mod literal;
mod math;
mod scan;
mod symbol;

pub use scan::Lexer;
