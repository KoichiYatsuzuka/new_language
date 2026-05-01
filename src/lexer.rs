use crate::token::Token;

pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    // Stack of indentation levels (in spaces; tab = 8 spaces)
    indent_stack: Vec<usize>,
    // Tokens buffered for emission (e.g. multiple DEDENTs)
    pending: Vec<Token>,
    at_line_start: bool,
    // Inside (), [], {} newlines are insignificant
    bracket_depth: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            indent_stack: vec![0],
            pending: Vec::new(),
            at_line_start: true,
            bracket_depth: 0,
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token();
            let done = tok == Token::Eof;
            tokens.push(tok);
            if done {
                break;
            }
        }
        tokens
    }

    pub fn next_token(&mut self) -> Token {
        if !self.pending.is_empty() {
            return self.pending.remove(0);
        }

        if self.at_line_start && self.bracket_depth == 0 {
            return self.handle_indent();
        }

        self.skip_spaces();

        match self.ch() {
            None => self.emit_eof(),
            Some('\n') | Some('\r') => self.lex_newline(),
            Some('#') => {
                self.skip_comment();
                self.next_token()
            }
            Some('"') | Some('\'') => self.lex_string(),
            Some(c) if c.is_ascii_digit() => self.lex_number(),
            Some(c) if c.is_alphabetic() || c == '_' => self.lex_word(),
            Some(_) => self.lex_symbol(),
        }
    }

    fn emit_eof(&mut self) -> Token {
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            self.pending.push(Token::Dedent);
        }
        if !self.pending.is_empty() {
            return self.pending.remove(0);
        }
        Token::Eof
    }

    fn handle_indent(&mut self) -> Token {
        self.at_line_start = false;
        loop {
            let (level, char_count) = self.measure_indent();
            let after = self.pos + char_count;

            match self.chars.get(after).copied() {
                // Blank line — skip silently
                Some('\n') | Some('\r') => {
                    self.pos = after;
                    self.consume_newline();
                    continue;
                }
                // Comment-only line — skip silently
                Some('#') => {
                    self.pos = after;
                    self.skip_comment();
                    self.consume_newline();
                    continue;
                }
                // EOF while handling indentation
                None => {
                    self.pos = after;
                    return self.emit_eof();
                }
                // Real content
                _ => {
                    let current = *self.indent_stack.last().unwrap();
                    self.pos = after;
                    if level > current {
                        self.indent_stack.push(level);
                        return Token::Indent;
                    } else if level < current {
                        while *self.indent_stack.last().unwrap() > level {
                            self.indent_stack.pop();
                            self.pending.push(Token::Dedent);
                        }
                        return self.pending.remove(0);
                    } else {
                        return self.next_token();
                    }
                }
            }
        }
    }

    // Returns (logical indent level, number of chars consumed).
    fn measure_indent(&self) -> (usize, usize) {
        let mut level = 0usize;
        let mut count = 0usize;
        let mut i = self.pos;
        while i < self.chars.len() {
            match self.chars[i] {
                ' ' => {
                    level += 1;
                    count += 1;
                }
                '\t' => {
                    level = (level / 8 + 1) * 8;
                    count += 1;
                }
                _ => break,
            }
            i += 1;
        }
        (level, count)
    }

    fn consume_newline(&mut self) {
        if self.ch() == Some('\r') && self.ch1() == Some('\n') {
            self.pos += 1;
        }
        if self.pos < self.chars.len() {
            self.pos += 1;
        }
    }

    fn skip_spaces(&mut self) {
        while matches!(self.ch(), Some(' ') | Some('\t')) {
            self.pos += 1;
        }
    }

    fn skip_comment(&mut self) {
        while !matches!(self.ch(), None | Some('\n') | Some('\r')) {
            self.pos += 1;
        }
    }

    fn lex_newline(&mut self) -> Token {
        self.consume_newline();
        if self.bracket_depth > 0 {
            return self.next_token();
        }
        self.at_line_start = true;
        Token::Newline
    }

    fn lex_string(&mut self) -> Token {
        let quote = self.bump().unwrap();
        let triple = self.ch() == Some(quote) && self.ch1() == Some(quote);
        if triple {
            self.pos += 2;
        }
        let mut s = String::new();
        loop {
            match self.ch() {
                None => break,
                Some('\\') => {
                    self.pos += 1;
                    match self.bump() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some('\\') => s.push('\\'),
                        Some('\'') => s.push('\''),
                        Some('"') => s.push('"'),
                        Some('0') => s.push('\0'),
                        Some(c) => {
                            s.push('\\');
                            s.push(c);
                        }
                        None => break,
                    }
                }
                Some(c) if c == quote => {
                    if triple {
                        if self.ch1() == Some(quote) && self.ch2() == Some(quote) {
                            self.pos += 3;
                            break;
                        } else {
                            s.push(c);
                            self.pos += 1;
                        }
                    } else {
                        self.pos += 1;
                        break;
                    }
                }
                Some(c) => {
                    s.push(c);
                    self.pos += 1;
                }
            }
        }
        Token::Str(s)
    }

    fn lex_number(&mut self) -> Token {
        let start = self.pos;

        // Hex / octal / binary prefix
        if self.ch() == Some('0') {
            match self.ch1() {
                Some('x') | Some('X') => {
                    self.pos += 2;
                    while matches!(self.ch(), Some(c) if c.is_ascii_hexdigit() || c == '_') {
                        self.pos += 1;
                    }
                    let raw: String = self.chars[start..self.pos].iter().collect();
                    let clean = raw.replace('_', "");
                    return Token::Int(i64::from_str_radix(&clean[2..], 16).unwrap_or(0));
                }
                Some('o') | Some('O') => {
                    self.pos += 2;
                    while matches!(self.ch(), Some(c) if matches!(c, '0'..='7') || c == '_') {
                        self.pos += 1;
                    }
                    let raw: String = self.chars[start..self.pos].iter().collect();
                    let clean = raw.replace('_', "");
                    return Token::Int(i64::from_str_radix(&clean[2..], 8).unwrap_or(0));
                }
                Some('b') | Some('B') => {
                    self.pos += 2;
                    while matches!(self.ch(), Some(c) if matches!(c, '0' | '1') || c == '_') {
                        self.pos += 1;
                    }
                    let raw: String = self.chars[start..self.pos].iter().collect();
                    let clean = raw.replace('_', "");
                    return Token::Int(i64::from_str_radix(&clean[2..], 2).unwrap_or(0));
                }
                _ => {}
            }
        }

        // Decimal integer part
        while matches!(self.ch(), Some(c) if c.is_ascii_digit() || c == '_') {
            self.pos += 1;
        }

        let mut is_float = false;

        // Fractional part
        if self.ch() == Some('.') && matches!(self.ch1(), Some(c) if c.is_ascii_digit()) {
            is_float = true;
            self.pos += 1;
            while matches!(self.ch(), Some(c) if c.is_ascii_digit() || c == '_') {
                self.pos += 1;
            }
        }

        // Exponent part
        if matches!(self.ch(), Some('e') | Some('E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.ch(), Some('+') | Some('-')) {
                self.pos += 1;
            }
            while matches!(self.ch(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }

        let raw: String = self.chars[start..self.pos].iter().collect();
        let clean = raw.replace('_', "");

        if is_float {
            Token::Float(clean.parse().unwrap_or(0.0))
        } else {
            Token::Int(clean.parse().unwrap_or(0))
        }
    }

    fn lex_word(&mut self) -> Token {
        let start = self.pos;
        while matches!(self.ch(), Some(c) if c.is_alphanumeric() || c == '_') {
            self.pos += 1;
        }
        let word: String = self.chars[start..self.pos].iter().collect();

        match word.as_str() {
            "let" => Token::Let,
            "const" => Token::Const,
            "mut" => Token::Mut,
            "True" => Token::True,
            "False" => Token::False,
            "None" => Token::None,
            "and" => Token::And,
            "or" => Token::Or,
            "not" => self.maybe_two_word("in", Token::NotIn, Token::Not),
            "in" => Token::In,
            "is" => self.maybe_two_word("not", Token::IsNot, Token::Is),
            "if" => Token::If,
            "elif" => Token::Elif,
            "else" => Token::Else,
            "match" => Token::Match,
            "for" => Token::For,
            "while" => Token::While,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "pass" => Token::Pass,
            "return" => Token::Return,
            "yield" => self.maybe_two_word("from", Token::YieldFrom, Token::Yield),
            "try" => Token::Try,
            "except" => Token::Except,
            "finally" => Token::Finally,
            "raise" => Token::Raise,
            "fn" => Token::Fn,
            "class" => Token::Class,
            "lambda" => Token::Lambda,
            "template" => Token::Template,
            "import" => Token::Import,
            "from" => Token::From,
            "as" => Token::As,
            "del" => Token::Del,
            "global" => Token::Global,
            "nonlocal" => Token::Nonlocal,
            "with" => Token::With,
            "async" => Token::Async,
            "await" => Token::Await,
            "assert" => Token::Assert,
            _ => Token::Ident(word),
        }
    }

    // Peeks ahead past spaces to see if the next identifier matches `second`.
    // Consumes the second word on match; otherwise leaves pos unchanged.
    fn maybe_two_word(&mut self, second: &str, combined: Token, single: Token) -> Token {
        let saved = self.pos;
        while matches!(self.ch(), Some(' ') | Some('\t')) {
            self.pos += 1;
        }
        let word_start = self.pos;
        while matches!(self.ch(), Some(c) if c.is_alphanumeric() || c == '_') {
            self.pos += 1;
        }
        let word: String = self.chars[word_start..self.pos].iter().collect();
        if word == second {
            combined
        } else {
            self.pos = saved;
            single
        }
    }

    fn lex_symbol(&mut self) -> Token {
        let c = self.bump().unwrap();
        match c {
            '(' => {
                self.bracket_depth += 1;
                Token::LParen
            }
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
            '+' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::PlusEq
                } else {
                    Token::Plus
                }
            }
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
            '%' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::PercentEq
                } else {
                    Token::Percent
                }
            }
            '@' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::AtEq
                } else {
                    Token::At
                }
            }
            '=' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::EqEq
                } else {
                    Token::Eq
                }
            }
            '!' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::NotEq
                } else {
                    Token::Unknown('!')
                }
            }
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
            '&' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::AmpEq
                } else {
                    Token::Amp
                }
            }
            '|' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::PipeEq
                } else {
                    Token::Pipe
                }
            }
            '^' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::CaretEq
                } else {
                    Token::Caret
                }
            }
            '~' => Token::Tilde,
            ':' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::ColonEq
                } else {
                    Token::Colon
                }
            }
            ',' => Token::Comma,
            ';' => Token::Semicolon,
            '.' => {
                if self.ch() == Some('.') && self.ch1() == Some('.') {
                    self.pos += 2;
                    Token::Ellipsis
                } else {
                    Token::Dot
                }
            }
            c => Token::Unknown(c),
        }
    }

    fn ch(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn ch1(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn ch2(&self) -> Option<char> {
        self.chars.get(self.pos + 2).copied()
    }

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

    fn lex(s: &str) -> Vec<Token> {
        Lexer::new(s).tokenize()
    }

    #[test]
    fn test_variable_keywords() {
        assert_eq!(lex("let const mut"), vec![
            Token::Let, Token::Const, Token::Mut, Token::Eof,
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
        // Newlines inside brackets are insignificant; no INDENT/DEDENT should appear.
        assert!(!t.contains(&Token::Indent));
        assert!(!t.contains(&Token::Dedent));
        // Only the trailing newline after ')' (outside brackets) generates NEWLINE.
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
}
