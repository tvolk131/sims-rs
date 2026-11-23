//! Recursive-descent parser for the two-tier `.antics` language.

use crate::ast::{
    AssignOp, BconDecl, Branches, CallTarget, CompareOp, Condition, Item, LocalRef, Module,
    Operand, OperandRef, PrimitiveCall, Stmt, StrDecl, StrEntryDecl, StringRef, Target, TreeDecl,
    TreeHeader,
};
use crate::error::{Error, Result, Span};
use crate::lexer::{Token, TokenKind, tokenize};

/// Maximum structural nesting depth for blocks and control flow in `.antics`.
const MAX_NESTING_DEPTH: usize = 64;

/// Parses a `.antics` source string into a [`Module`] AST.
pub fn parse_module(source: &str) -> Result<Module> {
    let tokens = tokenize(source)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        depth: 0,
    };
    parser.parse_module()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .unwrap_or_else(|| self.tokens.last().expect("lexer emits Eof"))
    }

    fn peek_at(&self, offset: usize) -> &Token {
        self.tokens
            .get(self.pos + offset)
            .unwrap_or_else(|| self.tokens.last().expect("lexer emits Eof"))
    }

    fn bump(&mut self) -> Token {
        let tok = self.peek().clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn span(&self) -> Span {
        self.peek().span
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if &self.peek().kind == kind {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, description: &str) -> Result<Span> {
        let tok = self.peek().clone();
        if &tok.kind == kind {
            self.bump();
            Ok(tok.span)
        } else {
            Err(Error::at(
                tok.span,
                format!("expected {description}, found {:?}", tok.kind),
            ))
        }
    }

    fn expect_ident(&mut self, description: &str) -> Result<(String, Span)> {
        let tok = self.bump();
        match tok.kind {
            TokenKind::Ident(s) => Ok((s, tok.span)),
            other => Err(Error::at(
                tok.span,
                format!("expected {description}, found {other:?}"),
            )),
        }
    }

    fn expect_string(&mut self, description: &str) -> Result<(String, Span)> {
        let tok = self.bump();
        match tok.kind {
            TokenKind::Str(s) => Ok((s, tok.span)),
            other => Err(Error::at(
                tok.span,
                format!("expected {description}, found {other:?}"),
            )),
        }
    }

    fn expect_int(&mut self, description: &str) -> Result<(i64, Span)> {
        let tok = self.bump();
        match tok.kind {
            TokenKind::Int(v) => Ok((v, tok.span)),
            other => Err(Error::at(
                tok.span,
                format!("expected {description}, found {other:?}"),
            )),
        }
    }

    fn expect_u8(&mut self, description: &str) -> Result<u8> {
        let (v, span) = self.expect_int(description)?;
        u8::try_from(v).map_err(|_| Error::at(span, format!("{description} {v} out of u8 range")))
    }

    fn expect_i8(&mut self, description: &str) -> Result<i8> {
        let (v, span) = self.expect_int(description)?;
        i8::try_from(v).map_err(|_| Error::at(span, format!("{description} {v} out of i8 range")))
    }

    fn expect_u16(&mut self, description: &str) -> Result<u16> {
        let (v, span) = self.expect_int(description)?;
        u16::try_from(v).map_err(|_| Error::at(span, format!("{description} {v} out of u16 range")))
    }

    fn expect_i16(&mut self, description: &str) -> Result<i16> {
        let (v, span) = self.expect_int(description)?;
        i16::try_from(v).map_err(|_| Error::at(span, format!("{description} {v} out of i16 range")))
    }

    fn expect_u32(&mut self, description: &str) -> Result<u32> {
        let (v, span) = self.expect_int(description)?;
        u32::try_from(v).map_err(|_| Error::at(span, format!("{description} {v} out of u32 range")))
    }

    fn parse_module(&mut self) -> Result<Module> {
        let mut items = Vec::new();
        while self.peek().kind != TokenKind::Eof {
            let (kw, span) =
                self.expect_ident("top-level keyword ('tree', 'interaction', 'bcon', 'str')")?;
            match kw.as_str() {
                "tree" => items.push(Item::Tree(self.parse_tree(span, false)?)),
                "interaction" => items.push(Item::Tree(self.parse_tree(span, true)?)),
                "bcon" => items.push(Item::Bcon(self.parse_bcon(span)?)),
                "str" => items.push(Item::Str(self.parse_str_table(span)?)),
                other => {
                    return Err(Error::at(
                        span,
                        format!("unknown top-level declaration '{other}'"),
                    ));
                }
            }
        }
        Ok(Module { items })
    }

    fn parse_bcon(&mut self, span: Span) -> Result<BconDecl> {
        let id = self.expect_u16("BCON chunk id")?;
        let label = if let TokenKind::Str(s) = &self.peek().kind {
            let s = s.clone();
            self.bump();
            s
        } else {
            format!("BCON_{id}")
        };
        let mut flags = 0u8;
        if self.eat(&TokenKind::LParen) {
            while !self.eat(&TokenKind::RParen) {
                let (key, kspan) = self.expect_ident("BCON header key")?;
                self.expect(&TokenKind::Colon, "':'")?;
                match key.as_str() {
                    "flags" => flags = self.expect_u8("BCON flags")?,
                    other => {
                        return Err(Error::at(kspan, format!("unknown BCON option '{other}'")));
                    }
                }
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RParen, "')'")?;
                    break;
                }
            }
        }
        let mut values = Vec::new();
        let mut entry_names = Vec::new();
        if self.eat(&TokenKind::LBrace) {
            while !self.eat(&TokenKind::RBrace) {
                let (name, _nspan) = self.expect_ident("named BCON constant")?;
                self.expect(&TokenKind::Assign, "'='")?;
                let val = self.expect_i16("BCON constant value")?;
                values.push(val);
                entry_names.push(Some(name));
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RBrace, "'}'")?;
                    break;
                }
            }
            self.eat(&TokenKind::Semicolon);
        } else {
            self.expect(&TokenKind::Assign, "'='")?;
            self.expect(&TokenKind::LBracket, "'['")?;
            while !self.eat(&TokenKind::RBracket) {
                values.push(self.expect_i16("BCON constant value")?);
                entry_names.push(None);
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RBracket, "']'")?;
                    break;
                }
            }
            self.eat(&TokenKind::Semicolon);
        }
        Ok(BconDecl {
            span,
            id,
            label,
            flags,
            values,
            entry_names,
        })
    }

    fn parse_str_table(&mut self, span: Span) -> Result<StrDecl> {
        let id = self.expect_u16("STR# chunk id")?;
        let label = if let TokenKind::Str(s) = &self.peek().kind {
            let s = s.clone();
            self.bump();
            s
        } else {
            format!("STR_{id}")
        };
        let mut entries = Vec::new();
        if self.eat(&TokenKind::LBrace) {
            while !self.eat(&TokenKind::RBrace) {
                if let TokenKind::Str(_) = &self.peek().kind {
                    let (text, espan) = self.expect_string("string table entry")?;
                    entries.push(StrEntryDecl {
                        span: espan,
                        name: None,
                        text,
                        comment: String::new(),
                    });
                } else {
                    let (name, nspan) = self.expect_ident("named string entry")?;
                    self.expect(&TokenKind::Assign, "'='")?;
                    let (text, _) = self.expect_string("string entry value")?;
                    entries.push(StrEntryDecl {
                        span: nspan,
                        name: Some(name),
                        text,
                        comment: String::new(),
                    });
                }
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RBrace, "'}'")?;
                    break;
                }
            }
            self.eat(&TokenKind::Semicolon);
        } else {
            self.expect(&TokenKind::Assign, "'='")?;
            self.expect(&TokenKind::LBracket, "'['")?;
            while !self.eat(&TokenKind::RBracket) {
                let (text, espan) = self.expect_string("string table entry")?;
                entries.push(StrEntryDecl {
                    span: espan,
                    name: None,
                    text,
                    comment: String::new(),
                });
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RBracket, "']'")?;
                    break;
                }
            }
            self.eat(&TokenKind::Semicolon);
        }
        Ok(StrDecl {
            span,
            id,
            label,
            entries,
        })
    }

    fn parse_tree(&mut self, span: Span, is_interaction: bool) -> Result<TreeDecl> {
        let id = self.expect_u16("BHAV tree id")?;
        let label = if let TokenKind::Str(s) = &self.peek().kind {
            let s = s.clone();
            self.bump();
            s
        } else {
            format!("Tree_{id}")
        };
        let mut header = TreeHeader::default();
        if self.eat(&TokenKind::LParen) {
            while !self.eat(&TokenKind::RParen) {
                let (key, kspan) = self.expect_ident("tree header field")?;
                self.expect(&TokenKind::Colon, "':'")?;
                match key.as_str() {
                    "format" => header.format = self.expect_u16("BHAV format")?,
                    "kind" => header.kind = self.expect_u8("BHAV kind")?,
                    "args" => header.args = Some(self.expect_u8("BHAV args")?),
                    "locals" => header.locals = Some(self.expect_u16("BHAV locals")?),
                    "flags" => header.flags = self.expect_u16("BHAV flags")?,
                    "revision" => header.revision = self.expect_u32("BHAV revision")?,
                    "trailing" => {
                        self.expect(&TokenKind::LBracket, "'['")?;
                        let mut bytes = Vec::new();
                        while !self.eat(&TokenKind::RBracket) {
                            bytes.push(self.expect_u8("BHAV trailing byte")?);
                            if !self.eat(&TokenKind::Comma) {
                                self.expect(&TokenKind::RBracket, "']'")?;
                                break;
                            }
                        }
                        header.trailing = bytes;
                    }
                    "id" => {
                        let explicit = self.expect_u16("BHAV id")?;
                        if explicit != id {
                            return Err(Error::at(
                                kspan,
                                format!("header id {explicit} disagrees with tree id {id}"),
                            ));
                        }
                    }
                    other => {
                        return Err(Error::at(
                            kspan,
                            format!("unknown tree header option '{other}'"),
                        ));
                    }
                }
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RParen, "')'")?;
                    break;
                }
            }
        }
        let body = self.parse_block()?;
        Ok(TreeDecl {
            span,
            is_interaction,
            id,
            label,
            header,
            body,
        })
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>> {
        let span = self.expect(&TokenKind::LBrace, "'{'")?;
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(Error::at(
                span,
                format!("block nesting exceeds maximum depth of {MAX_NESTING_DEPTH}"),
            ));
        }
        self.depth += 1;
        let mut stmts = Vec::new();
        while !self.eat(&TokenKind::RBrace) {
            if self.peek().kind == TokenKind::Eof {
                self.depth -= 1;
                return Err(Error::at(self.span(), "unexpected EOF inside block"));
            }
            match self.parse_stmt() {
                Ok(stmt) => stmts.push(stmt),
                Err(err) => {
                    self.depth -= 1;
                    return Err(err);
                }
            }
        }
        self.depth -= 1;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt> {
        let tok = self.peek().clone();
        match &tok.kind {
            TokenKind::Label(name) => {
                let name = name.clone();
                let span = tok.span;
                self.bump();
                self.expect(&TokenKind::Colon, "':' after label")?;
                Ok(Stmt::Label { span, name })
            }
            TokenKind::Ident(ident) => {
                // Support bare label syntax `name:` as well as `@name:`.
                if self.peek_at(1).kind == TokenKind::Colon {
                    let name = ident.clone();
                    let span = tok.span;
                    self.bump();
                    self.bump();
                    return Ok(Stmt::Label { span, name });
                }
                match ident.as_str() {
                    "op" => self.parse_raw_op(),
                    "let" => self.parse_let_stmt(),
                    "local" if self.peek_at(1).kind != TokenKind::LBracket => self.parse_let_stmt(),
                    "if" => self.parse_if(),
                    "while" => self.parse_while(),
                    "break" => {
                        self.bump();
                        self.expect(&TokenKind::Semicolon, "';'")?;
                        Ok(Stmt::Break { span: tok.span })
                    }
                    "continue" => {
                        self.bump();
                        self.expect(&TokenKind::Semicolon, "';'")?;
                        Ok(Stmt::Continue { span: tok.span })
                    }
                    "return" => self.parse_return(),
                    "idle" => self.parse_idle(),
                    "call" | "subroutine" => self.parse_call_stmt(),
                    "sleep"
                    | "generic_call"
                    | "expr"
                    | "random"
                    | "find_best_object"
                    | "idle_for_input"
                    | "remove_object"
                    | "run_functional_tree"
                    | "play_sound"
                    | "alter_budget"
                    | "stop_all_sounds"
                    | "set_motive_change"
                    | "clear_motive_changes"
                    | "set_to_next"
                    | "dialog"
                    | "create_object"
                    | "animate"
                    | "route"
                    | "route_to"
                    | "snap" => self.parse_primitive_stmt(),
                    _ => self.parse_assign_stmt(),
                }
            }
            other => Err(Error::at(
                tok.span,
                format!("unexpected token {other:?} at start of statement"),
            )),
        }
    }

    fn parse_let_stmt(&mut self) -> Result<Stmt> {
        let (_, span) = self.expect_ident("'let' or 'local'")?;
        let (name, nspan) = self.expect_ident("local variable name")?;
        if is_reserved_ident(&name) {
            return Err(Error::at(
                nspan,
                format!("cannot use reserved keyword '{name}' as a local variable name"),
            ));
        }
        let init = if self.eat(&TokenKind::Assign) {
            Some(self.parse_operand()?)
        } else {
            None
        };
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(Stmt::Let { span, name, init })
    }

    fn parse_raw_op(&mut self) -> Result<Stmt> {
        let (_, span) = self.expect_ident("'op'")?;
        let opcode = self.expect_u16("opcode")?;
        self.expect(&TokenKind::LParen, "'('")?;
        let mut operands = [0u8; 8];
        for (i, slot) in operands.iter_mut().enumerate() {
            *slot = self.expect_u8("operand byte")?;
            if i < 7 {
                self.expect(&TokenKind::Comma, "',' between operand bytes")?;
            }
        }
        self.eat(&TokenKind::Comma);
        self.expect(&TokenKind::RParen, "')'")?;
        let branches = self.parse_branches()?;
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(Stmt::RawOp {
            span,
            opcode,
            operands,
            branches,
        })
    }

    fn parse_branches(&mut self) -> Result<Branches> {
        self.expect(&TokenKind::Arrow, "'->'")?;
        let (first_kw, fspan) = self.expect_ident("'true'")?;
        if first_kw != "true" {
            return Err(Error::at(fspan, "expected 'true:' branch after '->'"));
        }
        self.expect(&TokenKind::Colon, "':'")?;
        let on_true = self.parse_target()?;
        self.expect(&TokenKind::Comma, "','")?;
        let (second_kw, sspan) = self.expect_ident("'false'")?;
        if second_kw != "false" {
            return Err(Error::at(sspan, "expected 'false:' branch"));
        }
        self.expect(&TokenKind::Colon, "':'")?;
        let on_false = self.parse_target()?;
        Ok(Branches { on_true, on_false })
    }

    fn parse_optional_branches(&mut self) -> Result<Option<Branches>> {
        if self.peek().kind == TokenKind::Arrow {
            Ok(Some(self.parse_branches()?))
        } else {
            Ok(None)
        }
    }

    fn parse_target(&mut self) -> Result<Target> {
        let tok = self.bump();
        match tok.kind {
            TokenKind::Label(name) => Ok(Target::Label(name)),
            TokenKind::Ident(s) => match s.as_str() {
                "true" => Ok(Target::ReturnTrue),
                "false" => Ok(Target::ReturnFalse),
                "error" => Ok(Target::Error),
                "raw" if self.eat(&TokenKind::LParen) => {
                    let b = self.expect_u8("raw branch target")?;
                    self.expect(&TokenKind::RParen, "')'")?;
                    Ok(Target::Raw(b))
                }
                _ => Ok(Target::Label(s)),
            },
            TokenKind::Int(v) => match v {
                253 => Ok(Target::Error),
                254 => Ok(Target::ReturnTrue),
                255 => Ok(Target::ReturnFalse),
                0..=250 => Ok(Target::Node(v as u8)),
                _ => Err(Error::at(
                    tok.span,
                    format!(
                        "invalid branch target {v} (valid: 0..=250, 253=error, 254=true, 255=false)"
                    ),
                )),
            },
            other => Err(Error::at(
                tok.span,
                format!("expected branch target, found {other:?}"),
            )),
        }
    }

    fn parse_if(&mut self) -> Result<Stmt> {
        let (_, span) = self.expect_ident("'if'")?;
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(Error::at(
                span,
                format!("conditional nesting exceeds maximum depth of {MAX_NESTING_DEPTH}"),
            ));
        }
        self.depth += 1;
        let result = (|| {
            let cond = self.parse_condition()?;
            let then_body = self.parse_block()?;
            let else_body = if matches!(&self.peek().kind, TokenKind::Ident(s) if s == "else") {
                self.bump();
                if matches!(&self.peek().kind, TokenKind::Ident(s) if s == "if") {
                    vec![self.parse_if()?]
                } else {
                    self.parse_block()?
                }
            } else {
                Vec::new()
            };
            Ok(Stmt::If {
                span,
                cond,
                then_body,
                else_body,
            })
        })();
        self.depth -= 1;
        result
    }

    fn parse_while(&mut self) -> Result<Stmt> {
        let (_, span) = self.expect_ident("'while'")?;
        let cond = self.parse_condition()?;
        let body = self.parse_block()?;
        Ok(Stmt::While { span, cond, body })
    }

    fn parse_return(&mut self) -> Result<Stmt> {
        let (_, span) = self.expect_ident("'return'")?;
        let target = match &self.peek().kind {
            TokenKind::Ident(s) if s == "true" => {
                self.bump();
                Target::ReturnTrue
            }
            TokenKind::Ident(s) if s == "false" => {
                self.bump();
                Target::ReturnFalse
            }
            TokenKind::Ident(s) if s == "error" => {
                self.bump();
                Target::Error
            }
            other => {
                return Err(Error::at(
                    self.span(),
                    format!("expected 'true', 'false', or 'error' after 'return', found {other:?}"),
                ));
            }
        };
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(Stmt::Return { span, target })
    }

    fn parse_idle(&mut self) -> Result<Stmt> {
        let (_, span) = self.expect_ident("'idle'")?;
        self.expect(&TokenKind::LParen, "'('")?;
        let ticks = self.parse_operand()?;
        self.expect(&TokenKind::RParen, "')'")?;
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(Stmt::Idle { span, ticks })
    }

    fn parse_call_args_and_target(&mut self) -> Result<(CallTarget, Vec<i16>, bool)> {
        let target = match &self.peek().kind {
            TokenKind::Int(_) => CallTarget::Id(self.expect_u16("tree id")?),
            TokenKind::Str(s) => {
                let name = s.clone();
                self.bump();
                CallTarget::Name(name)
            }
            TokenKind::Ident(s) => {
                let name = s.clone();
                self.bump();
                CallTarget::Name(name)
            }
            other => {
                return Err(Error::at(
                    self.span(),
                    format!("expected tree id or name in call, found {other:?}"),
                ));
            }
        };
        let mut args = Vec::new();
        let mut pass_temps = false;
        if self.eat(&TokenKind::LParen) {
            while !self.eat(&TokenKind::RParen) {
                if matches!(&self.peek().kind, TokenKind::Ident(s) if s == "temps") {
                    let tspan = self.span();
                    if !args.is_empty() {
                        return Err(Error::at(
                            tspan,
                            "'temps' cannot be mixed with literal call arguments",
                        ));
                    }
                    self.bump();
                    pass_temps = true;
                    self.expect(&TokenKind::RParen, "')' after 'temps'")?;
                    break;
                }
                args.push(self.expect_i16("call argument")?);
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RParen, "')'")?;
                    break;
                }
            }
        }
        Ok((target, args, pass_temps))
    }

    fn parse_call_stmt(&mut self) -> Result<Stmt> {
        let (kw, span) = self.expect_ident("'call' or 'subroutine'")?;
        let (target, args, pass_temps) = if kw == "subroutine" && self.eat(&TokenKind::LParen) {
            let t = match &self.peek().kind {
                TokenKind::Int(_) => CallTarget::Id(self.expect_u16("subroutine id")?),
                TokenKind::Str(s) => {
                    let s = s.clone();
                    self.bump();
                    CallTarget::Name(s)
                }
                TokenKind::Ident(s) => {
                    let s = s.clone();
                    self.bump();
                    CallTarget::Name(s)
                }
                other => {
                    return Err(Error::at(
                        self.span(),
                        format!("expected subroutine id or name, found {other:?}"),
                    ));
                }
            };
            let mut args = Vec::new();
            let mut pass_temps = false;
            while self.eat(&TokenKind::Comma) {
                if pass_temps {
                    return Err(Error::at(
                        self.span(),
                        "'temps' cannot be followed by additional subroutine arguments",
                    ));
                }
                if matches!(&self.peek().kind, TokenKind::Ident(s) if s == "temps") {
                    let tspan = self.span();
                    if !args.is_empty() {
                        return Err(Error::at(
                            tspan,
                            "'temps' cannot be mixed with literal subroutine arguments",
                        ));
                    }
                    self.bump();
                    pass_temps = true;
                    continue;
                }
                args.push(self.expect_i16("subroutine argument")?);
            }
            self.expect(&TokenKind::RParen, "')'")?;
            (t, args, pass_temps)
        } else {
            self.parse_call_args_and_target()?
        };
        let branches = self.parse_optional_branches()?;
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(Stmt::Call {
            span,
            target,
            args,
            pass_temps,
            branches,
        })
    }

    fn parse_primitive_stmt(&mut self) -> Result<Stmt> {
        let span = self.span();
        let prim = self.parse_primitive_call()?;
        let branches = self.parse_optional_branches()?;
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(Stmt::Primitive {
            span,
            prim,
            branches,
        })
    }

    fn parse_string_ref(&mut self, description: &str) -> Result<StringRef> {
        match &self.peek().kind {
            TokenKind::Int(_) => Ok(StringRef::Index(self.expect_u8(description)?)),
            TokenKind::Ident(s) if s == "str" => {
                self.bump();
                self.expect(&TokenKind::Dot, "'.'")?;
                let (name, _) = self.expect_ident("string entry name")?;
                Ok(StringRef::Named(name))
            }
            other => Err(Error::at(
                self.span(),
                format!("expected {description} (u8 or str.<name>), found {other:?}"),
            )),
        }
    }

    fn parse_local_ref(&mut self, description: &str) -> Result<LocalRef> {
        match &self.peek().kind {
            TokenKind::Int(_) => Ok(LocalRef::Index(self.expect_u8(description)?)),
            TokenKind::Ident(s) if s == "local" && self.peek_at(1).kind == TokenKind::LBracket => {
                self.bump();
                self.expect(&TokenKind::LBracket, "'['")?;
                let idx = self.expect_u8(description)?;
                self.expect(&TokenKind::RBracket, "']'")?;
                Ok(LocalRef::Index(idx))
            }
            TokenKind::Ident(_) => {
                let (name, _) = self.expect_ident(description)?;
                Ok(LocalRef::Named(name))
            }
            other => Err(Error::at(
                self.span(),
                format!("expected {description}, found {other:?}"),
            )),
        }
    }

    fn parse_function_index(&mut self) -> Result<u16> {
        match &self.peek().kind {
            TokenKind::Int(_) => self.expect_u16("function index"),
            TokenKind::Ident(s) => {
                let name = s.clone();
                let span = self.span();
                self.bump();
                match name.as_str() {
                    "prep" => Ok(0),
                    "cook" => Ok(1),
                    "surface" => Ok(2),
                    "dispose" => Ok(3),
                    "eat" => Ok(6),
                    "pick_up" => Ok(7),
                    "sit" => Ok(8),
                    "clean" => Ok(10),
                    "repair" => Ok(11),
                    "serve" => Ok(12),
                    "rest" => Ok(14),
                    "category4" => Ok(15),
                    other => Err(Error::at(span, format!("unknown function name '{other}'"))),
                }
            }
            other => Err(Error::at(
                self.span(),
                format!("expected function index or name, found {other:?}"),
            )),
        }
    }

    fn parse_set_to_next_mode(&mut self) -> Result<u8> {
        match &self.peek().kind {
            TokenKind::Int(_) => self.expect_u8("set_to_next mode"),
            TokenKind::Ident(s) => {
                let name = s.clone();
                let span = self.span();
                self.bump();
                match name.as_str() {
                    "all" => Ok(0),
                    "person" => Ok(1),
                    "non_person" => Ok(2),
                    "part" | "tile" => Ok(3),
                    "guid" => Ok(4),
                    "neighbor" => Ok(5),
                    "category" => Ok(6),
                    "neighbor_guid" => Ok(7),
                    "on_tile" => Ok(8),
                    "beside" => Ok(9),
                    "career" => Ok(10),
                    "family" => Ok(12),
                    other => Err(Error::at(
                        span,
                        format!("unknown set_to_next mode '{other}'"),
                    )),
                }
            }
            other => Err(Error::at(
                self.span(),
                format!("expected set_to_next mode, found {other:?}"),
            )),
        }
    }

    fn parse_create_mode(&mut self) -> Result<u8> {
        match &self.peek().kind {
            TokenKind::Int(_) => self.expect_u8("create_object mode"),
            TokenKind::Ident(s) => {
                let name = s.clone();
                let span = self.span();
                self.bump();
                match name.as_str() {
                    "ahead" => Ok(0),
                    "my_slot" => Ok(1),
                    "stack_ahead" => Ok(3),
                    "stack_slot" => Ok(4),
                    "here" => Ok(5),
                    "off_lot" => Ok(6),
                    "param_pos" => Ok(7),
                    "local_pos" => Ok(8),
                    "turned" => Ok(9),
                    other => Err(Error::at(
                        span,
                        format!("unknown create_object mode '{other}'"),
                    )),
                }
            }
            other => Err(Error::at(
                self.span(),
                format!("expected create_object mode, found {other:?}"),
            )),
        }
    }

    fn parse_dialog_kind(&mut self) -> Result<u8> {
        match &self.peek().kind {
            TokenKind::Int(_) => self.expect_u8("dialog kind"),
            TokenKind::Ident(s) => {
                let name = s.clone();
                let span = self.span();
                self.bump();
                match name.as_str() {
                    "ok" | "message" => Ok(0),
                    "yes_no" => Ok(1),
                    "yes_no_cancel" | "three_choice" => Ok(2),
                    "text_entry" => Ok(3),
                    other => Err(Error::at(span, format!("unknown dialog kind '{other}'"))),
                }
            }
            other => Err(Error::at(
                self.span(),
                format!("expected dialog kind, found {other:?}"),
            )),
        }
    }

    fn parse_primitive_call(&mut self) -> Result<PrimitiveCall> {
        let (name, span) = self.expect_ident("primitive name")?;
        self.expect(&TokenKind::LParen, "'('")?;
        let prim = match name.as_str() {
            "sleep" => {
                let op = self.parse_operand()?;
                let OperandRef::Direct(direct) = op else {
                    return Err(Error::at(
                        span,
                        "sleep() expects a parameter index (param[N] or integer N)",
                    ));
                };
                if direct.scope != 9 && direct.scope != 7 {
                    return Err(Error::at(
                        span,
                        "sleep() expects a parameter index (param[N] or integer N)",
                    ));
                }
                PrimitiveCall::Sleep {
                    param_index: direct.index,
                }
            }
            "generic_call" => {
                let routine = self.expect_u16("generic sims call routine")?;
                PrimitiveCall::GenericCall { routine }
            }
            "expr" => {
                let lhs = self.parse_operand()?;
                let operator = self.parse_any_expr_op()?;
                let rhs = self.parse_operand()?;
                PrimitiveCall::ExprRaw { lhs, operator, rhs }
            }
            "random" => {
                let dst = self.parse_operand()?;
                self.expect(&TokenKind::Comma, "','")?;
                let range = self.parse_operand()?;
                PrimitiveCall::Random { dst, range }
            }
            "find_best_object" => {
                let function = self.parse_function_index()?;
                let mut flags = 0u8;
                if self.eat(&TokenKind::Comma) {
                    if matches!(&self.peek().kind, TokenKind::Ident(s) if s == "flags")
                        && self.peek_at(1).kind == TokenKind::Colon
                    {
                        self.bump();
                        self.bump();
                    }
                    flags = self.expect_u8("find_best_object flags")?;
                }
                PrimitiveCall::FindBestObject { function, flags }
            }
            "idle_for_input" => {
                let op = self.parse_operand()?;
                let OperandRef::Direct(direct) = op else {
                    return Err(Error::at(
                        span,
                        "idle_for_input() expects a parameter index (param[N] or integer N)",
                    ));
                };
                if direct.scope != 9 && direct.scope != 7 {
                    return Err(Error::at(
                        span,
                        "idle_for_input() expects a parameter index (param[N] or integer N)",
                    ));
                }
                let allow_push = if self.eat(&TokenKind::Comma) {
                    self.expect_u16("allow_push flag")?
                } else {
                    0
                };
                PrimitiveCall::IdleForInput {
                    param_index: direct.index,
                    allow_push,
                }
            }
            "remove_object" => {
                let target = match &self.peek().kind {
                    TokenKind::Int(_) => self.expect_u16("remove_object target")?,
                    TokenKind::Ident(s) => {
                        let name = s.clone();
                        let tspan = self.span();
                        self.bump();
                        match name.as_str() {
                            "me" | "self" => 0,
                            "stack_obj" => 1,
                            other => {
                                return Err(Error::at(
                                    tspan,
                                    format!(
                                        "unknown remove_object target '{other}' (expected me, stack_obj, 0, 1, or 2)"
                                    ),
                                ));
                            }
                        }
                    }
                    other => {
                        return Err(Error::at(
                            self.span(),
                            format!("expected remove_object target, found {other:?}"),
                        ));
                    }
                };
                let mut flags = 0u8;
                while self.eat(&TokenKind::Comma) {
                    if matches!(&self.peek().kind, TokenKind::Ident(_))
                        && self.peek_at(1).kind == TokenKind::Colon
                    {
                        let (k, kspan) = self.expect_ident("remove_object option")?;
                        self.expect(&TokenKind::Colon, "':'")?;
                        match k.as_str() {
                            "flags" => flags |= self.expect_u8("remove_object flags")?,
                            "no_yield" => {
                                if self.parse_bool()? {
                                    flags |= 1;
                                }
                            }
                            "skip_resets" => {
                                if self.parse_bool()? {
                                    flags |= 2;
                                }
                            }
                            other => {
                                return Err(Error::at(
                                    kspan,
                                    format!("unknown remove_object parameter '{other}'"),
                                ));
                            }
                        }
                    } else {
                        flags = self.expect_u8("remove_object flags")?;
                    }
                }
                PrimitiveCall::RemoveObject { target, flags }
            }
            "run_functional_tree" => {
                let function = self.parse_function_index()?;
                let mut change_icon = false;
                if self.eat(&TokenKind::Comma) {
                    if matches!(&self.peek().kind, TokenKind::Ident(s) if s == "change_icon")
                        && self.peek_at(1).kind == TokenKind::Colon
                    {
                        self.bump();
                        self.bump();
                    }
                    change_icon = self.parse_bool()?;
                }
                PrimitiveCall::RunFunctionalTree {
                    function,
                    change_icon,
                }
            }
            "play_sound" => {
                let sound_id = self.expect_u16("FWAV sound id")?;
                let mut use_stack_object = false;
                if self.eat(&TokenKind::Comma) {
                    if matches!(&self.peek().kind, TokenKind::Ident(s) if s == "stack_obj") {
                        self.bump();
                        if self.eat(&TokenKind::Colon) {
                            use_stack_object = self.parse_bool()?;
                        } else {
                            use_stack_object = true;
                        }
                    } else {
                        use_stack_object = self.parse_bool()?;
                    }
                }
                PrimitiveCall::PlaySound {
                    sound_id,
                    use_stack_object,
                }
            }
            "alter_budget" => {
                let amount = self.parse_operand()?;
                let mut category = 0u8;
                let mut flags = 0u8;
                while self.eat(&TokenKind::Comma) {
                    let (k, kspan) = self.expect_ident("alter_budget option")?;
                    self.expect(&TokenKind::Colon, "':'")?;
                    match k.as_str() {
                        "category" => category = self.expect_u8("budget category")?,
                        "flags" => flags |= self.expect_u8("alter_budget flags")?,
                        "test_only" => {
                            if self.parse_bool()? {
                                flags |= 1;
                            }
                        }
                        "income" => {
                            if self.parse_bool()? {
                                flags |= 2;
                            }
                        }
                        other => {
                            return Err(Error::at(
                                kspan,
                                format!("unknown alter_budget parameter '{other}'"),
                            ));
                        }
                    }
                }
                PrimitiveCall::AlterBudget {
                    amount,
                    category,
                    flags,
                }
            }
            "stop_all_sounds" => {
                let use_stack_object = if self.peek().kind != TokenKind::RParen {
                    self.parse_bool()?
                } else {
                    false
                };
                PrimitiveCall::StopAllSounds { use_stack_object }
            }
            "set_motive_change" => {
                let motive = self.parse_motive_id()?;
                self.expect(&TokenKind::Comma, "','")?;
                let delta_per_hour = self.parse_operand()?;
                self.expect(&TokenKind::Comma, "','")?;
                let stop_at = self.parse_operand()?;
                PrimitiveCall::SetMotiveChange {
                    motive,
                    delta_per_hour,
                    stop_at,
                }
            }
            "clear_motive_changes" => PrimitiveCall::ClearMotiveChanges,
            "set_to_next" => {
                let mode = self.parse_set_to_next_mode()?;
                let mut guid = 0u32;
                let mut dest = None;
                let mut local = LocalRef::Index(0);
                let mut saw_named = false;
                while self.eat(&TokenKind::Comma) {
                    if matches!(&self.peek().kind, TokenKind::Ident(_))
                        && self.peek_at(1).kind == TokenKind::Colon
                    {
                        saw_named = true;
                        let (k, kspan) = self.expect_ident("set_to_next option")?;
                        self.expect(&TokenKind::Colon, "':'")?;
                        match k.as_str() {
                            "guid" => guid = self.expect_u32("GUID")?,
                            "dest" | "into" => dest = Some(self.parse_operand()?),
                            "local" => local = self.parse_local_ref("set_to_next local")?,
                            other => {
                                return Err(Error::at(
                                    kspan,
                                    format!("unknown set_to_next parameter '{other}'"),
                                ));
                            }
                        }
                    } else {
                        if saw_named {
                            return Err(Error::at(
                                self.span(),
                                "positional argument cannot follow named arguments in set_to_next()",
                            ));
                        }
                        guid = self.expect_u32("GUID")?;
                    }
                }
                PrimitiveCall::SetToNext {
                    mode,
                    guid,
                    dest,
                    local,
                }
            }
            "create_object" => {
                let guid = self.expect_u32("object GUID")?;
                let mut mode = 5u8;
                let mut flags = 0u8;
                let mut local = LocalRef::Index(0);
                let mut positional = 0usize;
                let mut saw_named = false;
                while self.eat(&TokenKind::Comma) {
                    if matches!(&self.peek().kind, TokenKind::Ident(_))
                        && self.peek_at(1).kind == TokenKind::Colon
                    {
                        saw_named = true;
                        let (k, kspan) = self.expect_ident("create_object option")?;
                        self.expect(&TokenKind::Colon, "':'")?;
                        match k.as_str() {
                            "mode" => mode = self.parse_create_mode()?,
                            "flags" => flags = self.expect_u8("create_object flags")?,
                            "local" => local = self.parse_local_ref("create_object local")?,
                            other => {
                                return Err(Error::at(
                                    kspan,
                                    format!("unknown create_object parameter '{other}'"),
                                ));
                            }
                        }
                    } else {
                        if saw_named {
                            return Err(Error::at(
                                self.span(),
                                "positional argument cannot follow named arguments in create_object()",
                            ));
                        }
                        match positional {
                            0 => mode = self.parse_create_mode()?,
                            1 => flags = self.expect_u8("create_object flags")?,
                            2 => local = self.parse_local_ref("create_object local")?,
                            _ => {
                                return Err(Error::at(
                                    self.span(),
                                    "too many positional arguments to create_object()",
                                ));
                            }
                        }
                        positional += 1;
                    }
                }
                PrimitiveCall::CreateObject {
                    guid,
                    mode,
                    flags,
                    local,
                }
            }
            "dialog" => {
                let mut kind = 0u8;
                let mut message = StringRef::Index(0);
                let mut title = StringRef::Index(0);
                let mut button1 = StringRef::Index(0);
                let mut button2 = StringRef::Index(0);
                let mut button3 = StringRef::Index(0);
                let mut mode4_str = StringRef::Index(0);
                let mut flags = 0u8;
                let mut positional = 0usize;
                let mut saw_named = false;
                while self.peek().kind != TokenKind::RParen {
                    if matches!(&self.peek().kind, TokenKind::Ident(_))
                        && self.peek_at(1).kind == TokenKind::Colon
                    {
                        saw_named = true;
                        let (k, kspan) = self.expect_ident("dialog option")?;
                        self.expect(&TokenKind::Colon, "':'")?;
                        match k.as_str() {
                            "kind" => kind = self.parse_dialog_kind()?,
                            "message" | "text" => {
                                message = self.parse_string_ref("dialog message index")?;
                            }
                            "title" => title = self.parse_string_ref("dialog title index")?,
                            "button1" => button1 = self.parse_string_ref("dialog button1 index")?,
                            "button2" => button2 = self.parse_string_ref("dialog button2 index")?,
                            "button3" => button3 = self.parse_string_ref("dialog button3 index")?,
                            "mode4_str" | "icon_str" => {
                                mode4_str = self.parse_string_ref("dialog mode4 string index")?;
                            }
                            "flags" => flags |= self.expect_u8("dialog flags")?,
                            "inform" => {
                                if self.parse_bool()? {
                                    flags |= 1;
                                }
                            }
                            "non_modal" => {
                                if self.parse_bool()? {
                                    flags |= 0x80;
                                }
                            }
                            "icon_mode" => {
                                let mode = self.expect_u8("dialog icon_mode")?;
                                if mode > 7 {
                                    return Err(Error::at(
                                        kspan,
                                        "dialog icon_mode must be in 0..=7",
                                    ));
                                }
                                flags = (flags & !(7 << 1)) | (mode << 1);
                            }
                            "temp" => {
                                let t = self.expect_u8("dialog temp index")?;
                                if t > 7 {
                                    return Err(Error::at(kspan, "dialog temp must be in 0..=7"));
                                }
                                flags = (flags & !(7 << 4)) | (t << 4);
                            }
                            other => {
                                return Err(Error::at(
                                    kspan,
                                    format!("unknown dialog parameter '{other}'"),
                                ));
                            }
                        }
                    } else {
                        if saw_named {
                            return Err(Error::at(
                                self.span(),
                                "positional argument cannot follow named arguments in dialog()",
                            ));
                        }
                        match positional {
                            0 => kind = self.parse_dialog_kind()?,
                            1 => message = self.parse_string_ref("dialog message index")?,
                            2 => flags = self.expect_u8("dialog flags")?,
                            3 => title = self.parse_string_ref("dialog title index")?,
                            4 => button1 = self.parse_string_ref("dialog button1 index")?,
                            5 => button2 = self.parse_string_ref("dialog button2 index")?,
                            6 => button3 = self.parse_string_ref("dialog button3 index")?,
                            _ => {
                                return Err(Error::at(
                                    self.span(),
                                    "too many positional arguments to dialog()",
                                ));
                            }
                        }
                        positional += 1;
                    }
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                PrimitiveCall::Dialog {
                    kind,
                    message,
                    title,
                    button1,
                    button2,
                    button3,
                    mode4_str,
                    flags,
                }
            }
            "animate" => {
                let mut clip_id = 0u16;
                let mut table = 0u8;
                let mut flags = 0u8;
                let mut events = 0i8;
                let mut local_index = LocalRef::Index(0);
                let mut positional = 0usize;
                while self.peek().kind != TokenKind::RParen {
                    if matches!(&self.peek().kind, TokenKind::Ident(_))
                        && self.peek_at(1).kind == TokenKind::Colon
                    {
                        let (k, kspan) = self.expect_ident("animate option")?;
                        self.expect(&TokenKind::Colon, "':'")?;
                        match k.as_str() {
                            "clip" => clip_id = self.expect_u16("clip id")?,
                            "table" => table = self.expect_u8("animation table")?,
                            "flags" => flags = self.expect_u8("animation flags")?,
                            "events" => events = self.expect_i8("animation event count")?,
                            "local" => local_index = self.parse_local_ref("event local")?,
                            other => {
                                return Err(Error::at(
                                    kspan,
                                    format!("unknown animate parameter '{other}'"),
                                ));
                            }
                        }
                    } else {
                        match positional {
                            0 => clip_id = self.expect_u16("clip id")?,
                            1 => table = self.expect_u8("animation table")?,
                            2 => flags = self.expect_u8("animation flags")?,
                            3 => events = self.expect_i8("animation event count")?,
                            4 => local_index = self.parse_local_ref("event local")?,
                            _ => {
                                return Err(Error::at(
                                    self.span(),
                                    "too many positional arguments to animate()",
                                ));
                            }
                        }
                        positional += 1;
                    }
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                PrimitiveCall::Animate {
                    clip_id,
                    table,
                    flags,
                    events,
                    local_index,
                }
            }
            "route" | "route_to" => {
                let mut slot = 0u16;
                let mut flags = 0u8;
                let mut positional = 0usize;
                while self.peek().kind != TokenKind::RParen {
                    if matches!(&self.peek().kind, TokenKind::Ident(_))
                        && self.peek_at(1).kind == TokenKind::Colon
                    {
                        let (k, kspan) = self.expect_ident("route option")?;
                        self.expect(&TokenKind::Colon, "':'")?;
                        match k.as_str() {
                            "slot" => slot = self.expect_u16("routing slot")?,
                            "flags" => flags = self.expect_u8("routing flags")?,
                            other => {
                                return Err(Error::at(
                                    kspan,
                                    format!("unknown route parameter '{other}'"),
                                ));
                            }
                        }
                    } else {
                        match positional {
                            0 => slot = self.expect_u16("routing slot")?,
                            1 => flags = self.expect_u8("routing flags")?,
                            _ => {
                                return Err(Error::at(
                                    self.span(),
                                    "too many positional arguments to route_to()",
                                ));
                            }
                        }
                        positional += 1;
                    }
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                PrimitiveCall::RouteTo { slot, flags }
            }
            "snap" => {
                let slot = self.expect_u16("snap slot")?;
                let flags = if self.eat(&TokenKind::Comma) {
                    self.expect_u8("snap flags")?
                } else {
                    0
                };
                PrimitiveCall::Snap { slot, flags }
            }
            other => {
                return Err(Error::at(span, format!("unknown primitive '{other}'")));
            }
        };
        self.expect(&TokenKind::RParen, "')'")?;
        Ok(prim)
    }

    fn parse_bool(&mut self) -> Result<bool> {
        let (ident, span) = self.expect_ident("boolean ('true' or 'false')")?;
        match ident.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            other => Err(Error::at(
                span,
                format!("expected boolean, found '{other}'"),
            )),
        }
    }

    fn parse_motive_id(&mut self) -> Result<u8> {
        match &self.peek().kind {
            TokenKind::Int(_) => self.expect_u8("motive id"),
            TokenKind::Ident(name) => {
                let span = self.span();
                let name = name.clone();
                self.bump();
                motive_by_name(&name)
                    .ok_or_else(|| Error::at(span, format!("unknown motive '{name}'")))
            }
            other => Err(Error::at(
                self.span(),
                format!("expected motive name or id, found {other:?}"),
            )),
        }
    }

    fn parse_any_expr_op(&mut self) -> Result<u8> {
        let tok = self.bump();
        match tok.kind {
            TokenKind::Gt => Ok(0),
            TokenKind::Lt => Ok(1),
            TokenKind::EqEq => Ok(2),
            TokenKind::PlusEq => Ok(3),
            TokenKind::MinusEq => Ok(4),
            TokenKind::Assign => Ok(5),
            TokenKind::StarEq => Ok(6),
            TokenKind::SlashEq => Ok(7),
            TokenKind::PercentEq => Ok(12),
            TokenKind::AmpEq => Ok(13),
            TokenKind::GtEq => Ok(14),
            TokenKind::LtEq => Ok(15),
            TokenKind::BangEq => Ok(16),
            TokenKind::PipeEq => Ok(18),
            TokenKind::CaretEq => Ok(19),
            TokenKind::Ident(s) => match s.as_str() {
                "flag_test" => Ok(8),
                "flag_set" => Ok(9),
                "flag_clear" => Ok(10),
                "inc_lt" => Ok(11),
                "dec_gt" => Ok(17),
                "sqrt" => Ok(20),
                other => Err(Error::at(
                    tok.span,
                    format!("unknown expression operator '{other}'"),
                )),
            },
            other => Err(Error::at(
                tok.span,
                format!("expected expression operator, found {other:?}"),
            )),
        }
    }

    fn parse_assign_stmt(&mut self) -> Result<Stmt> {
        let span = self.span();
        let lhs = self.parse_operand()?;
        let op_tok = self.bump();
        let op = match op_tok.kind {
            TokenKind::Assign => AssignOp::Set,
            TokenKind::PlusEq => AssignOp::Add,
            TokenKind::MinusEq => AssignOp::Sub,
            TokenKind::StarEq => AssignOp::Mul,
            TokenKind::SlashEq => AssignOp::Div,
            TokenKind::PercentEq => AssignOp::Mod,
            TokenKind::AmpEq => AssignOp::BitAnd,
            TokenKind::PipeEq => AssignOp::BitOr,
            TokenKind::CaretEq => AssignOp::BitXor,
            TokenKind::Ident(s) if s == "flag_set" => AssignOp::FlagSet,
            TokenKind::Ident(s) if s == "flag_clear" => AssignOp::FlagClear,
            TokenKind::Ident(s) if s == "sqrt" => AssignOp::Sqrt,
            other => {
                return Err(Error::at(
                    op_tok.span,
                    format!("expected assignment operator (:=, +=, -=, ...), found {other:?}"),
                ));
            }
        };
        let rhs = self.parse_operand()?;
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(Stmt::Assign { span, lhs, op, rhs })
    }

    fn parse_condition(&mut self) -> Result<Condition> {
        // Parse consecutive `!` iteratively so deep negation chains cannot overflow the stack.
        let mut negated = false;
        while self.eat(&TokenKind::Bang) {
            negated = !negated;
        }
        let span = self.span();
        let base = if let TokenKind::Ident(ident) = &self.peek().kind {
            match ident.as_str() {
                "call" => {
                    self.bump();
                    let (target, args, pass_temps) = self.parse_call_args_and_target()?;
                    Condition::Call {
                        span,
                        target,
                        args,
                        pass_temps,
                    }
                }
                "route"
                | "route_to"
                | "find_best_object"
                | "dialog"
                | "animate"
                | "snap"
                | "set_to_next"
                | "play_sound"
                | "create_object"
                | "remove_object"
                | "run_functional_tree"
                | "alter_budget"
                | "sleep"
                | "generic_call"
                | "random"
                | "idle_for_input"
                | "stop_all_sounds"
                | "set_motive_change"
                | "clear_motive_changes" => {
                    let prim = self.parse_primitive_call()?;
                    Condition::Primitive { span, prim }
                }
                _ => self.parse_compare_condition(span)?,
            }
        } else {
            self.parse_compare_condition(span)?
        };
        Ok(if negated {
            Condition::Not(Box::new(base))
        } else {
            base
        })
    }

    fn parse_compare_condition(&mut self, span: Span) -> Result<Condition> {
        let lhs = self.parse_operand()?;
        let op_tok = self.bump();
        let op = match op_tok.kind {
            TokenKind::Gt => CompareOp::Gt,
            TokenKind::Lt => CompareOp::Lt,
            TokenKind::EqEq => CompareOp::Eq,
            TokenKind::GtEq => CompareOp::Ge,
            TokenKind::LtEq => CompareOp::Le,
            TokenKind::BangEq => CompareOp::Ne,
            TokenKind::Ident(s) if s == "flag_test" => CompareOp::FlagTest,
            TokenKind::Ident(s) if s == "inc_lt" => CompareOp::IncLt,
            TokenKind::Ident(s) if s == "dec_gt" => CompareOp::DecGt,
            other => {
                return Err(Error::at(
                    op_tok.span,
                    format!("expected comparison operator (<, <=, ==, !=, >, >=), found {other:?}"),
                ));
            }
        };
        let rhs = self.parse_operand()?;
        Ok(Condition::Compare { span, lhs, op, rhs })
    }

    fn parse_bracket_index(&mut self) -> Result<i16> {
        self.expect(&TokenKind::LBracket, "'['")?;
        let index = self.expect_i16("operand index")?;
        self.expect(&TokenKind::RBracket, "']'")?;
        Ok(index)
    }

    fn parse_operand(&mut self) -> Result<OperandRef> {
        let tok = self.peek().clone();
        match &tok.kind {
            TokenKind::Int(_) => {
                let val = self.expect_i16("integer literal")?;
                Ok(OperandRef::Direct(Operand::literal(val)))
            }
            TokenKind::Ident(root) => {
                let root = root.clone();
                let span = tok.span;
                self.bump();
                match root.as_str() {
                    "literal" => {
                        self.expect(&TokenKind::LParen, "'('")?;
                        let v = self.expect_i16("literal value")?;
                        self.expect(&TokenKind::RParen, "')'")?;
                        Ok(OperandRef::Direct(Operand::literal(v)))
                    }
                    "scope" => {
                        self.expect(&TokenKind::LParen, "'('")?;
                        let s = self.expect_u8("scope id")?;
                        self.expect(&TokenKind::Comma, "','")?;
                        let i = self.expect_i16("scope index")?;
                        self.expect(&TokenKind::RParen, "')'")?;
                        Ok(OperandRef::Direct(Operand::new(s, i)))
                    }
                    "global" => Ok(OperandRef::Direct(Operand::new(
                        6,
                        self.parse_bracket_index()?,
                    ))),
                    "temp" => Ok(OperandRef::Direct(Operand::new(
                        8,
                        self.parse_bracket_index()?,
                    ))),
                    "param" => Ok(OperandRef::Direct(Operand::new(
                        9,
                        self.parse_bracket_index()?,
                    ))),
                    "stack_obj" => Ok(OperandRef::Direct(Operand::new(10, 0))),
                    "local" => Ok(OperandRef::Direct(Operand::new(
                        25,
                        self.parse_bracket_index()?,
                    ))),
                    "bcon" | "const" => {
                        if self.eat(&TokenKind::Dot) {
                            let (name, _) = self.expect_ident("BCON constant name")?;
                            return Ok(OperandRef::NamedBcon(name));
                        }
                        self.expect(&TokenKind::LBracket, "'[' or '.'")?;
                        let first = self.expect_i16("constant index")?;
                        let index = if self.eat(&TokenKind::Colon) || self.eat(&TokenKind::Comma) {
                            let entry = self.expect_u8("constant entry index")?;
                            // sims-vm resolves BCON addresses as table = addr >> 7 (0..=191),
                            // entry = addr & 127 (0..=127).
                            let table = u16::try_from(first)
                                .ok()
                                .filter(|&t| t <= 191)
                                .ok_or_else(|| {
                                    Error::at(
                                        span,
                                        format!("BCON table index {first} out of 0..=191 range"),
                                    )
                                })?;
                            if entry > 127 {
                                return Err(Error::at(
                                    span,
                                    format!("BCON entry index {entry} out of 0..=127 range"),
                                ));
                            }
                            ((table << 7) | u16::from(entry)) as i16
                        } else {
                            first
                        };
                        self.expect(&TokenKind::RBracket, "']'")?;
                        Ok(OperandRef::Direct(Operand::new(26, index)))
                    }
                    "str" => {
                        self.expect(&TokenKind::Dot, "'.'")?;
                        let (name, _) = self.expect_ident("string entry name")?;
                        Ok(OperandRef::NamedStr(name))
                    }
                    "my" => self.parse_scoped_member(span, true).map(OperandRef::Direct),
                    "stack" => self
                        .parse_scoped_member(span, false)
                        .map(OperandRef::Direct),
                    other if is_reserved_ident(other) => {
                        Err(Error::at(span, format!("unexpected keyword '{other}'")))
                    }
                    other => Ok(OperandRef::NamedLocal(other.to_owned())),
                }
            }
            other => Err(Error::at(
                tok.span,
                format!("expected operand, found {other:?}"),
            )),
        }
    }

    fn parse_scoped_member(&mut self, span: Span, is_my: bool) -> Result<Operand> {
        self.expect(&TokenKind::Dot, "'.'")?;
        let (field, fspan) = self.expect_ident(
            "member name (attr, data, motive, person, skill, slot, temp, id, in_use, room, prep_score, cook_score)",
        )?;
        match field.as_str() {
            "attr" => Ok(Operand::new(
                if is_my { 0 } else { 1 },
                self.parse_bracket_index()?,
            )),
            "data" => Ok(Operand::new(
                if is_my { 3 } else { 4 },
                self.parse_bracket_index()?,
            )),
            "id" => Ok(Operand::new(if is_my { 3 } else { 4 }, 11)),
            "in_use" => Ok(Operand::new(if is_my { 3 } else { 4 }, 25)),
            "room" => Ok(Operand::new(if is_my { 3 } else { 4 }, 29)),
            "prep_score" => Ok(Operand::new(if is_my { 3 } else { 4 }, 31)),
            "cook_score" => Ok(Operand::new(if is_my { 3 } else { 4 }, 32)),
            "temp" if !is_my => Ok(Operand::new(13, self.parse_bracket_index()?)),
            "slot" => Ok(Operand::new(
                if is_my { 20 } else { 16 },
                self.parse_bracket_index()?,
            )),
            "person" => {
                let scope = if is_my { 18 } else { 19 };
                if self.eat(&TokenKind::Dot) {
                    let (sname, sspan) = self.expect_ident("person data field name")?;
                    let id = person_field_by_name(&sname).ok_or_else(|| {
                        Error::at(sspan, format!("unknown person data field '{sname}'"))
                    })?;
                    Ok(Operand::new(scope, i16::from(id)))
                } else {
                    Ok(Operand::new(scope, self.parse_bracket_index()?))
                }
            }
            "priority" => Ok(Operand::new(if is_my { 18 } else { 19 }, 33)),
            "skill" => {
                let scope = if is_my { 18 } else { 19 };
                self.expect(&TokenKind::Dot, "'.'")?;
                let (sname, sspan) = self.expect_ident("skill name")?;
                let id = skill_by_name(&sname)
                    .ok_or_else(|| Error::at(sspan, format!("unknown skill '{sname}'")))?;
                Ok(Operand::new(scope, i16::from(id)))
            }
            "motive" => {
                let scope = if is_my { 14 } else { 15 };
                if self.eat(&TokenKind::Dot) {
                    let (mname, mspan) = self.expect_ident("motive name")?;
                    let id = motive_by_name(&mname)
                        .ok_or_else(|| Error::at(mspan, format!("unknown motive '{mname}'")))?;
                    Ok(Operand::new(scope, i16::from(id)))
                } else {
                    Ok(Operand::new(scope, self.parse_bracket_index()?))
                }
            }
            other => Err(Error::at(
                fspan,
                format!(
                    "unknown {} member '{other}' (started at {span})",
                    if is_my { "my" } else { "stack" }
                ),
            )),
        }
    }
}

fn is_reserved_ident(name: &str) -> bool {
    matches!(
        name,
        "tree"
            | "interaction"
            | "bcon"
            | "const"
            | "str"
            | "let"
            | "local"
            | "if"
            | "else"
            | "while"
            | "break"
            | "continue"
            | "return"
            | "idle"
            | "call"
            | "subroutine"
            | "op"
            | "my"
            | "stack"
            | "stack_obj"
            | "global"
            | "temp"
            | "param"
            | "literal"
            | "scope"
            | "true"
            | "false"
            | "error"
    )
}

/// Maps human-readable Sims 1 motive names to their canonical `sims-vm` indices.
#[must_use]
pub fn motive_by_name(name: &str) -> Option<u8> {
    match name {
        "mood" => Some(3),
        "energy" => Some(5),
        "comfort" => Some(6),
        "hunger" => Some(7),
        "hygiene" => Some(8),
        "bladder" => Some(9),
        "alertness" => Some(11),
        "room" => Some(13),
        "social" => Some(14),
        "fun" => Some(15),
        _ => None,
    }
}

/// Maps human-readable Sims 1 skill names to their canonical `person.data` indices.
#[must_use]
pub fn skill_by_name(name: &str) -> Option<u8> {
    match name {
        "cooking" => Some(10),
        "charisma" => Some(11),
        "mechanical" | "mechanic" => Some(12),
        "creativity" => Some(15),
        "body" => Some(17),
        "logic" => Some(18),
        _ => None,
    }
}

/// Maps human-readable `person.data` field names (skills + action priority) to indices.
#[must_use]
pub fn person_field_by_name(name: &str) -> Option<u8> {
    match name {
        "priority" => Some(33),
        other => skill_by_name(other),
    }
}
