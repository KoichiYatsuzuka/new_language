use std::sync::Arc;

use crate::token::{Span, Spanned, Token};

/// ソーステキストの各文字インデックスに対応する `(行番号, 列番号)` を事前計算する。
///
/// 字句解析中に位置情報を高速に参照できるよう、解析開始前に全文字を走査して
/// テーブルを構築する。改行文字で行カウンタをインクリメントする。
///
/// # 引数
/// - `chars` — ソーステキストを文字単位に分割した配列
///
/// # 戻り値
/// `chars[i]` に対応する `(line, col)` を格納した `Vec`。
/// 最後の要素は EOF 位置（`chars.len()` 番目）を表す。
fn compute_positions(chars: &[char]) -> Vec<(usize, usize)> {
    let mut positions = Vec::with_capacity(chars.len() + 1);
    let mut line = 1usize;
    let mut col = 1usize;

    // 各文字の位置を記録し、改行で行・列カウンタを更新する
    for &c in chars {
        positions.push((line, col));
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }

    // EOF 位置を末尾に追加する
    positions.push((line, col));
    positions
}

/// 字句解析器（Lexer）。
///
/// ソーステキストを受け取り、`tokenize()` でトークン列（`Vec<Spanned>`）に変換する。
/// Python スタイルのインデントブロック（`INDENT` / `DEDENT` トークン）と
/// 括弧内での改行無視をサポートする。
///
/// # フィールド
/// - `chars`         — ソーステキストを文字単位に展開した配列
/// - `pos`           — 現在の読み取り位置（`chars` のインデックス）
/// - `positions`     — 各文字インデックスに対応する `(line, col)` テーブル
/// - `filename`      — エラーメッセージ・`Span` に埋め込むファイル名
/// - `indent_stack`  — インデントレベルのスタック。先頭は常に 0
/// - `pending`       — 先読みで生成した `INDENT` / `DEDENT` トークンのバッファ
/// - `at_line_start` — 次に読み取るのが行頭かどうかを示すフラグ
/// - `bracket_depth` — `(` / `[` / `{` の入れ子深さ。0 より大きい間は改行を無視する
pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    positions: Vec<(usize, usize)>,
    filename: Arc<str>,
    indent_stack: Vec<usize>,
    pending: Vec<Spanned>, // INDENT/DEDENT などのバッファ
    at_line_start: bool,
    bracket_depth: usize, // (), [], {} の深さ
}

impl Lexer {
    /// 新しい `Lexer` を生成する。
    ///
    /// ソーステキストと対応するファイル名を受け取り、解析に必要な状態を初期化する。
    /// `tokenize()` を呼び出す前に使用する。
    ///
    /// # 引数
    /// - `source`   — 字句解析するソーステキスト
    /// - `filename` — `Span` に埋め込むファイル名（`Arc<str>` に変換可能な型）
    ///
    /// # 戻り値
    /// 初期状態の `Lexer`
    pub fn new(source: &str, filename: impl Into<Arc<str>>) -> Self {
        // ソーステキストを文字配列に展開し、位置テーブルを構築する
        let chars: Vec<char> = source.chars().collect();
        let positions = compute_positions(&chars);
        Self {
            chars,
            positions,
            pos: 0,
            filename: filename.into(),
            indent_stack: vec![0], // インデントスタックは常に 0 から始める
            pending: Vec::new(),
            at_line_start: true,   // ファイル先頭は行頭扱い
            bracket_depth: 0,
        }
    }

