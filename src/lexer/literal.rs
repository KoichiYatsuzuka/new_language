/// 文字列リテラル・数値リテラルの字句解析メソッド群。
///
/// `Lexer` の実装を機能単位に分割した補助ファイル。
/// すべてのメソッドは `scan.rs` で定義された `Lexer` に対する `impl` ブロックとして提供する。
use crate::token::{FStrPart, Token};

use super::scan::Lexer;

impl Lexer {
    // --- 文字列リテラル ---

    /// 文字列の内容を読み取って `String` として返す共通ルーティン。
    ///
    /// `raw=true` のときはバックスラッシュエスケープを処理せずそのまま保持する。
    /// 呼び出し前に文字列プレフィックス（`r`, `f`, `m` など）は消費済みであること。
    ///
    /// # 引数
    /// - `raw` — `true` のときエスケープ処理をスキップする（raw 文字列モード）
    ///
    /// # 戻り値
    /// 解析した文字列の内容
    pub(super) fn lex_string_inner(&mut self, raw: bool) -> String {
        let quote = self.bump().unwrap();
        let triple = self.ch() == Some(quote) && self.ch1() == Some(quote);
        if triple {
            self.pos += 2;
        }
        let mut s = String::new();
        loop {
            match self.ch() {
                None => break,
                Some('\\') if !raw => {
                    self.pos += 1;
                    match self.bump() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some('\\') => s.push('\\'),
                        Some('\'') => s.push('\''),
                        Some('"') => s.push('"'),
                        Some('0') => s.push('\0'),
                        Some(ch) => {
                            s.push('\\');
                            s.push(ch);
                        }
                        None => break,
                    }
                }
                Some('\\') => {
                    // raw モード: バックスラッシュをそのまま保持する
                    s.push('\\');
                    self.pos += 1;
                    if let Some(ch) = self.ch() {
                        s.push(ch);
                        self.pos += 1;
                    }
                }
                Some(ch) if ch == quote => {
                    if triple {
                        if self.ch1() == Some(quote) && self.ch2() == Some(quote) {
                            self.pos += 3;
                            break;
                        } else {
                            s.push(ch);
                            self.pos += 1;
                        }
                    } else {
                        self.pos += 1;
                        break;
                    }
                }
                Some(ch) => {
                    s.push(ch);
                    self.pos += 1;
                }
            }
        }
        s
    }

    /// 通常の文字列リテラル（プレフィックスなし）を解析して `Token::Str` を返す。
    pub(super) fn lex_string(&mut self) -> Token {
        Token::Str(self.lex_string_inner(false))
    }

    /// f-string を解析して `Token::FStr(Vec<FStrPart>)` を返す。
    ///
    /// `{expr}` をリテラル部分と式部分に分割する。
    /// `{{` / `}}` はエスケープされた `{` / `}` として処理する。
    ///
    /// # 引数
    /// - `raw` — `true` のときエスケープ処理をスキップする（`fr""` / `rf""` 用）
    ///
    /// # 戻り値
    /// `Token::FStr(Vec<FStrPart>)`
    pub(super) fn lex_fstring(&mut self, raw: bool) -> Token {
        let quote = self.bump().unwrap();
        let triple = self.ch() == Some(quote) && self.ch1() == Some(quote);
        if triple {
            self.pos += 2;
        }
        let mut parts: Vec<FStrPart> = Vec::new();
        let mut lit = String::new();
        loop {
            match self.ch() {
                None => break,
                Some('\\') if !raw => {
                    self.pos += 1;
                    match self.bump() {
                        Some('n') => lit.push('\n'),
                        Some('t') => lit.push('\t'),
                        Some('r') => lit.push('\r'),
                        Some('\\') => lit.push('\\'),
                        Some('\'') => lit.push('\''),
                        Some('"') => lit.push('"'),
                        Some('0') => lit.push('\0'),
                        Some('{') => lit.push('{'),
                        Some('}') => lit.push('}'),
                        Some(ch) => {
                            lit.push('\\');
                            lit.push(ch);
                        }
                        None => break,
                    }
                }
                Some('\\') => {
                    lit.push('\\');
                    self.pos += 1;
                    if let Some(ch) = self.ch() {
                        lit.push(ch);
                        self.pos += 1;
                    }
                }
                Some('{') if self.ch1() == Some('{') => {
                    lit.push('{');
                    self.pos += 2;
                }
                Some('{') => {
                    if !lit.is_empty() {
                        parts.push(FStrPart::Lit(std::mem::take(&mut lit)));
                    }
                    self.pos += 1; // consume {
                    let mut expr_src = String::new();
                    let mut depth = 1usize;
                    while let Some(ch) = self.ch() {
                        match ch {
                            '{' => {
                                depth += 1;
                                expr_src.push(ch);
                                self.pos += 1;
                            }
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    self.pos += 1;
                                    break;
                                }
                                expr_src.push(ch);
                                self.pos += 1;
                            }
                            _ => {
                                expr_src.push(ch);
                                self.pos += 1;
                            }
                        }
                    }
                    parts.push(FStrPart::Expr(expr_src));
                }
                Some('}') if self.ch1() == Some('}') => {
                    lit.push('}');
                    self.pos += 2;
                }
                Some(ch) if ch == quote => {
                    if triple {
                        if self.ch1() == Some(quote) && self.ch2() == Some(quote) {
                            self.pos += 3;
                            break;
                        } else {
                            lit.push(ch);
                            self.pos += 1;
                        }
                    } else {
                        self.pos += 1;
                        break;
                    }
                }
                Some(ch) => {
                    lit.push(ch);
                    self.pos += 1;
                }
            }
        }
        if !lit.is_empty() {
            parts.push(FStrPart::Lit(lit));
        }
        Token::FStr(parts)
    }

    // --- 数値リテラル ---

    /// 数値リテラルのトップレベルエントリ。
    ///
    /// 先頭が `0x` / `0o` / `0b` で始まる場合は `lex_radix_int()` に委譲し、
    /// それ以外は `lex_decimal_number()` で 10 進数・浮動小数点数として解析する。
    ///
    /// # 戻り値
    /// `Token::Int(i64)` または `Token::Float(f64)`
    pub(super) fn lex_number(&mut self) -> Token {
        let start = self.pos;

        // 先頭が `0` の場合、次の文字でプレフィックスを判定する
        if self.ch() == Some('0') {
            match self.ch1() {
                // 16 進数: `0x` / `0X`
                Some('x') | Some('X') => {
                    return self.lex_radix_int(start, 16, |ch| ch.is_ascii_hexdigit())
                }
                // 8 進数: `0o` / `0O`
                Some('o') | Some('O') => {
                    return self.lex_radix_int(start, 8, |ch| matches!(ch, '0'..='7'))
                }
                // 2 進数: `0b` / `0B`
                Some('b') | Some('B') => {
                    return self.lex_radix_int(start, 2, |ch| matches!(ch, '0' | '1'))
                }
                _ => {}
            }
        }

        // プレフィックスなしの場合は 10 進数・浮動小数点として解析する
        self.lex_decimal_number(start)
    }

    /// プレフィックス付き整数リテラル（16 進 / 8 進 / 2 進）を解析する。
    ///
    /// `self.pos` は先頭の `0` を指している前提で呼ばれる。
    /// `0x` / `0o` / `0b` の 2 文字プレフィックスを自動的にスキップする。
    /// アンダースコア区切り（例: `0xFF_FF`）をサポートする。
    ///
    /// # 引数
    /// - `token_start` — トークン開始位置（`chars` のインデックス）
    /// - `base`        — 基数（2, 8, 16 のいずれか）
    /// - `is_digit`    — 対象の基数で有効な桁文字を判定するクロージャ
    ///
    /// # 戻り値
    /// 解析した整数値を持つ `Token::Int(i64)`（パース失敗時は `0`）
    fn lex_radix_int<F>(&mut self, token_start: usize, base: u32, is_digit: F) -> Token
    where
        F: Fn(char) -> bool,
    {
        // `0x` / `0o` / `0b` の 2 文字プレフィックスを読み飛ばす
        self.pos += 2;

        // 有効な桁文字とアンダースコアを消費する
        while matches!(self.ch(), Some(ch) if is_digit(ch) || ch == '_') {
            self.pos += 1;
        }

        // 生文字列を収集し、アンダースコアを除去してからパースする
        let raw: String = self.chars[token_start..self.pos].iter().collect();
        let clean = raw.replace('_', "");
        // clean[2..] でプレフィックス部分を除いた数字列を基数でパースする
        Token::Int(i64::from_str_radix(&clean[2..], base).unwrap_or(0))
    }

    /// 10 進整数または浮動小数点リテラルを解析する。
    ///
    /// 整数部のあとに `.` + 小数部、または `e` / `E` + 指数部が続く場合は
    /// 浮動小数点として扱う。アンダースコア区切りをサポートする。
    ///
    /// # 引数
    /// - `token_start` — トークン開始位置（`chars` のインデックス）
    ///
    /// # 戻り値
    /// - 浮動小数点部が含まれる場合: `Token::Float(f64)`（パース失敗時は `0.0`）
    /// - 整数のみの場合:             `Token::Int(i64)`（パース失敗時は `0`）
    fn lex_decimal_number(&mut self, token_start: usize) -> Token {
        // 整数部を消費する
        while matches!(self.ch(), Some(ch) if ch.is_ascii_digit() || ch == '_') {
            self.pos += 1;
        }

        let mut is_float = false;

        // `.` に続けて数字がある場合は小数部として消費する
        if self.ch() == Some('.') && matches!(self.ch1(), Some(ch) if ch.is_ascii_digit()) {
            is_float = true;
            self.pos += 1; // `.` を消費する
            while matches!(self.ch(), Some(ch) if ch.is_ascii_digit() || ch == '_') {
                self.pos += 1;
            }
        }

        // `e` / `E` による指数部を処理する
        if matches!(self.ch(), Some('e') | Some('E')) {
            is_float = true;
            self.pos += 1; // `e` / `E` を消費する
                           // 符号（`+` / `-`）が続く場合は消費する
            if matches!(self.ch(), Some('+') | Some('-')) {
                self.pos += 1;
            }
            // 指数部の数字を消費する
            while matches!(self.ch(), Some(ch) if ch.is_ascii_digit()) {
                self.pos += 1;
            }
        }

        // 収集した文字列からアンダースコアを除去してパースする
        let raw: String = self.chars[token_start..self.pos].iter().collect();
        let clean = raw.replace('_', "");
        if is_float {
            Token::Float(clean.parse().unwrap_or(0.0))
        } else {
            Token::Int(clean.parse().unwrap_or(0))
        }
    }
}
