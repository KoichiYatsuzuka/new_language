use std::collections::HashMap;

use crate::ast::{BinOp, CallArg, Expr, FieldKind, Param, Stmt, TemplateParam, UnaryOp};
use crate::token::{Span, Spanned, Token};

pub struct Parser {
    tokens: Vec<Spanned>,
    pos: usize,
    /// trait name → (template_params, fields: [(name, kind, type_ann, has_default)], virtual_methods: [name])
    known_traits: HashMap<String, (Vec<TemplateParam>, Vec<(String, FieldKind, String, bool)>, Vec<String>)>,
}

impl Parser {
    pub fn new(tokens: Vec<Spanned>) -> Self {
        Self { tokens, pos: 0, known_traits: HashMap::new() }
    }

    fn current(&self) -> &Token {
        self.tokens.get(self.pos).map(|s| &s.token).unwrap_or(&Token::Eof)
    }

    fn peek1(&self) -> &Token {
        self.tokens.get(self.pos + 1).map(|s| &s.token).unwrap_or(&Token::Eof)
    }

    fn current_span(&self) -> Span {
        self.tokens.get(self.pos).map(|s| s.span.clone()).unwrap_or_else(Span::unknown)
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
                if *self.current() == Token::Colon { self.advance(); self.parse_type_expr()?; }
                self.eat(&Token::Eq)?;
                Ok(Stmt::Let(name, self.parse_expr()?))
            }
            Token::Const => {
                self.advance();
                let name = self.expect_ident()?;
                if *self.current() == Token::Colon { self.advance(); self.parse_type_expr()?; }
                self.eat(&Token::Eq)?;
                Ok(Stmt::Const(name, self.parse_expr()?))
            }
            Token::Mut => {
                self.advance();
                let name = self.expect_ident()?;
                if *self.current() == Token::Colon { self.advance(); self.parse_type_expr()?; }
                self.eat(&Token::Eq)?;
                Ok(Stmt::Mut(name, self.parse_expr()?))
            }
            Token::Freeze => {
                let span = self.current_span();
                self.advance();
                let name = self.expect_ident()?;
                Ok(Stmt::Freeze(name, span))
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
            Token::Yield => {
                self.advance();
                Ok(Stmt::Yield(self.parse_expr()?))
            }
            Token::Fn => self.parse_fn_def(),
            Token::Gen => self.parse_gen_def(),
            Token::Class => self.parse_class_def(),
            Token::Trait => self.parse_trait_def(),
            Token::Ident(_) => match self.peek1().clone() {
                Token::Eq => {
                    let span = self.current_span();
                    let name = self.expect_ident()?;
                    self.advance(); // consume `=`
                    Ok(Stmt::Assign { name, value: self.parse_expr()?, span })
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
                    // May be: expr stmt, attr assign (self.x = v), or attr compound (self.x += v)
                    let expr = self.parse_expr()?;
                    match self.current().clone() {
                        Token::Eq => {
                            self.advance();
                            Ok(Stmt::AttrAssign { target: expr, value: self.parse_expr()? })
                        }
                        Token::PlusEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::Add,      value: self.parse_expr()? }) }
                        Token::MinusEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::Sub,      value: self.parse_expr()? }) }
                        Token::StarEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::Mul,      value: self.parse_expr()? }) }
                        Token::SlashEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::Div,     value: self.parse_expr()? }) }
                        Token::SlashSlashEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::FloorDiv, value: self.parse_expr()? }) }
                        Token::PercentEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::Mod,   value: self.parse_expr()? }) }
                        Token::StarStarEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::Pow,  value: self.parse_expr()? }) }
                        Token::AmpEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::BitAnd,   value: self.parse_expr()? }) }
                        Token::PipeEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::BitOr,   value: self.parse_expr()? }) }
                        Token::CaretEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::BitXor, value: self.parse_expr()? }) }
                        Token::LtLtEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::LShift,  value: self.parse_expr()? }) }
                        Token::GtGtEq => { self.advance(); Ok(Stmt::AttrCompoundAssign { target: expr, op: BinOp::RShift,  value: self.parse_expr()? }) }
                        _ => Ok(Stmt::Expr(expr)),
                    }
                }
            },
            _ => Ok(Stmt::Expr(self.parse_expr()?)),
        }
    }

    fn parse_compound(&mut self, op: BinOp) -> Result<Stmt, String> {
        let span = self.current_span(); // span of the variable identifier
        let name = self.expect_ident()?;
        self.advance(); // consume the compound-assignment operator
        Ok(Stmt::CompoundAssign { name, op, value: self.parse_expr()?, span })
    }

    fn parse_fn_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // consume `fn`
        let name = self.expect_ident()?;
        let template_params = self.parse_template_params()?;
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
        let return_type = if *self.current() == Token::Arrow {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.eat(&Token::Colon)?;
        // Detect virtual body: NEWLINE INDENT ELLIPSIS [NEWLINE] DEDENT
        let (body, is_virtual) = if self.is_virtual_body() {
            self.advance(); // Newline
            self.advance(); // Indent
            self.advance(); // Ellipsis
            while matches!(self.current(), Token::Newline | Token::Semicolon) {
                self.advance();
            }
            if *self.current() == Token::Dedent {
                self.advance();
            }
            (vec![], true)
        } else {
            (self.parse_block()?, false)
        };
        Ok(Stmt::FnDef { name, template_params, params, return_type, body, is_virtual })
    }

    fn parse_gen_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // consume `gen`
        let name = self.expect_ident()?;
        let template_params = self.parse_template_params()?;
        self.eat(&Token::LParen)?;
        let mut params = Vec::new();
        while *self.current() != Token::RParen && *self.current() != Token::Eof {
            let param = self.parse_param()?;
            if param.mutable && param.name != "self" {
                return Err(format!(
                    "ParseError: generator function `{name}`: parameter `{}` cannot be `mut`; \
                     generator parameters must be `let` or `const`",
                    param.name
                ));
            }
            params.push(param);
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.eat(&Token::RParen)?;
        let yield_type = if *self.current() == Token::Arrow {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        self.eat(&Token::Colon)?;
        let body = self.parse_block()?;
        if Self::body_has_return(&body) {
            return Err(format!(
                "ParseError: generator function `{name}` must not contain a `return` statement"
            ));
        }
        Ok(Stmt::GenDef { name, template_params, params, yield_type, body })
    }

    /// Returns `true` if `stmts` contains a `return` statement at any depth,
    /// without descending into nested `fn`/`gen` definitions.
    fn body_has_return(stmts: &[Stmt]) -> bool {
        for stmt in stmts {
            match stmt {
                Stmt::Return(_) => return true,
                Stmt::If { branches, else_body } => {
                    if branches.iter().any(|(_, b)| Self::body_has_return(b)) {
                        return true;
                    }
                    if else_body.as_deref().map_or(false, Self::body_has_return) {
                        return true;
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Block(body) => {
                    if Self::body_has_return(body) {
                        return true;
                    }
                }
                // Do not descend into nested fn/gen — they have their own return scope.
                _ => {}
            }
        }
        false
    }

    /// Returns true when the upcoming token sequence is NEWLINE INDENT ELLIPSIS,
    /// indicating a virtual method body (`...`).
    fn is_virtual_body(&self) -> bool {
        let t0 = self.tokens.get(self.pos).map(|s| &s.token).unwrap_or(&Token::Eof);
        let t1 = self.tokens.get(self.pos + 1).map(|s| &s.token).unwrap_or(&Token::Eof);
        let t2 = self.tokens.get(self.pos + 2).map(|s| &s.token).unwrap_or(&Token::Eof);
        matches!(t0, Token::Newline) && matches!(t1, Token::Indent) && matches!(t2, Token::Ellipsis)
    }

    fn parse_trait_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // consume `trait`
        let name = self.expect_ident()?;
        let template_params = self.parse_template_params()?;
        if *self.current() == Token::LParen {
            return Err(format!("StaticTypeError: trait `{name}` cannot inherit from another type"));
        }
        self.eat(&Token::Colon)?;
        let body = self.parse_class_body()?;

        // Check that non-virtual methods have required type annotations (same rules as classes)
        for stmt in &body {
            if let Stmt::FnDef { name: mname, params, return_type, is_virtual, .. } = stmt {
                if !is_virtual {
                    if return_type.is_none() {
                        return Err(format!(
                            "StaticTypeError: trait method `{mname}` is missing a return type annotation"
                        ));
                    }
                    for p in params {
                        if p.name != "self" && p.type_ann.is_none() {
                            return Err(format!(
                                "StaticTypeError: parameter `{}` of trait method `{mname}` is missing a type annotation",
                                p.name
                            ));
                        }
                    }
                } else {
                    // Virtual methods also need full annotations
                    if return_type.is_none() {
                        return Err(format!(
                            "StaticTypeError: virtual method `{mname}` is missing a return type annotation"
                        ));
                    }
                    for p in params {
                        if p.name != "self" && p.type_ann.is_none() {
                            return Err(format!(
                                "StaticTypeError: parameter `{}` of virtual method `{mname}` is missing a type annotation",
                                p.name
                            ));
                        }
                    }
                }
            }
        }

        let fields: Vec<(String, FieldKind, String, bool)> = body.iter()
            .filter_map(|s| {
                if let Stmt::Field { name: fname, kind, type_ann, default } = s {
                    Some((fname.clone(), kind.clone(), type_ann.clone(), default.is_some()))
                } else {
                    None
                }
            })
            .collect();
        let virtual_methods: Vec<String> = body.iter()
            .filter_map(|s| {
                if let Stmt::FnDef { name: mname, is_virtual: true, .. } = s {
                    Some(mname.clone())
                } else {
                    None
                }
            })
            .collect();
        self.known_traits.insert(name.clone(), (template_params.clone(), fields, virtual_methods));

        Ok(Stmt::TraitDef { name, template_params, body })
    }

    fn parse_class_def(&mut self) -> Result<Stmt, String> {
        self.advance(); // consume `class`
        let name = self.expect_ident()?;
        let template_params = self.parse_template_params()?;
        // bases_with_args: (trait_name, concrete_type_args)
        let mut bases_with_args: Vec<(String, Vec<String>)> = Vec::new();
        let mut bases = Vec::new();
        if *self.current() == Token::LParen {
            self.advance();
            while *self.current() != Token::RParen && *self.current() != Token::Eof {
                let base_name = self.expect_ident()?;
                let type_args = self.parse_type_args()?;
                bases_with_args.push((base_name.clone(), type_args));
                bases.push(base_name);
                if *self.current() == Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
            self.eat(&Token::RParen)?;
        }
        // Only traits are allowed as bases; class-to-class inheritance is not supported.
        for base in &bases {
            if !self.known_traits.contains_key(base.as_str()) {
                return Err(format!(
                    "ParseError: class `{name}` cannot inherit from `{base}` (only traits are allowed as bases)"
                ));
            }
        }
        self.eat(&Token::Colon)?;
        let mut body = self.parse_class_body()?;

        // Collect trait required fields and check virtual method overrides.
        // trait_required: (trait_name, field_name, type_ann)
        let mut trait_required: Vec<(String, String, String)> = Vec::new();
        for (base, concrete_args) in &bases_with_args {
            if let Some((trait_tparams, trait_fields, virtual_methods)) = self.known_traits.get(base).cloned() {
                // Build substitution map from trait template params to concrete type args
                let type_map: HashMap<String, String> = trait_tparams.iter()
                    .zip(concrete_args.iter())
                    .map(|(tp, arg)| (tp.name.clone(), arg.clone()))
                    .collect();
                for virt in &virtual_methods {
                    let overridden = body.iter().any(|s| {
                        matches!(s, Stmt::FnDef { name: n, is_virtual: false, .. } if n == virt)
                    });
                    if !overridden {
                        return Err(format!(
                            "StaticTypeError: class `{name}` must override virtual method `{virt}` from trait `{base}`"
                        ));
                    }
                }
                for (fname, _kind, ftype, has_default) in &trait_fields {
                    if !has_default {
                        // Substitute type variables in field type annotation
                        let resolved_type = type_map.get(ftype).cloned().unwrap_or_else(|| ftype.clone());
                        trait_required.push((base.clone(), fname.clone(), resolved_type));
                    }
                }
            }
        }

        // Class's own required fields (mut/let without default).
        let class_required: Vec<(String, String)> = body.iter()
            .filter_map(|s| {
                if let Stmt::Field { name: fname, kind: FieldKind::Mut | FieldKind::Let, type_ann, default: None } = s {
                    Some((fname.clone(), type_ann.clone()))
                } else {
                    None
                }
            })
            .collect();

        // Auto-generate __init__ when there are any required fields (trait + class).
        // Skip generation only when an existing __init__ has exactly the same signature.
        if !trait_required.is_empty() || !class_required.is_empty() {
            let all_required: Vec<(String, String)> = trait_required.iter()
                .map(|(_, fname, ftype)| (fname.clone(), ftype.clone()))
                .chain(class_required.iter().cloned())
                .collect();

            let has_exact_match = body.iter().any(|s| {
                if let Stmt::FnDef { name: n, params, .. } = s {
                    n == "__init__" && Self::init_sig_matches(&all_required, params)
                } else {
                    false
                }
            });

            if !has_exact_match {
                let mut params = vec![Param { name: "self".to_string(), mutable: true, type_ann: None }];
                for (_, fname, ftype) in &trait_required {
                    params.push(Param { name: fname.clone(), mutable: false, type_ann: Some(ftype.clone()) });
                }
                for (fname, ftype) in &class_required {
                    params.push(Param { name: fname.clone(), mutable: false, type_ann: Some(ftype.clone()) });
                }
                let mut init_body: Vec<Stmt> = Vec::new();
                for (tname, fname, _) in &trait_required {
                    init_body.push(Stmt::AttrAssign {
                        target: Expr::TraitAccess {
                            object: Box::new(Expr::Ident("self".to_string())),
                            trait_name: tname.clone(),
                            attr: fname.clone(),
                        },
                        value: Expr::Ident(fname.clone()),
                    });
                }
                for (fname, _) in &class_required {
                    init_body.push(Stmt::AttrAssign {
                        target: Expr::Attr {
                            object: Box::new(Expr::Ident("self".to_string())),
                            attr: fname.clone(),
                        },
                        value: Expr::Ident(fname.clone()),
                    });
                }
                body.push(Stmt::FnDef {
                    name: "__init__".to_string(),
                    template_params: vec![],
                    params,
                    return_type: Some("None".to_string()),
                    body: init_body,
                    is_virtual: false,
                });
            }
        }

        Ok(Stmt::ClassDef { name, template_params, bases, body })
    }

    /// Parses the indented body of a class definition.
    /// Field declarations (`mut`/`let`/`const`) require a type annotation.
    fn parse_class_body(&mut self) -> Result<Vec<Stmt>, String> {
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
            stmts.push(self.parse_class_stmt()?);
        }
        if *self.current() == Token::Dedent {
            self.advance();
        }
        Ok(stmts)
    }

    /// Parses a single statement inside a class body.
    /// Field declarations require `: Type` annotations.
    /// `const` fields are class variables and must include `= default`.
    fn parse_class_stmt(&mut self) -> Result<Stmt, String> {
        match self.current().clone() {
            Token::Mut | Token::Let | Token::Const => {
                let kind = match self.current() {
                    Token::Mut   => FieldKind::Mut,
                    Token::Let   => FieldKind::Let,
                    _            => FieldKind::Const,
                };
                let keyword = match &kind {
                    FieldKind::Mut   => "mut",
                    FieldKind::Let   => "let",
                    FieldKind::Const => "const",
                };
                self.advance();
                let fname = self.expect_ident()?;
                if *self.current() != Token::Colon {
                    return Err(format!(
                        "class field `{fname}` must have a type annotation (e.g., `{keyword} {fname}: int = 0`)"
                    ));
                }
                self.advance(); // consume `:`
                let type_ann = self.parse_type_expr()?;
                let default = if *self.current() == Token::Eq {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                if kind == FieldKind::Const && default.is_none() {
                    return Err(format!(
                        "class variable `{fname}` declared with `const` must have an initial value (e.g., `const {fname}: int = 0`)"
                    ));
                }
                Ok(Stmt::Field { name: fname, kind, type_ann, default })
            }
            Token::Fn => self.parse_fn_def(),
            Token::Gen => self.parse_gen_def(),
            Token::Pass => {
                self.advance();
                Ok(Stmt::Pass)
            }
            tok => Err(format!("unexpected statement in class body: `{tok}`")),
        }
    }

    /// Returns `true` when the given `params` list is an exact signature match for the
    /// default constructor built from `required_fields` (same non-self count and types).
    fn init_sig_matches(required_fields: &[(String, String)], params: &[Param]) -> bool {
        let non_self: Vec<_> = params.iter().filter(|p| p.name != "self").collect();
        non_self.len() == required_fields.len()
            && non_self.iter().zip(required_fields.iter()).all(|(p, (_, ftype))| {
                p.type_ann.as_deref() == Some(ftype.as_str())
            })
    }

    /// Parses optional concrete type arguments at a call/inheritance site: `[Type1, Type2]`.
    /// Returns an empty vec if the current token is not `[`.
    fn parse_type_args(&mut self) -> Result<Vec<String>, String> {
        if *self.current() != Token::LBracket {
            return Ok(vec![]);
        }
        self.advance(); // consume `[`
        let mut args = Vec::new();
        while *self.current() != Token::RBracket && *self.current() != Token::Eof {
            args.push(self.parse_type_expr()?);
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.eat(&Token::RBracket)?;
        Ok(args)
    }

    /// Parses optional template parameters: `[T1: Trait1 and Trait2, T2: Trait3]`.
    /// Returns an empty vec if the current token is not `[`.
    fn parse_template_params(&mut self) -> Result<Vec<TemplateParam>, String> {
        if *self.current() != Token::LBracket {
            return Ok(vec![]);
        }
        self.advance(); // consume `[`
        let mut params = Vec::new();
        while *self.current() != Token::RBracket && *self.current() != Token::Eof {
            let name = self.expect_ident()?;
            self.eat(&Token::Colon)?;
            let mut constraints = vec![self.expect_ident()?];
            // `and`-combined constraints: `T: TraitA and TraitB`
            while *self.current() == Token::And {
                self.advance();
                constraints.push(self.expect_ident()?);
            }
            params.push(TemplateParam { name, constraints });
            if *self.current() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        self.eat(&Token::RBracket)?;
        Ok(params)
    }

    fn parse_param(&mut self) -> Result<Param, String> {
        let mutable = if *self.current() == Token::Mut {
            self.advance();
            true
        } else {
            false
        };
        let name = self.expect_ident()?;
        // Capture optional type annotation: : Type
        let type_ann = if *self.current() == Token::Colon {
            self.advance();
            Some(self.parse_type_expr()?)
        } else {
            None
        };
        Ok(Param { name, mutable, type_ann })
    }

    // Parses a type expression and returns the base type name (generic args are skipped).
    // Accepts identifiers and keyword-tokens that are valid type names (e.g. `None`).
    fn parse_type_expr(&mut self) -> Result<String, String> {
        let base = match self.current().clone() {
            Token::Ident(name) => { self.advance(); name }
            Token::None => { self.advance(); "None".to_string() }
            tok => return Err(format!("expected type name, got `{tok}`")),
        };
        // Skip optional generic parameters: list[int], dict[str, int], etc.
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
        Ok(base)
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
            let span = self.current_span();
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinOp { op: BinOp::Or, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_not()?;
        while *self.current() == Token::And {
            let span = self.current_span();
            self.advance();
            let right = self.parse_not()?;
            left = Expr::BinOp { op: BinOp::And, left: Box::new(left), right: Box::new(right), span };
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
        let span = self.current_span();
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
            return Ok(Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span });
        }
        Ok(left)
    }

    fn parse_bitor(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bitxor()?;
        while *self.current() == Token::Pipe {
            let span = self.current_span();
            self.advance();
            let right = self.parse_bitxor()?;
            left = Expr::BinOp { op: BinOp::BitOr, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_bitxor(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_bitand()?;
        while *self.current() == Token::Caret {
            let span = self.current_span();
            self.advance();
            let right = self.parse_bitand()?;
            left = Expr::BinOp { op: BinOp::BitXor, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_bitand(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_shift()?;
        while *self.current() == Token::Amp {
            let span = self.current_span();
            self.advance();
            let right = self.parse_shift()?;
            left = Expr::BinOp { op: BinOp::BitAnd, left: Box::new(left), right: Box::new(right), span };
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_additive()?;
        loop {
            let span = self.current_span();
            let op = match self.current() {
                Token::LtLt => Some(BinOp::LShift),
                Token::GtGt => Some(BinOp::RShift),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let right = self.parse_additive()?;
                left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let span = self.current_span();
            let op = match self.current() {
                Token::Plus => Some(BinOp::Add),
                Token::Minus => Some(BinOp::Sub),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                let right = self.parse_multiplicative()?;
                left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span };
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let span = self.current_span();
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
                left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span };
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
            let span = self.current_span();
            self.advance();
            let exp = self.parse_unary()?; // right-associative
            Ok(Expr::BinOp { op: BinOp::Pow, left: Box::new(base), right: Box::new(exp), span })
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
                        // Keyword argument: Ident `=` expr  (not `==`)
                        let arg = if let Token::Ident(name) = self.current().clone() {
                            if *self.peek1() == Token::Eq {
                                let name = name.clone();
                                self.advance(); // consume Ident
                                self.advance(); // consume `=`
                                CallArg::Keyword { name, value: self.parse_expr()? }
                            } else {
                                CallArg::Positional(self.parse_expr()?)
                            }
                        } else {
                            CallArg::Positional(self.parse_expr()?)
                        };
                        args.push(arg);
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
                Token::ColonColon => {
                    self.advance(); // consume `::`
                    let trait_name = self.expect_ident()?;
                    self.eat(&Token::Dot)?;
                    let attr = self.expect_ident()?;
                    expr = Expr::TraitAccess { object: Box::new(expr), trait_name, attr };
                }
                Token::LBracket => {
                    // Template instantiation: `expr[T1, T2]`
                    self.advance(); // consume `[`
                    let mut type_args = Vec::new();
                    while *self.current() != Token::RBracket && *self.current() != Token::Eof {
                        type_args.push(self.parse_type_expr()?);
                        if *self.current() == Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    self.eat(&Token::RBracket)?;
                    expr = Expr::TemplateInstantiate { base: Box::new(expr), type_args };
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
        let tokens = Lexer::new(src, "").tokenize();
        Parser::new(tokens).parse_program().expect("parse error")
    }

    fn parse_fails(src: &str) -> String {
        let tokens = Lexer::new(src, "").tokenize();
        Parser::new(tokens).parse_program().expect_err("expected parse error")
    }

    #[test]
    fn test_literal_expr() {
        let stmts = parse("42");
        assert!(matches!(stmts[0], Stmt::Expr(Expr::Int(42))));
    }

    #[test]
    fn test_freeze_stmt() {
        let stmts = parse("mut x = 5\nfreeze x\n");
        assert!(matches!(&stmts[0], Stmt::Mut(name, ..) if name == "x"));
        assert!(matches!(&stmts[1], Stmt::Freeze(name, ..) if name == "x"));
    }

    #[test]
    fn test_freeze_requires_ident() {
        let tokens = crate::lexer::Lexer::new("freeze 42\n", "").tokenize();
        let err = Parser::new(tokens).parse_program().expect_err("expected parse error");
        assert!(err.contains("expected identifier"), "got: {err}");
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
        assert!(matches!(&stmts[1], Stmt::Assign { name, value: Expr::Int(5), .. } if name == "x"));
    }

    #[test]
    fn test_compound_assign() {
        let stmts = parse("mut x = 0\nx += 1");
        assert!(matches!(
            &stmts[1],
            Stmt::CompoundAssign { name, op: BinOp::Add, value: Expr::Int(1), .. } if name == "x"
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
    fn test_class_with_non_trait_base_errors() {
        // Class-to-class inheritance is not allowed; only traits may be listed as bases.
        let err = parse_fails("class Bar(Foo):\n    pass\n");
        assert!(err.contains("cannot inherit from `Foo`"), "got: {err}");
    }

    #[test]
    fn test_class_multiple_non_trait_bases_errors() {
        let err = parse_fails("class C(A, B):\n    pass\n");
        assert!(err.contains("cannot inherit from"), "got: {err}");
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
        // Fields WITH defaults don't produce an auto-init; no auto-init here.
        let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\n    fn move(mut self, dx: int, dy: int) -> None:\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            // 2 fields + 1 method (no auto-init: both fields have defaults)
            assert_eq!(body.len(), 3);
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_field_parsed_as_field_stmt() {
        // Field declarations produce Stmt::Field nodes with type annotation.
        let src = "class Foo:\n    mut x: int = 0\n    let y: str = \"\"\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::Field { name, kind: FieldKind::Mut, type_ann, .. }
                if name == "x" && type_ann == "int"));
            assert!(matches!(&body[1], Stmt::Field { name, kind: FieldKind::Let, type_ann, .. }
                if name == "y" && type_ann == "str"));
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_generated() {
        // Auto __init__ is generated for mut/let fields WITHOUT a default value.
        let src = "class Point:\n    mut x: int\n    mut y: int\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some(), "auto __init__ should be present for required fields");
            if let Some(Stmt::FnDef { params, return_type, .. }) = init {
                assert_eq!(params.len(), 3); // self + x + y
                assert_eq!(params[0].name, "self");
                assert_eq!(params[1].name, "x");
                assert_eq!(params[2].name, "y");
                assert_eq!(params[1].type_ann.as_deref(), Some("int"));
                assert_eq!(return_type.as_deref(), Some("None"));
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_not_generated_all_fields_have_defaults() {
        // No auto-init when all fields have default values.
        let src = "class Point:\n    mut x: int = 0\n    mut y: int = 0\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_none(), "no auto __init__ when all fields have defaults");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_generated_with_list_field() {
        // Fields without defaults always trigger auto-init regardless of type.
        let src = "class Foo:\n    mut items: list[int]\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some(), "auto __init__ should be present for required fields");
            if let Some(Stmt::FnDef { params, .. }) = init {
                assert_eq!(params[1].type_ann.as_deref(), Some("list"));
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_override_exact_match() {
        // Explicit __init__ with same types/count suppresses auto-init (override).
        let src = "class Foo:\n    mut x: int\n    fn __init__(mut self, x: int) -> None:\n        self.x = x\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let inits: Vec<_> = body.iter()
                .filter(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(inits.len(), 1, "exact-match explicit __init__ overrides auto-init");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_overload_different_sig() {
        // Explicit __init__ with different count coexists as overload.
        let src = "class Foo:\n    mut x: int\n    fn __init__(mut self, x: int, y: int) -> None:\n        self.x = x\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let inits: Vec<_> = body.iter()
                .filter(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(inits.len(), 2, "different-sig explicit __init__ + auto-init both present");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_auto_init_not_generated_without_required_fields() {
        // No auto __init__ when class has no fields.
        let src = "class Foo:\n    fn greet(self) -> str:\n        pass\n";
        let stmts = parse(src);
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_none(), "no auto __init__ when there are no required fields");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_class_field_requires_type_annotation() {
        // Field declarations without `: Type` must produce a parse error.
        let result = std::panic::catch_unwind(|| {
            parse("class Foo:\n    mut x = 0\n")
        });
        assert!(result.is_err(), "missing type annotation should cause a parse error");
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

    // --- keyword arguments ---

    #[test]
    fn test_call_positional_args() {
        let stmts = parse("f(1, 2)");
        if let Stmt::Expr(Expr::Call { args, .. }) = &stmts[0] {
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0], CallArg::Positional(_)));
            assert!(matches!(&args[1], CallArg::Positional(_)));
        } else {
            panic!("expected Call");
        }
    }

    #[test]
    fn test_call_keyword_arg() {
        let stmts = parse("f(x=1, y=2)");
        if let Stmt::Expr(Expr::Call { args, .. }) = &stmts[0] {
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0], CallArg::Keyword { name, .. } if name == "x"));
            assert!(matches!(&args[1], CallArg::Keyword { name, .. } if name == "y"));
        } else {
            panic!("expected Call");
        }
    }

    #[test]
    fn test_call_mixed_args() {
        let stmts = parse("f(1, y=2)");
        if let Stmt::Expr(Expr::Call { args, .. }) = &stmts[0] {
            assert_eq!(args.len(), 2);
            assert!(matches!(&args[0], CallArg::Positional(_)));
            assert!(matches!(&args[1], CallArg::Keyword { name, .. } if name == "y"));
        } else {
            panic!("expected Call");
        }
    }

    // --- trait ---

    #[test]
    fn test_trait_empty() {
        let stmts = parse("trait Foo:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::TraitDef { name, .. } if name == "Foo"));
    }

    #[test]
    fn test_trait_with_fields() {
        let stmts = parse("trait HasName:\n    mut name: str\n    let id: int\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::Field { name, kind: FieldKind::Mut, type_ann, .. }
                if name == "name" && type_ann == "str"));
            assert!(matches!(&body[1], Stmt::Field { name, kind: FieldKind::Let, type_ann, .. }
                if name == "id" && type_ann == "int"));
        } else {
            panic!("expected TraitDef");
        }
    }

    #[test]
    fn test_trait_virtual_method_is_virtual() {
        let stmts = parse("trait Animal:\n    fn speak(self) -> str:\n        ...\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::FnDef { name, is_virtual: true, .. } if name == "speak"),
                "method with `...` body should have is_virtual: true");
        } else {
            panic!("expected TraitDef");
        }
    }

    #[test]
    fn test_trait_non_virtual_method_is_not_virtual() {
        let stmts = parse("trait Logger:\n    fn log(self, msg: str) -> None:\n        pass\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::FnDef { name, is_virtual: false, .. } if name == "log"),
                "method with real body should have is_virtual: false");
        } else {
            panic!("expected TraitDef");
        }
    }

    #[test]
    fn test_trait_virtual_body_is_empty() {
        let stmts = parse("trait T:\n    fn f(self) -> int:\n        ...\n");
        if let Stmt::TraitDef { body, .. } = &stmts[0] {
            if let Stmt::FnDef { body: fn_body, is_virtual, .. } = &body[0] {
                assert!(*is_virtual);
                assert!(fn_body.is_empty(), "virtual method body should be empty");
            } else {
                panic!("expected FnDef");
            }
        } else {
            panic!("expected TraitDef");
        }
    }

    #[test]
    fn test_trait_cannot_inherit() {
        let result = std::panic::catch_unwind(|| parse("trait Foo(Bar):\n    pass\n"));
        assert!(result.is_err(), "trait with base class should cause a parse error");
    }

    #[test]
    fn test_class_inherits_trait_ok() {
        let stmts = parse(concat!(
            "trait Animal:\n",
            "    fn speak(self) -> str:\n",
            "        ...\n",
            "class Dog(Animal):\n",
            "    fn speak(self) -> str:\n",
            "        pass\n",
        ));
        assert_eq!(stmts.len(), 2);
        assert!(matches!(&stmts[0], Stmt::TraitDef { name, .. } if name == "Animal"));
        assert!(matches!(&stmts[1], Stmt::ClassDef { name, .. } if name == "Dog"));
    }

    #[test]
    fn test_class_missing_virtual_override_error() {
        let result = std::panic::catch_unwind(|| parse(concat!(
            "trait Animal:\n",
            "    fn speak(self) -> str:\n",
            "        ...\n",
            "class Cat(Animal):\n",
            "    pass\n",
        )));
        assert!(result.is_err(), "missing virtual method override should cause a parse error");
    }

    #[test]
    fn test_class_inherits_trait_combined_init_generated() {
        // TraitDef then ClassDef — combined __init__ takes trait fields first, class fields second.
        let stmts = parse(concat!(
            "trait HasX:\n",
            "    mut x: int\n",
            "class Point(HasX):\n",
            "    mut y: int\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some(), "combined __init__ should be generated");
            if let Some(Stmt::FnDef { params, return_type, .. }) = init {
                // self + x (from trait HasX) + y (from class Point)
                assert_eq!(params.len(), 3);
                assert_eq!(params[0].name, "self");
                assert_eq!(params[1].name, "x");
                assert_eq!(params[2].name, "y");
                assert_eq!(params[1].type_ann.as_deref(), Some("int"));
                assert_eq!(params[2].type_ann.as_deref(), Some("int"));
                assert_eq!(return_type.as_deref(), Some("None"));
            }
        } else {
            panic!("expected ClassDef at stmts[1]");
        }
    }

    #[test]
    fn test_class_inherits_trait_combined_init_body_uses_trait_access() {
        let stmts = parse(concat!(
            "trait HasX:\n",
            "    mut x: int\n",
            "class Point(HasX):\n",
            "    mut y: int\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            if let Some(Stmt::FnDef { body: init_body, .. }) =
                body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"))
            {
                // First assignment must be TraitAccess (trait field x)
                assert!(matches!(&init_body[0],
                    Stmt::AttrAssign { target: Expr::TraitAccess { trait_name, attr, .. }, .. }
                    if trait_name == "HasX" && attr == "x"
                ), "trait field assignment should use TraitAccess");
                // Second assignment must be regular Attr (class field y)
                assert!(matches!(&init_body[1],
                    Stmt::AttrAssign { target: Expr::Attr { attr, .. }, .. }
                    if attr == "y"
                ), "class field assignment should use Attr");
            } else {
                panic!("__init__ not found");
            }
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_trait_access_expr_parsed() {
        // self::MyTrait.field — 括弧なし形式のみ有効
        let stmts = parse("self::MyTrait.field\n");
        if let Stmt::Expr(Expr::TraitAccess { trait_name, attr, .. }) = &stmts[0] {
            assert_eq!(trait_name, "MyTrait");
            assert_eq!(attr, "field");
        } else {
            panic!("expected Stmt::Expr(Expr::TraitAccess)");
        }
    }

    #[test]
    fn test_fn_is_not_virtual_by_default() {
        let stmts = parse("fn hello() -> None:\n    pass\n");
        assert!(matches!(&stmts[0], Stmt::FnDef { is_virtual: false, .. }));
    }

    #[test]
    fn test_class_method_is_not_virtual() {
        let stmts = parse("class Foo:\n    fn greet(self) -> str:\n        pass\n");
        if let Stmt::ClassDef { body, .. } = &stmts[0] {
            assert!(matches!(&body[0], Stmt::FnDef { name, is_virtual: false, .. } if name == "greet"));
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_trait_combined_init_override_by_exact_match() {
        // Explicit __init__ with same sig as combined auto-init suppresses generation.
        let stmts = parse(concat!(
            "trait HasX:\n",
            "    mut x: int\n",
            "class Foo(HasX):\n",
            "    mut y: int\n",
            "    fn __init__(mut self, x: int, y: int) -> None:\n",
            "        pass\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            let inits: Vec<_> = body.iter()
                .filter(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"))
                .collect();
            assert_eq!(inits.len(), 1, "exact-match explicit __init__ should override auto-init");
        } else {
            panic!("expected ClassDef");
        }
    }

    #[test]
    fn test_trait_with_multiple_virtual_methods_all_must_be_overridden() {
        // All virtual methods must be overridden.
        let result = std::panic::catch_unwind(|| parse(concat!(
            "trait Ops:\n",
            "    fn add(self, x: int) -> int:\n",
            "        ...\n",
            "    fn sub(self, x: int) -> int:\n",
            "        ...\n",
            "class MyOps(Ops):\n",
            "    fn add(self, x: int) -> int:\n",    // overrides add
            "        pass\n",
            // sub NOT overridden → error
        )));
        assert!(result.is_err(), "not overriding all virtual methods should be a parse error");
    }

    #[test]
    fn test_trait_class_only_trait_required_fields_no_class_fields() {
        // Class has no own required fields; combined init takes only trait fields.
        let stmts = parse(concat!(
            "trait Named:\n",
            "    mut name: str\n",
            "class Widget(Named):\n",
            "    pass\n",
        ));
        if let Stmt::ClassDef { body, .. } = &stmts[1] {
            let init = body.iter().find(|s| matches!(s, Stmt::FnDef { name, .. } if name == "__init__"));
            assert!(init.is_some());
            if let Some(Stmt::FnDef { params, .. }) = init {
                assert_eq!(params.len(), 2); // self + name
                assert_eq!(params[1].name, "name");
            }
        } else {
            panic!("expected ClassDef");
        }
    }
}
