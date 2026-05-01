use crate::ast::{BinOp, Expr, Stmt, UnaryOp};
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
                _ => Ok(Stmt::Expr(self.parse_expr()?)),
            },
            _ => Ok(Stmt::Expr(self.parse_expr()?)),
        }
    }

    fn parse_compound(&mut self, op: BinOp) -> Result<Stmt, String> {
        let name = self.expect_ident()?;
        self.advance(); // consume the compound-assignment operator
        Ok(Stmt::CompoundAssign(name, op, self.parse_expr()?))
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
        while *self.current() == Token::LParen {
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
            tok => Err(format!("unexpected token: `{tok}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(src: &str) -> Vec<Stmt> {
        let tokens = Lexer::new(src).tokenize();
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
        // 2 + 3 * 4 should be 2 + (3 * 4)
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
        assert!(matches!(
            &stmts[0],
            Stmt::Expr(Expr::Call { .. })
        ));
    }

    #[test]
    fn test_unary_neg() {
        let stmts = parse("-5");
        assert!(matches!(
            &stmts[0],
            Stmt::Expr(Expr::UnaryOp { op: UnaryOp::Neg, .. })
        ));
    }

    #[test]
    fn test_power_right_assoc() {
        // 2 ** 3 ** 2 should be 2 ** (3 ** 2)
        let stmts = parse("2 ** 3 ** 2");
        if let Stmt::Expr(Expr::BinOp { op: BinOp::Pow, right, .. }) = &stmts[0] {
            assert!(matches!(right.as_ref(), Expr::BinOp { op: BinOp::Pow, .. }));
        } else {
            panic!("unexpected AST");
        }
    }
}
