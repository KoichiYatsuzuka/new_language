/// 識別子・キーワード解析メソッド群。
///
/// `Lexer` の実装を機能単位に分割した補助ファイル。
/// すべてのメソッドは `scan.rs` で定義された `Lexer` に対する `impl` ブロックとして提供する。
use crate::token::Token;

use super::scan::Lexer;

impl Lexer {
    /// 識別子またはキーワードを解析して対応するトークンを返す。
    ///
    /// アルファベット・数字・アンダースコアを消費し、得られた単語文字列を
    /// キーワードテーブルと照合する。`not` / `is` / `yield` などは
    /// `maybe_two_word()` で複合キーワードの可能性を先読みする。
    ///
    /// # 戻り値
    /// キーワードに対応する `Token`、または `Token::Ident(String)`
    pub(super) fn lex_word(&mut self) -> Token {
        // 識別子に使用できる文字（英数字・アンダースコア）を消費する
        let start = self.pos;
        while matches!(self.ch(), Some(ch) if ch.is_alphanumeric() || ch == '_') {
            self.pos += 1;
        }
        let word: String = self.chars[start..self.pos].iter().collect();

        // 単語をキーワードテーブルと照合する
        match word.as_str() {
            "let" => Token::Let,
            "const" => Token::Const,
            "mut" => Token::Mut,
            "static" => Token::Static,
            "freeze" => Token::Freeze,
            "True" => Token::True,
            "False" => Token::False,
            "None" => Token::None,
            "Undefined" => Token::Undefined,
            "and" => Token::And,
            "or" => Token::Or,
            // `not` は `not in` の可能性があるため先読みする
            "not" => self.maybe_two_word("in", Token::NotIn, Token::Not),
            "in" => Token::In,
            // `is` は `is not` の可能性があるため先読みする
            "is" => self.maybe_two_word("not", Token::IsNot, Token::Is),
            "if" => Token::If,
            "elif" => Token::Elif,
            "else" => Token::Else,
            "match" => Token::Match,
            "case" => Token::Case,
            "for" => Token::For,
            "while" => Token::While,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "pass" => Token::Pass,
            "return" => Token::Return,
            // `yield` は `yield from` の可能性があるため先読みする
            "yield" => self.maybe_two_word("from", Token::YieldFrom, Token::Yield),
            "block_return" => Token::BlockReturn,
            "loop_yield" => Token::LoopYield,
            "block" => Token::Block,
            "break_point" => Token::BreakPoint,
            "try" => Token::Try,
            "except" => Token::Except,
            "finally" => Token::Finally,
            "raise" => Token::Raise,
            "fn" => Token::Fn,
            "gen" => Token::Gen,
            "class" => Token::Class,
            "enum" => Token::Enum,
            "trait" => Token::Trait,
            "protocol" => Token::Protocol,
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
            "public" => Token::Public,
            "private" => Token::Private,
            "protected" => Token::Protected,
            "class_method" => Token::ClassMethod,
            "Self" => Token::SelfType,
            "new_type" => Token::NewType,
            "Any" => Token::Any,
            "Union" => Token::Union,
            "Option" => Token::Option,
            "Intersection" => Token::Intersection,
            "on" => Token::On,
            "off" => Token::Off,
            "once" => Token::Once,
            "mustbe" => Token::MustBe,
            // キーワードに一致しない場合は識別子トークンを返す
            _ => Token::Ident(word),
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
        while matches!(self.ch(), Some(ch) if ch.is_alphanumeric() || ch == '_') {
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
}
