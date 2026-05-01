use crate::ast::{BinOp, Expr, Param, Stmt, UnaryOp};
use crate::token::Token;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn current(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn peek1(&self) -> &Token {
        self.tokens.get(self.pos + 1).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
    }

    fn eat(&mut self, expected: &Token) -> Result<(), String> {
        if self.current() == expected {
            self.advance();
            Ok(())
        } else {
            Err(format!("expected `{}`, got `{}`", expected, self.current()))
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(
            self.current(),
            Token::Newline | Token::Indent | Token::Dedent | Token::Semicolon
        ) {
            self.advance();
        }
    }

    pub fn parse_program(&mut self) -> Result<Vec<Stmt>, String> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while *self.current() != Token::Eof {
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        Ok(stmts)
    }

    // Parses an indented block: NEWLINE INDENT stmt* DEDENT
    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        self.eat(&Token::Newline)?;
        self.eat(&Token::Indent)?;
        let mut stmts = Vec::new();
        loop {
            while matches!(self.current(), Token::Newline | Token::Semicolon) {
                self.advance();
            }
            if matches!(self.current(), Token::Dedent | Token::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
        }
        if *self.current() == Token::Dedent {
            self.advance();
        }
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        match self.current().clone() {
            Token::Let => {
                self.advance();
                let name = self.expect_ident()?;
                self.eat(&Token::Eq)?;
                Ok(Stmt::Let(name, self.parse_expr()?))
            }
            Token::Const => {
                self.advance();
                let name = self.expect_ident()?;
                self.eat(&Token::Eq)?;
                Ok(Stmt::Const(name, self.parse_expr()?))
            }
            Token::Mut => {
                self.advance();
                let name = self.expect_ident()?;
                self.eat(&Token::Eq)?;
                Ok(Stmt::Mut(name, self.parse_expr()?))
            }
            Token::Pass => {
                self.advance();
                Ok(Stmt::Pass)
            }
            Token::Break => {
                self.advance();
                Ok(Stmt::Break)
            }
            Token::Continue => {
                self.advance();
                Ok(Stmt::Continue)
            }
            Token::Return => {
                self.advance();
                if matches!(self.current(), Token::Newline | Token::Eof | Token::Semicolon | Token::Dedent) {
                    Ok(Stmt::Return(None))
                } else {
                    Ok(Stmt::Return(Some(self.parse_expr()?)))
                }
            }
            Token::BlockReturn => {
                self.advance();
                Ok(Stmt::BlockReturn(self.parse_expr()?))
            }
            Token::BlockYield => {
                self.advance();
                Ok(Stmt::BlockYield(self.parse_expr()?))
            }
            Token::If => {
                self.advance();
                let cond = self.parse_expr()?;
                self.eat(&Token::Colon)?;
                let body = self.parse_block()?;
                let mut branches = vec![(cond, body)];
                let mut else_body = None;
                loop {
                    match self.current().clone() {
                        Token::Elif => {
                            self.advance();
                            let c = self.parse_expr()?;
                            self.eat(&Token::Colon)?;
                            let b = self.parse_block()?;
                            branches.push((c, b));
                        }
                        Token::Else => {
                            self.advance();
                            self.eat(&Token::Colon)?;
                            else_body = Some(self.parse_block()?);
                            break;
                        }
                        _ => break,
                    }
                }
                Ok(Stmt::If { branches, else_body })
            }
            Token::While => {
                self.advance();
                let cond = self.parse_expr()?;
                self.eat(&Token::Colon)?;
                let body = self.parse_block()?;
                Ok(Stmt::While { cond, body })
            }
            Token::For => {
                self.advance();
                let target = self.expect_ident()?;
                self.eat(&Token::In)?;
                let iter = self.parse_expr()?;
                self.eat(&Token::Colon)?;
                let body = self.parse_block()?;
                Ok(Stmt::For { target, iter, body })
            }
            Token::Block => {
                self.advance();
                self.eat(&Token::Colon)?;
                let body = self.parse_block()?;
                Ok(Stmt::Block(body))
            }
            Token::Fn => self.parse_fn_def(),
            Token::Class => self.parse_class_def(),
            Token::Ident(_) => match self.peek1().clone() {
                Token::Eq => {
                    let name = self.expect_ident()?;
                    self.advance(); // consume `=`
                    Ok(Stmt::Assign(name, self.parse_expr()?))
                }
                Token::PlusEq => self.parse_compound(BinOp::Add),
                Token::MinusEq => self.parse_compound(BinOp::Sub),
                Token::StarEq => self.parse_compound(BinOp::Mul),
                Token::SlashEq => self.parse_compound(BinOp::Div),
                Token::SlashSlashEq => self.parse_compound(BinOp::FloorDiv),
                Token::PercentEq => self.parse_compound(BinOp::Mod),
                Token::StarStarEq => self.parse_compound(BinOp::Pow),
                Token::AmpEq => self.parse_compound(BinOp::BitAnd),
                Token::PipeEq => self.parse_compound(BinOp::BitOr),
                Token::CaretEq => self.parse_compound(BinOp::BitXor),
                Token::LtLtEq => self.parse_compound(BinOp::LShift),
                Token::GtGtEq => self.parse_compound(BinOp::RShift),
                _ => {
                    // May be: expression statement or attribute assignment (self.x = v)
                    let expr = self.parse_expr()?;
                    if *self.current() == Token::Eq {
                        self.advance();
                        Ok(Stmt::AttrAssign { target: expr, value: self.parse_expr()? })
                    } else {
                        Ok(Stmt::Expr(expr))
                    }
                }
            },
            _ => Ok(Stmt::Expr(self.parse_expr()?)),
        }
    }

    fn parse_compound(&mut self, op: BinOp) -> Result<Stmt, String> {
        let name = self.expect_ident()?;
        self.advance(); // consume the compound-assignment operator
        Ok(Stmt::CompoundAssign(name, op, self.parse_expr()?))
    }

    fn parse_fn_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // consume `fn`
        let name = self.expect_ident()?;
        self.eat(&Token::LParen)?;
        let mut params = Vec::new();
        while *self.current() != Token::RParen && *self.current() != Token::Eof {
            params.push(self.parse_param()?);
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.eat(&Token::RParen)?;
        // Optional return type: -> Type
        if *self.current() == Token::Arrow {
            self.advance();
            self.skip_type_expr()?;
        }
        self.eat(&Token::Colon)?;
        let body = self.parse_block()?;
        Ok(Stmt::FnDef { name, params, body })
    }

    fn parse_class_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // consume `class`
        let name = self.expect_ident()?;
        // Optional base classes: (Base1, Base2, ...)
        let mut bases = Vec::new();
        if *self.current() == Token::LParen {
            self.advance();
            while *self.current() != Token::RParen && *self.current() != Token::Eof {
                bases.push(self.expect_ident()?);
                if *self.current() == Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
            self.eat(&Token::RParen)?;
        }
        self.eat(&Token::Colon)?;
        let body = self.parse_block()?;
        Ok(Stmt::ClassDef { name, bases, body })
    }

    fn parse_param(&mut self) -> Result<Param, String> {
        let mutable = if *self.current() == Token::Mut {
            self.advance();
            true
        } else {
            false
        };
        let name = self.expect_ident()?;
        // Skip optional type annotation: : Type
        if *self.current() == Token::Colon {
            self.advance();
            self.skip_type_expr()?;
        }
        Ok(Param { name, mutable })
    }

    // Skips a type expression of the form: Ident [ '[' ... ']' ]
    fn skip_type_expr(&mut self) -> Result<(), String> {
        self.expect_ident()?;
        // Optional generic parameters: list[int], dict[str, int], etc.
        if *self.current() == Token::LBracket {
            self.advance();
            let mut depth = 1usize;
            while depth > 0 && *self.current() != Token::Eof {
                if *self.current() == Token::LBracket {
                    depth += 1;
                } else if *self.current() == Token::RBracket {
                    depth -= 1;
                }
                self.advance();
            }
        }
        Ok(())
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        if let Token::Ident(name) = self.current().clone() {
            self.advance();
            Ok(name)
        } else {
            Err(format!("expected identifier, got `{}`", self.current()))
        }
    }

    // --- Expression parsing (precedence climbing) ---

    pub fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while *self.current() == Token::Or {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinOp { op: BinOp::Or, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_not()?;
        while *self.current() == Token::And {
            self.advance();
            let right = self.parse_not()?;
            left = Expr::BinOp { op: BinOp::And, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr, String> {
        if *self.current() == Token::Not {
            self.advance();
            let operand = self.parse_not()?;
            return Ok(Expr::UnaryOp { op: UnaryOp::Not, operand: Box::new(operand) });
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let left = self.parse_bitor()?;
        let op = match self.current() {
            Token::EqEq => Some(BinOp::Eq),
            Token::NotEq => Some(BinOp::NotEq),
            Token::Lt => Some(BinOp::Lt),
            Token::Gt => Some(BinOp::Gt),
            Token::LtEq => Some(BinOp::LtEq),
            Token::GtEq => Some(BinOp::GtEq),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let right = self.parse_bitor()?;
            return Ok(Expr::BinOp { op, left: Box::new(left), right: Box::new(right) });
        }
        Ok(left)
    }

    fn parse_bitor(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bitxor()?;
        while *self.current() == Token::Pipe {
            self.advance();
            let right = self.parse_bitxor()?;
            left = Expr::BinOp { op: BinOp::BitOr, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_bitxor(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bitand()?;
        while *self.current() == Token::Caret {
            self.advance();
            let right = self.parse_bitand()?;
            left = Expr::BinOp { op: BinOp::BitXor, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_bitand(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_shift()?;
        while *self.current() == Token::Amp {
            self.advance();
            let right = self.parse_shift()?;
            left = Expr::BinOp { op: BinOp::BitAnd, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.current() {
                Token::LtLt => Some(BinOp::LShift),
                Token::GtGt => Some(BinOp::RShift),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let right = self.parse_additive()?;
                left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right) };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.current() {
                Token::Plus => Some(BinOp::Add),
                Token::Minus => Some(BinOp::Sub),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let right = self.parse_multiplicative()?;
                left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right) };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.current() {
                Token::Star => Some(BinOp::Mul),
                Token::Slash => Some(BinOp::Div),
                Token::SlashSlash => Some(BinOp::FloorDiv),
                Token::Percent => Some(BinOp::Mod),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let right = self.parse_unary()?;
                left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right) };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.current() {
            Token::Minus => {
                self.advance();
                Ok(Expr::UnaryOp { op: UnaryOp::Neg, operand: Box::new(self.parse_unary()?) })
            }
            Token::Tilde => {
                self.advance();
                Ok(Expr::UnaryOp { op: UnaryOp::BitNot, operand: Box::new(self.parse_unary()?) })
            }
            Token::Plus => {
                self.advance();
                self.parse_unary()
            }
            _ => self.parse_power(),
        }
    }

    fn parse_power(&mut self) -> Result<Expr, String> {
        let base = self.parse_call()?;
        if *self.current() == Token::StarStar {
            self.advance();
            let exp = self.parse_unary()?; // right-associative
            Ok(Expr::BinOp { op: BinOp::Pow, left: Box::new(base), right: Box::new(exp) })
        } else {
            Ok(base)
        }
    }

    fn parse_call(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.current() {
                Token::LParen => {
                    self.advance(); // consume `(`
                    let mut args = Vec::new();
                    while *self.current() != Token::RParen && *self.current() != Token::Eof {
                        args.push(self.parse_expr()?);
                        if *self.current() == Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.eat(&Token::RParen)?;
                    expr = Expr::Call { func: Box::new(expr), args };
                }
                Token::Dot => {
                    self.advance(); // consume `.`
                    let attr = self.expect_ident()?;
                    expr = Expr::Attr { object: Box::new(expr), attr };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.current().clone() {
            Token::Int(n) => { self.advance(); Ok(Expr::Int(n)) }
            Token::Float(f) => { self.advance(); Ok(Expr::Float(f)) }
            Token::Str(s) => { self.advance(); Ok(Expr::Str(s)) }
            Token::True => { self.advance(); Ok(Expr::Bool(true)) }
            Token::False => { self.advance(); Ok(Expr::Bool(false)) }
            Token::None => { self.advance(); Ok(Expr::None) }
            Token::Ident(name) => { self.advance(); Ok(Expr::Ident(name)) }
            Token::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.eat(&Token::RParen)?;
                Ok(expr)
            }
            Token::LBracket => {
                self.advance(); // consume `[`
                let mut items = Vec::new();
                while *self.current() != Token::RBracket && *self.current() != Token::Eof {
                    items.push(self.parse_expr()?);
                    if *self.current() == Token::Comma {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.eat(&Token::RBracket)?;
                Ok(Expr::List(items))
            }
            tok => Err(format!("unexpected token: `{tok}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(src: &str) -> Vec<Stmt> {
        let tokens = Lexer::new(src, "").tokenize().into_iter().map(|s| s.token).collect();
        Parser::new(tokens).parse_program().expect("parse error")
    }

    #[test]
    fn test_literal_expr() {
        let stmts = parse("42");
        assert!(matches!(stmts[0], Stmt::Expr(Expr::Int(42))));
    }

    #[test]
    fn test_let_decl() {
        let stmts = parse("let x = 10");
        assert!(matches!(&stmts[0], Stmt::Let(name, Expr::Int(10)) if name == "x"));
    }

    #[test]
    fn test_mut_decl() {
        let stmts = parse("mut y = 3.14");
        assert!(matches!(&stmts[0], Stmt::Mut(name, Expr::Float(_)) if name == "y"));
    }

    #[test]
    fn test_assign() {
        let stmts = parse("mut x = 0\nx = 5");
        assert!(matches!(&stmts[1], Stmt::Assign(name, Expr::Int(5)) if name == "x"));
    }

    #[test]
    fn test_compound_assign() {
        let stmts = parse("mut x = 0\nx += 1");
        assert!(matches!(
            &stmts[1],
            Stmt::CompoundAssign(name, BinOp::Add, Expr::Int(1)) if name == "x"
        ));
    }

    #[test]
    fn test_binop_precedence() {
        let stmts = parse("2 + 3 * 4");
        if let Stmt::Expr(Expr::BinOp { op: BinOp::Add, right, .. }) = &stmts[0] {
            assert!(matches!(right.as_ref(), Expr::BinOp { op: BinOp::Mul, .. }));
        } else {
            panic!("unexpected AST");
        }
    }

    #[test]
    fn test_call_expr() {
        let stmts = parse(r#"print("hello")"#);
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::Call { .. })));
    }

    #[test]
    fn test_unary_neg() {
        let stmts = parse("-5");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::UnaryOp { op: UnaryOp::Neg, .. })));
    }

    #[test]
    fn test_power_right_assoc() {
        let stmts = parse("2 ** 3 ** 2");
        if let Stmt::Expr(Expr::BinOp { op: BinOp::Pow, right, .. }) = &stmts[0] {
            assert!(matches!(right.as_ref(), Expr::BinOp { op: BinOp::Pow, .. }));
        } else {
            panic!("unexpected AST");
        }
    }

    #[test]
    fn test_if_stmt() {
        let stmts = parse("if True:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::If { branches, else_body: None } if branches.len() == 1));
    }

    #[test]
    fn test_if_else_stmt() {
        let stmts = parse("if True:\n    pass\nelse:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::If { else_body: Some(_), .. }));
    }

    #[test]
    fn test_if_elif_else_stmt() {
        let stmts = parse("if True:\n    pass\nelif False:\n    pass\nelse:\n    pass\n");
        if let Stmt::If { branches, else_body } = &stmts[0] {
            assert_eq!(branches.len(), 2);
            assert!(else_body.is_some());
        } else {
            panic!("expected If");
        }
    }

    #[test]
    fn test_while_stmt() {
        let stmts = parse("while True:\n    break\n");
        assert!(matches!(&stmts[0], Stmt::While { .. }));
    }

    #[test]
    fn test_for_stmt() {
        let stmts = parse("for i in [1, 2, 3]:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::For { target, .. } if target == "i"));
    }

    #[test]
    fn test_block_stmt() {
        let stmts = parse("block:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::Block(_)));
    }

    #[test]
    fn test_list_literal() {
        let stmts = parse("[1, 2, 3]");
        assert!(matches!(&stmts[0], Stmt::Expr(Expr::List(_))));
    }

    // --- fn ---

    #[test]
    fn test_fn_def() {
        let stmts = parse("fn add(a, b):\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::FnDef { name, .. } if name == "add"));
    }

    #[test]
    fn test_fn_no_params() {
        let stmts = parse("fn hello():\n    pass\n");
        if let Stmt::FnDef { params, .. } = &stmts[0] {
            assert!(params.is_empty());
        } else {
            panic!("expected FnDef");
        }
    }

    #[test]
    fn test_fn_mut_param() {
        let stmts = parse("fn modify(mut x):\n    pass\n");
        if let Stmt::FnDef { params, .. } = &stmts[0] {
            assert!(params[0].mutable);
            assert_eq!(params[0].name, "x");
        } else {
            panic!("expected FnDef");
        }
    }

    #[test]
    fn test_fn_type_annotations() {
        // type annotations on params and return type are parsed without error
        let stmts = parse("fn add(a: int, b: int) -> int:\n    pass\n");
        if let Stmt::FnDef { params, .. } = &stmts[0] {
            assert_eq!(params.len(), 2);
            assert_eq!(params[0].name, "a");
            assert_eq!(params[1].name, "b");
        } else {
            panic!("expected FnDef");
        }
    }

    #[test]
    fn test_fn_generic_type_annotation() {
        let stmts = parse("fn first(items: list[int]) -> int:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::FnDef { name, .. } if name == "first"));
    }

    #[test]
    fn test_fn_with_body() {
        let stmts = parse("fn abs(x):\n    if x < 0:\n        return -x\n    return x\n");
        if let Stmt::FnDef { body, .. } = &stmts[0] {
            assert_eq!(body.len(), 2);
        } else {
            panic!("expected FnDef");
        }
    }

    // --- class ---

    #[test]
    fn test_class_empty() {
        let stmts = parse("class Foo:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::ClassDef { name, bases, .. }
            if name == "Foo" && bases.is_empty()));
    }

    #[test]
    fn test_class_with_base() {
        let stmts = parse("class Bar(Foo):\n    pass\n");
        if let Stmt::ClassDef { name, bases, .. } = &stmts[0] {
            assert_eq!(name, "Bar");
            assert_eq!(bases, &["Foo"]);
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_multiple_bases() {
        let stmts = parse("class C(A, B):\n    pass\n");
        if let Stmt::ClassDef { bases, .. } = &stmts[0] {
            assert_eq!(bases.len(), 2);
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_with_method() {
        let stmts = parse("class Foo:\n    fn greet(self):\n        pass\n");
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::FnDef { name, .. } if name == "greet"));
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_multiple_methods() {
        let src = "class Counter:\n    fn inc(mut self):\n        pass\n    fn dec(mut self):\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert_eq!(body.len(), 2);
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_method_with_params() {
        let src = "class Adder:\n    fn add(self, a: int, b: int) -> int:\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            if let Stmt::FnDef { params, .. } = &body[0] {
                assert_eq!(params.len(), 3); // self, a, b
            } else {
                panic!("expected FnDef");
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_with_field_and_method() {
        let src = "class Point:\n    mut x = 0\n    mut y = 0\n    fn move(mut self, dx, dy):\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert_eq!(body.len(), 3); // 2 fields + 1 method
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_nested_if() {
        let src = "if True:\n    if False:\n        pass\n    pass\n";
        let stmts = parse(src);
        if let Stmt::If { branches, .. } = &stmts[0] {
            assert_eq!(branches[0].1.len(), 2);
        } else {
            panic!("expected If");
        }
    }
}
