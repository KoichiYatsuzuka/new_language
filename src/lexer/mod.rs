/// 字句解析器モジュール。
///
/// ソーステキストをトークン列（`Vec<Spanned>`）に変換する `Lexer` と、
/// それを支援するサブモジュール群で構成される。
///
/// ## サブモジュール
/// - `scan`    — `Lexer` 本体（インデント処理・トークン生成ループ）
/// - `chars`   — 現在位置の文字参照・消費ヘルパー（`ch()` / `ch1()` / `bump()` 等）
/// - `keyword` — 識別子・キーワード・複合キーワードの解析
/// - `literal` — 文字列・数値リテラルの解析（f-string・raw 文字列・基数表現を含む）
/// - `symbol`  — 演算子・区切り記号トークンの解析
/// - `math`    — `m"..."` / `$...$` 数学文字列の LaTeX-like 表記を Unicode に変換するユーティリティ
mod chars;
mod keyword;
mod literal;
mod math;
mod scan;
mod symbol;

pub use scan::Lexer;
