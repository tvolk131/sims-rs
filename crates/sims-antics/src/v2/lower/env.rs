//! Object environment: dialog string tables, enum/bitflags resolution, and `ObjectEnv` lookups
//! shared by every tree of one object.

use super::*;

impl DialogStringTable {
    pub(super) fn from_existing(existing: Option<&StringsDecl>) -> Self {
        let mut tbl = Self::default();
        if let Some(st) = existing {
            tbl.label = st.label.clone();
            for (span, name, text) in &st.entries {
                tbl.entries.push((*span, name.clone(), text.clone()));
                tbl.by_text
                    .entry(text.clone())
                    .or_insert_with(|| name.clone());
                tbl.used_names.insert(name.clone());
            }
        }
        tbl
    }

    pub(super) fn intern(&mut self, text: &str, span: Span) -> Result<String> {
        if let Some(existing) = self.by_text.get(text) {
            return Ok(existing.clone());
        }
        if self.entries.len() >= 255 {
            return Err(Error::at(
                span,
                "STR# 301 dialog string table exceeds maximum of 255 entries",
            ));
        }
        let name = loop {
            let candidate = format!("__dlg_str_{}", self.next_auto_idx);
            self.next_auto_idx += 1;
            if self.used_names.insert(candidate.clone()) {
                break candidate;
            }
        };
        self.entries.push((span, name.clone(), text.to_owned()));
        self.by_text.insert(text.to_owned(), name.clone());
        Ok(name)
    }
}

/// `Height` (object data word 4), a prelude type declared in `std::flags`.
pub(super) const HEIGHT_KEY: &str = "bitflags::std::flags::Height";
/// `Placement`, a prelude type declared in `std::flags`.
pub(super) const PLACEMENT_KEY: &str = "bitflags::std::flags::Placement";

/// The enums and bitflags of the built-in `std::flags` and `std::engine` modules, keyed as an
/// import of them keys them (`bitflags::std::flags::Height`). Every object resolves them, so
/// typed engine fields name the same type whether or not the source imports it.
pub(super) struct StdTypes {
    pub(super) enums: BTreeMap<String, ResolvedEnum>,
    pub(super) bitflags: BTreeMap<String, ResolvedBitflags>,
}

pub(super) fn std_types() -> Result<&'static StdTypes> {
    static TYPES: std::sync::OnceLock<std::result::Result<StdTypes, String>> =
        std::sync::OnceLock::new();
    TYPES
        .get_or_init(|| {
            let mut types = StdTypes {
                enums: BTreeMap::new(),
                bitflags: BTreeMap::new(),
            };
            for m in ["flags", "engine"] {
                let module = format!("std::{m}");
                let src = crate::v2::loader::std_module_source(&[m.to_owned()])
                    .ok_or_else(|| format!("missing built-in module '{module}'"))?;
                let file = crate::v2::parser::parse_v2_std_file(src)
                    .map_err(|e| format!("built-in module '{module}': {e}"))?;
                for item in &file.items {
                    match item {
                        V2Item::Enum(d) => {
                            let key = format!("enum::{module}::{}", d.name);
                            types.enums.insert(key.clone(), resolve_enum_decl(key, d));
                        }
                        V2Item::Bitflags(d) => {
                            let key = format!("bitflags::{module}::{}", d.name);
                            types
                                .bitflags
                                .insert(key.clone(), resolve_bitflags_decl(key, d));
                        }
                        _ => {}
                    }
                }
            }
            Ok(types)
        })
        .as_ref()
        .map_err(|e| Error::new(e.clone()))
}

pub(super) fn bit_to_mask(bit: u8) -> i16 {
    debug_assert!((1..=16).contains(&bit));
    (1u16 << (bit - 1)) as i16
}

pub(super) fn resolve_enum_decl(canonical_key: String, decl: &EnumDecl) -> ResolvedEnum {
    let mut variants = BTreeMap::new();
    for v in &decl.variants {
        variants.insert(v.name.clone(), v.value);
    }
    ResolvedEnum {
        canonical_key,
        display_name: decl.name.clone(),
        variants,
    }
}

