//! Recursive-descent parser for the high-level `.antics` v2 authoring language.

use crate::art::FurnitureShape;
use crate::audio::SoundPreset;
use crate::error::{Error, Result, Span};
use crate::parser::motive_by_name;
use crate::v2::ast::{
    AbiParam, AbiParamSlot, AdvertiseDecl, BconTableDecl, BitflagVariant, BitflagsDecl, CallArg,
    ConstDecl, ConstIntExpr, EnumDecl, EnumVariant, ExternBconDecl, ExternBconEntry, ExternFnDecl,
    ExternModDecl, FnDecl, FnParam, ForEachIter, FunctionalDecl, GuardBody, GuardDecl,
    InteractionDecl, LifecycleBlock, MatchArm, MatchExprArm, MatchPattern, MetaField,
    MetaFieldKind, ObjectDecl, ObjectMember, ParamAssignDecl, ParamDecl, ParamType, ParamValue,
    ReturnAbi, SlotDecl, SoundDecl, SpriteDecl, StateDecl, StateInitValue, StateType, StringsDecl,
    TemplateDecl, TileDecl, TreeFrame, TreeMemberDecl, UseDecl, UseItem, V2AssignOp, V2BinOp,
    V2Expr, V2File, V2Item, V2Stmt, V2Type, V2UnaryOp, VariantDecl,
};
use crate::v2::lexer::{Token, TokenKind, tokenize};
use crate::v2::specialize::{self, Arm, BlockItem, ConstBinding, Selector};
use std::collections::BTreeSet;

const MAX_NESTING_DEPTH: usize = 64;

/// Highest explicit `state name @ <slot>` attribute index accepted by the parser.
pub(crate) const MAX_STATE_SLOT: u16 = 255;
/// Number of shared per-thread SimAntics temporaries (`temp[0..=7]`).
const TEMP_COUNT: i64 = 8;

