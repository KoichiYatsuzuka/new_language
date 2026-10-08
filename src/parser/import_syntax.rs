// parser/import_syntax.rs — import 文のモジュール指定（`..a.b`）の構文。
//
// ⚠ 以前は import の解析が 2 実装（`imports/` と、拡張用の構文だけを読む `imports_editor.rs`・削除済み）
//   あり、モジュール指定の読み方を両方で共有するためにここへ出した。今は `imports/` だけが使う。

use crate::parser::Parser;
use crate::token::Token;

impl Parser {
    /// モジュール指定 `[.]* IDENT ('.' IDENT)*` を読み、`(先頭のドットの数, 各部分)` を返す。
    ///
    /// - `a.b` → `(0, ["a", "b"])`
    /// - `.a` → `(1, ["a"])`（同じディレクトリ）
    /// - `..a.b` → `(2, ["a", "b"])`（1 つ上のディレクトリ）
    ///
    /// 探索の規則は [`crate::module_path`]。
    /// ⚠ 字句解析器は `...` を `Ellipsis` の 1 トークンにする（`..` は `Dot` 2 つ）ので、
    ///   `Ellipsis` はドット 3 つとして数える。
    /// ⚠ ドットの後にモジュール名が無い書き方（`from . import x`・`from .. import x`）は
    ///   `allow_bare_dots` のときだけ受け、各部分は空で返す（`from` の形だけ。`import .` は誤り）。
    pub(crate) fn parse_module_ref(&mut self, allow_bare_dots: bool) -> Result<(u32, Vec<String>), String> {
        let mut level: u32 = 0;
        loop {
            match self.current() {
                Token::Dot => level += 1,
                Token::Ellipsis => level += 3,
                _ => break,
            }
            self.advance();
        }
        if level > 0 && allow_bare_dots && *self.current() == Token::Import {
            return Ok((level, Vec::new()));
        }
        if level > 0 && !matches!(self.current(), Token::Ident(_)) {
            return Err(format!(
                "expected a module name after `{}`, got `{}` \
                 (to import a module from this directory, write `import .name`)",
                ".".repeat(level as usize),
                self.current()
            ));
        }
        let mut segments = vec![self.expect_ident()?];
        while *self.current() == Token::Dot {
            self.advance();
            segments.push(self.expect_ident()?);
        }
        Ok((level, segments))
    }
}

#[cfg(test)]
mod tests {
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn module_ref(src: &str) -> Result<(u32, Vec<String>), String> {
        let mut p = Parser::new(Lexer::new(src, "<test>").tokenize(), None);
        p.parse_module_ref(false)
    }

    fn segs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn counts_leading_dots() {
        assert_eq!(module_ref("a.b").unwrap(), (0, segs(&["a", "b"])));
        assert_eq!(module_ref(".a").unwrap(), (1, segs(&["a"])));
        assert_eq!(module_ref("..a.b").unwrap(), (2, segs(&["a", "b"])));
        // ⚠ `...` は字句解析器が `Ellipsis` の 1 トークンにする。
        assert_eq!(module_ref("...a").unwrap(), (3, segs(&["a"])));
        assert_eq!(module_ref("....a").unwrap(), (4, segs(&["a"])));
    }

    #[test]
    fn bare_dots_are_accepted_only_for_from() {
        let mut p = Parser::new(Lexer::new(".. import x", "<test>").tokenize(), None);
        assert_eq!(p.parse_module_ref(true).unwrap(), (2, Vec::<String>::new()));
    }

    #[test]
    fn dots_without_a_module_name_are_an_error() {
        let e = module_ref(". import x").unwrap_err();
        assert!(e.contains("expected a module name after `.`"), "{e}");
        assert!(e.contains("import .name"), "{e}");
    }
}