pub(super) fn resolve_bitflags_decl(
    canonical_key: String,
    decl: &BitflagsDecl,
) -> ResolvedBitflags {
    let mut flags = BTreeMap::new();
    for f in &decl.flags {
        flags.insert(f.name.clone(), f.bit);
    }
    ResolvedBitflags {
        canonical_key,
        display_name: decl.name.clone(),
        flags,
    }
}

impl<'a> ObjectEnv<'a> {
    pub(super) fn resolve_const_int(&self, expr: &ConstIntExpr, span: Span) -> Result<i16> {
        match expr {
            ConstIntExpr::Literal(v) => Ok(*v),
            ConstIntExpr::Param { name, negate } => {
                let Some(&val) = self.i16_params.get(name) else {
                    if self.bool_params.contains_key(name) {
                        return Err(Error::at(
                            span,
                            format!("bool param '{name}' cannot be used as an i16 constant"),
                        ));
                    }
                    return Err(Error::at(span, format!("unknown i16 parameter '{name}'")));
                };
                if *negate {
                    val.checked_neg().ok_or_else(|| {
                        Error::at(span, format!("negating param '{name}' overflows i16"))
                    })
                } else {
                    Ok(val)
                }
            }
        }
    }

    pub(super) fn lookup_fn(
        &self,
        owner_module: &str,
        name: &str,
        allow_local_fns: bool,
    ) -> Option<(String, FnDecl, bool)> {
        if allow_local_fns && let Some((m, decl)) = self.local_fns.get(name) {
            return Some((m.clone(), decl.clone(), true));
        }
        self.resolved
            .module_scopes
            .get(owner_module)
            .and_then(|s: &ModuleScope| s.functions.get(name))
            .map(|(m, decl)| (m.clone(), decl.clone(), false))
    }

    /// Unified lexical type lookup used identically by declarations, paths, and constant folding:
    /// 1. Object-local scope (when `allow_local`)
    /// 2. Built-in types (`Height`, `Placement`, `Self`)
    /// 3. Module scope of `owner_module` (enums, bitflags, objects)
    pub(super) fn lookup_named_type(
        &self,
        owner_module: &str,
        name: &str,
        allow_local: bool,
    ) -> Option<ResolvedNamedType> {
        if allow_local {
            if let Some(key) = self.local_enums.get(name) {
                return Some(ResolvedNamedType::Enum(key.clone()));
            }
            if let Some(key) = self.local_bitflags.get(name) {
                return Some(ResolvedNamedType::Bitflags(key.clone()));
            }
        }
        match name {
            "Height" => return Some(ResolvedNamedType::Bitflags(HEIGHT_KEY.to_owned())),
            "Placement" => return Some(ResolvedNamedType::Bitflags(PLACEMENT_KEY.to_owned())),
            "Self" => return Some(ResolvedNamedType::Object(self.self_object_key.clone())),
            _ => {}
        }
        if let Some(scope) = self.resolved.module_scopes.get(owner_module) {
            if let Some((def_mod, decl)) = scope.enums.get(name) {
                return Some(ResolvedNamedType::Enum(format!(
                    "enum::{def_mod}::{}",
                    decl.name
                )));
            }
            if let Some((def_mod, decl)) = scope.bitflags.get(name) {
                return Some(ResolvedNamedType::Bitflags(format!(
                    "bitflags::{def_mod}::{}",
                    decl.name
                )));
            }
            if let Some((def_mod, decl)) = scope.objects.get(name) {
                return Some(ResolvedNamedType::Object(format!(
                    "object::{def_mod}::{}",
                    object_type_ident(&decl.name)
                )));
            }
        }
        None
    }

    pub(super) fn lookup_enum(
        &self,
        owner_module: &str,
        name: &str,
        allow_local: bool,
    ) -> Option<&ResolvedEnum> {
        match self.lookup_named_type(owner_module, name, allow_local) {
            Some(ResolvedNamedType::Enum(key)) => self.all_enums.get(&key),
            _ => None,
        }
    }

    pub(super) fn lookup_bitflags(
        &self,
        owner_module: &str,
        name: &str,
        allow_local: bool,
    ) -> Option<&ResolvedBitflags> {
        match self.lookup_named_type(owner_module, name, allow_local) {
            Some(ResolvedNamedType::Bitflags(key)) => self.all_bitflags.get(&key),
            _ => None,
        }
    }