/// Parses a `.antics` v2 source file into a [`V2File`] AST.
pub fn parse_v2_file(source: &str) -> Result<V2File> {
    let tokens = tokenize(source)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        depth: 0,
        bound_vars: std::collections::BTreeSet::new(),
    };
    parser.parse_file()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    /// Every variable name bound so far in the file (parameters, `let`s, loop and event
    /// bindings). A const binding may not yield one of these: a variable can change, or be
    /// written through a frame-word alias, between the test and a use in the region.
    bound_vars: std::collections::BTreeSet<String>,
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

    /// An identifier that introduces a binding. `stack_obj` is the running frame's stack object
    /// and cannot be rebound: lowering and its `stack_obj` analysis read the name as that cell.
    fn expect_binding_ident(&mut self, desc: &str) -> Result<(String, Span)> {
        let (name, span) = self.expect_ident(desc)?;
        if name == "stack_obj" {
            return Err(Error::at(
                span,
                format!("'stack_obj' is the frame's stack object and cannot be used as a {desc}"),
            ));
        }
        self.bound_vars.insert(name.clone());
        Ok((name, span))
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
            let (kw, kw_span) = self.expect_ident(
                "top-level item ('use', 'const', 'fn', 'template', 'object', 'enum', 'bitflags', 'extern')",
            )?;
            match kw.as_str() {
                "extern" => {
                    let (what, wspan) = self.expect_ident("'mod' or 'bcon' after 'extern'")?;
                    match what.as_str() {
                        "mod" => {
                            items.push(V2Item::ExternMod(self.parse_extern_mod(kw_span, is_pub)?))
                        }
                        "bcon" => {
                            items
                                .push(V2Item::ExternBcon(self.parse_extern_bcon(kw_span, is_pub)?));
                        }
                        other => {
                            return Err(Error::at(
                                wspan,
                                format!("expected 'mod' or 'bcon' after 'extern', found '{other}'"),
                            ));
                        }
                    }
                }
                "const" => items.push(V2Item::Const(self.parse_const(kw_span, is_pub)?)),
                "fn" => items.push(V2Item::Fn(self.parse_fn(kw_span, is_pub)?)),
                "template" => items.push(V2Item::Template(self.parse_template(kw_span, is_pub)?)),
                "enum" => items.push(V2Item::Enum(self.parse_enum(kw_span, is_pub)?)),
                "bitflags" => items.push(V2Item::Bitflags(self.parse_bitflags(kw_span, is_pub)?)),
                "object" => {
                    if is_pub {
                        return Err(Error::at(
                            span,
                            "'object' declarations cannot be marked 'pub' (objects are automatically importable by type name)",
                        ));
                    }
                    items.push(V2Item::Object(self.parse_object(kw_span)?));
                }
                other => {
                    return Err(Error::at(
                        kw_span,
                        format!(
                            "unexpected top-level keyword '{other}' in .antics v2 file \
                             (expected 'use', 'pub', 'const', 'fn', 'template', 'enum', 'bitflags', 'extern', or 'object')"
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

    fn parse_enum(&mut self, span: Span, is_pub: bool) -> Result<EnumDecl> {
        let (name, nspan) = self.expect_ident("enum name")?;
        if crate::v2::ast::is_reserved_type_name(&name) {
            return Err(Error::at(
                nspan,
                format!("'{name}' is a reserved built-in type name"),
            ));
        }
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut variants = Vec::new();
        let mut seen = BTreeSet::new();
        let mut seen_vals = BTreeSet::new();
        let mut next_val: i32 = 0;
        while !self.eat(&TokenKind::RBrace) {
            let (vname, vspan) = self.expect_ident("enum variant name")?;
            if !seen.insert(vname.clone()) {
                return Err(Error::at(
                    vspan,
                    format!("duplicate variant '{vname}' in enum '{name}'"),
                ));
            }
            let value = if self.eat(&TokenKind::Assign) {
                let v = self.expect_i16("enum discriminant")?;
                next_val = i32::from(v) + 1;
                v
            } else {
                let v = i16::try_from(next_val).map_err(|_| {
                    Error::at(
                        vspan,
                        format!("enum '{name}' variant '{vname}' discriminant overflows i16"),
                    )
                })?;
                next_val += 1;
                v
            };
            if !seen_vals.insert(value) {
                return Err(Error::at(
                    vspan,
                    format!("duplicate discriminant value {value} in enum '{name}'"),
                ));
            }
            variants.push(EnumVariant {
                span: vspan,
                name: vname,
                value,
            });
            if !self.eat(&TokenKind::Comma) {
                self.expect(&TokenKind::RBrace, "'}'")?;
                break;
            }
        }
        self.eat(&TokenKind::Semicolon);
        if variants.is_empty() {
            return Err(Error::at(
                span,
                format!("enum '{name}' must declare at least one variant"),
            ));
        }
        Ok(EnumDecl {
            span,
            is_pub,
            name,
            variants,
        })
    }

    fn parse_bitflags(&mut self, span: Span, is_pub: bool) -> Result<BitflagsDecl> {
        let (name, nspan) = self.expect_ident("bitflags name")?;
        if crate::v2::ast::is_reserved_type_name(&name) {
            return Err(Error::at(
                nspan,
                format!("'{name}' is a reserved built-in type name"),
            ));
        }
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut flags = Vec::new();
        let mut seen = BTreeSet::new();
        let mut seen_bits = BTreeSet::new();
        let mut next_bit: u16 = 1;
        while !self.eat(&TokenKind::RBrace) {
            let (fname, fspan) = self.expect_ident("bitflag name")?;
            if fname == "None" {
                return Err(Error::at(
                    fspan,
                    format!("'None' is reserved for the empty bitmask (0) on bitflags '{name}'"),
                ));
            }
            if !seen.insert(fname.clone()) {
                return Err(Error::at(
                    fspan,
                    format!("duplicate flag '{fname}' in bitflags '{name}'"),
                ));
            }
            let bit_u16 = if self.eat(&TokenKind::Assign) {
                let b = self.expect_u16("1-based bitflag index (1..=16)")?;
                next_bit = b.saturating_add(1);
                b
            } else {
                let b = next_bit;
                next_bit = next_bit.saturating_add(1);
                b
            };
            if !(1..=16).contains(&bit_u16) {
                return Err(Error::at(
                    fspan,
                    format!(
                        "bitflags '{name}::{fname}' bit index {bit_u16} must be in 1..=16 \
                         (SimAntics 1-based bit positions)"
                    ),
                ));
            }
            if !seen_bits.insert(bit_u16) {
                return Err(Error::at(
                    fspan,
                    format!("duplicate bit index {bit_u16} in bitflags '{name}'"),
                ));
            }
            flags.push(BitflagVariant {
                span: fspan,
                name: fname,
                bit: bit_u16 as u8,
            });
            if !self.eat(&TokenKind::Comma) {
                self.expect(&TokenKind::RBrace, "'}'")?;
                break;
            }
        }
        self.eat(&TokenKind::Semicolon);
        if flags.is_empty() {
            return Err(Error::at(
                span,
                format!("bitflags '{name}' must declare at least one flag"),
            ));
        }
        Ok(BitflagsDecl {
            span,
            is_pub,
            name,
            flags,
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
                V2Type::Bool | V2Type::I16 | V2Type::ObjectId | V2Type::Named(_) => {
                    return Ok(V2Type::MutRef(Box::new(inner)));
                }
                other => {
                    return Err(Error::at(
                        span,
                        format!(
                            "cannot take '&mut' of type '{}' (supported: bool, i16, ObjectId, enum, bitflags)",
                            other.display_name()
                        ),
                    ));
                }
            }
        }
        let (name, _nspan) = self.expect_ident("type name")?;
        match name.as_str() {
            "i16" => Ok(V2Type::I16),
            "bool" => Ok(V2Type::Bool),
            "Sim" => Ok(V2Type::Sim),
            "Object" => Ok(V2Type::Object),
            "ObjectId" => Ok(V2Type::ObjectId),
            "Motive" => Ok(V2Type::Motive),
            "Skill" => Ok(V2Type::Skill),
            "Sound" => Ok(V2Type::Sound),
            "Slot" => Ok(V2Type::Slot),
            "Guid" => Ok(V2Type::Guid),
            other => Ok(V2Type::Named(other.to_owned())),
        }
    }

    fn parse_fn_params(&mut self, allow_defaults: bool) -> Result<Vec<FnParam>> {
        self.expect(&TokenKind::LParen, "'('")?;
        let mut params = Vec::new();
        let mut seen_names = BTreeSet::new();
        let mut seen_default = false;
        while !self.eat(&TokenKind::RParen) {
            let (name, span) = self.expect_binding_ident("parameter name")?;
            if !seen_names.insert(name.clone()) {
                return Err(Error::at(
                    span,
                    format!("duplicate parameter name '{name}'"),
                ));
            }
            self.expect(&TokenKind::Colon, "':'")?;
            let ty = self.parse_type()?;
            let default = if self.eat(&TokenKind::Assign) {
                if !allow_defaults {
                    return Err(Error::at(
                        span,
                        format!("functional parameter '{name}' cannot have a default value"),
                    ));
                }
                if matches!(ty, V2Type::MutRef(_)) {
                    return Err(Error::at(
                        span,
                        format!("'&mut' parameter '{name}' cannot have a default value"),
                    ));
                }
                seen_default = true;
                Some(self.parse_expr()?)
            } else {
                if seen_default {
                    return Err(Error::at(
                        span,
                        format!(
                            "required parameter '{name}' cannot follow a parameter with a default value"
                        ),
                    ));
                }
                None
            };
            params.push(FnParam {
                span,
                name,
                ty,
                default,
            });
            if !self.eat(&TokenKind::Comma) {
                self.expect(&TokenKind::RParen, "')'")?;
                break;
            }
        }
        Ok(params)
    }

    fn parse_fn(&mut self, span: Span, is_pub: bool) -> Result<FnDecl> {
        let (name, _) = self.expect_ident("function name")?;
        let params = self.parse_fn_params(true)?;
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

    /// Parses `temp[N]` (after `@`) and validates `N` against the 8 SimAntics temporaries.
    fn parse_temp_slot(&mut self, what: &str) -> Result<u8> {
        let (kw, kspan) = self.expect_ident("'temp' ABI binding")?;
        if kw != "temp" {
            return Err(Error::at(
                kspan,
                format!("{what} ABI binding must be 'temp[N]', found '{kw}'"),
            ));
        }
        self.expect(&TokenKind::LBracket, "'['")?;
        let (v, vspan) = self.parse_signed_int("temp slot")?;
        self.expect(&TokenKind::RBracket, "']'")?;
        if !(0..TEMP_COUNT).contains(&v) {
            return Err(Error::at(
                vspan,
                format!(
                    "{what} temp slot {v} is out of range (SimAntics has temp[0]..=temp[{}])",
                    TEMP_COUNT - 1
                ),
            ));
        }
        Ok(v as u8)
    }

    /// Parses a BHAV-call ABI parameter list `(a: i16, b: i16 @ temp[1], ...)`.
    /// When `allow_sim` is set, a leading `name: Sim` binds the Sim-on-object context.
    fn parse_abi_params(
        &mut self,
        what: &str,
        allow_sim: bool,
    ) -> Result<(Option<String>, Vec<AbiParam>)> {
        self.expect(&TokenKind::LParen, "'('")?;
        let mut sim_binding = None;
        let mut params: Vec<AbiParam> = Vec::new();
        let mut seen_names = BTreeSet::new();
        let mut seen_temps: BTreeSet<u8> = BTreeSet::new();
        let mut first = true;
        while !self.eat(&TokenKind::RParen) {
            let (name, span) = self.expect_binding_ident("parameter name")?;
            if !seen_names.insert(name.clone()) {
                return Err(Error::at(
                    span,
                    format!("duplicate parameter name '{name}'"),
                ));
            }
            self.expect(&TokenKind::Colon, "':'")?;
            let ty = self.parse_type()?;
            if ty == V2Type::Sim {
                if !(allow_sim && first) {
                    return Err(Error::at(
                        span,
                        format!(
                            "'{name}: Sim' is only allowed as the first parameter of a 'tree' (it selects the Sim-on-object context)"
                        ),
                    ));
                }
                sim_binding = Some(name);
            } else {
                let slot = if self.eat(&TokenKind::At) {
                    let t = self.parse_temp_slot(&format!("{what} parameter '{name}'"))?;
                    if !seen_temps.insert(t) {
                        return Err(Error::at(
                            span,
                            format!(
                                "duplicate '@ temp[{t}]' binding for parameter '{name}' in {what}"
                            ),
                        ));
                    }
                    AbiParamSlot::Temp(t)
                } else {
                    AbiParamSlot::Positional
                };
                if self.peek().kind == TokenKind::Assign {
                    return Err(Error::at(
                        span,
                        format!("{what} parameter '{name}' cannot have a default value"),
                    ));
                }
                params.push(AbiParam {
                    span,
                    name,
                    ty,
                    slot,
                });
            }
            first = false;
            if !self.eat(&TokenKind::Comma) {
                self.expect(&TokenKind::RParen, "')'")?;
                break;
            }
        }
        Ok((sim_binding, params))
    }

    /// Parses an optional `-> bool | -> Result | -> T @ temp[N] | -> Result<T @ temp[N]>` ABI.
    fn parse_return_abi(&mut self, what: &str) -> Result<ReturnAbi> {
        if !self.eat(&TokenKind::Arrow) {
            return Ok(ReturnAbi::Void);
        }
        let rspan = self.span();
        if matches!(&self.peek().kind, TokenKind::Ident(s) if s == "Result") {
            self.bump();
            if !self.eat(&TokenKind::Lt) {
                return Ok(ReturnAbi::Result);
            }
            let ty = self.parse_type()?;
            if !self.eat(&TokenKind::At) {
                return Err(Error::at(
                    rspan,
                    format!(
                        "{what} 'Result<{}>' return requires an explicit '@ temp[N]' binding (e.g. 'Result<i16 @ temp[0]>')",
                        ty.display_name()
                    ),
                ));
            }
            let temp = self.parse_temp_slot(&format!("{what} return"))?;
            self.expect(&TokenKind::Gt, "'>'")?;
            return Ok(ReturnAbi::ResultValue { ty, temp });
        }
        let ty = self.parse_type()?;
        if self.eat(&TokenKind::At) {
            let temp = self.parse_temp_slot(&format!("{what} return"))?;
            return Ok(ReturnAbi::Value { ty, temp });
        }
        if ty == V2Type::Bool {
            return Ok(ReturnAbi::Bool);
        }
        Err(Error::at(
            rspan,
            format!(
                "{what} '-> {}' return requires an explicit '@ temp[N]' binding (e.g. '-> {} @ temp[0]')",
                ty.display_name(),
                ty.display_name()
            ),
        ))
    }

    /// Parses `extern mod name { @id(N) fn f(...) [-> Abi]; ... }` (after `extern mod`).
    fn parse_extern_mod(&mut self, span: Span, is_pub: bool) -> Result<ExternModDecl> {
        let (name, _) = self.expect_ident("extern mod name")?;
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut fns: Vec<ExternFnDecl> = Vec::new();
        while !self.eat(&TokenKind::RBrace) {
            let fspan = self.span();
            let mut id: Option<u16> = None;
            while self.eat(&TokenKind::At) {
                let (attr, aspan) = self.expect_ident("attribute name ('id')")?;
                if attr != "id" {
                    return Err(Error::at(
                        aspan,
                        format!("unknown annotation '@{attr}' on extern fn (expected '@id(N)')"),
                    ));
                }
                self.expect(&TokenKind::LParen, "'('")?;
                let ispan = self.span();
                let v = self.expect_u16("BHAV tree id")?;
                self.expect(&TokenKind::RParen, "')'")?;
                if !(256..=4095).contains(&v) && v < 8192 {
                    return Err(Error::at(
                        ispan,
                        format!(
                            "extern BHAV @id({v}) must be a global (256..=4095) or semi-global (8192..) tree"
                        ),
                    ));
                }
                if id.replace(v).is_some() {
                    return Err(Error::at(aspan, "duplicate @id annotation"));
                }
            }
            let (kw, kspan) = self.expect_ident("'fn' inside extern mod")?;
            if kw != "fn" {
                return Err(Error::at(
                    kspan,
                    format!("expected 'fn' inside 'extern mod {name}', found '{kw}'"),
                ));
            }
            let (fname, nspan) = self.expect_ident("extern fn name")?;
            let Some(id) = id else {
                return Err(Error::at(
                    nspan,
                    format!("extern fn '{name}::{fname}' requires an '@id(N)' BHAV annotation"),
                ));
            };
            if fns.iter().any(|f| f.name == fname) {
                return Err(Error::at(
                    nspan,
                    format!("duplicate extern fn '{fname}' in 'extern mod {name}'"),
                ));
            }
            let what = format!("extern fn '{name}::{fname}'");
            let (_, params) = self.parse_abi_params(&what, false)?;
            let ret = self.parse_return_abi(&what)?;
            self.expect(&TokenKind::Semicolon, "';' after extern fn signature")?;
            fns.push(ExternFnDecl {
                span: fspan,
                id,
                name: fname,
                params,
                ret,
            });
        }
        Ok(ExternModDecl {
            span,
            is_pub,
            name,
            fns,
        })
    }

    /// Parses `extern bcon name { entry @ <address>; entry @ <bcon_id>[<index>]; ... }`.
    fn parse_extern_bcon(&mut self, span: Span, is_pub: bool) -> Result<ExternBconDecl> {
        let (name, _) = self.expect_ident("extern bcon name")?;
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut entries: Vec<ExternBconEntry> = Vec::new();
        while !self.eat(&TokenKind::RBrace) {
            let (ename, espan) = self.expect_ident("extern bcon entry name")?;
            if entries.iter().any(|e| e.name == ename) {
                return Err(Error::at(
                    espan,
                    format!("duplicate entry '{ename}' in 'extern bcon {name}'"),
                ));
            }
            self.expect(&TokenKind::At, "'@' (constant address)")?;
            let aspan = self.span();
            let (first, _) = self.parse_signed_int("BCON address")?;
            let address = if self.eat(&TokenKind::LBracket) {
                let (entry, _) = self.parse_signed_int("BCON entry index")?;
                self.expect(&TokenKind::RBracket, "']'")?;
                let table = u16::try_from(first)
                    .ok()
                    .and_then(bcon_chunk_table)
                    .ok_or_else(|| {
                        Error::at(
                            aspan,
                            format!(
                                "BCON chunk id {first} is not addressable (expected 4096..=4159 private, 8192..=8255 semi-global, or 256..=319 global)"
                            ),
                        )
                    })?;
                if !(0..=127).contains(&entry) {
                    return Err(Error::at(
                        aspan,
                        format!("BCON entry index {entry} is out of range 0..=127"),
                    ));
                }
                (table << 7) | entry as u16
            } else {
                u16::try_from(first)
                    .ok()
                    .filter(|&a| (a >> 7) <= 191)
                    .ok_or_else(|| {
                        Error::at(
                            aspan,
                            format!(
                                "BCON address {first} is out of range 0..=24575 (table = address >> 7 must be <= 191)"
                            ),
                        )
                    })?
            };
            entries.push(ExternBconEntry {
                span: espan,
                name: ename,
                address,
            });
            let _ = self.eat(&TokenKind::Semicolon) || self.eat(&TokenKind::Comma);
        }
        Ok(ExternBconDecl {
            span,
            is_pub,
            name,
            entries,
        })
    }

    /// Parses a private `bcon <id> "Label" { name = value, ... }` table member.
    fn parse_bcon_table(&mut self, span: Span) -> Result<BconTableDecl> {
        let id = self.expect_u16("BCON chunk id")?;
        let label = match &self.peek().kind {
            TokenKind::Str(s) | TokenKind::Ident(s) => {
                let s = s.clone();
                self.bump();
                s
            }
            other => {
                return Err(Error::at(
                    self.span(),
                    format!("expected BCON table name (e.g. \"LampTuning\"), found {other:?}"),
                ));
            }
        };
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut entries: Vec<(Span, String, i16)> = Vec::new();
        while !self.eat(&TokenKind::RBrace) {
            let (ename, espan) = self.expect_ident("BCON entry name")?;
            self.expect(&TokenKind::Assign, "'='")?;
            let val = self.expect_i16("BCON constant value")?;
            entries.push((espan, ename, val));
            if !self.eat(&TokenKind::Comma) && !self.eat(&TokenKind::Semicolon) {
                self.expect(&TokenKind::RBrace, "'}'")?;
                break;
            }
        }
        Ok(BconTableDecl {
            span,
            id,
            label,
            entries,
        })
    }

    /// Parses `tree [ident] ["Label"] [(params)] [-> Abi] { ... }` (after `tree`).
    fn parse_tree_member(&mut self, span: Span, pinned_id: Option<u16>) -> Result<TreeMemberDecl> {
        let ident = if let TokenKind::Ident(s) = &self.peek().kind {
            let s = s.clone();
            self.bump();
            Some(s)
        } else {
            None
        };
        let label = if let TokenKind::Str(s) = &self.peek().kind {
            let s = s.clone();
            self.bump();
            Some(s)
        } else {
            None
        };
        let (ident, label) = match (ident, label) {
            (Some(i), Some(l)) => (i, l),
            (Some(i), None) => (i.clone(), i),
            (None, Some(l)) => (snake_case_ident(&l), l),
            (None, None) => {
                return Err(Error::at(
                    span,
                    "'tree' requires a name or a \"Label\" (e.g. 'tree \"CT - Is Fridge\" { ... }')",
                ));
            }
        };
        let what = format!("tree '{ident}'");
        let (sim_binding, params) = if self.peek().kind == TokenKind::LParen {
            self.parse_abi_params(&what, true)?
        } else {
            (None, Vec::new())
        };
        let ret = self.parse_return_abi(&what)?;
        let body = self.parse_block()?;
        Ok(TreeMemberDecl {
            span,
            pinned_id,
            ident,
            label,
            sim_binding,
            inherit_context: false,
            params,
            ret,
            body,
            frame: None,
        })
    }

    /// Parses the `key: value, ...)` tail of `@frame(` (the `(` is consumed).
    fn parse_frame_fields(&mut self, span: Span) -> Result<TreeFrame> {
        let mut frame = TreeFrame {
            span,
            ..TreeFrame::default()
        };
        while !self.eat(&TokenKind::RParen) {
            let (key, kspan) =
                self.expect_ident("'args', 'locals', 'format', 'revision', 'kind', or 'empty'")?;
            if key == "empty" && self.peek().kind != TokenKind::Colon {
                if std::mem::replace(&mut frame.empty, true) {
                    return Err(Error::at(kspan, "duplicate @frame field 'empty'"));
                }
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RParen, "')'")?;
                    break;
                }
                continue;
            }
            self.expect(&TokenKind::Colon, "':'")?;
            let dup = match key.as_str() {
                "args" => frame
                    .args
                    .replace(u8::try_from(self.expect_u16("BHAV args")?).map_err(|_| {
                        Error::at(kspan, "@frame args must fit in a byte (0..=255)")
                    })?)
                    .is_some(),
                "locals" => frame
                    .locals
                    .replace(self.expect_u16("BHAV locals")?)
                    .is_some(),
                "format" => frame
                    .format
                    .replace(self.expect_u16("BHAV format")?)
                    .is_some(),
                "revision" => frame
                    .revision
                    .replace(self.expect_u32("BHAV revision")?)
                    .is_some(),
                "kind" => frame
                    .kind
                    .replace(u8::try_from(self.expect_u16("BHAV kind")?).map_err(|_| {
                        Error::at(kspan, "@frame kind must fit in a byte (0..=255)")
                    })?)
                    .is_some(),
                other => {
                    return Err(Error::at(
                        kspan,
                        format!(
                            "unknown @frame field '{other}' (expected args, locals, format, revision, kind, or empty)"
                        ),
                    ));
                }
            };
            if dup {
                return Err(Error::at(kspan, format!("duplicate @frame field '{key}'")));
            }
            if !self.eat(&TokenKind::Comma) {
                self.expect(&TokenKind::RParen, "')'")?;
                break;
            }
        }
        Ok(frame)
    }

    /// Parses `NAME = <integer>;` after `const`.
    fn parse_const(&mut self, span: Span, is_pub: bool) -> Result<ConstDecl> {
        let (name, _) = self.expect_ident("constant name")?;
        self.expect(&TokenKind::Assign, "'='")?;
        let (value, vspan) = self.parse_signed_int("constant value")?;
        if !(i64::from(i16::MIN)..=i64::from(u32::MAX)).contains(&value) {
            return Err(Error::at(
                vspan,
                format!("constant {value} must be an i16 or a 32-bit GUID"),
            ));
        }
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(ConstDecl {
            span,
            is_pub,
            name,
            value,
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
            // Parse optional member annotations.
            let mut pinned_id: Option<u16> = None;
            let mut pinned_guard_id: Option<u16> = None;
            let mut frame: Option<TreeFrame> = None;
            let mut inherit_context = None;
            while self.eat(&TokenKind::At) {
                let (attr, aspan) =
                    self.expect_ident("attribute name ('id', 'guard_id', 'frame', or 'context')")?;
                self.expect(&TokenKind::LParen, "'('")?;
                if attr == "frame" {
                    if frame.is_some() {
                        return Err(Error::at(aspan, "duplicate @frame annotation"));
                    }
                    frame = Some(self.parse_frame_fields(aspan)?);
                    continue;
                }
                if attr == "context" {
                    if inherit_context.replace(aspan).is_some() {
                        return Err(Error::at(aspan, "duplicate @context annotation"));
                    }
                    let (mode, span) = self.expect_ident("'inherit'")?;
                    if mode != "inherit" {
                        return Err(Error::at(span, "expected '@context(inherit)'"));
                    }
                    self.expect(&TokenKind::RParen, "')'")?;
                    continue;
                }
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
                && !matches!(
                    kw.as_str(),
                    "init"
                        | "main"
                        | "load"
                        | "reset"
                        | "cleanup"
                        | "functional"
                        | "interaction"
                        | "tree"
                )
            {
                return Err(Error::at(
                    kw_span,
                    "@id/@guard_id annotations are only allowed on init, main, load, reset, cleanup, functional, interaction, or tree",
                ));
            }
            if let Some(f) = &frame
                && !matches!(
                    kw.as_str(),
                    "tree" | "init" | "main" | "load" | "reset" | "cleanup"
                )
            {
                return Err(Error::at(
                    f.span,
                    "@frame(...) is only allowed on a standalone 'tree' or on init, main, load, reset, or cleanup",
                ));
            }
            if let Some(span) = inherit_context
                && kw != "tree"
            {
                return Err(Error::at(
                    span,
                    "@context(inherit) is only allowed on a standalone 'tree'",
                ));
            }
            if pinned_guard_id.is_some() && !matches!(kw.as_str(), "functional" | "interaction") {
                return Err(Error::at(
                    kw_span,
                    "@guard_id is only allowed on functional or interaction declarations",
                ));
            }

            match kw.as_str() {
                "const" => {
                    members.push(ObjectMember::Const(self.parse_const(kw_span, false)?));
                }
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
                "enum" => {
                    members.push(ObjectMember::Enum(self.parse_enum(kw_span, false)?));
                }
                "bitflags" => {
                    members.push(ObjectMember::Bitflags(self.parse_bitflags(kw_span, false)?));
                }
                "init" => {
                    let body = self.parse_block()?;
                    members.push(ObjectMember::Init(LifecycleBlock {
                        span: kw_span,
                        pinned_id,
                        frame,
                        body,
                    }));
                }
                "main" => {
                    let body = self.parse_block()?;
                    members.push(ObjectMember::Main(LifecycleBlock {
                        span: kw_span,
                        pinned_id,
                        frame,
                        body,
                    }));
                }
                "load" => {
                    let body = self.parse_block()?;
                    members.push(ObjectMember::Load(LifecycleBlock {
                        span: kw_span,
                        pinned_id,
                        frame,
                        body,
                    }));
                }
                "reset" => {
                    let body = self.parse_block()?;
                    members.push(ObjectMember::Reset(LifecycleBlock {
                        span: kw_span,
                        pinned_id,
                        frame,
                        body,
                    }));
                }
                "cleanup" => {
                    let body = self.parse_block()?;
                    members.push(ObjectMember::Cleanup(LifecycleBlock {
                        span: kw_span,
                        pinned_id,
                        frame,
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
                "tree" => {
                    let mut tree = self.parse_tree_member(kw_span, pinned_id)?;
                    if let Some(span) = inherit_context {
                        if tree.sim_binding.is_some() {
                            return Err(Error::at(
                                span,
                                "@context(inherit) cannot declare a Sim parameter",
                            ));
                        }
                        tree.inherit_context = true;
                    }
                    tree.frame = frame;
                    members.push(ObjectMember::Tree(tree));
                }
                "bcon" => {
                    members.push(ObjectMember::Bcon(self.parse_bcon_table(kw_span)?));
                }
                "extern" => {
                    let (what, wspan) = self.expect_ident("'mod' or 'bcon' after 'extern'")?;
                    match what.as_str() {
                        "mod" => members.push(ObjectMember::ExternMod(
                            self.parse_extern_mod(kw_span, false)?,
                        )),
                        "bcon" => members.push(ObjectMember::ExternBcon(
                            self.parse_extern_bcon(kw_span, false)?,
                        )),
                        other => {
                            return Err(Error::at(
                                wspan,
                                format!("expected 'mod' or 'bcon' after 'extern', found '{other}'"),
                            ));
                        }
                    }
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
                "variant" if matches!(self.peek().kind, TokenKind::Str(_)) => {
                    members.push(ObjectMember::Variant(self.parse_variant_decl(kw_span)?));
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

    /// `variant "Name" { guid: ..., price: ..., init: tree, main: tree, description: "..." }`
    fn parse_variant_decl(&mut self, span: Span) -> Result<VariantDecl> {
        let (name, _) = self.expect_string("variant catalog name")?;
        self.expect(&TokenKind::LBrace, "'{'")?;
        let mut decl = VariantDecl {
            span,
            name,
            guid: None,
            price: None,
            description: None,
            init: None,
            main: None,
        };
        while !self.eat(&TokenKind::RBrace) {
            let (key, kspan) = self.expect_ident("variant field")?;
            self.expect(&TokenKind::Colon, "':'")?;
            let dup = match key.as_str() {
                "guid" => decl
                    .guid
                    .replace((kspan, self.expect_u32("variant guid")?))
                    .is_some(),
                "price" => {
                    let (v, vspan) = self.parse_signed_int("variant price")?;
                    decl.price.replace((vspan, v)).is_some()
                }
                "description" => decl
                    .description
                    .replace(self.expect_string("variant description")?.0)
                    .is_some(),
                "init" | "main" => {
                    let (ident, ispan) = self.expect_ident("standalone tree name")?;
                    let slot = if key == "init" {
                        &mut decl.init
                    } else {
                        &mut decl.main
                    };
                    slot.replace((ispan, ident)).is_some()
                }
                other => {
                    return Err(Error::at(
                        kspan,
                        format!(
                            "unknown variant field '{other}' (expected guid, price, description, init, or main)"
                        ),
                    ));
                }
            };
            if dup {
                return Err(Error::at(kspan, format!("duplicate variant field '{key}'")));
            }
            if !self.eat(&TokenKind::Comma) && !self.eat(&TokenKind::Semicolon) {
                self.expect(&TokenKind::RBrace, "'}'")?;
                break;
            }
        }
        self.eat_member_terminator();
        Ok(decl)
    }

    fn eat_member_terminator(&mut self) {
        let _ = self.eat(&TokenKind::Semicolon) || self.eat(&TokenKind::Comma);
    }

    fn parse_param_member(&mut self, span: Span) -> Result<ObjectMember> {
        // A tuning parameter is a constant, so a const binding may yield it.
        let (name, name_span) = self.expect_ident("parameter name")?;
        if name == "stack_obj" {
            return Err(Error::at(
                name_span,
                "'stack_obj' is the frame's stack object and cannot be used as a parameter name",
            ));
        }
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
        let slot = if self.eat(&TokenKind::At) {
            let sspan = self.span();
            let (v, _) = self.parse_signed_int("state attribute slot")?;
            let slot = u16::try_from(v)
                .ok()
                .filter(|&s| s <= MAX_STATE_SLOT)
                .ok_or_else(|| {
                    Error::at(
                        sspan,
                        format!(
                            "state '{name}' attribute slot {v} is out of range 0..={MAX_STATE_SLOT}"
                        ),
                    )
                })?;
            Some((sspan, slot))
        } else {
            None
        };
        self.expect(&TokenKind::Colon, "':'")?;
        let (ty_name, _ty_span) =
            self.expect_ident("state type ('bool', 'i16', 'ObjectId', enum, or bitflags)")?;
        let ty = match ty_name.as_str() {
            "bool" => StateType::Bool,
            "i16" => StateType::I16,
            "ObjectId" => StateType::ObjectId,
            other => StateType::Named(other.to_owned()),
        };
        let init = if self.eat(&TokenKind::Assign) {
            let ispan = self.span();
            let val = self.parse_state_init_value()?;
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
            slot,
            ty,
            init,
        })
    }

    fn parse_state_init_value(&mut self) -> Result<StateInitValue> {
        let mut lhs = self.parse_state_init_primary()?;
        while self.peek().kind == TokenKind::Pipe {
            let pspan = self.bump().span;
            let rhs = self.parse_state_init_primary()?;
            lhs = StateInitValue::BitOr(pspan, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_state_init_primary(&mut self) -> Result<StateInitValue> {
        let ispan = self.span();
        match &self.peek().kind {
            TokenKind::Ident(s) if s == "true" => {
                self.bump();
                Ok(StateInitValue::Bool(true))
            }
            TokenKind::Ident(s) if s == "false" => {
                self.bump();
                Ok(StateInitValue::Bool(false))
            }
            TokenKind::Ident(s) => {
                let first = s.clone();
                self.bump();
                if self.peek().kind == TokenKind::ColonColon {
                    let mut segs = vec![first];
                    while self.eat(&TokenKind::ColonColon) {
                        let (seg, _) = self.expect_ident("enum/bitflags variant name")?;
                        segs.push(seg);
                    }
                    Ok(StateInitValue::Path(ispan, segs))
                } else {
                    Ok(StateInitValue::Param(first))
                }
            }
            TokenKind::Int(_) | TokenKind::Minus | TokenKind::Plus => {
                Ok(StateInitValue::Int(self.expect_i16("state initial value")?))
            }
            other => Err(Error::at(
                ispan,
                format!("expected state initial value, found {other:?}"),
            )),
        }
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
        if self.eat(&TokenKind::Assign) {
            let (ctor, cspan) = self.expect_ident("slot constructor ('routing' or 'container')")?;
            let mut slot = match ctor.as_str() {
                "routing" => default_routing_slot(span, name),
                "container" => default_container_slot(span, name),
                other => {
                    return Err(Error::at(
                        cspan,
                        format!(
                            "unknown slot constructor '{other}' (expected 'routing' or 'container')"
                        ),
                    ));
                }
            };
            self.expect(&TokenKind::LParen, "'('")?;
            slot.offset = self.parse_f32_3()?;
            while self.eat(&TokenKind::Comma) {
                if self.peek().kind == TokenKind::RParen {
                    break;
                }
                let (key, kspan) = self.expect_ident("slot option name")?;
                self.expect(&TokenKind::Colon, "':'")?;
                self.parse_slot_field(&mut slot, &key, kspan, false)?;
            }
            self.expect(&TokenKind::RParen, "')'")?;
            self.eat_member_terminator();
            return Ok(slot);
        }
        self.expect(&TokenKind::LBrace, "'{' or '='")?;
        let mut slot = default_routing_slot(span, name);
        while !self.eat(&TokenKind::RBrace) {
            let (key, kspan) = self.expect_ident("slot field name")?;
            self.expect(&TokenKind::Colon, "':'")?;
            self.parse_slot_field(&mut slot, &key, kspan, true)?;
            if !self.eat(&TokenKind::Comma) && !self.eat(&TokenKind::Semicolon) {
                self.expect(&TokenKind::RBrace, "'}'")?;
                break;
            }
        }
        self.eat(&TokenKind::Semicolon);
        Ok(slot)
    }

    fn parse_slot_field(
        &mut self,
        slot: &mut SlotDecl,
        key: &str,
        kspan: Span,
        allow_kind_offset: bool,
    ) -> Result<()> {
        match key {
            "kind" if allow_kind_offset => {
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
            "offset" if allow_kind_offset => {
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
            "min_dist" => {
                slot.proximity[0] = self.expect_i32("slot min_dist")?;
            }
            "max_dist" => {
                slot.proximity[1] = self.expect_i32("slot max_dist")?;
            }
            "optimal_dist" => {
                slot.proximity[2] = self.expect_i32("slot optimal_dist")?;
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
        Ok(())
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

    fn parse_guard(&mut self, span: Span, allow_autonomous: bool) -> Result<GuardDecl> {
        let mut sim_binding = "sim".to_owned();
        let mut autonomous_binding: Option<String> = None;
        if self.eat(&TokenKind::LParen) && !self.eat(&TokenKind::RParen) {
            sim_binding = self.expect_binding_ident("guard sim parameter")?.0;
            if self.eat(&TokenKind::Comma) && self.peek().kind != TokenKind::RParen {
                let (auto_name, aspan) =
                    self.expect_binding_ident("guard is_autonomous parameter")?;
                if !allow_autonomous {
                    return Err(Error::at(
                        aspan,
                        "functional guards do not support a second 'is_autonomous' parameter (only interaction guards do)",
                    ));
                }
                if auto_name == sim_binding {
                    return Err(Error::at(
                        aspan,
                        format!("duplicate guard parameter name '{auto_name}'"),
                    ));
                }
                autonomous_binding = Some(auto_name);
                let _ = self.eat(&TokenKind::Comma);
            }
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
            autonomous_binding,
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
            self.parse_fn_params(false)?
        } else {
            Vec::new()
        };
        let return_type = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
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
                return_type,
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
                    guard = Some(self.parse_guard(fspan, false)?);
                    self.eat_member_terminator();
                }
                "run" => {
                    if body.is_some() {
                        return Err(Error::at(fspan, "duplicate 'run' in functional block"));
                    }
                    if self.eat(&TokenKind::LParen) && !self.eat(&TokenKind::RParen) {
                        sim_binding = self.expect_binding_ident("sim parameter name")?.0;
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
            return_type,
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
        let mut lock: Option<(Span, String)> = None;
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
                "lock" => {
                    if lock.is_some() {
                        return Err(Error::at(
                            fspan,
                            format!("duplicate 'lock' in interaction '{label}'"),
                        ));
                    }
                    self.expect(&TokenKind::Colon, "':'")?;
                    let lspan = self.span();
                    let (base, bspan) = self.expect_ident("'self'")?;
                    if base != "self" {
                        return Err(Error::at(
                            bspan,
                            format!(
                                "interaction 'lock:' must target a bool state on 'self' (e.g. 'lock: self.occupied'), found '{base}'"
                            ),
                        ));
                    }
                    self.expect(&TokenKind::Dot, "'.'")?;
                    let (field_name, _) = self.expect_ident("state field name")?;
                    lock = Some((lspan, field_name));
                    self.eat_member_terminator();
                }
                "guard" => {
                    if guard.is_some() {
                        return Err(Error::at(
                            fspan,
                            format!("duplicate 'guard' in interaction '{label}'"),
                        ));
                    }
                    guard = Some(self.parse_guard(fspan, true)?);
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
                        sim_binding = self.expect_binding_ident("sim parameter name")?.0;
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
            lock,
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
        let mut items = Vec::new();
        let mut binds = false;
        let res = loop {
            if self.eat(&TokenKind::RBrace) {
                break Ok(());
            }
            if self.peek().kind == TokenKind::Eof {
                break Err(Error::at(span, "unterminated '{ ... }' block"));
            }
            if self.at_const_binding() {
                binds = true;
                match self.parse_const_binding() {
                    Ok(b) => items.push(BlockItem::Bind(b)),
                    Err(e) => break Err(e),
                }
                continue;
            }
            match self.parse_stmt() {
                Ok(s) => items.push(BlockItem::Stmt(s)),
                Err(e) => break Err(e),
            }
        };
        self.leave_nesting();
        res?;
        if !binds {
            return Ok(items
                .into_iter()
                .map(|i| match i {
                    BlockItem::Stmt(s) => s,
                    BlockItem::Bind(_) => unreachable!("no bindings"),
                })
                .collect());
        }
        specialize::expand_block(items)
    }

    /// `const NAME = ..` or `const (A, B) = ..` inside a block.
    fn at_const_binding(&self) -> bool {
        self.is_ident("const")
            && matches!(
                (&self.peek_at(1).kind, &self.peek_at(2).kind),
                (TokenKind::Ident(_), TokenKind::Assign) | (TokenKind::LParen, _)
            )
    }

    /// `const NAME = <selector>;` / `const (A, B) = <selector>;`, where the selector is a
    /// `match` or an `if` chain whose arms yield compile-time operands or leave the block (see
    /// [`specialize`]).
    fn parse_const_binding(&mut self) -> Result<ConstBinding> {
        let span = self.bump().span; // 'const'
        let (names, tuple) = if self.eat(&TokenKind::LParen) {
            let mut names = Vec::new();
            loop {
                let (n, s) = self.expect_ident("bound name")?;
                names.push((s, n));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if self.peek().kind == TokenKind::RParen {
                    break;
                }
            }
            self.expect(&TokenKind::RParen, "')'")?;
            if names.len() < 2 {
                return Err(Error::at(
                    span,
                    "a tuple const binding names at least two values",
                ));
            }
            (names, true)
        } else {
            let (n, s) = self.expect_ident("bound name")?;
            (vec![(s, n)], false)
        };
        self.expect(&TokenKind::Assign, "'='")?;
        let sel_span = self.span();
        let selector = if self.eat_ident("match") {
            self.parse_binding_match(sel_span)?
        } else if self.eat_ident("if") {
            self.parse_binding_if(sel_span)?
        } else {
            return Err(Error::at(
                sel_span,
                "a const binding inside a block chooses its value with 'match' or 'if' (a plain constant belongs at object or file scope)",
            ));
        };
        self.expect(&TokenKind::Semicolon, "';'")?;
        let binding = ConstBinding {
            span,
            names,
            tuple,
            selector,
        };
        // Names are substituted where they are used, so a yielded name must mean the same
        // value there as at the test: a variable could have changed in between.
        if let Some((yspan, n)) = specialize::yielded_idents_at(&binding)
            .into_iter()
            .find(|(_, n)| self.bound_vars.contains(n))
        {
            return Err(Error::at(
                yspan,
                format!(
                    "a const binding yields compile-time operands, and '{n}' is a variable; \
                     use a 'let' with 'match' or 'if' for a run-time value"
                ),
            ));
        }
        Ok(binding)
    }

    fn parse_binding_match(&mut self, span: Span) -> Result<Selector> {
        self.enter_nesting(span)?;
        let res = (|| {
            let scrutinee = self.parse_expr()?;
            let open_span = self.expect(&TokenKind::LBrace, "'{'")?;
            let mut arms = Vec::new();
            loop {
                if self.eat(&TokenKind::RBrace) {
                    break;
                }
                if self.peek().kind == TokenKind::Eof {
                    return Err(Error::at(open_span, "unterminated 'match { ... }' block"));
                }
                let arm_span = self.span();
                let pattern = self.parse_match_pattern()?;
                self.expect(&TokenKind::FatArrow, "'=>'")?;
                let arm = self.parse_binding_arm(true)?;
                arms.push((arm_span, pattern, arm));
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RBrace, "',' or '}'")?;
                    break;
                }
            }
            if arms.is_empty() {
                return Err(Error::at(span, "'match' must have at least one arm"));
            }
            Ok(Selector::Match {
                span,
                scrutinee,
                arms,
            })
        })();
        self.leave_nesting();
        res
    }

    fn parse_binding_if(&mut self, span: Span) -> Result<Selector> {
        self.enter_nesting(span)?;
        let res = (|| {
            let mut arms = Vec::new();
            let mut arm_span = span;
            loop {
                let cond = self.parse_expr()?;
                let arm = self.parse_binding_arm(false)?;
                arms.push((arm_span, cond, arm));
                if !self.eat_ident("else") {
                    return Err(Error::at(
                        span,
                        "a const binding 'if' needs an 'else' (every path binds the names or leaves the block)",
                    ));
                }
                if self.is_ident("if") {
                    arm_span = self.bump().span;
                    continue;
                }
                let else_arm = self.parse_binding_arm(false)?;
                return Ok(Selector::If {
                    arms,
                    else_arm: Box::new(else_arm),
                });
            }
        })();
        self.leave_nesting();
        res
    }

    /// One arm of a binding: a value or tuple (`v`, `(a, b)`; braced as `{ v }` in an `if`),
    /// `break` / `continue` / `return ..` (in a `match`), or a block that leaves (`{ ..; return
    /// false; }`).
    fn parse_binding_arm(&mut self, in_match: bool) -> Result<Arm> {
        let span = self.span();
        if self.peek().kind == TokenKind::LBrace {
            // `{ value }` or a block of statements.
            let (saved_pos, saved_depth) = (self.pos, self.depth);
            self.bump();
            let starts_stmt = matches!(
                &self.peek().kind,
                TokenKind::Ident(k) if matches!(k.as_str(), "break" | "continue" | "return" | "let" | "if" | "match" | "while" | "loop" | "for" | "repeat" | "with" | "defer" | "animate")
            ) || matches!(self.peek().kind, TokenKind::RawBlock(_));
            if !starts_stmt
                && let Ok(value) = self.parse_expr()
                && self.eat(&TokenKind::RBrace)
            {
                return Ok(values_arm(span, value));
            }
            self.pos = saved_pos;
            self.depth = saved_depth;
            return Ok(Arm::Diverge(self.parse_block()?));
        }
        if !in_match {
            return Err(Error::at(span, "expected '{' after the condition"));
        }
        if self.is_ident("break") || self.is_ident("continue") || self.is_ident("return") {
            let kw_span = self.span();
            let stmt = if self.eat_ident("break") {
                V2Stmt::Break { span: kw_span }
            } else if self.eat_ident("continue") {
                V2Stmt::Continue { span: kw_span }
            } else {
                self.bump(); // 'return'
                if matches!(self.peek().kind, TokenKind::Comma | TokenKind::RBrace) {
                    V2Stmt::Return {
                        span: kw_span,
                        value: None,
                    }
                } else if self.is_ident("error")
                    && matches!(self.peek_at(1).kind, TokenKind::Comma | TokenKind::RBrace)
                {
                    self.bump();
                    V2Stmt::ReturnError {
                        span: kw_span,
                        from_try: false,
                    }
                } else {
                    V2Stmt::Return {
                        span: kw_span,
                        value: Some(self.parse_expr()?),
                    }
                }
            };
            return Ok(Arm::Diverge(vec![stmt]));
        }
        Ok(values_arm(span, self.parse_expr()?))
    }

    fn parse_stmt(&mut self) -> Result<V2Stmt> {
        let span = self.span();
        if let TokenKind::RawBlock(code) = &self.peek().kind {
            let code = code.clone();
            self.bump();
            return Ok(V2Stmt::RawAir { span, code });
        }
        if self.at_const_binding() {
            return Err(Error::at(
                span,
                "a const binding must be a statement of a '{ ... }' block (its region is the statements after it)",
            ));
        }
        if self.peek().kind == TokenKind::At
            && matches!(&self.peek_at(1).kind, TokenKind::Ident(a) if a == "slot")
        {
            return self.parse_slot_let_stmt(span);
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
                "loop" if self.peek_at(1).kind == TokenKind::LBrace => {
                    self.bump();
                    let body = self.parse_block()?;
                    return Ok(V2Stmt::While {
                        span,
                        cond: V2Expr::Bool(span, true),
                        cancel_on_abort: false,
                        body,
                    });
                }
                "animate" if self.peek_at(1).kind == TokenKind::LParen => {
                    return self.parse_animate_stmt(span);
                }
                "for" => return self.parse_for_stmt(span),
                "if" => return self.parse_if_stmt(span),
                "match" => return self.parse_match_stmt(span),
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
                    if matches!(&self.peek().kind, TokenKind::Ident(s) if s == "error")
                        && self.peek_at(1).kind == TokenKind::Semicolon
                    {
                        self.bump();
                        self.bump();
                        return Ok(V2Stmt::ReturnError {
                            span,
                            from_try: false,
                        });
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

    /// `animate(...);` or `animate(...) |event| { ... }`.
    fn parse_animate_stmt(&mut self, span: Span) -> Result<V2Stmt> {
        let call = self.parse_postfix_expr()?;
        let V2Expr::Call {
            args, type_args, ..
        } = call
        else {
            // `animate(...)?;` or another postfix form: an ordinary expression statement (only
            // meaningful when a user function named `animate` exists).
            self.expect(&TokenKind::Semicolon, "';'")?;
            return Ok(V2Stmt::Expr { span, expr: call });
        };
        if !type_args.is_empty() {
            return Err(Error::at(span, "'animate' does not take type arguments"));
        }
        if self.eat(&TokenKind::Pipe) {
            let (event, _) = self.expect_binding_ident("animation event binding name")?;
            self.expect(&TokenKind::Pipe, "'|'")?;
            let body = self.parse_block()?;
            return Ok(V2Stmt::Animate {
                span,
                args,
                event: Some(event),
                body,
            });
        }
        self.expect(
            &TokenKind::Semicolon,
            "';' or an event handler '|event| { ... }' after 'animate(...)'",
        )?;
        Ok(V2Stmt::Animate {
            span,
            args,
            event: None,
            body: Vec::new(),
        })
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
        // `let Some(name [: Type]) = expr else { ... };`
        if self.eat_ident("Some") {
            self.expect(&TokenKind::LParen, "'('")?;
            let (name, _) = self.expect_binding_ident("binding name")?;
            let ty = if self.eat(&TokenKind::Colon) {
                Some(self.parse_type()?)
            } else {
                None
            };
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
                ty,
                init,
                else_body,
            });
        }

        let _is_mut = self.eat_ident("mut");
        let (name, _) = self.expect_binding_ident("variable name")?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        if self.peek().kind == TokenKind::Semicolon {
            let Some(ty) = ty else {
                return Err(Error::at(
                    span,
                    format!("uninitialized 'let {name};' needs a type ('let {name}: i16;')"),
                ));
            };
            self.bump();
            return Ok(V2Stmt::LetUninit {
                span,
                name,
                ty,
                slot: None,
            });
        }
        self.expect(&TokenKind::Assign, "'='")?;
        let init = self.parse_expr()?;
        self.expect(&TokenKind::Semicolon, "';'")?;
        Ok(V2Stmt::Let {
            span,
            name,
            ty,
            init,
            slot: None,
        })
    }

    /// `@slot(N) let x ..;`: a `let` whose variable lives in frame word `local[N]`.
    fn parse_slot_let_stmt(&mut self, span: Span) -> Result<V2Stmt> {
        self.bump(); // '@'
        self.bump(); // 'slot'
        self.expect(&TokenKind::LParen, "'('")?;
        let slot_span = self.span();
        let n = self.expect_u16("local slot")?;
        let n = u8::try_from(n)
            .map_err(|_| Error::at(slot_span, format!("local slot {n} is out of range 0..=255")))?;
        self.expect(&TokenKind::RParen, "')'")?;
        if !matches!(&self.peek().kind, TokenKind::Ident(kw) if kw == "let") {
            return Err(Error::at(
                self.span(),
                "'@slot(N)' must be followed by a 'let' statement",
            ));
        }
        let let_span = self.span();
        match self.parse_let_stmt(let_span)? {
            V2Stmt::Let {
                span: _,
                name,
                ty,
                init,
                slot: _,
            } => Ok(V2Stmt::Let {
                span,
                name,
                ty,
                init,
                slot: Some(n),
            }),
            V2Stmt::LetUninit {
                span: _,
                name,
                ty,
                slot: _,
            } => Ok(V2Stmt::LetUninit {
                span,
                name,
                ty,
                slot: Some(n),
            }),
            _ => Err(Error::at(
                let_span,
                "'@slot(N)' applies to 'let x = ..;' and 'let x: T;', not 'let Some(..)'",
            )),
        }
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
        let (binding, _) = self.expect_binding_ident("loop variable name")?;
        if !self.eat_ident("in") {
            return Err(Error::at(self.span(), "expected 'in' in 'for' loop"));
        }
        let ispan = self.span();
        if let Some(iter) = self.try_parse_range_iter()? {
            let body = self.parse_block()?;
            if self.eat_ident("else") {
                return Err(Error::at(
                    self.span(),
                    "'for ... else' requires an object iterator",
                ));
            }
            return Ok(V2Stmt::ForEach {
                span,
                binding,
                iter,
                body,
                else_body: None,
            });
        }
        let (src, _) = self
            .expect_ident("iterator ('a..b', 'objects_by_guid', 'objects_of', 'lot', or 'self')")?;
        let iter = match src.as_str() {
            "objects_by_guid" => {
                self.expect(&TokenKind::LParen, "'('")?;
                let guid = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "')'")?;
                ForEachIter::ObjectsByGuid(guid)
            }
            "lot" => {
                self.expect(&TokenKind::Dot, "'.'")?;
                let (m, mspan) = self.expect_ident("'sims' or 'objects'")?;
                self.expect(&TokenKind::LParen, "'('")?;
                self.expect(&TokenKind::RParen, "')'")?;
                match m.as_str() {
                    "sims" => ForEachIter::LotSims,
                    "objects" => ForEachIter::LotObjects,
                    other => {
                        return Err(Error::at(
                            mspan,
                            format!(
                                "unknown lot iterator 'lot.{other}()' (expected 'lot.sims()' or 'lot.objects()')"
                            ),
                        ));
                    }
                }
            }
            "objects_of" => {
                self.expect(&TokenKind::ColonColon, "'::'")?;
                self.expect(&TokenKind::Lt, "'<'")?;
                let (object_type, ospan) = self.expect_ident("object type name")?;
                self.expect(&TokenKind::Gt, "'>'")?;
                self.expect(&TokenKind::LParen, "'('")?;
                self.expect(&TokenKind::RParen, "')'")?;
                ForEachIter::ObjectsOf {
                    span: ospan,
                    object_type,
                }
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
                        "unsupported 'for' iterator '{other}' (expected 'a..b', 'objects_of::<Type>()', 'objects_by_guid(0x...)', 'lot.sims()', 'lot.objects()', or 'self.parts()')"
                    ),
                ));
            }
        };
        let body = self.parse_block()?;
        let else_body = if self.eat_ident("else") {
            Some(self.parse_block()?)
        } else {
            None
        };
        Ok(V2Stmt::ForEach {
            span,
            binding,
            iter,
            body,
            else_body,
        })
    }

    /// Parses `a..b`, `a..=b`, or `(a..b).rev()` when the iterator is a range (anything other
    /// than the built-in object iterators).
    fn try_parse_range_iter(&mut self) -> Result<Option<ForEachIter>> {
        if self.peek().kind == TokenKind::LParen {
            // `(a..b).rev()`, or a range whose start is parenthesized (`(n + 1)..m`).
            // Once the start is followed by `..`/`..=` inside the parentheses, this is the
            // reversed form and its errors are reported as such.
            let (saved_pos, saved_depth) = (self.pos, self.depth);
            self.bump();
            let rev = match self.parse_expr() {
                Ok(start)
                    if matches!(self.peek().kind, TokenKind::DotDot | TokenKind::DotDotEq) =>
                {
                    let inclusive = self.eat(&TokenKind::DotDotEq);
                    if !inclusive {
                        self.bump();
                    }
                    let end = self.parse_expr()?;
                    Some((start, end, inclusive))
                }
                _ => None,
            };
            if let Some((start, end, inclusive)) = rev {
                self.expect(&TokenKind::RParen, "')'")?;
                self.expect(&TokenKind::Dot, "'.rev()'")?;
                let (m, mspan) = self.expect_ident("'rev'")?;
                if m != "rev" {
                    return Err(Error::at(
                        mspan,
                        format!("unknown range adapter '.{m}()' (expected '.rev()')"),
                    ));
                }
                self.expect(&TokenKind::LParen, "'('")?;
                self.expect(&TokenKind::RParen, "')'")?;
                return Ok(Some(ForEachIter::Range {
                    start,
                    end,
                    inclusive,
                    rev: true,
                }));
            }
            self.pos = saved_pos;
            self.depth = saved_depth;
        }
        // The builtin iterators, by their call syntax: `objects_by_guid(...)`, `objects_of::<T>()`,
        // and the method forms `lot.sims()`, `lot.objects()`, `self.parts()` (another `lot.x()`
        // or `self.x()` gets the iterator diagnostic). Anything else, such as `self.count..3` or
        // a local named `objects_by_guid`, is a range start.
        let is_method_call = |p: &Self| {
            p.peek_at(1).kind == TokenKind::Dot
                && matches!(p.peek_at(2).kind, TokenKind::Ident(_))
                && p.peek_at(3).kind == TokenKind::LParen
        };
        let builtin = match &self.peek().kind {
            TokenKind::Ident(s) => match s.as_str() {
                "objects_by_guid" => self.peek_at(1).kind == TokenKind::LParen,
                "objects_of" => self.peek_at(1).kind == TokenKind::ColonColon,
                "lot" | "self" => is_method_call(self),
                _ => false,
            },
            _ => false,
        };
        if builtin {
            return Ok(None);
        }
        let (start, end, inclusive) = self.parse_range_bounds()?;
        Ok(Some(ForEachIter::Range {
            start,
            end,
            inclusive,
            rev: false,
        }))
    }

    fn parse_range_bounds(&mut self) -> Result<(V2Expr, V2Expr, bool)> {
        let start = self.parse_expr()?;
        let inclusive = if self.eat(&TokenKind::DotDotEq) {
            true
        } else {
            self.expect(&TokenKind::DotDot, "'..' or '..='")?;
            false
        };
        let end = self.parse_expr()?;
        Ok((start, end, inclusive))
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

    /// A match arm's body without braces: a statement (`return x;`), or an expression or
    /// assignment ended by `,` or by the match's closing `}` (`0 => f(),`, `1 => x = 2,`).
    fn parse_arm_stmt(&mut self) -> Result<V2Stmt> {
        let (saved_pos, saved_depth) = (self.pos, self.depth);
        let stmt_err = match self.parse_stmt() {
            Ok(s) => {
                let _ = self.eat(&TokenKind::Comma);
                return Ok(s);
            }
            Err(e) => e,
        };
        self.pos = saved_pos;
        self.depth = saved_depth;
        let span = self.span();
        let short = (|| {
            let lhs = self.parse_expr()?;
            let stmt = if let Some(op) = self.try_eat_assign_op() {
                let value = self.parse_expr()?;
                V2Stmt::Assign {
                    span,
                    target: lhs,
                    op,
                    value,
                }
            } else {
                V2Stmt::Expr { span, expr: lhs }
            };
            if self.eat(&TokenKind::Comma) || self.peek().kind == TokenKind::RBrace {
                Ok(stmt)
            } else {
                Err(Error::at(
                    self.span(),
                    "expected ',' or '}' after a match arm",
                ))
            }
        })();
        // Neither form fits: the statement's diagnostic is the one to report.
        short.map_err(|_| stmt_err)
    }

    fn parse_match_stmt(&mut self, span: Span) -> Result<V2Stmt> {
        self.bump(); // 'match'
        let scrutinee = self.parse_expr()?;
        let open_span = self.expect(&TokenKind::LBrace, "'{'")?;
        self.enter_nesting(open_span)?;
        let mut arms = Vec::new();
        let res = loop {
            if self.eat(&TokenKind::RBrace) {
                break Ok(arms);
            }
            if self.peek().kind == TokenKind::Eof {
                break Err(Error::at(open_span, "unterminated 'match { ... }' block"));
            }
            let arm_span = self.span();
            let pattern = match self.parse_match_pattern() {
                Ok(p) => p,
                Err(e) => break Err(e),
            };
            if let Err(e) = self.expect(&TokenKind::FatArrow, "'=>'") {
                break Err(e);
            }
            let body = if self.peek().kind == TokenKind::LBrace {
                match self.parse_block() {
                    Ok(b) => {
                        let _ = self.eat(&TokenKind::Comma);
                        b
                    }
                    Err(e) => break Err(e),
                }
            } else {
                match self.parse_arm_stmt() {
                    Ok(s) => vec![s],
                    Err(e) => break Err(e),
                }
            };
            arms.push(MatchArm {
                span: arm_span,
                pattern,
                body,
            });
        };
        self.leave_nesting();
        let arms = res?;
        if arms.is_empty() {
            return Err(Error::at(span, "'match' must have at least one arm"));
        }
        Ok(V2Stmt::Match {
            span,
            scrutinee,
            arms,
        })
    }

    /// Expression `match s { P => value, ... }` (the `match` keyword is already consumed).
    fn parse_match_expr(&mut self, span: Span) -> Result<V2Expr> {
        // Entered before the scrutinee, so `match match match ... x {..} {..}` is bounded too. A
        // match level costs about two expression levels of stack (scrutinee and arm parses), so
        // it counts twice: the depth budget is sized for the default 2 MiB test-thread stack.
        self.enter_nesting(span)?;
        if let Err(e) = self.enter_nesting(span) {
            self.leave_nesting();
            return Err(e);
        }
        let mut arms = Vec::new();
        let mut scrutinee = None;
        let res = (|| {
            scrutinee = Some(self.parse_expr()?);
            let open_span = self.expect(&TokenKind::LBrace, "'{'")?;
            loop {
                if self.eat(&TokenKind::RBrace) {
                    return Ok(());
                }
                if self.peek().kind == TokenKind::Eof {
                    return Err(Error::at(
                        open_span,
                        "unterminated 'match { ... }' expression",
                    ));
                }
                let arm_span = self.span();
                let pattern = self.parse_match_pattern()?;
                self.expect(&TokenKind::FatArrow, "'=>'")?;
                if self.peek().kind == TokenKind::LBrace {
                    return Err(Error::at(
                        self.span(),
                        "a 'match' expression arm is a value ('Pattern => expr'), not a block",
                    ));
                }
                let value = self.parse_expr()?;
                arms.push(MatchExprArm {
                    span: arm_span,
                    pattern,
                    value,
                });
                if !self.eat(&TokenKind::Comma) {
                    self.expect(&TokenKind::RBrace, "',' or '}'")?;
                    return Ok(());
                }
            }
        })();
        self.leave_nesting();
        self.leave_nesting();
        res?;
        let scrutinee = scrutinee.expect("set before the arms parse");
        if arms.is_empty() {
            return Err(Error::at(span, "'match' must have at least one arm"));
        }
        Ok(V2Expr::Match {
            span,
            scrutinee: Box::new(scrutinee),
            arms,
        })
    }

    /// Expression `if c { a } else { b }` (the `if` keyword is already consumed). Like a
    /// `match` level, it counts twice against the nesting budget.
    fn parse_if_expr(&mut self, span: Span) -> Result<V2Expr> {
        self.enter_nesting(span)?;
        if let Err(e) = self.enter_nesting(span) {
            self.leave_nesting();
            return Err(e);
        }
        let res = (|| {
            let cond = self.parse_expr()?;
            let then_value = self.parse_if_expr_branch()?;
            if !self.eat_ident("else") {
                return Err(Error::at(
                    span,
                    "an 'if' expression needs an 'else' branch (every path yields a value)",
                ));
            }
            let else_value = if self.is_ident("if") {
                let elif_span = self.bump().span;
                self.parse_if_expr(elif_span)?
            } else {
                self.parse_if_expr_branch()?
            };
            Ok(V2Expr::If {
                span,
                cond: Box::new(cond),
                then_value: Box::new(then_value),
                else_value: Box::new(else_value),
            })
        })();
        self.leave_nesting();
        self.leave_nesting();
        res
    }

    /// `{ value }`: one branch of an `if` expression.
    fn parse_if_expr_branch(&mut self) -> Result<V2Expr> {
        self.expect(&TokenKind::LBrace, "'{'")?;
        let value = self.parse_expr()?;
        if self.peek().kind != TokenKind::RBrace {
            return Err(Error::at(
                self.span(),
                "an 'if' expression branch is one value ('{ expr }'), not a block of statements",
            ));
        }
        self.bump();
        Ok(value)
    }

    /// A match pattern, with `A | B | ..` alternatives.
    fn parse_match_pattern(&mut self) -> Result<MatchPattern> {
        let span = self.span();
        let first = self.parse_single_match_pattern()?;
        if self.peek().kind != TokenKind::Pipe {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.eat(&TokenKind::Pipe) {
            alts.push(self.parse_single_match_pattern()?);
        }
        // Identifiers stay: `match dialog(..)` names its buttons (`Yes | Cancel`).
        if let Some(MatchPattern::Wildcard(s)) =
            alts.iter().find(|a| matches!(a, MatchPattern::Wildcard(_)))
        {
            return Err(Error::at(
                *s,
                "'_' cannot be an alternative of an OR-pattern (it would match everything)",
            ));
        }
        Ok(MatchPattern::Or(span, alts))
    }

    fn parse_single_match_pattern(&mut self) -> Result<MatchPattern> {
        let span = self.span();
        match &self.peek().kind {
            TokenKind::Ident(s) if s == "_" => {
                self.bump();
                Ok(MatchPattern::Wildcard(span))
            }
            TokenKind::Ident(s) => {
                let first = s.clone();
                self.bump();
                if self.peek().kind == TokenKind::ColonColon {
                    let mut segs = vec![first];
                    while self.eat(&TokenKind::ColonColon) {
                        let (seg, _) = self.expect_ident("pattern path segment")?;
                        segs.push(seg);
                    }
                    Ok(MatchPattern::Path(span, segs))
                } else {
                    Ok(MatchPattern::Ident(span, first))
                }
            }
            TokenKind::Int(_) | TokenKind::Minus | TokenKind::Plus => {
                let val = self.expect_i16("match integer pattern")?;
                let inclusive = self.eat(&TokenKind::DotDotEq);
                if !inclusive && !self.eat(&TokenKind::DotDot) {
                    return Ok(MatchPattern::Int(span, val));
                }
                let (hi, hspan) = self.parse_signed_int("range pattern end")?;
                let hi = if inclusive { hi } else { hi - 1 };
                let hi = i16::try_from(hi).map_err(|_| {
                    Error::at(hspan, format!("range pattern end {hi} out of i16 range"))
                })?;
                if hi < val {
                    return Err(Error::at(span, "empty range pattern"));
                }
                Ok(MatchPattern::Range(span, val, hi))
            }
            other => Err(Error::at(
                span,
                format!("expected match pattern, found {other:?}"),
            )),
        }
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

    fn parse_turbofish_type_args(&mut self) -> Result<Vec<V2Type>> {
        self.expect(&TokenKind::ColonColon, "'::'")?;
        self.expect(&TokenKind::Lt, "'<'")?;
        let mut types = Vec::new();
        while !self.eat(&TokenKind::Gt) {
            types.push(self.parse_type()?);
            if !self.eat(&TokenKind::Comma) {
                self.expect(&TokenKind::Gt, "'>'")?;
                break;
            }
        }
        if types.is_empty() {
            return Err(Error::at(
                self.span(),
                "turbofish '::<...>' requires at least one type argument",
            ));
        }
        Ok(types)
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
                    if self.peek().kind == TokenKind::ColonColon
                        && self.peek_at(1).kind == TokenKind::Lt
                    {
                        let type_args = self.parse_turbofish_type_args()?;
                        let args = self.parse_call_args()?;
                        expr = V2Expr::MethodCall {
                            span,
                            receiver: Box::new(expr),
                            method: field,
                            type_args,
                            args,
                        };
                    } else if self.peek().kind == TokenKind::LParen {
                        let args = self.parse_call_args()?;
                        expr = V2Expr::MethodCall {
                            span,
                            receiver: Box::new(expr),
                            method: field,
                            type_args: Vec::new(),
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
                TokenKind::ColonColon if self.peek_at(1).kind == TokenKind::Lt => {
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
                    let type_args = self.parse_turbofish_type_args()?;
                    let args = self.parse_call_args()?;
                    expr = V2Expr::Call {
                        span,
                        callee: Box::new(expr),
                        type_args,
                        args,
                    };
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
                TokenKind::Question => {
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
                    expr = V2Expr::Try {
                        span,
                        inner: Box::new(expr),
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
                        type_args: Vec::new(),
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
                    "match" => self.parse_match_expr(tok.span),
                    "if" => self.parse_if_expr(tok.span),
                    _ => {
                        if self.peek().kind == TokenKind::ColonColon
                            && self.peek_at(1).kind != TokenKind::Lt
                        {
                            let mut segs = vec![s];
                            while self.peek().kind == TokenKind::ColonColon
                                && self.peek_at(1).kind != TokenKind::Lt
                            {
                                self.bump();
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
                let res = (|| {
                    let first = self.parse_expr()?;
                    if self.eat(&TokenKind::Comma) {
                        let mut elems = vec![first];
                        while !self.eat(&TokenKind::RParen) {
                            elems.push(self.parse_expr()?);
                            if !self.eat(&TokenKind::Comma) {
                                self.expect(&TokenKind::RParen, "')'")?;
                                break;
                            }
                        }
                        Ok(V2Expr::Tuple(tok.span, elems))
                    } else {
                        self.expect(&TokenKind::RParen, "')'")?;
                        Ok(first)
                    }
                })();
                self.leave_nesting();
                res
            }
            other => Err(Error::at(
                tok.span,
                format!("expected expression, found {other:?}"),
            )),
        }
    }
}

/// Maps an addressable BCON chunk id to its SimAntics operand table index (`address >> 7`),
/// matching `sims-vm`'s `Program::constant`.
pub(crate) fn bcon_chunk_table(id: u16) -> Option<u16> {
    match id {
        4096..=4159 => Some(id - 4096),
        8192..=8255 => Some(id - 8192 + 64),
        256..=319 => Some(id - 256 + 128),
        _ => None,
    }
}

/// Derives a `snake_case` call identifier from a tree label (`"CT - Is Fridge"` -> `ct_is_fridge`).
pub(crate) fn snake_case_ident(label: &str) -> String {
    let mut out = String::new();
    let mut pending_sep = false;
    let mut prev_lower = false;
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && prev_lower {
                pending_sep = true;
            }
            if pending_sep && !out.is_empty() {
                out.push('_');
            }
            pending_sep = false;
            prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_sep = true;
            prev_lower = false;
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert_str(0, "tree_");
    }
    out
}

fn default_routing_slot(span: Span, name: String) -> SlotDecl {
    SlotDecl {
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
    }
}

fn default_container_slot(span: Span, name: String) -> SlotDecl {
    SlotDecl {
        span,
        name,
        kind: 0,
        offset: [0.0, 0.0, 0.64],
        standing: 0,
        sitting: 0,
        ground: 0,
        flags: 1,
        snap_target: -1,
        proximity: [0, 0, 0],
        max_size: 100,
        gradient: 0.0,
        height: 5,
        facing: 0,
        resolution: 16,
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

/// A binding arm's value: a tuple yields one operand per name.
fn values_arm(span: Span, value: V2Expr) -> Arm {
    match value {
        V2Expr::Tuple(_, items) => Arm::Values(span, items),
        v => Arm::Values(span, vec![v]),
    }
}