    /// ソーステキスト全体をトークン列に変換して返す。
    ///
    /// `Token::Eof` が現れるまで `next_token()` を繰り返し呼び出す。
    /// 返すベクタの最後の要素は必ず `Token::Eof` になる。
    ///
    /// # 戻り値
    /// ソーステキスト全体に対応する `Vec<Spanned>`（末尾は `Token::Eof`）
    pub fn tokenize(&mut self) -> Vec<Spanned> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            let done = tok.token == Token::Eof;
            tokens.push(tok);
            // EOF を受け取ったらループを終了する
            if done {
                break;
            }
        }
        tokens
    }

    /// 次のトークンを 1 つ読み取って返す。
    ///
    /// 処理の優先順位は以下の通り:
    /// 1. `pending` バッファに先行生成済みトークンがあればそれを返す
    /// 2. 行頭かつ括弧の外であればインデント処理を行う
    /// 3. 空白をスキップしてから現在文字の種類に応じてトークンを生成する
    ///
    /// # 戻り値
    /// 次のトークンとその位置情報を含む `Spanned`
    pub fn next_token(&mut self) -> Spanned {
        // 先行生成済みトークン（INDENT/DEDENT など）があれば先に返す
        if !self.pending.is_empty() {
            return self.pending.remove(0);
        }

        // 行頭かつ括弧の外の場合はインデント処理を優先する
        if self.at_line_start && self.bracket_depth == 0 {
            return self.handle_indent();
        }

        // 行中の空白（スペース・タブ）を読み飛ばす
        self.skip_spaces();
        let start = self.pos;

        match self.ch() {
            // 入力終端に達したら EOF トークンを生成する
            None => self.emit_eof(),

            // 改行文字を処理する
            Some('\n') | Some('\r') => {
                self.consume_newline();
                if self.bracket_depth > 0 {
                    // 括弧内の改行は無視して次のトークンを読む
                    return self.next_token();
                }
                // 括弧の外の改行は論理的な行末（Newline トークン）を生成する
                self.at_line_start = true;
                self.spanned(Token::Newline, start)
            }

            // `#` で始まる行末コメントを読み飛ばして再帰する
            Some('#') => {
                self.skip_comment();
                self.next_token()
            }

            // 引用符で始まる文字列リテラルを解析する
            Some('"') | Some('\'') => {
                let tok = self.lex_string();
                self.spanned(tok, start)
            }

            // 数字で始まる数値リテラルを解析する
            Some(c) if c.is_ascii_digit() => {
                let tok = self.lex_number();
                self.spanned(tok, start)
            }

            // アルファベットまたは `_` で始まる識別子・キーワードを解析する
            Some(c) if c.is_alphabetic() || c == '_' => {
                let tok = self.lex_word();
                self.spanned(tok, start)
            }

            // その他の文字は記号トークンとして解析する
            Some(_) => {
                let tok = self.lex_symbol();
                self.spanned(tok, start)
            }
        }
    }

    // --- 位置情報ヘルパー ---

    /// 指定位置 `pos` に対応する `Span` を生成する。
    ///
    /// # 引数
    /// - `pos` — `chars` 配列のインデックス
    ///
    /// # 戻り値
    /// `pos` に対応するファイル名・行番号・列番号を持つ `Span`
    fn span_at(&self, pos: usize) -> Span {
        let (line, col) = self.positions.get(pos).copied().unwrap_or((1, 1));
        Span { file: self.filename.clone(), line, col }
    }

    /// トークンと開始位置から `Spanned` を生成する。
    ///
    /// # 引数
    /// - `token` — トークン種別と値
    /// - `start` — `chars` 配列上のトークン開始インデックス
    ///
    /// # 戻り値
    /// `token` と `start` の位置情報を持つ `Spanned`
    fn spanned(&self, token: Token, start: usize) -> Spanned {
        Spanned { token, span: self.span_at(start) }
    }

    // --- EOF 処理 ---

    /// EOF トークンを生成する。
    ///
    /// EOF に達した時点でインデントスタックに残っているレベルを
    /// すべて `DEDENT` トークンとして `pending` バッファに積む。
    /// これにより、ファイル末尾でのブロック閉じを確実に処理する。
    ///
    /// # 戻り値
    /// `pending` バッファが空になった後に `Token::Eof` を返す。
    /// バッファに残りがある場合は先に `DEDENT` を返す。
    fn emit_eof(&mut self) -> Spanned {
        let span = self.span_at(self.pos);

        // インデントスタックが残っていれば DEDENT を生成して pending に積む
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            self.pending.push(Spanned { token: Token::Dedent, span: span.clone() });
        }

        // pending があれば先に返す（再帰は使わず先頭を直接取り出す）
        if !self.pending.is_empty() {
            return self.pending.remove(0);
        }

        Spanned { token: Token::Eof, span }
    }

    // --- インデント処理 ---

    /// 行頭のインデントを処理して `INDENT` / `DEDENT` / 通常トークンを返す。
    ///
    /// 空行・コメント行はスキップして次の実コンテンツ行まで進む。
    /// 実コンテンツ行のインデントレベルを現在のスタックトップと比較し、
    /// レベルが増えれば `INDENT`、減れば必要な数の `DEDENT`、
    /// 変化なければ次のトークンを読む。
    ///
    /// # 戻り値
    /// `INDENT` / `DEDENT` / 次の通常トークン
    fn handle_indent(&mut self) -> Spanned {
        // 行頭フラグをリセットする（この関数内で処理を完結させる）
        self.at_line_start = false;
        loop {
            let (level, char_count) = self.measure_indent();
            let after = self.pos + char_count;

            match self.chars.get(after).copied() {
                // 空行はスキップして次の行へ進む
                Some('\n') | Some('\r') => {
                    self.pos = after;
                    self.consume_newline();
                    continue;
                }
                // コメント行はスキップして次の行へ進む
                Some('#') => {
                    self.pos = after;
                    self.skip_comment();
                    self.consume_newline();
                    continue;
                }
                // EOF に達したら EOF 処理に委譲する
                None => {
                    self.pos = after;
                    return self.emit_eof();
                }
                // 実コンテンツ行：インデントレベルに応じて INDENT / DEDENT を生成する
                _ => {
                    let current = *self.indent_stack.last().unwrap();
                    self.pos = after;
                    let span = self.span_at(after);
                    if level > current {
                        // インデント増加：スタックに積んで INDENT トークンを返す
                        self.indent_stack.push(level);
                        return Spanned { token: Token::Indent, span };
                    } else if level < current {
                        // インデント減少：スタックを巻き戻して必要な数の DEDENT を生成する
                        while *self.indent_stack.last().unwrap() > level {
                            self.indent_stack.pop();
                            self.pending.push(Spanned { token: Token::Dedent, span: span.clone() });
                        }
                        return self.pending.remove(0);
                    } else {
                        // インデントレベル変化なし：通常のトークン読み取りに進む
                        return self.next_token();
                    }
                }
            }
        }
    }

    /// 現在位置からスペース・タブを読み取り、インデントレベルと消費文字数を返す。
    ///
    /// タブは 8 の倍数へ切り上げて展開する（Python 互換）。
    /// `self.pos` は変更しない（計測のみ）。
    ///
    /// # 戻り値
    /// `(level, count)` のタプル:
    /// - `level` — 空白文字を展開した後のインデントレベル（列数相当）
    /// - `count` — 消費したソース文字数（`self.pos` へのオフセットに使う）
    fn measure_indent(&self) -> (usize, usize) {
        let mut level = 0usize;
        let mut count = 0usize;
        let mut i = self.pos;

        while i < self.chars.len() {
            match self.chars[i] {
                // スペース 1 文字 = レベル 1 増加
                ' ' => {
                    level += 1;
                    count += 1;
                }
                // タブは 8 の倍数に切り上げて展開する
                '\t' => {
                    level = (level / 8 + 1) * 8;
                    count += 1;
                }
                // 空白以外が現れたら計測終了
                _ => break,
            }
            i += 1;
        }
        (level, count)
    }

    /// 現在位置の改行（`\r\n` / `\n` / `\r`）を消費して `pos` を進める。
    fn consume_newline(&mut self) {
        // Windows 改行 `\r\n` は 2 文字まとめて消費する
        if self.ch() == Some('\r') && self.ch1() == Some('\n') {
            self.pos += 1;
        }
        if self.pos < self.chars.len() {
            self.pos += 1;
        }
    }

    /// 現在位置のスペース・タブを読み飛ばす（行中の空白スキップ専用）。
    ///
    /// 改行文字には触れない。インデント計測とは別に、トークン間の空白を
    /// スキップする目的で `next_token()` の冒頭で呼ばれる。
    fn skip_spaces(&mut self) {
        while matches!(self.ch(), Some(' ') | Some('\t')) {
            self.pos += 1;
        }
    }

    /// `#` から行末までをコメントとして読み飛ばす。
    ///
    /// 改行文字自体は消費しない（`consume_newline()` に委ねる）。
    fn skip_comment(&mut self) {
        while !matches!(self.ch(), None | Some('\n') | Some('\r')) {
            self.pos += 1;
        }
    }

    // --- 文字列リテラル ---

    /// 文字列リテラルを解析して `Token::Str` を返す。
    ///
    /// シングルクォート・ダブルクォートのどちらでも解析できる。
    /// 先頭の引用符が 3 つ並んでいる場合はトリプルクォート文字列として
    /// 複数行にまたがる内容を解析する。
    ///
    /// エスケープシーケンス: `\n`, `\t`, `\r`, `\\`, `\'`, `\"`, `\0` を認識する。
    /// 未知のエスケープは `\` を保持してそのまま追記する。
    ///
    /// # 戻り値
    /// エスケープ処理済みの文字列値を持つ `Token::Str(String)`
    fn lex_string(&mut self) -> Token {
        // 先頭の引用符文字を取得し、トリプルクォートか判定する
        let quote = self.bump().unwrap();
        let triple = self.ch() == Some(quote) && self.ch1() == Some(quote);
        if triple {
            // トリプルクォートの残り 2 文字分を読み飛ばす
            self.pos += 2;
        }
        let mut s = String::new();

        loop {
            match self.ch() {
                // EOF に達した場合はループを抜ける（未閉じ文字列）
                None => break,

                // バックスラッシュエスケープを処理する
                Some('\\') => {
                    self.pos += 1; // `\` を消費する
                    match self.bump() {
                        Some('n')  => s.push('\n'),
                        Some('t')  => s.push('\t'),
                        Some('r')  => s.push('\r'),
                        Some('\\') => s.push('\\'),
                        Some('\'') => s.push('\''),
                        Some('"')  => s.push('"'),
                        Some('0')  => s.push('\0'),
                        // 未知のエスケープシーケンスはそのまま保持する
                        Some(c) => {
                            s.push('\\');
                            s.push(c);
                        }
                        None => break,
                    }
                }

                // 引用符文字に達した場合：終端判定を行う
                Some(c) if c == quote => {
                    if triple {
                        // トリプルクォートの終端は引用符が 3 つ連続する場合のみ
                        if self.ch1() == Some(quote) && self.ch2() == Some(quote) {
                            self.pos += 3;
                            break;
                        } else {
                            // 引用符 1 つだけなら文字列の一部として追記する
                            s.push(c);
                            self.pos += 1;
                        }
                    } else {
                        // 通常クォートは 1 つの引用符で終端
                        self.pos += 1;
                        break;
                    }
                }

                // 通常の文字はそのまま文字列に追記する
                Some(c) => {
                    s.push(c);
                    self.pos += 1;
                }
            }
        }
        Token::Str(s)
    }

    // --- 数値リテラル ---

    /// 数値リテラルのトップレベルエントリ。
    ///
    /// 先頭が `0x` / `0o` / `0b` で始まる場合は `lex_radix_int()` に委譲し、
    /// それ以外は `lex_decimal_number()` で 10 進数・浮動小数点数として解析する。
    ///
    /// # 戻り値
    /// `Token::Int(i64)` または `Token::Float(f64)`
    fn lex_number(&mut self) -> Token {
        let start = self.pos;

        // 先頭が `0` の場合、次の文字でプレフィックスを判定する
        if self.ch() == Some('0') {
            match self.ch1() {
                // 16 進数: `0x` / `0X`
                Some('x') | Some('X') =>
                    return self.lex_radix_int(start, 16, |c| c.is_ascii_hexdigit()),
                // 8 進数: `0o` / `0O`
                Some('o') | Some('O') =>
                    return self.lex_radix_int(start, 8, |c| matches!(c, '0'..='7')),
                // 2 進数: `0b` / `0B`
                Some('b') | Some('B') =>
                    return self.lex_radix_int(start, 2, |c| matches!(c, '0' | '1')),
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
    /// - `start`    — トークン開始位置（`chars` のインデックス）
    /// - `base`     — 基数（2, 8, 16 のいずれか）
    /// - `is_digit` — 対象の基数で有効な桁文字を判定するクロージャ
    ///
    /// # 戻り値
    /// 解析した整数値を持つ `Token::Int(i64)`（パース失敗時は `0`）
    fn lex_radix_int<F>(&mut self, start: usize, base: u32, is_digit: F) -> Token
    where
        F: Fn(char) -> bool,
    {
        // `0x` / `0o` / `0b` の 2 文字プレフィックスを読み飛ばす
        self.pos += 2;

        // 有効な桁文字とアンダースコアを消費する
        while matches!(self.ch(), Some(c) if is_digit(c) || c == '_') {
            self.pos += 1;
        }

        // 生文字列を収集し、アンダースコアを除去してからパースする
        let raw: String = self.chars[start..self.pos].iter().collect();
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
    /// - `start` — トークン開始位置（`chars` のインデックス）
    ///
    /// # 戻り値
    /// - 浮動小数点部が含まれる場合: `Token::Float(f64)`（パース失敗時は `0.0`）
    /// - 整数のみの場合:             `Token::Int(i64)`（パース失敗時は `0`）
    fn lex_decimal_number(&mut self, start: usize) -> Token {
        // 整数部を消費する
        while matches!(self.ch(), Some(c) if c.is_ascii_digit() || c == '_') {
            self.pos += 1;
        }

        let mut is_float = false;

        // `.` に続けて数字がある場合は小数部として消費する
        if self.ch() == Some('.') && matches!(self.ch1(), Some(c) if c.is_ascii_digit()) {
            is_float = true;
            self.pos += 1; // `.` を消費する
            while matches!(self.ch(), Some(c) if c.is_ascii_digit() || c == '_') {
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
            while matches!(self.ch(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }

        // 収集した文字列からアンダースコアを除去してパースする
        let raw: String = self.chars[start..self.pos].iter().collect();
        let clean = raw.replace('_', "");
        if is_float {
            Token::Float(clean.parse().unwrap_or(0.0))
        } else {
            Token::Int(clean.parse().unwrap_or(0))
        }
    }

    // --- 識別子・キーワード ---

    /// 識別子またはキーワードを解析して対応するトークンを返す。
    ///
    /// アルファベット・数字・アンダースコアを消費し、得られた単語文字列を
    /// キーワードテーブルと照合する。`not` / `is` / `yield` などは
    /// `maybe_two_word()` で複合キーワードの可能性を先読みする。
    ///
    /// # 戻り値
    /// キーワードに対応する `Token`、または `Token::Ident(String)`
    fn lex_word(&mut self) -> Token {
        // 識別子に使用できる文字（英数字・アンダースコア）を消費する
        let start = self.pos;
        while matches!(self.ch(), Some(c) if c.is_alphanumeric() || c == '_') {
            self.pos += 1;
        }
        let word: String = self.chars[start..self.pos].iter().collect();

        // 単語をキーワードテーブルと照合する
        match word.as_str() {
            "let"          => Token::Let,
            "const"        => Token::Const,
            "mut"          => Token::Mut,
            "static"       => Token::Static,
            "freeze"       => Token::Freeze,
            "True"         => Token::True,
            "False"        => Token::False,
            "None"         => Token::None,
            "and"          => Token::And,
            "or"           => Token::Or,
            // `not` は `not in` の可能性があるため先読みする
            "not"          => self.maybe_two_word("in", Token::NotIn, Token::Not),
            "in"           => Token::In,
            // `is` は `is not` の可能性があるため先読みする
            "is"           => self.maybe_two_word("not", Token::IsNot, Token::Is),
            "if"           => Token::If,
            "elif"         => Token::Elif,
            "else"         => Token::Else,
            "match"        => Token::Match,
            "case"         => Token::Case,
            "for"          => Token::For,
            "while"        => Token::While,
            "break"        => Token::Break,
            "continue"     => Token::Continue,
            "pass"         => Token::Pass,
            "return"       => Token::Return,
            // `yield` は `yield from` の可能性があるため先読みする
            "yield"        => self.maybe_two_word("from", Token::YieldFrom, Token::Yield),
            "block_return" => Token::BlockReturn,
            "loop_yield"   => Token::LoopYield,
            "block"        => Token::Block,
            "try"          => Token::Try,
            "except"       => Token::Except,
            "finally"      => Token::Finally,
            "raise"        => Token::Raise,
            "fn"           => Token::Fn,
            "gen"          => Token::Gen,
            "class"        => Token::Class,
            "enum"         => Token::Enum,
            "trait"        => Token::Trait,
            "lambda"       => Token::Lambda,
            "template"     => Token::Template,
            "import"       => Token::Import,
            "from"         => Token::From,
            "as"           => Token::As,
            "del"          => Token::Del,
            "global"       => Token::Global,
            "nonlocal"     => Token::Nonlocal,
            "with"         => Token::With,
            "async"        => Token::Async,
            "await"        => Token::Await,
            "assert"       => Token::Assert,
            "public"       => Token::Public,
            "private"      => Token::Private,
            "protected"    => Token::Protected,
            "class_method" => Token::ClassMethod,
            "Self"         => Token::SelfType,
            "new_type"     => Token::NewType,
            "Any"          => Token::Any,
            "Union"        => Token::Union,
            "Option"       => Token::Option,
            // キーワードに一致しない場合は識別子トークンを返す
            _              => Token::Ident(word),
        }
    }

    /// 複合キーワード（2 単語からなるキーワード）を先読みして判定する。
    ///
    /// 現在位置の後に続く空白をスキップし、次の単語が `second` と一致すれば
    /// `combined` トークンを返す。一致しない場合は `self.pos` を元に戻して
    /// `single` トークンを返す。
    ///
    /// 対象の複合キーワード: `not in`, `is not`, `yield from`
    ///
    /// # 引数
    /// - `second`   — 期待する 2 番目の単語（例: `"in"`, `"not"`, `"from"`）
    /// - `combined` — 2 つの単語が一致した場合に返す複合トークン
    /// - `single`   — 2 番目の単語が一致しなかった場合に返す単独トークン
    ///
    /// # 戻り値
    /// `combined` または `single`
    fn maybe_two_word(&mut self, second: &str, combined: Token, single: Token) -> Token {
        // 巻き戻しに備えて現在位置を保存する
        let saved = self.pos;

        // 空白をスキップして次の単語の先頭へ移動する
        while matches!(self.ch(), Some(' ') | Some('\t')) {
            self.pos += 1;
        }

        // 次の単語を読み取る
        let word_start = self.pos;
        while matches!(self.ch(), Some(c) if c.is_alphanumeric() || c == '_') {
            self.pos += 1;
        }
        let word: String = self.chars[word_start..self.pos].iter().collect();

        if word == second {
            // 期待する単語と一致したので複合トークンを返す
            combined
        } else {
            // 一致しなかったので位置を元に戻して単独トークンを返す
            self.pos = saved;
            single
        }
    }

    // --- 記号 ---

    /// 記号文字から演算子・区切り記号トークンを生成する。
    ///
    /// 先頭文字を消費したうえで、複数文字からなるトークン（`==`, `->`, `**=` など）は
    /// 1 文字先読みをして確定させる。括弧文字では `bracket_depth` の更新も行う。
    ///
    /// # 戻り値
    /// 対応する `Token`。未知の文字の場合は `Token::Unknown(char)`
    fn lex_symbol(&mut self) -> Token {
        let c = self.bump().unwrap();
        match c {
            // 開き括弧：bracket_depth をインクリメントする
            '(' => {
                self.bracket_depth += 1;
                Token::LParen
            }
            // 閉じ括弧：bracket_depth が 0 を下回らないようにデクリメントする
            ')' => {
                if self.bracket_depth > 0 {
                    self.bracket_depth -= 1;
                }
                Token::RParen
            }
            '[' => {
                self.bracket_depth += 1;
                Token::LBracket
            }
            ']' => {
                if self.bracket_depth > 0 {
                    self.bracket_depth -= 1;
                }
                Token::RBracket
            }
            '{' => {
                self.bracket_depth += 1;
                Token::LBrace
            }
            '}' => {
                if self.bracket_depth > 0 {
                    self.bracket_depth -= 1;
                }
                Token::RBrace
            }

            // `+` または `+=`
            '+' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::PlusEq
                } else {
                    Token::Plus
                }
            }

            // `-` / `-=` / `->` の 3 種を判定する
            '-' => match self.ch() {
                Some('=') => {
                    self.pos += 1;
                    Token::MinusEq
                }
                Some('>') => {
                    self.pos += 1;
                    Token::Arrow
                }
                _ => Token::Minus,
            },

            // `*` / `*=` / `**` / `**=` の 4 種を判定する
            '*' => match self.ch() {
                Some('*') => {
                    self.pos += 1;
                    if self.ch() == Some('=') {
                        self.pos += 1;
                        Token::StarStarEq
                    } else {
                        Token::StarStar
                    }
                }
                Some('=') => {
                    self.pos += 1;
                    Token::StarEq
                }
                _ => Token::Star,
            },

            // `/` / `/=` / `//` / `//=` の 4 種を判定する
            '/' => match self.ch() {
                Some('/') => {
                    self.pos += 1;
                    if self.ch() == Some('=') {
                        self.pos += 1;
                        Token::SlashSlashEq
                    } else {
                        Token::SlashSlash
                    }
                }
                Some('=') => {
                    self.pos += 1;
                    Token::SlashEq
                }
                _ => Token::Slash,
            },

            // `%` または `%=`
            '%' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::PercentEq
                } else {
                    Token::Percent
                }
            }

            // `@` または `@=`
            '@' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::AtEq
                } else {
                    Token::At
                }
            }

            // `=` または `==`
            '=' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::EqEq
                } else {
                    Token::Eq
                }
            }

            // `!=`（`!` 単独は未知文字）
            '!' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::NotEq
                } else {
                    Token::Unknown('!')
                }
            }

            // `<` / `<=` / `<<` / `<<=` の 4 種を判定する
            '<' => match self.ch() {
                Some('<') => {
                    self.pos += 1;
                    if self.ch() == Some('=') {
                        self.pos += 1;
                        Token::LtLtEq
                    } else {
                        Token::LtLt
                    }
                }
                Some('=') => {
                    self.pos += 1;
                    Token::LtEq
                }
                _ => Token::Lt,
            },

            // `>` / `>=` / `>>` / `>>=` の 4 種を判定する
            '>' => match self.ch() {
                Some('>') => {
                    self.pos += 1;
                    if self.ch() == Some('=') {
                        self.pos += 1;
                        Token::GtGtEq
                    } else {
                        Token::GtGt
                    }
                }
                Some('=') => {
                    self.pos += 1;
                    Token::GtEq
                }
                _ => Token::Gt,
            },

            // `&` または `&=`
            '&' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::AmpEq
                } else {
                    Token::Amp
                }
            }

            // `|` または `|=`
            '|' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::PipeEq
                } else {
                    Token::Pipe
                }
            }

            // `^` または `^=`
            '^' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::CaretEq
                } else {
                    Token::Caret
                }
            }

            // `~`（単独のみ）
            '~' => Token::Tilde,

            // `:` / `::` / `:=` の 3 種を判定する
            ':' => {
                if self.ch() == Some(':') {
                    self.pos += 1;
                    Token::ColonColon
                } else if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::ColonEq
                } else {
                    Token::Colon
                }
            }

            ',' => Token::Comma,
            ';' => Token::Semicolon,

            // `.` または `...`（`..` は存在しない）
            '.' => {
                if self.ch() == Some('.') && self.ch1() == Some('.') {
                    self.pos += 2;
                    Token::Ellipsis
                } else {
                    Token::Dot
                }
            }

            // 上記以外はすべて未知文字として返す
            c => Token::Unknown(c),
        }
    }

    // --- 文字アクセス ---

    /// 現在位置の文字を返す（消費しない）。
    ///
    /// # 戻り値
    /// `Some(char)` または入力終端の場合 `None`
    fn ch(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// 現在位置の 1 つ先の文字を返す（消費しない）。
    ///
    /// # 戻り値
    /// `Some(char)` または範囲外の場合 `None`
    fn ch1(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    /// 現在位置の 2 つ先の文字を返す（消費しない）。
    ///
    /// トリプルクォート終端の判定などで使用する。
    ///
    /// # 戻り値
    /// `Some(char)` または範囲外の場合 `None`
    fn ch2(&self) -> Option<char> {
        self.chars.get(self.pos + 2).copied()
    }

    /// 現在位置の文字を返しつつ `pos` を 1 進める。
    ///
    /// # 戻り値
    /// `Some(char)` または入力終端の場合 `None`（`pos` は変化しない）
    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::Token;

    /// テスト用ヘルパー: ソース文字列を字句解析してトークン種別のみを返す。
    fn lex(s: &str) -> Vec<Token> {
        Lexer::new(s, "").tokenize().into_iter().map(|s| s.token).collect()
    }

    #[test]
    fn test_variable_keywords() {
        assert_eq!(lex("let const mut freeze"), vec![
            Token::Let, Token::Const, Token::Mut, Token::Freeze, Token::Eof,
        ]);
    }

    #[test]
    fn test_value_literals() {
        assert_eq!(lex("True False None"), vec![
            Token::True, Token::False, Token::None, Token::Eof,
        ]);
    }

    #[test]
    fn test_two_word_keywords() {
        assert_eq!(lex("not in"), vec![Token::NotIn, Token::Eof]);
        assert_eq!(lex("is not"), vec![Token::IsNot, Token::Eof]);
        assert_eq!(lex("yield from"), vec![Token::YieldFrom, Token::Eof]);
    }

    #[test]
    fn test_not_followed_by_other_word() {
        let tokens = lex("not insert");
        assert_eq!(tokens[0], Token::Not);
        assert_eq!(tokens[1], Token::Ident("insert".to_string()));
    }

    #[test]
    fn test_arithmetic_operators() {
        assert_eq!(lex("+ - * / // % ** @"), vec![
            Token::Plus, Token::Minus, Token::Star, Token::Slash,
            Token::SlashSlash, Token::Percent, Token::StarStar, Token::At,
            Token::Eof,
        ]);
    }

    #[test]
    fn test_compound_assignment() {
        assert_eq!(lex("+= -= *= /= //= %= **= @="), vec![
            Token::PlusEq, Token::MinusEq, Token::StarEq, Token::SlashEq,
            Token::SlashSlashEq, Token::PercentEq, Token::StarStarEq, Token::AtEq,
            Token::Eof,
        ]);
    }

    #[test]
    fn test_comparison_operators() {
        assert_eq!(lex("== != < > <= >="), vec![
            Token::EqEq, Token::NotEq, Token::Lt, Token::Gt,
            Token::LtEq, Token::GtEq,
            Token::Eof,
        ]);
    }

    #[test]
    fn test_bitwise_operators() {
        assert_eq!(lex("& | ^ ~ << >>"), vec![
            Token::Amp, Token::Pipe, Token::Caret, Token::Tilde,
            Token::LtLt, Token::GtGt,
            Token::Eof,
        ]);
    }

    #[test]
    fn test_shift_assign() {
        assert_eq!(lex("<<= >>="), vec![Token::LtLtEq, Token::GtGtEq, Token::Eof]);
    }

    #[test]
    fn test_integer_literals() {
        let t = lex("42 0 1_000_000 0xFF 0o17 0b1010");
        assert_eq!(t[0], Token::Int(42));
        assert_eq!(t[1], Token::Int(0));
        assert_eq!(t[2], Token::Int(1_000_000));
        assert_eq!(t[3], Token::Int(255));
        assert_eq!(t[4], Token::Int(15));
        assert_eq!(t[5], Token::Int(10));
    }

    #[test]
    fn test_float_literals() {
        let t = lex("3.14 1.0e10 2.5E-3");
        assert_eq!(t[0], Token::Float(3.14));
        assert_eq!(t[1], Token::Float(1.0e10));
        assert_eq!(t[2], Token::Float(2.5e-3));
    }

    #[test]
    fn test_string_literals() {
        let t = lex(r#""hello" 'world'"#);
        assert_eq!(t[0], Token::Str("hello".to_string()));
        assert_eq!(t[1], Token::Str("world".to_string()));
    }

    #[test]
    fn test_string_escape() {
        let t = lex(r#""\n\t\\""#);
        assert_eq!(t[0], Token::Str("\n\t\\".to_string()));
    }

    #[test]
    fn test_triple_quoted_string() {
        let t = lex(r#""""hello world""""#);
        assert_eq!(t[0], Token::Str("hello world".to_string()));
    }

    #[test]
    fn test_indentation() {
        let src = "if True:\n    pass\n";
        let t = lex(src);
        assert!(t.contains(&Token::If));
        assert!(t.contains(&Token::Indent));
        assert!(t.contains(&Token::Pass));
        assert!(t.contains(&Token::Dedent));
    }

    #[test]
    fn test_nested_indentation() {
        let src = "if True:\n    if False:\n        pass\nx\n";
        let t = lex(src);
        let dedent_count = t.iter().filter(|tok| **tok == Token::Dedent).count();
        assert_eq!(dedent_count, 2);
    }

    #[test]
    fn test_blank_lines_skipped() {
        let src = "x\n\ny\n";
        let t = lex(src);
        assert_eq!(t[0], Token::Ident("x".to_string()));
        assert_eq!(t[1], Token::Newline);
        assert_eq!(t[2], Token::Ident("y".to_string()));
    }

    #[test]
    fn test_comment_skipped() {
        let src = "x # comment\ny\n";
        let t = lex(src);
        assert_eq!(t[0], Token::Ident("x".to_string()));
        assert_eq!(t[1], Token::Newline);
        assert_eq!(t[2], Token::Ident("y".to_string()));
    }

    #[test]
    fn test_newline_inside_brackets_ignored() {
        let src = "(\n    1,\n    2\n)\n";
        let t = lex(src);
        assert!(!t.contains(&Token::Indent));
        assert!(!t.contains(&Token::Dedent));
        let newline_count = t.iter().filter(|tok| **tok == Token::Newline).count();
        assert_eq!(newline_count, 1);
    }

    #[test]
    fn test_arrow_ellipsis_walrus() {
        assert_eq!(lex("-> ... :="), vec![
            Token::Arrow, Token::Ellipsis, Token::ColonEq, Token::Eof,
        ]);
    }

    #[test]
    fn test_delimiters() {
        assert_eq!(lex("()[]{}"), vec![
            Token::LParen, Token::RParen,
            Token::LBracket, Token::RBracket,
            Token::LBrace, Token::RBrace,
            Token::Eof,
        ]);
    }

    #[test]
    fn test_span_line_col() {
        let src = "let x = 1\nmut y = 2\n";
        let spanned = Lexer::new(src, "test.tl").tokenize();
        // `let` は行1・列1
        assert_eq!(spanned[0].span.line, 1);
        assert_eq!(spanned[0].span.col, 1);
        // `mut` は行2・列1
        let mut_tok = spanned.iter().find(|s| s.token == Token::Mut).unwrap();
        assert_eq!(mut_tok.span.line, 2);
        assert_eq!(mut_tok.span.col, 1);
    }

    #[test]
    fn test_span_filename() {
        let spanned = Lexer::new("x\n", "foo.tl").tokenize();
        assert_eq!(&*spanned[0].span.file, "foo.tl");
    }

    // --- trait / :: ---

    #[test]
    fn test_trait_keyword() {
        assert_eq!(lex("trait"), vec![Token::Trait, Token::Eof]);
    }

    #[test]
    fn test_colon_colon_token() {
        assert_eq!(lex("::"), vec![Token::ColonColon, Token::Eof]);
    }

    #[test]
    fn test_colon_vs_colon_colon_vs_colon_eq() {
        assert_eq!(lex(": :: :="), vec![
            Token::Colon, Token::ColonColon, Token::ColonEq, Token::Eof,
        ]);
    }

    #[test]
    fn test_trait_access_syntax_tokens() {
        // self::MyTrait.field — 括弧なし形式
        let tokens = lex("self::MyTrait.field");
        assert_eq!(tokens[0], Token::Ident("self".to_string()));
        assert_eq!(tokens[1], Token::ColonColon);
        assert_eq!(tokens[2], Token::Ident("MyTrait".to_string()));
        assert_eq!(tokens[3], Token::Dot);
        assert_eq!(tokens[4], Token::Ident("field".to_string()));
    }
}