    pub(super) fn lookup_object_schema(
        &self,
        owner_module: &str,
        name: &str,
        allow_local: bool,
    ) -> Option<&ResolvedObjectSchema> {
        match self.lookup_named_type(owner_module, name, allow_local) {
            Some(ResolvedNamedType::Object(key)) => self.object_schemas.get(&key),
            _ => None,
        }
    }

    pub(super) fn resolve_value_type(
        &self,
        ty: &V2Type,
        owner_module: &str,
        allow_local: bool,
        span: Span,
    ) -> Result<ValueType> {
        match ty {
            V2Type::I16 => Ok(ValueType::I16),
            V2Type::Bool => Ok(ValueType::Bool),
            V2Type::ObjectId => Ok(ValueType::ObjectId),
            V2Type::Named(name) => match self.lookup_named_type(owner_module, name, allow_local) {
                Some(ResolvedNamedType::Enum(k)) if self.all_enums.contains_key(&k) => {
                    Ok(ValueType::Enum(k))
                }
                Some(ResolvedNamedType::Bitflags(k)) if self.all_bitflags.contains_key(&k) => {
                    Ok(ValueType::Bitflags(k))
                }
                Some(ResolvedNamedType::Object(k)) if self.object_schemas.contains_key(&k) => {
                    Ok(ValueType::ObjectRef(k))
                }
                _ => Err(Error::at(span, format!("unknown type '{name}'"))),
            },
            other => Err(Error::at(
                span,
                format!("unsupported value type '{}'", other.display_name()),
            )),
        }
    }

    pub(super) fn lookup_extern_mod(
        &self,
        owner_module: &str,
        name: &str,
        allow_local: bool,
    ) -> Option<(String, bool, &ExternModDecl)> {
        if allow_local && let Some((m, decl)) = self.local_extern_mods.get(name) {
            return Some((m.clone(), true, decl));
        }
        self.resolved
            .module_scopes
            .get(owner_module)
            .and_then(|s| s.extern_mods.get(name))
            .map(|(m, decl)| (m.clone(), false, decl))
    }

    pub(super) fn lookup_extern_bcon(
        &self,
        owner_module: &str,
        name: &str,
        allow_local: bool,
    ) -> Option<&ExternBconDecl> {
        if allow_local && let Some((_, decl)) = self.local_extern_bcons.get(name) {
            return Some(decl);
        }
        self.resolved
            .module_scopes
            .get(owner_module)
            .and_then(|s| s.extern_bcons.get(name))
            .map(|(_, decl)| decl)
    }

    /// Value of a `const` item visible from `owner_module` (object/template-local first).
    pub(super) fn lookup_const(
        &self,
        owner_module: &str,
        name: &str,
        allow_local: bool,
    ) -> Option<i64> {
        if allow_local && let Some(&v) = self.local_consts.get(name) {
            return Some(v);
        }
        self.resolved
            .module_scopes
            .get(owner_module)
            .and_then(|s| s.consts.get(name))
            .map(|(_, decl)| decl.value)
    }

    /// `true` if `name` refers to a private BCON table or `extern bcon` block in this scope.
    pub(super) fn is_bcon_table_name(
        &self,
        owner_module: &str,
        name: &str,
        allow_local: bool,
    ) -> bool {
        (allow_local && self.bcon_tables.contains_key(name))
            || self
                .lookup_extern_bcon(owner_module, name, allow_local)
                .is_some()
    }

    /// Resolves `table.entry` to its `.air` constant operand and (for private tables) its
    /// compile-time value. Returns `None` if `table` is not a BCON table name in scope.
    pub(super) fn lookup_bcon_entry(
        &self,
        owner_module: &str,
        table: &str,
        entry: &str,
        allow_local: bool,
        span: Span,
    ) -> Option<Result<(String, Option<i16>)>> {
        if allow_local && let Some(t) = self.bcon_tables.get(table) {
            return Some(match t.entries.get(entry) {
                Some(&(addr, val)) => Ok((format!("bcon[{addr}]"), Some(val))),
                None => Err(Error::at(
                    span,
                    format!(
                        "BCON table '{}' (BCON {}) has no entry '{entry}'",
                        t.label, t.id
                    ),
                )),
            });
        }
        let decl = self.lookup_extern_bcon(owner_module, table, allow_local)?;
        Some(match decl.entries.iter().find(|e| e.name == entry) {
            Some(e) => Ok((format!("bcon[{}]", e.address), None)),
            None => Err(Error::at(
                span,
                format!("extern bcon '{table}' has no entry '{entry}'"),
            )),
        })
    }

