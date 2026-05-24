/// 記号・演算子トークン解析メソッド。
///
/// `Lexer` の実装を機能単位に分割した補助ファイル。
/// メソッドは `scan.rs` で定義された `Lexer` に対する `impl` ブロックとして提供する。

use crate::token::Token;

use super::scan::Lexer;

impl Lexer {
    /// 記号文字から演算子・区切り記号トークンを生成する。
    ///
    /// 先頭文字を消費したうえで、複数文字からなるトークン（`==`, `->`, `**=` など）は
    /// 1 文字先読みをして確定させる。括弧文字では `bracket_depth` の更新も行う。
    ///
    /// # 戻り値
    /// 対応する `Token`。未知の文字の場合は `Token::Unknown(char)`
    pub(super) fn lex_symbol(&mut self) -> Token {
        let ch = self.bump().unwrap();
        match ch {
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

            // `=` / `==` / `=>`
            '=' => {
                if self.ch() == Some('=') {
                    self.pos += 1;
                    Token::EqEq
                } else if self.ch() == Some('>') {
                    self.pos += 1;
                    Token::FatArrow
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

            // `<` / `<=` / `<<` / `<<=` / `<-` の 5 種を判定する
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
                Some('-') => {
                    self.pos += 1;
                    Token::LeftArrow
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
            other => Token::Unknown(other),
        }
    }
}
