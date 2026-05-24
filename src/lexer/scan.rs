use std::sync::Arc;

use crate::token::{Span, Spanned, Token};

use super::math::render_math_str;

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
    for &ch in chars {
        positions.push((line, col));
        if ch == '\n' {
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
    pub(super) chars: Vec<char>,
    pub(super) pos: usize,
    positions: Vec<(usize, usize)>,
    filename: Arc<str>,
    indent_stack: Vec<usize>,
    pending: Vec<Spanned>, // INDENT/DEDENT などのバッファ
    at_line_start: bool,
    pub(super) bracket_depth: usize, // (), [], {} の深さ
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

            // f"" / r"" / m"" / b"" 単一プレフィックス文字列
            Some(ch) if (ch == 'f' || ch == 'r' || ch == 'm' || ch == 'b')
                && matches!(self.ch1(), Some('"') | Some('\'')) => {
                self.pos += 1;
                let tok = match ch {
                    'f' => self.lex_fstring(false),
                    'r' => Token::Str(self.lex_string_inner(true)),
                    // Math strings use raw mode so \alpha, \times etc. reach render_math_str intact
                    'm' => Token::Str(render_math_str(&self.lex_string_inner(true))),
                    _ => Token::Str(self.lex_string_inner(false)),
                };
                self.spanned(tok, start)
            }

            // fr"" / rf"" 二重プレフィックス（raw f-string）
            Some(ch) if (ch == 'f' || ch == 'r')
                && (self.ch1() == Some('f') || self.ch1() == Some('r'))
                && matches!(self.ch2(), Some('"') | Some('\'')) => {
                self.pos += 2;
                let tok = self.lex_fstring(true);
                self.spanned(tok, start)
            }

            // $...$ 数学文字列 (LaTeX インライン数式スタイル)
            Some('$') => {
                self.pos += 1;
                let mut s = String::new();
                while let Some(ch) = self.ch() {
                    if ch == '$' { self.pos += 1; break; }
                    s.push(ch);
                    self.pos += 1;
                }
                self.spanned(Token::Str(render_math_str(&s)), start)
            }

            // 数字で始まる数値リテラルを解析する
            Some(ch) if ch.is_ascii_digit() => {
                let tok = self.lex_number();
                self.spanned(tok, start)
            }

            // アルファベットまたは `_` で始まる識別子・キーワードを解析する
            Some(ch) if ch.is_alphabetic() || ch == '_' => {
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
    /// - `char_pos` — `chars` 配列のインデックス
    ///
    /// # 戻り値
    /// `char_pos` に対応するファイル名・行番号・列番号を持つ `Span`
    fn span_at(&self, char_pos: usize) -> Span {
        let (line, col) = self.positions.get(char_pos).copied().unwrap_or((1, 1));
        Span { file: self.filename.clone(), line, col }
    }

    /// トークンと開始位置から `Spanned` を生成する。
    ///
    /// # 引数
    /// - `token`     — トークン種別と値
    /// - `start_pos` — `chars` 配列上のトークン開始インデックス
    ///
    /// # 戻り値
    /// `token` と `start_pos` の位置情報を持つ `Spanned`
    fn spanned(&self, token: Token, start_pos: usize) -> Spanned {
        Spanned { token, span: self.span_at(start_pos) }
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

}