    /// Resolves an `extern mod` fn or standalone tree signature's declared types.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_callable_sig(
        &self,
        display: String,
        id: u16,
        context: CallContext,
        params: &[AbiParam],
        ret: &ReturnAbi,
        decl_module: &str,
        allow_local: bool,
        span: Span,
    ) -> Result<CallableSig> {
        let resolve = |ty: &V2Type, pspan: Span, what: &str| -> Result<ValueType> {
            self.resolve_value_type(ty, decl_module, allow_local, pspan)
                .map_err(|_| {
                    Error::at(
                        pspan,
                        format!(
                            "{what} has unsupported type '{}' in {display} (expected i16, bool, ObjectId, enum, bitflags, or object type)",
                            ty.display_name()
                        ),
                    )
                })
        };
        let mut out_params = Vec::with_capacity(params.len());
        let mut positional = 0usize;
        for p in params {
            let ty = resolve(&p.ty, p.span, &format!("parameter '{}'", p.name))?;
            if p.slot == AbiParamSlot::Positional {
                positional += 1;
            }
            out_params.push(ResolvedAbiParam {
                name: p.name.clone(),
                ty,
                slot: p.slot,
            });
        }
        if positional > 8 {
            return Err(Error::at(
                span,
                format!(
                    "{display} declares {positional} positional parameters (SimAntics passes at most 8 via temps)"
                ),
            ));
        }
        // Positional args passed through `call(temps)` land in `temp[0..positional]`; an explicit
        // `#[temp(N)]` binding inside that window would be overwritten.
        for p in params {
            if let AbiParamSlot::Temp(t) = p.slot
                && usize::from(t) < positional
            {
                return Err(Error::at(
                    p.span,
                    format!(
                        "parameter '{}' '#[temp({t})]' collides with positional parameter #{t} of {display}, which is passed in temp[{t}] by 'call(temps)' (bind explicit temps at or above temp[{positional}])",
                        p.name
                    ),
                ));
            }
        }
        let ret = match ret {
            ReturnAbi::Void => ResolvedReturnAbi::Void,
            ReturnAbi::Bool => ResolvedReturnAbi::Bool,
            ReturnAbi::Result => ResolvedReturnAbi::Result,
            ReturnAbi::Value {
                ty,
                temp,
                option: false,
            } => ResolvedReturnAbi::Value {
                ty: resolve(ty, span, "return value")?,
                temp: *temp,
            },
            ReturnAbi::Value {
                ty,
                temp,
                option: true,
            } => ResolvedReturnAbi::OptionValue {
                ty: resolve(ty, span, "return value")?,
                temp: *temp,
            },
            ReturnAbi::ResultValue { ty, temp } => ResolvedReturnAbi::ResultValue {
                ty: resolve(ty, span, "return value")?,
                temp: *temp,
            },
        };
        Ok(CallableSig {
            display,
            id,
            context,
            params: out_params,
            ret,
            // An `extern` declares its callee's whole frame; standalone trees refine this from
            // `#[frame(args = )]`.
            frame_args: Some(positional),
        })
    }

    pub(super) fn display_value_type(&self, ty: &ValueType) -> String {
        match ty {
            ValueType::I16 => "i16".to_owned(),
            ValueType::Bool => "bool".to_owned(),
            ValueType::ObjectId => "ObjectId".to_owned(),
            ValueType::Enum(k) => self
                .all_enums
                .get(k)
                .map_or_else(|| k.clone(), |e| e.display_name.clone()),
            ValueType::Bitflags(k) => self
                .all_bitflags
                .get(k)
                .map_or_else(|| k.clone(), |b| b.display_name.clone()),
            ValueType::ObjectRef(k) => self
                .object_schemas
                .get(k)
                .map_or_else(|| k.clone(), |o| o.display_name.clone()),
        }
    }
}
