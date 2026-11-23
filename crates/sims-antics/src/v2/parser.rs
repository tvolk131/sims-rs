//! Recursive-descent parser for the high-level `.antics` v2 authoring language.

use crate::art::FurnitureShape;
use crate::audio::SoundPreset;
use crate::error::{Error, Result, Span};
use crate::parser::motive_by_name;
use crate::v2::ast::{
    AdvertiseDecl, CallArg, ConstIntExpr, FnDecl, FnParam, ForEachIter, FunctionalDecl, GuardBody,
    GuardDecl, InteractionDecl, LifecycleBlock, MetaField, MetaFieldKind, ObjectDecl, ObjectMember,
    ParamAssignDecl, ParamDecl, ParamType, ParamValue, SlotDecl, SoundDecl, SpriteDecl, StateDecl,
    StateInitValue, StateType, StringsDecl, TemplateDecl, TileDecl, UseDecl, UseItem, V2AssignOp,
    V2BinOp, V2Expr, V2File, V2Item, V2Stmt, V2Type, V2UnaryOp,
};
use crate::v2::lexer::{Token, TokenKind, tokenize};

const MAX_NESTING_DEPTH: usize = 64;

/// Parses a `.antics` v2 source file into a [`V2File`] AST.
pub fn parse_v2_file(source: &str) -> Result<V2File> {
    let tokens = tokenize(source)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        depth: 0,
    };
    parser.parse_file()
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

    fn eat_ident(&mut self, expected: &str) -> bool {
        if matches!(&self.peek().kind, TokenKind::Ident(s) if s == expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn is_ident(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Ident(s) if s == expected)
    }

    fn expect(&mut self, kind: &TokenKind, desc: &str) -> Result<Span> {
        let tok = self.peek().clone();
        if &tok.kind == kind {
            self.bump();
            Ok(tok.span)
        } else {
            Err(Error::at(
                tok.span,
                format!("expected {desc}, found {:?}", tok.kind),
            ))
        }
    }

    fn expect_ident(&mut self, desc: &str) -> Result<(String, Span)> {
        let tok = self.bump();
        match tok.kind {
            TokenKind::Ident(s) => Ok((s, tok.span)),
            other => Err(Error::at(
                tok.span,
                format!("expected {desc}, found {other:?}"),
            )),
        }
    }

    fn expect_string(&mut self, desc: &str) -> Result<(String, Span)> {
        let tok = self.bump();
        match tok.kind {
            TokenKind::Str(s) => Ok((s, tok.span)),
            other => Err(Error::at(
                tok.span,
                format!("expected {desc}, found {other:?}"),
            )),
        }
    }

    fn parse_signed_int(&mut self, desc: &str) -> Result<(i64, Span)> {
        let span = self.span();
        let neg = self.eat(&TokenKind::Minus);
        let _ = self.eat(&TokenKind::Plus);
        let tok = self.bump();
        match tok.kind {
            TokenKind::Int(v) => {
                let val = if neg {
                    v.checked_neg()
                        .ok_or_else(|| Error::at(span, format!("{desc} integer overflow")))?
                } else {
                    v
                };
                Ok((val, span))
            }
            other => Err(Error::at(
                tok.span,
                format!("expected {desc}, found {other:?}"),
            )),
        }
    }

    fn parse_signed_float(&mut self, desc: &str) -> Result<(f64, Span)> {
        let span = self.span();
        let neg = self.eat(&TokenKind::Minus);
        let _ = self.eat(&TokenKind::Plus);
        let tok = self.bump();
        match tok.kind {
            TokenKind::Float(v) => Ok((if neg { -v } else { v }, span)),
            TokenKind::Int(v) => Ok((if neg { -(v as f64) } else { v as f64 }, span)),
            other => Err(Error::at(
                tok.span,
                format!("expected {desc}, found {other:?}"),
            )),
        }
    }

    fn expect_u16(&mut self, desc: &str) -> Result<u16> {
        let (v, span) = self.parse_signed_int(desc)?;
        u16::try_from(v).map_err(|_| Error::at(span, format!("{desc} {v} out of u16 range")))
    }

    fn expect_i16(&mut self, desc: &str) -> Result<i16> {
        let (v, span) = self.parse_signed_int(desc)?;
        i16::try_from(v).map_err(|_| Error::at(span, format!("{desc} {v} out of i16 range")))
    }

    fn expect_u32(&mut self, desc: &str) -> Result<u32> {
        let (v, span) = self.parse_signed_int(desc)?;
        u32::try_from(v).map_err(|_| Error::at(span, format!("{desc} {v} out of u32 range")))
    }

    fn expect_i32(&mut self, desc: &str) -> Result<i32> {
        let (v, span) = self.parse_signed_int(desc)?;
        i32::try_from(v).map_err(|_| Error::at(span, format!("{desc} {v} out of i32 range")))
    }

    fn expect_bool(&mut self, desc: &str) -> Result<bool> {
        let (ident, span) = self.expect_ident(desc)?;
        match ident.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            other => Err(Error::at(
                span,
                format!("expected boolean ('true' or 'false') for {desc}, found '{other}'"),
            )),
        }
    }

    fn enter_nesting(&mut self, span: Span) -> Result<()> {
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(Error::at(
                span,
                format!("nesting depth exceeds maximum of {MAX_NESTING_DEPTH}"),
            ));
        }
        self.depth += 1;
        Ok(())
    }

    fn leave_nesting(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn parse_file(&mut self) -> Result<V2File> {
        let mut uses = Vec::new();
        let mut items = Vec::new();
        while self.peek().kind != TokenKind::Eof {
            let span = self.span();
            if self.eat_ident("use") {
                uses.push(self.parse_use(span)?);
                continue;
            }
            let is_pub = self.eat_ident("pub");
            let (kw, kw_span) =
                self.expect_ident("top-level item ('use', 'fn', 'template', 'object')")?;
            match kw.as_str() {
                "fn" => items.push(V2Item::Fn(self.parse_fn(kw_span, is_pub)?)),
                "template" => items.push(V2Item::Template(self.parse_template(kw_span, is_pub)?)),
                "object" => {
                    if is_pub {
                        return Err(Error::at(
                            span,
                            "'object' declarations cannot be marked 'pub' (export templates or functions instead)",
                        ));
                    }
                    items.push(V2Item::Object(self.parse_object(kw_span)?));
                }
                other => {
                    return Err(Error::at(
                        kw_span,
                        format!(
                            "unexpected top-level keyword '{other}' in .antics v2 file \
                             (expected 'use', 'pub', 'fn', 'template', or 'object')"
                        ),
                    ));
                }
            }
        }
        Ok(V2File { uses, items })
    }

    fn parse_use(&mut self, span: Span) -> Result<UseDecl> {
        let mut segs = Vec::new();
        let (first, _) = self.expect_ident("module path segment")?;
        segs.push(first);
        let mut items = Vec::new();

        while self.eat(&TokenKind::ColonColon) {
            if self.eat(&TokenKind::LBrace) {
                while !self.eat(&TokenKind::RBrace) {
                    let (name, ispan) = self.expect_ident("imported item name")?;
                    let alias = if self.eat_ident("as") {
                        Some(self.expect_ident("import alias")?.0)
                    } else {
                        None
                    };
                    items.push(UseItem {
                        span: ispan,
                        name,
                        alias,
                    });
                    if !self.eat(&TokenKind::Comma) {
                        self.expect(&TokenKind::RBrace, "'}'")?;
                        break;
                    }
                }
                break;
            }
            let (seg, seg_span) = self.expect_ident("module or item name")?;
            if self.peek().kind == TokenKind::ColonColon {
                segs.push(seg);
            } else {
                let alias = if self.eat_ident("as") {
                    Some(self.expect_ident("import alias")?.0)
                } else {
                    None
                };
                items.push(UseItem {
                    span: seg_span,
                    name: seg,
                    alias,
                });
                break;
            }
        }

        self.expect(&TokenKind::Semicolon, "';'")?;
        if segs.len() < 2 || items.is_empty() {
            return Err(Error::at(
                span,
                "use import must specify at least '<pack>::<module>::<Item>'",
            ));
        }
        Ok(UseDecl {
            span,
            module_path: segs,
            items,
        })
    }

    fn parse_type(&mut self) -> Result<V2Type> {
        let span = self.span();
        if self.eat(&TokenKind::Amp) {
            if !self.eat_ident("mut") {
                return Err(Error::at(
                    span,
                    "only mutable references '&mut <type>' are supported",
                ));
            }
            self.enter_nesting(span)?;
            let inner = self.parse_type();
            self.leave_nesting();
            let inner = inner?;
            match inner {
                V2Type::Bool | V2Type::I16 | V2Type::ObjectId => {
                    return Ok(V2Type::MutRef(Box::new(inner)));
                }
                other => {
                    return Err(Error::at(
                        span,
                        format!(
                            "cannot take '&mut' of type '{}' (supported: bool, i16, ObjectId)",
                            other.display_name()
                        ),
                    ));
                }
            }
        }
        let (name, nspan) = self.expect_ident("type name")?;
        match name.as_str() {
            "i16" => Ok(V2Type::I16),
            "bool" => Ok(V2Type::Bool),
            "Sim" => Ok(V2Type::Sim),
            "Object" => Ok(V2Type::Object),
            "ObjectId" => Ok(V2Type::ObjectId),
            "Motive" => Ok(V2Type::Motive),
            "Sound" => Ok(V2Type::Sound),
            "Slot" => Ok(V2Type::Slot),
            other => Err(Error::at(nspan, format!("unknown type '{other}'"))),
        }
    }

    fn parse_fn_params(&mut self) -> Result<Vec<FnParam>> {
        self.expect(&TokenKind::LParen, "'('")?;
        let mut params = Vec::new();
        while !self.eat(&TokenKind::RParen) {
            let (name, span) = self.expect_ident("parameter name")?;
            self.expect(&TokenKind::Colon, "':'")?;
            let ty = self.parse_type()?;
            params.push(FnParam { span, name, ty });
            if !self.eat(&TokenKind::Comma) {
                self.expect(&TokenKind::RParen, "')'")?;
                break;
            }
        }
        Ok(params)
    }

    fn parse_fn(&mut self, span: Span, is_pub: bool) -> Result<FnDecl> {
        let (name, _) = self.expect_ident("function name")?;
        let params = self.parse_fn_params()?;
        let return_type = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        Ok(FnDecl {
            span,
            is_pub,
            name,
            params,
            return_type,
            body,
        })
    }

    fn parse_template(&mut self, span: Span, is_pub: bool) -> Result<TemplateDecl> {
        let (name, _) = self.expect_ident("template name")?;
        let members = self.parse_object_members()?;
        Ok(TemplateDecl {
            span,
            is_pub,
            name,
            members,
        })
    }

    fn parse_object(&mut self, span: Span) -> Result<ObjectDecl> {
        let (name, _) = self.expect_string("object display name")?;
        let from_template = if self.eat_ident("from") {
            let (tpl, tspan) = self.expect_ident("template name")?;
            Some((tpl, tspan))
        } else {
            None
        };
        let members = self.parse_object_members()?;
        Ok(ObjectDecl {
            span,
            name,
            from_template,
            members,
        })
    }

    fn parse_object_members(&mut self) -> Result<Vec<ObjectMember>> {
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut members = Vec::new();
        while !self.eat(&TokenKind::RBrace) {
            // Parse optional `@id(N)` / `@guard_id(N)` annotations.
            let mut pinned_id: Option<u16> = None;
            let mut pinned_guard_id: Option<u16> = None;
            while self.eat(&TokenKind::At) {
                let (attr, aspan) = self.expect_ident("attribute name ('id' or 'guard_id')")?;
                self.expect(&TokenKind::LParen, "'('")?;
                let id = self.expect_u16("BHAV tree id")?;
                self.expect(&TokenKind::RParen, "')'")?;
                match attr.as_str() {
                    "id" => {
                        if pinned_id.replace(id).is_some() {
                            return Err(Error::at(aspan, "duplicate @id annotation"));
                        }
                    }
                    "guard_id" => {
                        if pinned_guard_id.replace(id).is_some() {
                            return Err(Error::at(aspan, "duplicate @guard_id annotation"));
                        }
                    }
                    other => {
                        return Err(Error::at(aspan, format!("unknown annotation '@{other}'")));
                    }
                }
            }

            let span = self.span();
            let (kw, kw_span) = self.expect_ident("object or template member")?;
            if (pinned_id.is_some() || pinned_guard_id.is_some())
                && !matches!(kw.as_str(), "init" | "main" | "functional" | "interaction")
            {
                return Err(Error::at(
                    kw_span,
                    "@id/@guard_id annotations are only allowed on init, main, functional, or interaction",
                ));
            }
            if pinned_guard_id.is_some() && !matches!(kw.as_str(), "functional" | "interaction") {
                return Err(Error::at(
                    kw_span,
                    "@guard_id is only allowed on functional or interaction declarations",
                ));
            }

            match kw.as_str() {
                "param" => {
                    members.push(self.parse_param_member(kw_span)?);
                }
                "state" => {
                    if self.peek().kind == TokenKind::LBrace {
                        return Err(Error::at(
                            kw_span,
                            "nested 'state { ... }' blocks are not allowed in .antics v2; \
                             declare flat top-level 'state name: bool = false;' items instead",
                        ));
                    }
                    members.push(ObjectMember::State(self.parse_state_decl(kw_span)?));
                }
                "sound" => {
                    members.push(ObjectMember::Sound(self.parse_sound_decl(kw_span)?));
                }
                "slot" => {
                    members.push(ObjectMember::Slot(self.parse_slot_decl(kw_span)?));
                }
                "strings" => {
                    members.push(ObjectMember::Strings(self.parse_strings_decl(kw_span)?));
                }
                "init" => {
                    let body = self.parse_block()?;
                    members.push(ObjectMember::Init(LifecycleBlock {
                        span: kw_span,
                        pinned_id,
                        body,
                    }));
                }
                "main" => {
                    let body = self.parse_block()?;
                    members.push(ObjectMember::Main(LifecycleBlock {
                        span: kw_span,
                        pinned_id,
                        body,
                    }));
                }
                "functional" => {
                    members.push(ObjectMember::Functional(self.parse_functional_decl(
                        kw_span,
                        pinned_id,
                        pinned_guard_id,
                    )?));
                }
                "interaction" => {
                    members.push(ObjectMember::Interaction(self.parse_interaction_decl(
                        kw_span,
                        pinned_id,
                        pinned_guard_id,
                    )?));
                }
                "fn" => {
                    members.push(ObjectMember::Fn(self.parse_fn(kw_span, false)?));
                }
                "pub" => {
                    let (next, nspan) = self.expect_ident("'fn'")?;
                    if next != "fn" {
                        return Err(Error::at(
                            nspan,
                            "only 'fn' inside objects/templates can follow 'pub'",
                        ));
                    }
                    members.push(ObjectMember::Fn(self.parse_fn(nspan, true)?));
                }
                "multi_tile" => {
                    self.parse_multi_tile_block(kw_span, &mut members)?;
                }
                meta_key if is_meta_field_name(meta_key) => {
                    members.push(ObjectMember::Meta(
                        self.parse_meta_field(meta_key, kw_span)?,
                    ));
                }
                param_name => {
                    // Template parameter binding: `energy_step: 20;` or `energy_step = 20;`
                    if !self.eat(&TokenKind::Colon) && !self.eat(&TokenKind::Assign) {
                        return Err(Error::at(
                            span,
                            format!(
                                "unexpected member '{param_name}' (expected ':' or '=' for parameter binding)"
                            ),
                        ));
                    }
                    let value = self.parse_param_value()?;
                    self.eat_member_terminator();
                    members.push(ObjectMember::ParamAssign(ParamAssignDecl {
                        span: kw_span,
                        name: param_name.to_owned(),
                        value,
                    }));
                }
            }
        }
        Ok(members)
    }

    fn eat_member_terminator(&mut self) {
        let _ = self.eat(&TokenKind::Semicolon) || self.eat(&TokenKind::Comma);
    }

    fn parse_param_member(&mut self, span: Span) -> Result<ObjectMember> {
        let (name, _) = self.expect_ident("parameter name")?;
        if self.eat(&TokenKind::Assign) {
            // `param energy_step = 20;` binding form
            let value = self.parse_param_value()?;
            self.eat_member_terminator();
            return Ok(ObjectMember::ParamAssign(ParamAssignDecl {
                span,
                name,
                value,
            }));
        }
        self.expect(&TokenKind::Colon, "':'")?;
        let (ty_name, ty_span) = self.expect_ident("param type ('i16' or 'bool')")?;
        let ty = match ty_name.as_str() {
            "i16" => ParamType::I16,
            "bool" => ParamType::Bool,
            other => {
                return Err(Error::at(
                    ty_span,
                    format!(
                        "param '{name}' has unsupported type '{other}' (expected 'i16' or 'bool')"
                    ),
                ));
            }
        };
        let default = if self.eat(&TokenKind::Assign) {
            let val = self.parse_param_value()?;
            match (ty, val) {
                (ParamType::I16, ParamValue::I16(_)) | (ParamType::Bool, ParamValue::Bool(_)) => {}
                (ParamType::I16, ParamValue::Bool(_)) => {
                    return Err(Error::at(
                        span,
                        format!("param '{name}' declared as i16 cannot default to bool"),
                    ));
                }
                (ParamType::Bool, ParamValue::I16(_)) => {
                    return Err(Error::at(
                        span,
                        format!("param '{name}' declared as bool cannot default to integer"),
                    ));
                }
            }
            Some(val)
        } else {
            None
        };
        self.eat_member_terminator();
        Ok(ObjectMember::Param(ParamDecl {
            span,
            name,
            ty,
            default,
        }))
    }

    fn parse_param_value(&mut self) -> Result<ParamValue> {
        let span = self.span();
        match &self.peek().kind {
            TokenKind::Ident(s) if s == "true" => {
                self.bump();
                Ok(ParamValue::Bool(true))
            }
            TokenKind::Ident(s) if s == "false" => {
                self.bump();
                Ok(ParamValue::Bool(false))
            }
            TokenKind::Int(_) | TokenKind::Minus | TokenKind::Plus => {
                let v = self.expect_i16("i16 parameter value")?;
                Ok(ParamValue::I16(v))
            }
            other => Err(Error::at(
                span,
                format!("expected i16 or bool parameter value, found {other:?}"),
            )),
        }
    }

    fn parse_state_decl(&mut self, span: Span) -> Result<StateDecl> {
        let (name, _) = self.expect_ident("state field name")?;
        self.expect(&TokenKind::Colon, "':'")?;
        let (ty_name, ty_span) = self.expect_ident("state type ('bool', 'i16', or 'ObjectId')")?;
        let ty = match ty_name.as_str() {
            "bool" => StateType::Bool,
            "i16" => StateType::I16,
            "ObjectId" => StateType::ObjectId,
            other => {
                return Err(Error::at(
                    ty_span,
                    format!(
                        "state '{name}' has unsupported type '{other}' (expected 'bool', 'i16', or 'ObjectId')"
                    ),
                ));
            }
        };
        let init = if self.eat(&TokenKind::Assign) {
            let ispan = self.span();
            let val = match &self.peek().kind {
                TokenKind::Ident(s) if s == "true" => {
                    self.bump();
                    StateInitValue::Bool(true)
                }
                TokenKind::Ident(s) if s == "false" => {
                    self.bump();
                    StateInitValue::Bool(false)
                }
                TokenKind::Ident(s) => {
                    let pname = s.clone();
                    self.bump();
                    StateInitValue::Param(pname)
                }
                TokenKind::Int(_) | TokenKind::Minus | TokenKind::Plus => {
                    StateInitValue::Int(self.expect_i16("state initial value")?)
                }
                other => {
                    return Err(Error::at(
                        ispan,
                        format!("expected state initial value, found {other:?}"),
                    ));
                }
            };
            match (&ty, &val) {
                (StateType::Bool, StateInitValue::Int(_)) => {
                    return Err(Error::at(
                        ispan,
                        format!("bool state '{name}' cannot initialize from an integer literal"),
                    ));
                }
                (StateType::I16 | StateType::ObjectId, StateInitValue::Bool(_)) => {
                    return Err(Error::at(
                        ispan,
                        format!(
                            "integer/ObjectId state '{name}' cannot initialize from a bool literal"
                        ),
                    ));
                }
                _ => {}
            }
            Some(val)
        } else {
            None
        };
        self.eat_member_terminator();
        Ok(StateDecl {
            span,
            name,
            ty,
            init,
        })
    }

    fn parse_sound_decl(&mut self, span: Span) -> Result<SoundDecl> {
        let (name, _) = self.expect_ident("sound identifier")?;
        let mut preset = SoundPreset::UiClick;
        let mut wav_name = None;
        let mut explicit_id = None;
        if self.eat(&TokenKind::Assign) || self.eat(&TokenKind::Colon) {
            if self.eat_ident("preset") {
                self.expect(&TokenKind::LParen, "'('")?;
                let (pname, pspan) = self.expect_string("sound preset name")?;
                preset = parse_sound_preset(&pname, pspan)?;
                while self.eat(&TokenKind::Comma) {
                    if self.peek().kind == TokenKind::RParen {
                        break;
                    }
                    let (opt, ospan) = self.expect_ident("sound option ('wav', 'name', 'id')")?;
                    self.expect(&TokenKind::Colon, "':'")?;
                    match opt.as_str() {
                        "wav" | "name" => {
                            let (wn, _) = self.expect_string("sound WAV name")?;
                            wav_name = Some(wn);
                        }
                        "id" => {
                            explicit_id = Some(self.expect_u16("FWAV sound id")?);
                        }
                        other => {
                            return Err(Error::at(
                                ospan,
                                format!("unknown sound option '{other}'"),
                            ));
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "')'")?;
            } else {
                let (pname, pspan) = self.expect_string("sound preset name")?;
                preset = parse_sound_preset(&pname, pspan)?;
            }
        }
        self.eat_member_terminator();
        Ok(SoundDecl {
            span,
            name,
            preset,
            wav_name,
            explicit_id,
        })
    }

    fn parse_slot_decl(&mut self, span: Span) -> Result<SlotDecl> {
        let (name, _) = self.expect_ident("slot name")?;
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut slot = SlotDecl {
            span,
            name,
            kind: 3,
            offset: [0.0, -0.75, 0.0],
            standing: 1,
            sitting: 0,
            ground: 1,
            flags: 1,
            snap_target: -1,
            proximity: [0, 0, 0],
            max_size: 100,
            gradient: 0.0,
            height: 5,
            facing: 0,
            resolution: 16,
        };
        while !self.eat(&TokenKind::RBrace) {
            let (key, kspan) = self.expect_ident("slot field name")?;
            self.expect(&TokenKind::Colon, "':'")?;
            match key.as_str() {
                "kind" => {
                    slot.kind = match &self.peek().kind {
                        TokenKind::Int(_) | TokenKind::Minus | TokenKind::Plus => {
                            self.expect_u16("slot kind")?
                        }
                        TokenKind::Ident(_) => {
                            let (kname, s) = self.expect_ident("slot kind")?;
                            match kname.as_str() {
                                "container" => 0,
                                "routing" => 3,
                                other => {
                                    return Err(Error::at(
                                        s,
                                        format!(
                                            "unknown slot kind '{other}' (expected 'routing', 'container', or u16)"
                                        ),
                                    ));
                                }
                            }
                        }
                        other => {
                            return Err(Error::at(
                                kspan,
                                format!("expected slot kind, found {other:?}"),
                            ));
                        }
                    };
                }
                "offset" => {
                    slot.offset = self.parse_f32_3()?;
                }
                "standing" => {
                    slot.standing = self.parse_bool_or_i32("slot standing")?;
                }
                "sitting" => {
                    slot.sitting = self.parse_bool_or_i32("slot sitting")?;
                }
                "ground" => {
                    slot.ground = self.parse_bool_or_i32("slot ground")?;
                }
                "flags" => {
                    slot.flags = self.expect_u32("slot flags")?;
                }
                "snap_target" => {
                    slot.snap_target = self.expect_i32("slot snap_target")?;
                }
                "proximity" => {
                    slot.proximity = self.parse_i32_3()?;
                }
                "max_size" => {
                    slot.max_size = self.expect_i32("slot max_size")?;
                }
                "gradient" => {
                    slot.gradient = self.parse_signed_float("slot gradient")?.0 as f32;
                }
                "height" => {
                    slot.height = self.expect_i32("slot height")?;
                }
                "facing" => {
                    slot.facing = self.parse_facing()?;
                }
                "resolution" => {
                    slot.resolution = self.expect_i32("slot resolution")?;
                }
                other => {
                    return Err(Error::at(kspan, format!("unknown slot field '{other}'")));
                }
            }
            if !self.eat(&TokenKind::Comma) && !self.eat(&TokenKind::Semicolon) {
                self.expect(&TokenKind::RBrace, "'}'")?;
                break;
            }
        }
        self.eat(&TokenKind::Semicolon);
        Ok(slot)
    }

    fn parse_bool_or_i32(&mut self, desc: &str) -> Result<i32> {
        match &self.peek().kind {
            TokenKind::Ident(s) if s == "true" => {
                self.bump();
                Ok(1)
            }
            TokenKind::Ident(s) if s == "false" => {
                self.bump();
                Ok(0)
            }
            _ => self.expect_i32(desc),
        }
    }

    fn parse_facing(&mut self) -> Result<i32> {
        match &self.peek().kind {
            TokenKind::Ident(_) => {
                let (dir, span) = self.expect_ident("slot facing direction")?;
                match dir.as_str() {
                    "north" | "n" => Ok(0),
                    "ne" | "northeast" => Ok(1),
                    "east" | "e" => Ok(2),
                    "se" | "southeast" => Ok(3),
                    "south" | "s" => Ok(4),
                    "sw" | "southwest" => Ok(5),
                    "west" | "w" => Ok(6),
                    "nw" | "northwest" => Ok(7),
                    "any" => Ok(-1),
                    other => Err(Error::at(
                        span,
                        format!(
                            "unknown slot facing '{other}' (expected north/ne/east/se/south/sw/west/nw/any or integer)"
                        ),
                    )),
                }
            }
            _ => self.expect_i32("slot facing"),
        }
    }

    fn parse_f32_3(&mut self) -> Result<[f32; 3]> {
        self.expect(&TokenKind::LBracket, "'['")?;
        let x = self.parse_signed_float("offset x")?.0 as f32;
        self.expect(&TokenKind::Comma, "','")?;
        let y = self.parse_signed_float("offset y")?.0 as f32;
        self.expect(&TokenKind::Comma, "','")?;
        let z = self.parse_signed_float("offset z")?.0 as f32;
        self.eat(&TokenKind::Comma);
        self.expect(&TokenKind::RBracket, "']'")?;
        Ok([x, y, z])
    }

    fn parse_i32_3(&mut self) -> Result<[i32; 3]> {
        self.expect(&TokenKind::LBracket, "'['")?;
        let a = self.expect_i32("proximity[0]")?;
        self.expect(&TokenKind::Comma, "','")?;
        let b = self.expect_i32("proximity[1]")?;
        self.expect(&TokenKind::Comma, "','")?;
        let c = self.expect_i32("proximity[2]")?;
        self.eat(&TokenKind::Comma);
        self.expect(&TokenKind::RBracket, "']'")?;
        Ok([a, b, c])
    }

    fn parse_strings_decl(&mut self, span: Span) -> Result<StringsDecl> {
        let id = if matches!(self.peek().kind, TokenKind::Int(_)) {
            self.expect_u16("STR# chunk id")?
        } else {
            301
        };
        let label = if matches!(self.peek().kind, TokenKind::Str(_)) {
            Some(self.expect_string("STR# label")?.0)
        } else {
            None
        };
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut entries = Vec::new();
        while !self.eat(&TokenKind::RBrace) {
            let (name, nspan) = self.expect_ident("string entry name")?;
            if !self.eat(&TokenKind::Assign) && !self.eat(&TokenKind::Colon) {
                return Err(Error::at(
                    self.span(),
                    "expected '=' or ':' after string entry name",
                ));
            }
            let (text, _) = self.expect_string("string text")?;
            entries.push((nspan, name, text));
            if !self.eat(&TokenKind::Comma) && !self.eat(&TokenKind::Semicolon) {
                self.expect(&TokenKind::RBrace, "'}'")?;
                break;
            }
        }
        self.eat(&TokenKind::Semicolon);
        Ok(StringsDecl {
            span,
            id,
            label,
            entries,
        })
    }

    fn parse_meta_field(&mut self, key: &str, span: Span) -> Result<MetaField> {
        if !self.eat(&TokenKind::Colon) && !self.eat(&TokenKind::Assign) {
            return Err(Error::at(
                self.span(),
                format!("expected ':' or '=' after '{key}'"),
            ));
        }
        let kind = match key {
            "guid" => MetaFieldKind::Guid(self.expect_u32("guid")?),
            "price" => MetaFieldKind::Price(self.expect_u16("price")?),
            "description" => MetaFieldKind::Description(self.expect_string("description")?.0),
            "category" => MetaFieldKind::Category(self.parse_category()?),
            "object_type" | "type" => MetaFieldKind::ObjectType(self.expect_u16("object_type")?),
            "objd_id" => MetaFieldKind::ObjdId(self.expect_u16("objd_id")?),
            "ttab_id" => MetaFieldKind::TtabId(self.expect_u16("ttab_id")?),
            "slot_id" => MetaFieldKind::SlotId(self.expect_u16("slot_id")?),
            "ctss_id" => MetaFieldKind::CtssId(self.expect_u16("ctss_id")?),
            "dgrp_id" => MetaFieldKind::DgrpId(self.expect_u16("dgrp_id")?),
            "spr2_id" => MetaFieldKind::Spr2Id(self.expect_u16("spr2_id")?),
            "palt_id" => MetaFieldKind::PaltId(self.expect_u16("palt_id")?),
            "master_id" => MetaFieldKind::MasterId(self.expect_u16("master_id")?),
            "interaction_group" => {
                MetaFieldKind::InteractionGroup(self.expect_i16("interaction_group")?)
            }
            "data_58" => MetaFieldKind::Data58(self.expect_i16("data_58")?),
            "attributes" => MetaFieldKind::Attributes(self.expect_u16("attributes")?),
            "sprite" => MetaFieldKind::Sprite(self.parse_sprite_decl()?),
            "graphics" => {
                self.expect(&TokenKind::LBracket, "'['")?;
                let mut list = Vec::new();
                while !self.eat(&TokenKind::RBracket) {
                    list.push(self.parse_sprite_decl()?);
                    if !self.eat(&TokenKind::Comma) {
                        self.expect(&TokenKind::RBracket, "']'")?;
                        break;
                    }
                }
                MetaFieldKind::Graphics(list)
            }
            "tiles" => MetaFieldKind::Tiles(self.parse_tiles_array()?),
            _ => unreachable!("guarded by is_meta_field_name"),
        };
        self.eat_member_terminator();
        Ok(MetaField { span, kind })
    }

    fn parse_multi_tile_block(
        &mut self,
        span: Span,
        members: &mut Vec<ObjectMember>,
    ) -> Result<()> {
        self.expect(&TokenKind::LBrace, "'{'")?;
        while !self.eat(&TokenKind::RBrace) {
            let (key, kspan) = self.expect_ident("multi_tile field ('master_id' or 'tiles')")?;
            if !self.eat(&TokenKind::Colon) && !self.eat(&TokenKind::Assign) {
                return Err(Error::at(
                    self.span(),
                    format!("expected ':' or '=' after '{key}'"),
                ));
            }
            match key.as_str() {
                "master_id" => {
                    let id = self.expect_u16("master_id")?;
                    members.push(ObjectMember::Meta(MetaField {
                        span: kspan,
                        kind: MetaFieldKind::MasterId(id),
                    }));
                }
                "tiles" => {
                    let tiles = self.parse_tiles_array()?;
                    members.push(ObjectMember::Meta(MetaField {
                        span: kspan,
                        kind: MetaFieldKind::Tiles(tiles),
                    }));
                }
                other => {
                    return Err(Error::at(
                        kspan,
                        format!("unknown multi_tile field '{other}'"),
                    ));
                }
            }
            if !self.eat(&TokenKind::Comma) && !self.eat(&TokenKind::Semicolon) {
                self.expect(&TokenKind::RBrace, "'}'")?;
                break;
            }
        }
        let _ = span;
        self.eat(&TokenKind::Semicolon);
        Ok(())
    }

    fn parse_tiles_array(&mut self) -> Result<Vec<TileDecl>> {
        self.expect(&TokenKind::LBracket, "'['")?;
        let mut tiles = Vec::new();
        while !self.eat(&TokenKind::RBracket) {
            let tspan = self.span();
            let _ = self.eat_ident("tile");
            self.expect(&TokenKind::LBrace, "'{'")?;
            let mut guid: Option<u32> = None;
            let mut name: Option<String> = None;
            let mut objd_id: Option<u16> = None;
            let mut sub_index: i16 = 0;
            let mut level: i16 = 0;
            let mut lead = false;
            let mut interaction_group: Option<i16> = None;
            let mut dgrp_id: Option<u16> = None;
            let mut shape: Option<FurnitureShape> = None;
            let mut slot_id: Option<u16> = None;
            let mut ttab_id: Option<u16> = None;
            let mut attributes: Option<u16> = None;
            while !self.eat(&TokenKind::RBrace) {
                let (key, kspan) = self.expect_ident("tile field")?;
                self.expect(&TokenKind::Colon, "':'")?;
                match key.as_str() {
                    "guid" => guid = Some(self.expect_u32("tile guid")?),
                    "name" => name = Some(self.expect_string("tile name")?.0),
                    "objd_id" => objd_id = Some(self.expect_u16("tile objd_id")?),
                    "offset" | "sub_index" => {
                        sub_index = self.parse_tile_sub_index(kspan)?;
                    }
                    "level" => level = self.expect_i16("tile level")?,
                    "lead" => lead = self.expect_bool("tile lead")?,
                    "interaction_group" => {
                        interaction_group = Some(self.expect_i16("tile interaction_group")?);
                    }
                    "dgrp_id" => dgrp_id = Some(self.expect_u16("tile dgrp_id")?),
                    "shape" => {
                        let (sname, sspan) = self.expect_string("tile shape")?;
                        shape = Some(parse_furniture_shape(&sname, sspan)?);
                    }
                    "slot_id" => slot_id = Some(self.expect_u16("tile slot_id")?),
                    "ttab_id" => ttab_id = Some(self.expect_u16("tile ttab_id")?),
                    "attributes" => attributes = Some(self.expect_u16("tile attributes")?),
                    other => {
                        return Err(Error::at(kspan, format!("unknown tile field '{other}'")));
                    }
                }
                if !self.eat(&TokenKind::Comma) && !self.eat(&TokenKind::Semicolon) {
                    self.expect(&TokenKind::RBrace, "'}'")?;
                    break;
                }
            }
            let guid = guid.ok_or_else(|| Error::at(tspan, "tile requires 'guid'"))?;
            tiles.push(TileDecl {
                span: tspan,
                guid,
                name,
                objd_id,
                sub_index,
                level,
                lead,
                interaction_group,
                dgrp_id,
                shape,
                slot_id,
                ttab_id,
                attributes,
            });
            if !self.eat(&TokenKind::Comma) {
                self.expect(&TokenKind::RBracket, "']'")?;
                break;
            }
        }
        Ok(tiles)
    }

    fn parse_tile_sub_index(&mut self, span: Span) -> Result<i16> {
        if self.eat(&TokenKind::LBracket) {
            let dx_i64 = self.parse_signed_int("tile offset dx")?.0;
            self.expect(&TokenKind::Comma, "','")?;
            let dy_i64 = self.parse_signed_int("tile offset dy")?.0;
            self.eat(&TokenKind::Comma);
            self.expect(&TokenKind::RBracket, "']'")?;
            let dx = i8::try_from(dx_i64)
                .map_err(|_| Error::at(span, format!("tile offset dx {dx_i64} out of i8 range")))?;
            let dy = u8::try_from(dy_i64)
                .map_err(|_| Error::at(span, format!("tile offset dy {dy_i64} out of u8 range")))?;
            let encoded = (i16::from(dx) << 8) | i16::from(dy);
            if encoded == -1 {
                return Err(Error::at(
                    span,
                    "tile offset [-1, 255] collides with master sentinel -1",
                ));
            }
            Ok(encoded)
        } else {
            let val = self.expect_i16("tile sub_index")?;
            if val == -1 {
                return Err(Error::at(
                    span,
                    "tile sub_index cannot be -1 (reserved for master OBJD)",
                ));
            }
            Ok(val)
        }
    }

    fn parse_category(&mut self) -> Result<u16> {
        match &self.peek().kind {
            TokenKind::Int(_) | TokenKind::Minus | TokenKind::Plus => self.expect_u16("category"),
            TokenKind::Ident(_) => {
                let (cat, span) = self.expect_ident("category name")?;
                match cat.as_str() {
                    "seating" => Ok(1),
                    "surfaces" => Ok(2),
                    "appliances" => Ok(4),
                    "electronics" => Ok(8),
                    "plumbing" => Ok(16),
                    "decorative" => Ok(32),
                    "misc" | "general" => Ok(64),
                    "lighting" => Ok(128),
                    other => Err(Error::at(
                        span,
                        format!(
                            "unknown category '{other}' (expected seating/surfaces/appliances/electronics/plumbing/decorative/misc/lighting or u16)"
                        ),
                    )),
                }
            }
            other => Err(Error::at(
                self.span(),
                format!("expected category name or u16, found {other:?}"),
            )),
        }
    }

    fn parse_sprite_decl(&mut self) -> Result<SpriteDecl> {
        let span = self.span();
        let (head, hspan) = self.expect_ident("'Sprite'")?;
        if head != "Sprite" {
            return Err(Error::at(
                hspan,
                format!("expected 'Sprite::procedural(...)', found '{head}'"),
            ));
        }
        self.expect(&TokenKind::ColonColon, "'::'")?;
        let (ctor, cspan) = self.expect_ident("'procedural'")?;
        if ctor != "procedural" {
            return Err(Error::at(
                cspan,
                format!("unknown Sprite constructor '{ctor}' (expected 'procedural')"),
            ));
        }
        self.expect(&TokenKind::LParen, "'('")?;
        let (shape_str, sspan) = self.expect_string("sprite shape name")?;
        let shape = parse_furniture_shape(&shape_str, sspan)?;
        let mut primary_rgb = [152u8, 112, 76];
        let mut accent_rgb = [204u8, 172, 132];
        let mut trim_rgb = [84u8, 58, 38];
        while self.eat(&TokenKind::Comma) {
            if self.peek().kind == TokenKind::RParen {
                break;
            }
            let (field, fspan) =
                self.expect_ident("sprite color field ('primary', 'accent', 'trim')")?;
            self.expect(&TokenKind::Colon, "':'")?;
            let rgb = self.parse_color_value()?;
            match field.as_str() {
                "primary" | "primary_rgb" => primary_rgb = rgb,
                "accent" | "accent_rgb" => accent_rgb = rgb,
                "trim" | "trim_rgb" => trim_rgb = rgb,
                other => {
                    return Err(Error::at(
                        fspan,
                        format!("unknown Sprite::procedural field '{other}'"),
                    ));
                }
            }
        }
        self.expect(&TokenKind::RParen, "')'")?;
        Ok(SpriteDecl {
            span,
            shape,
            primary_rgb,
            accent_rgb,
            trim_rgb,
        })
    }

    fn parse_color_value(&mut self) -> Result<[u8; 3]> {
        let span = self.span();
        match self.peek().kind {
            TokenKind::Color(rgb) => {
                self.bump();
                Ok(rgb)
            }
            TokenKind::LBracket => {
                self.bump();
                let r = self.parse_u8_channel("red channel")?;
                self.expect(&TokenKind::Comma, "','")?;
                let g = self.parse_u8_channel("green channel")?;
                self.expect(&TokenKind::Comma, "','")?;
                let b = self.parse_u8_channel("blue channel")?;
                self.eat(&TokenKind::Comma);
                self.expect(&TokenKind::RBracket, "']'")?;
                Ok([r, g, b])
            }
            ref other => Err(Error::at(
                span,
                format!("expected #RRGGBB color or [r, g, b] array, found {other:?}"),
            )),
        }
    }

    fn parse_u8_channel(&mut self, desc: &str) -> Result<u8> {
        let (v, span) = self.parse_signed_int(desc)?;
        u8::try_from(v).map_err(|_| Error::at(span, format!("{desc} {v} out of 0..=255 range")))
    }

    fn parse_const_int_expr(&mut self) -> Result<ConstIntExpr> {
        let span = self.span();
        let negate = self.eat(&TokenKind::Minus);
        let _ = self.eat(&TokenKind::Plus);
        match &self.peek().kind {
            TokenKind::Int(v) => {
                let v = *v;
                self.bump();
                let signed = if negate { -v } else { v };
                let val = i16::try_from(signed)
                    .map_err(|_| Error::at(span, format!("constant {signed} out of i16 range")))?;
                Ok(ConstIntExpr::Literal(val))
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.bump();
                Ok(ConstIntExpr::Param { name, negate })
            }
            other => Err(Error::at(
                span,
                format!("expected integer literal or parameter name, found {other:?}"),
            )),
        }
    }

    fn parse_guard(&mut self, span: Span) -> Result<GuardDecl> {
        let mut sim_binding = "sim".to_owned();
        if self.eat(&TokenKind::LParen) && !self.eat(&TokenKind::RParen) {
            sim_binding = self.expect_ident("guard sim parameter")?.0;
            self.expect(&TokenKind::RParen, "')'")?;
        }
        let kind = if self.eat(&TokenKind::FatArrow) {
            GuardBody::Expr(self.parse_expr()?)
        } else {
            GuardBody::Block(self.parse_block()?)
        };
        Ok(GuardDecl {
            span,
            sim_binding,
            kind,
        })
    }

    fn parse_functional_decl(
        &mut self,
        span: Span,
        pinned_id: Option<u16>,
        pinned_guard_id: Option<u16>,
    ) -> Result<FunctionalDecl> {
        let (kind, kind_span) = self.expect_ident("functional kind (e.g. 'prep', 'cook')")?;
        let params = if self.peek().kind == TokenKind::LParen {
            self.parse_fn_params()?
        } else {
            Vec::new()
        };
        self.expect(&TokenKind::LBrace, "'{'")?;

        // Check if this uses structured fields (`score:` / `guard` / `run`) or shorthand body.
        let is_structured = self.is_ident("score")
            || self.is_ident("guard")
            || self.is_ident("run")
            || self.peek().kind == TokenKind::RBrace;

        if !is_structured {
            let mut body = Vec::new();
            while !self.eat(&TokenKind::RBrace) {
                body.push(self.parse_stmt()?);
            }
            return Ok(FunctionalDecl {
                span,
                kind,
                kind_span,
                params,
                score: None,
                pinned_id,
                pinned_guard_id,
                guard: None,
                sim_binding: "sim".to_owned(),
                body,
            });
        }

        let mut score = None;
        let mut guard = None;
        let mut sim_binding = "sim".to_owned();
        let mut body: Option<Vec<V2Stmt>> = None;

        while !self.eat(&TokenKind::RBrace) {
            let (field, fspan) = self.expect_ident("functional field ('score', 'guard', 'run')")?;
            match field.as_str() {
                "score" => {
                    self.expect(&TokenKind::Colon, "':'")?;
                    score = Some(self.parse_const_int_expr()?);
                    self.eat_member_terminator();
                }
                "guard" => {
                    if guard.is_some() {
                        return Err(Error::at(fspan, "duplicate 'guard' in functional block"));
                    }
                    guard = Some(self.parse_guard(fspan)?);
                    self.eat_member_terminator();
                }
                "run" => {
                    if body.is_some() {
                        return Err(Error::at(fspan, "duplicate 'run' in functional block"));
                    }
                    if self.eat(&TokenKind::LParen) && !self.eat(&TokenKind::RParen) {
                        sim_binding = self.expect_ident("sim parameter name")?.0;
                        self.expect(&TokenKind::RParen, "')'")?;
                    }
                    body = Some(self.parse_block()?);
                    self.eat_member_terminator();
                }
                other => {
                    return Err(Error::at(
                        fspan,
                        format!(
                            "unknown functional field '{other}' (expected 'score', 'guard', or 'run')"
                        ),
                    ));
                }
            }
        }
        let body = body.ok_or_else(|| {
            Error::at(
                span,
                format!("functional '{kind}' requires a 'run(sim) {{ ... }}' block"),
            )
        })?;
        Ok(FunctionalDecl {
            span,
            kind,
            kind_span,
            params,
            score,
            pinned_id,
            pinned_guard_id,
            guard,
            sim_binding,
            body,
        })
    }

    fn parse_interaction_decl(
        &mut self,
        span: Span,
        pinned_id: Option<u16>,
        pinned_guard_id: Option<u16>,
    ) -> Result<InteractionDecl> {
        let (label, _) = self.expect_string("interaction label")?;
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut autonomy = 50u32;
        let mut attenuation_code = 0u32;
        let mut attenuation = 0.02f32;
        let mut flags = 0u32;
        let mut flags2 = 0u32;
        let mut joining_index = -1i32;
        let mut advertises = Vec::new();
        let mut guard = None;
        let mut sim_binding = "sim".to_owned();
        let mut run_body: Option<Vec<V2Stmt>> = None;

        while !self.eat(&TokenKind::RBrace) {
            let (field, fspan) = self.expect_ident("interaction field")?;
            match field.as_str() {
                "autonomy" => {
                    self.expect(&TokenKind::Colon, "':'")?;
                    autonomy = self.expect_u32("autonomy")?;
                    self.eat_member_terminator();
                }
                "attenuation_code" => {
                    self.expect(&TokenKind::Colon, "':'")?;
                    attenuation_code = self.expect_u32("attenuation_code")?;
                    self.eat_member_terminator();
                }
                "attenuation" => {
                    self.expect(&TokenKind::Colon, "':'")?;
                    attenuation = self.parse_signed_float("attenuation")?.0 as f32;
                    self.eat_member_terminator();
                }
                "flags" => {
                    self.expect(&TokenKind::Colon, "':'")?;
                    flags = self.expect_u32("interaction flags")?;
                    self.eat_member_terminator();
                }
                "flags2" => {
                    self.expect(&TokenKind::Colon, "':'")?;
                    flags2 = self.expect_u32("interaction flags2")?;
                    self.eat_member_terminator();
                }
                "joining_index" => {
                    self.expect(&TokenKind::Colon, "':'")?;
                    joining_index = self.expect_i32("joining_index")?;
                    self.eat_member_terminator();
                }
                "advertises" => {
                    self.expect(&TokenKind::Colon, "':'")?;
                    self.expect(&TokenKind::LBracket, "'['")?;
                    while !self.eat(&TokenKind::RBracket) {
                        advertises.push(self.parse_advertise_entry()?);
                        if !self.eat(&TokenKind::Comma) {
                            self.expect(&TokenKind::RBracket, "']'")?;
                            break;
                        }
                    }
                    self.eat_member_terminator();
                }
                "guard" => {
                    if guard.is_some() {
                        return Err(Error::at(
                            fspan,
                            format!("duplicate 'guard' in interaction '{label}'"),
                        ));
                    }
                    guard = Some(self.parse_guard(fspan)?);
                    self.eat_member_terminator();
                }
                "run" => {
                    if run_body.is_some() {
                        return Err(Error::at(
                            fspan,
                            format!("duplicate 'run' in interaction '{label}'"),
                        ));
                    }
                    if self.eat(&TokenKind::LParen) && !self.eat(&TokenKind::RParen) {
                        sim_binding = self.expect_ident("sim parameter name")?.0;
                        self.expect(&TokenKind::RParen, "')'")?;
                    }
                    run_body = Some(self.parse_block()?);
                    self.eat_member_terminator();
                }
                other => {
                    return Err(Error::at(
                        fspan,
                        format!("unknown interaction field '{other}'"),
                    ));
                }
            }
        }
        let run_body = run_body.ok_or_else(|| {
            Error::at(
                span,
                format!("interaction '{label}' requires a 'run(sim) {{ ... }}' block"),
            )
        })?;
        Ok(InteractionDecl {
            span,
            label,
            pinned_id,
            pinned_guard_id,
            autonomy,
            attenuation_code,
            attenuation,
            flags,
            flags2,
            joining_index,
            advertises,
            guard,
            sim_binding,
            run_body,
        })
    }

    fn parse_advertise_entry(&mut self) -> Result<AdvertiseDecl> {
        let (mname, span) = self.expect_ident("motive name")?;
        let motive_index = usize::from(
            motive_by_name(&mname)
                .ok_or_else(|| Error::at(span, format!("unknown motive '{mname}'")))?,
        );
        let delta = self.parse_const_int_expr()?;
        let mut minimum = ConstIntExpr::Literal(-100);
        let mut personality = 0u16;
        if self.eat(&TokenKind::LParen) {
            while !self.eat(&TokenKind::RParen) {
                let (k, kspan) = self.expect_ident("advertise option ('min' or 'personality')")?;
                self.expect(&TokenKind::Colon, "':'")?;
                match k.as_str() {
                    "min" => minimum = self.parse_const_int_expr()?,
                    "personality" => personality = self.expect_u16("personality")?,
                    other => {
                        return Err(Error::at(
                            kspan,
                            format!("unknown advertise option '{other}'"),
                        ));
                    }
                }
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RParen, "')'")?;
                    break;
                }
            }
        }
        Ok(AdvertiseDecl {
            span,
            motive_index,
            delta,
            minimum,
            personality,
        })
    }

    fn parse_block(&mut self) -> Result<Vec<V2Stmt>> {
        let span = self.expect(&TokenKind::LBrace, "'{'")?;
        self.enter_nesting(span)?;
        let mut stmts = Vec::new();
        let res = loop {
            if self.eat(&TokenKind::RBrace) {
                break Ok(stmts);
            }
            if self.peek().kind == TokenKind::Eof {
                break Err(Error::at(span, "unterminated '{ ... }' block"));
            }
            match self.parse_stmt() {
                Ok(s) => stmts.push(s),
                Err(e) => break Err(e),
            }
        };
        self.leave_nesting();
        res
    }

    fn parse_stmt(&mut self) -> Result<V2Stmt> {
        let span = self.span();
        if let TokenKind::RawBlock(code) = &self.peek().kind {
            let code = code.clone();
            self.bump();
            return Ok(V2Stmt::RawAir { span, code });
        }
        if let TokenKind::Ident(kw) = &self.peek().kind {
            match kw.as_str() {
                "let" => return self.parse_let_stmt(span),
                "with" => return self.parse_with_lock_stmt(span),
                "defer" => {
                    self.bump();
                    let body = self.parse_block()?;
                    return Ok(V2Stmt::Defer { span, body });
                }
                "repeat" => return self.parse_repeat_stmt(span),
                "while" => return self.parse_while_stmt(span),
                "for" => return self.parse_for_stmt(span),
                "if" => return self.parse_if_stmt(span),
                "break" => {
                    self.bump();
                    self.expect(&TokenKind::Semicolon, "';'")?;
                    return Ok(V2Stmt::Break { span });
                }
                "continue" => {
                    self.bump();
                    self.expect(&TokenKind::Semicolon, "';'")?;
                    return Ok(V2Stmt::Continue { span });
                }
                "return" => {
                    self.bump();
                    if self.eat(&TokenKind::Semicolon) {
                        return Ok(V2Stmt::Return { span, value: None });
                    }
                    let expr = self.parse_expr()?;
                    self.expect(&TokenKind::Semicolon, "';'")?;
                    return Ok(V2Stmt::Return {
                        span,
                        value: Some(expr),
                    });
                }
                _ => {}
            }
        }

        // Parse expression or assignment statement.
        let lhs = self.parse_expr()?;
        if let Some(op) = self.try_eat_assign_op() {
            let value = self.parse_expr()?;
            self.expect(&TokenKind::Semicolon, "';'")?;
            return Ok(V2Stmt::Assign {
                span,
                target: lhs,
                op,
                value,
            });
        }
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(V2Stmt::Expr { span, expr: lhs })
    }

    fn try_eat_assign_op(&mut self) -> Option<V2AssignOp> {
        let op = match &self.peek().kind {
            TokenKind::Assign => V2AssignOp::Set,
            TokenKind::PlusEq => V2AssignOp::Add,
            TokenKind::MinusEq => V2AssignOp::Sub,
            TokenKind::StarEq => V2AssignOp::Mul,
            TokenKind::SlashEq => V2AssignOp::Div,
            TokenKind::PercentEq => V2AssignOp::Mod,
            TokenKind::AmpEq => V2AssignOp::BitAnd,
            TokenKind::PipeEq => V2AssignOp::BitOr,
            TokenKind::CaretEq => V2AssignOp::BitXor,
            TokenKind::Ident(s) if s == "flag_set" => V2AssignOp::FlagSet,
            TokenKind::Ident(s) if s == "flag_clear" => V2AssignOp::FlagClear,
            _ => return None,
        };
        self.bump();
        Some(op)
    }

    fn parse_let_stmt(&mut self, span: Span) -> Result<V2Stmt> {
        self.bump(); // 'let'
        // `let Some(name) = expr else { ... };`
        if self.eat_ident("Some") {
            self.expect(&TokenKind::LParen, "'('")?;
            let (name, _) = self.expect_ident("binding name")?;
            self.expect(&TokenKind::RParen, "')'")?;
            self.expect(&TokenKind::Assign, "'='")?;
            let init = self.parse_expr()?;
            if !self.eat_ident("else") {
                return Err(Error::at(
                    self.span(),
                    "expected 'else { ... }' after 'let Some(...) = ...'",
                ));
            }
            let else_body = self.parse_block()?;
            self.eat(&TokenKind::Semicolon);
            return Ok(V2Stmt::LetSome {
                span,
                name,
                init,
                else_body,
            });
        }

        let _is_mut = self.eat_ident("mut");
        let (name, _) = self.expect_ident("variable name")?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&TokenKind::Assign, "'='")?;
        let init = self.parse_expr()?;
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(V2Stmt::Let {
            span,
            name,
            ty,
            init,
        })
    }

    fn parse_with_lock_stmt(&mut self, span: Span) -> Result<V2Stmt> {
        self.bump(); // 'with'
        if !self.eat_ident("lock") {
            return Err(Error::at(self.span(), "expected 'lock(...)' after 'with'"));
        }
        self.expect(&TokenKind::LParen, "'('")?;
        let lock_expr = self.parse_expr()?;
        self.expect(&TokenKind::RParen, "')'")?;
        let body = self.parse_block()?;
        Ok(V2Stmt::WithLock {
            span,
            lock_expr,
            body,
        })
    }

    fn parse_repeat_stmt(&mut self, span: Span) -> Result<V2Stmt> {
        self.bump(); // 'repeat'
        let count = self.parse_expr()?;
        let cancel_on_abort = self.eat_ident("cancel_on_abort");
        let body = self.parse_block()?;
        Ok(V2Stmt::Repeat {
            span,
            count,
            cancel_on_abort,
            body,
        })
    }

    fn parse_while_stmt(&mut self, span: Span) -> Result<V2Stmt> {
        self.bump(); // 'while'
        let cond = self.parse_expr()?;
        let cancel_on_abort = self.eat_ident("cancel_on_abort");
        let body = self.parse_block()?;
        Ok(V2Stmt::While {
            span,
            cond,
            cancel_on_abort,
            body,
        })
    }

    fn parse_for_stmt(&mut self, span: Span) -> Result<V2Stmt> {
        self.bump(); // 'for'
        let (binding, _) = self.expect_ident("loop variable name")?;
        if !self.eat_ident("in") {
            return Err(Error::at(self.span(), "expected 'in' in 'for' loop"));
        }
        let ispan = self.span();
        let (src, _) = self.expect_ident("iterator ('objects_by_guid' or 'self')")?;
        let iter = match src.as_str() {
            "objects_by_guid" => {
                self.expect(&TokenKind::LParen, "'('")?;
                let guid = self.expect_u32("object GUID")?;
                self.expect(&TokenKind::RParen, "')'")?;
                ForEachIter::ObjectsByGuid(guid)
            }
            "self" => {
                self.expect(&TokenKind::Dot, "'.'")?;
                let (m, mspan) = self.expect_ident("'parts'")?;
                if m != "parts" {
                    return Err(Error::at(
                        mspan,
                        format!("unknown self iterator 'self.{m}()' (expected 'self.parts()')"),
                    ));
                }
                self.expect(&TokenKind::LParen, "'('")?;
                self.expect(&TokenKind::RParen, "')'")?;
                ForEachIter::SelfParts
            }
            other => {
                return Err(Error::at(
                    ispan,
                    format!(
                        "unsupported 'for' iterator '{other}' (expected 'objects_by_guid(0x...)' or 'self.parts()')"
                    ),
                ));
            }
        };
        let body = self.parse_block()?;
        Ok(V2Stmt::ForEach {
            span,
            binding,
            iter,
            body,
        })
    }

    fn parse_if_stmt(&mut self, span: Span) -> Result<V2Stmt> {
        self.bump(); // 'if'
        let cond = self.parse_expr()?;
        let then_body = self.parse_block()?;
        let else_body = if self.eat_ident("else") {
            if self.is_ident("if") {
                let elif_span = self.span();
                self.enter_nesting(elif_span)?;
                let elif = self.parse_if_stmt(elif_span);
                self.leave_nesting();
                vec![elif?]
            } else {
                self.parse_block()?
            }
        } else {
            Vec::new()
        };
        Ok(V2Stmt::If {
            span,
            cond,
            then_body,
            else_body,
        })
    }

    fn parse_expr(&mut self) -> Result<V2Expr> {
        self.parse_or_expr()
    }

    fn parse_or_expr(&mut self) -> Result<V2Expr> {
        let mut lhs = self.parse_and_expr()?;
        let mut chain = 0usize;
        while self.peek().kind == TokenKind::OrOr {
            let span = self.bump().span;
            chain += 1;
            if self.depth + chain > MAX_NESTING_DEPTH {
                return Err(Error::at(
                    span,
                    format!("expression nesting exceeds maximum depth of {MAX_NESTING_DEPTH}"),
                ));
            }
            let rhs = self.parse_and_expr()?;
            lhs = V2Expr::Binary {
                span,
                lhs: Box::new(lhs),
                op: V2BinOp::Or,
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_and_expr(&mut self) -> Result<V2Expr> {
        let mut lhs = self.parse_cmp_expr()?;
        let mut chain = 0usize;
        while self.peek().kind == TokenKind::AndAnd {
            let span = self.bump().span;
            chain += 1;
            if self.depth + chain > MAX_NESTING_DEPTH {
                return Err(Error::at(
                    span,
                    format!("expression nesting exceeds maximum depth of {MAX_NESTING_DEPTH}"),
                ));
            }
            let rhs = self.parse_cmp_expr()?;
            lhs = V2Expr::Binary {
                span,
                lhs: Box::new(lhs),
                op: V2BinOp::And,
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_cmp_expr(&mut self) -> Result<V2Expr> {
        let lhs = self.parse_bitor_expr()?;
        let op = match &self.peek().kind {
            TokenKind::EqEq => Some(V2BinOp::Eq),
            TokenKind::BangEq => Some(V2BinOp::Ne),
            TokenKind::Lt => Some(V2BinOp::Lt),
            TokenKind::LtEq => Some(V2BinOp::Le),
            TokenKind::Gt => Some(V2BinOp::Gt),
            TokenKind::GtEq => Some(V2BinOp::Ge),
            TokenKind::Ident(s) if s == "flag_test" => Some(V2BinOp::FlagTest),
            _ => None,
        };
        if let Some(op) = op {
            let span = self.bump().span;
            let rhs = self.parse_bitor_expr()?;
            Ok(V2Expr::Binary {
                span,
                lhs: Box::new(lhs),
                op,
                rhs: Box::new(rhs),
            })
        } else {
            Ok(lhs)
        }
    }

    fn parse_bitor_expr(&mut self) -> Result<V2Expr> {
        let mut lhs = self.parse_bitxor_expr()?;
        let mut chain = 0usize;
        while self.peek().kind == TokenKind::Pipe {
            let span = self.bump().span;
            chain += 1;
            if self.depth + chain > MAX_NESTING_DEPTH {
                return Err(Error::at(
                    span,
                    format!("expression nesting exceeds maximum depth of {MAX_NESTING_DEPTH}"),
                ));
            }
            let rhs = self.parse_bitxor_expr()?;
            lhs = V2Expr::Binary {
                span,
                lhs: Box::new(lhs),
                op: V2BinOp::BitOr,
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_bitxor_expr(&mut self) -> Result<V2Expr> {
        let mut lhs = self.parse_bitand_expr()?;
        let mut chain = 0usize;
        while self.peek().kind == TokenKind::Caret {
            let span = self.bump().span;
            chain += 1;
            if self.depth + chain > MAX_NESTING_DEPTH {
                return Err(Error::at(
                    span,
                    format!("expression nesting exceeds maximum depth of {MAX_NESTING_DEPTH}"),
                ));
            }
            let rhs = self.parse_bitand_expr()?;
            lhs = V2Expr::Binary {
                span,
                lhs: Box::new(lhs),
                op: V2BinOp::BitXor,
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_bitand_expr(&mut self) -> Result<V2Expr> {
        let mut lhs = self.parse_add_expr()?;
        let mut chain = 0usize;
        while self.peek().kind == TokenKind::Amp {
            let span = self.bump().span;
            chain += 1;
            if self.depth + chain > MAX_NESTING_DEPTH {
                return Err(Error::at(
                    span,
                    format!("expression nesting exceeds maximum depth of {MAX_NESTING_DEPTH}"),
                ));
            }
            let rhs = self.parse_add_expr()?;
            lhs = V2Expr::Binary {
                span,
                lhs: Box::new(lhs),
                op: V2BinOp::BitAnd,
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_add_expr(&mut self) -> Result<V2Expr> {
        let mut lhs = self.parse_mul_expr()?;
        let mut chain = 0usize;
        loop {
            let op = match self.peek().kind {
                TokenKind::Plus => V2BinOp::Add,
                TokenKind::Minus => V2BinOp::Sub,
                _ => break,
            };
            let span = self.bump().span;
            chain += 1;
            if self.depth + chain > MAX_NESTING_DEPTH {
                return Err(Error::at(
                    span,
                    format!("expression nesting exceeds maximum depth of {MAX_NESTING_DEPTH}"),
                ));
            }
            let rhs = self.parse_mul_expr()?;
            lhs = V2Expr::Binary {
                span,
                lhs: Box::new(lhs),
                op,
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_mul_expr(&mut self) -> Result<V2Expr> {
        let mut lhs = self.parse_unary_expr()?;
        let mut chain = 0usize;
        loop {
            let op = match self.peek().kind {
                TokenKind::Star => V2BinOp::Mul,
                TokenKind::Slash => V2BinOp::Div,
                TokenKind::Percent => V2BinOp::Mod,
                _ => break,
            };
            let span = self.bump().span;
            chain += 1;
            if self.depth + chain > MAX_NESTING_DEPTH {
                return Err(Error::at(
                    span,
                    format!("expression nesting exceeds maximum depth of {MAX_NESTING_DEPTH}"),
                ));
            }
            let rhs = self.parse_unary_expr()?;
            lhs = V2Expr::Binary {
                span,
                lhs: Box::new(lhs),
                op,
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_unary_expr(&mut self) -> Result<V2Expr> {
        let span = self.span();
        match self.peek().kind {
            TokenKind::Bang => {
                self.bump();
                self.enter_nesting(span)?;
                let inner = self.parse_unary_expr();
                self.leave_nesting();
                Ok(V2Expr::Unary {
                    span,
                    op: V2UnaryOp::Not,
                    inner: Box::new(inner?),
                })
            }
            TokenKind::Minus => {
                self.bump();
                self.enter_nesting(span)?;
                let inner = self.parse_unary_expr();
                self.leave_nesting();
                Ok(V2Expr::Unary {
                    span,
                    op: V2UnaryOp::Neg,
                    inner: Box::new(inner?),
                })
            }
            TokenKind::Star => {
                self.bump();
                self.enter_nesting(span)?;
                let inner = self.parse_unary_expr();
                self.leave_nesting();
                Ok(V2Expr::Deref {
                    span,
                    inner: Box::new(inner?),
                })
            }
            TokenKind::Amp => {
                self.bump();
                if !self.eat_ident("mut") {
                    return Err(Error::at(
                        span,
                        "expected '&mut <place>' for mutable reference argument",
                    ));
                }
                self.enter_nesting(span)?;
                let inner = self.parse_unary_expr();
                self.leave_nesting();
                Ok(V2Expr::MutRef {
                    span,
                    inner: Box::new(inner?),
                })
            }
            _ => self.parse_postfix_expr(),
        }
    }

    fn parse_postfix_expr(&mut self) -> Result<V2Expr> {
        let mut expr = self.parse_primary_expr()?;
        let mut chain = 0usize;
        loop {
            match self.peek().kind {
                TokenKind::Dot => {
                    let span = self.bump().span;
                    chain += 1;
                    if self.depth + chain > MAX_NESTING_DEPTH {
                        return Err(Error::at(
                            span,
                            format!(
                                "expression nesting depth exceeds limit of {MAX_NESTING_DEPTH}"
                            ),
                        ));
                    }
                    let (field, _) = self.expect_ident("field or method name")?;
                    if self.peek().kind == TokenKind::LParen {
                        let args = self.parse_call_args()?;
                        expr = V2Expr::MethodCall {
                            span,
                            receiver: Box::new(expr),
                            method: field,
                            args,
                        };
                    } else {
                        expr = V2Expr::Field {
                            span,
                            base: Box::new(expr),
                            field,
                        };
                    }
                }
                TokenKind::LBracket => {
                    let span = self.bump().span;
                    chain += 1;
                    if self.depth + chain > MAX_NESTING_DEPTH {
                        return Err(Error::at(
                            span,
                            format!(
                                "expression nesting depth exceeds limit of {MAX_NESTING_DEPTH}"
                            ),
                        ));
                    }
                    self.enter_nesting(span)?;
                    let index = self.parse_expr();
                    self.leave_nesting();
                    let index = index?;
                    self.expect(&TokenKind::RBracket, "']'")?;
                    expr = V2Expr::Index {
                        span,
                        base: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                TokenKind::LParen => {
                    let span = self.span();
                    chain += 1;
                    if self.depth + chain > MAX_NESTING_DEPTH {
                        return Err(Error::at(
                            span,
                            format!(
                                "expression nesting depth exceeds limit of {MAX_NESTING_DEPTH}"
                            ),
                        ));
                    }
                    let args = self.parse_call_args()?;
                    expr = V2Expr::Call {
                        span,
                        callee: Box::new(expr),
                        args,
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_call_args(&mut self) -> Result<Vec<CallArg>> {
        let open_span = self.expect(&TokenKind::LParen, "'('")?;
        self.enter_nesting(open_span)?;
        let mut args = Vec::new();
        let res = loop {
            if self.eat(&TokenKind::RParen) {
                break Ok(args);
            }
            let aspan = self.span();
            let parsed = if matches!(&self.peek().kind, TokenKind::Ident(_))
                && self.peek_at(1).kind == TokenKind::Colon
            {
                match self.expect_ident("argument name") {
                    Ok((n, _)) => {
                        self.bump(); // ':'
                        self.parse_expr().map(|v| (Some(n), v))
                    }
                    Err(e) => Err(e),
                }
            } else {
                self.parse_expr().map(|v| (None, v))
            };
            match parsed {
                Ok((name, value)) => args.push(CallArg {
                    span: aspan,
                    name,
                    value,
                }),
                Err(e) => break Err(e),
            }
            if !self.eat(&TokenKind::Comma) {
                if let Err(e) = self.expect(&TokenKind::RParen, "')'") {
                    break Err(e);
                }
                break Ok(args);
            }
        };
        self.leave_nesting();
        res
    }

    fn parse_primary_expr(&mut self) -> Result<V2Expr> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Int(v) => {
                self.bump();
                Ok(V2Expr::Int(tok.span, v))
            }
            TokenKind::Str(s) => {
                self.bump();
                Ok(V2Expr::Str(tok.span, s))
            }
            TokenKind::Ident(s) => {
                self.bump();
                match s.as_str() {
                    "true" => Ok(V2Expr::Bool(tok.span, true)),
                    "false" => Ok(V2Expr::Bool(tok.span, false)),
                    _ => {
                        if self.peek().kind == TokenKind::ColonColon {
                            let mut segs = vec![s];
                            while self.eat(&TokenKind::ColonColon) {
                                let (seg, _) = self.expect_ident("path segment")?;
                                segs.push(seg);
                            }
                            Ok(V2Expr::Path(tok.span, segs))
                        } else {
                            Ok(V2Expr::Ident(tok.span, s))
                        }
                    }
                }
            }
            TokenKind::LParen => {
                self.bump();
                self.enter_nesting(tok.span)?;
                let inner = self.parse_expr();
                self.leave_nesting();
                let inner = inner?;
                self.expect(&TokenKind::RParen, "')'")?;
                Ok(inner)
            }
            other => Err(Error::at(
                tok.span,
                format!("expected expression, found {other:?}"),
            )),
        }
    }
}

fn is_meta_field_name(name: &str) -> bool {
    matches!(
        name,
        "guid"
            | "price"
            | "description"
            | "category"
            | "object_type"
            | "type"
            | "objd_id"
            | "ttab_id"
            | "slot_id"
            | "ctss_id"
            | "dgrp_id"
            | "spr2_id"
            | "palt_id"
            | "master_id"
            | "interaction_group"
            | "data_58"
            | "attributes"
            | "sprite"
            | "graphics"
            | "tiles"
    )
}

pub(crate) fn parse_furniture_shape(name: &str, span: Span) -> Result<FurnitureShape> {
    match name {
        "bench" => Ok(FurnitureShape::Bench),
        "cot" | "bed" => Ok(FurnitureShape::Cot),
        "basin" => Ok(FurnitureShape::Basin),
        "toilet" => Ok(FurnitureShape::Toilet),
        "sofa" | "couch" => Ok(FurnitureShape::Sofa),
        "television" | "tv" => Ok(FurnitureShape::Television),
        "bookshelf" | "bookcase" => Ok(FurnitureShape::Bookshelf),
        "table" => Ok(FurnitureShape::Table),
        "chair" => Ok(FurnitureShape::Chair),
        "floor_lamp" | "lamp" => Ok(FurnitureShape::FloorLamp),
        "trash_can" | "trash" => Ok(FurnitureShape::TrashCan),
        "front_door" | "door" => Ok(FurnitureShape::FrontDoor),
        "mailbox" => Ok(FurnitureShape::Mailbox),
        "double_bed" => Ok(FurnitureShape::DoubleBed),
        "double_bed_left" => Ok(FurnitureShape::DoubleBedLeft),
        "double_bed_right" => Ok(FurnitureShape::DoubleBedRight),
        "fridge" => Ok(FurnitureShape::Fridge),
        "counter" => Ok(FurnitureShape::Counter),
        "stove" => Ok(FurnitureShape::Stove),
        "meal_plate" | "plate" => Ok(FurnitureShape::MealPlate),
        "wall_phone" | "phone" => Ok(FurnitureShape::WallPhone),
        "shower" => Ok(FurnitureShape::Shower),
        "box" => Ok(FurnitureShape::Box),
        other => Err(Error::at(span, format!("unknown sprite shape '{other}'"))),
    }
}

pub(crate) fn parse_sound_preset(name: &str, span: Span) -> Result<SoundPreset> {
    match name {
        "ui_click" => Ok(SoundPreset::UiClick),
        "chime" => Ok(SoundPreset::Chime),
        "wood_creak" => Ok(SoundPreset::WoodCreak),
        "water_flush" => Ok(SoundPreset::WaterFlush),
        "soft_thud" => Ok(SoundPreset::SoftThud),
        "phone_ring" => Ok(SoundPreset::PhoneRing),
        "phone_dial" => Ok(SoundPreset::PhoneDial),
        "sizzle" => Ok(SoundPreset::Sizzle),
        "shower_spray" => Ok(SoundPreset::ShowerSpray),
        "wrench_clank" => Ok(SoundPreset::WrenchClank),
        other => Err(Error::at(span, format!("unknown sound preset '{other}'"))),
    }
}
