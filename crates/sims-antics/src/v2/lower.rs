//! High-level `.antics` v2 → `.air` (Antics IR) + `ObjectManifest` lowering compiler.
//!
//! Performs template expansion, parameter validation, state-to-attribute allocation,
//! context safety (`self` vs `sim`), automatic `stack_obj` preservation, hygienic
//! helper inlining with `&mut` references, `with lock(...)`, LIFO `defer { ... }`,
//! `repeat ... cancel_on_abort`, and short-circuit boolean/return lowering.

use crate::art::FurnitureShape;
use crate::error::{Error, Result, Span};
use crate::manifest::{
    CallbackSpec, InteractionSpec, MotiveAdSpec, ObjectManifest, SlotSpec, SoundSpec, SpriteSpec,
    TileSpec, TreeRef,
};
use crate::parser::{motive_by_name, person_field_by_name, skill_by_name};
use crate::v2::ast::{
    AbiParam, AbiParamSlot, BconTableDecl, BitflagsDecl, CallArg, ConstIntExpr, EnumDecl,
    ExternBconDecl, ExternModDecl, FnDecl, ForEachIter, FunctionalDecl, GuardBody, GuardDecl,
    InteractionDecl, MatchArm, MatchPattern, MetaField, MetaFieldKind, ObjectDecl, ObjectMember,
    ParamType, ParamValue, ReturnAbi, SlotDecl, SoundDecl, SpriteDecl, StateDecl, StateInitValue,
    StateType, StringsDecl, TemplateDecl, TileDecl, TreeMemberDecl, V2AssignOp, V2BinOp, V2Expr,
    V2Stmt, V2Type, V2UnaryOp, object_type_ident,
};
use crate::v2::loader::{ModuleScope, ResolvedModule};
use crate::v2::parser::snake_case_ident;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const MAX_INLINE_DEPTH: usize = 16;
const MAX_INLINE_EXPANSIONS: usize = 256;

/// Output of expanding and lowering one concrete v2 `.antics` `object` declaration.
#[derive(Debug, Clone)]
pub struct ExpandedV2Object {
    pub name: String,
    pub slug: String,
    pub guid: u32,
    pub manifest: ObjectManifest,
    /// Complete lowered `.air` (Antics IR) source text ready for `compile_air` / `sims-antics expand`.
    pub air_source: String,
}

/// Lowers every concrete `object` in a [`ResolvedModule`] into `.air` + [`ObjectManifest`].
pub fn lower_resolved_module(resolved: &ResolvedModule) -> Result<Vec<ExpandedV2Object>> {
    if resolved.objects.is_empty() {
        return Err(Error::new(format!(
            "module '{}' declares no 'object' items",
            resolved.module_id
        )));
    }
    let mut out = Vec::with_capacity(resolved.objects.len());
    for obj in &resolved.objects {
        out.push(lower_object(resolved, obj)?);
    }
    Ok(out)
}

#[derive(Debug, Clone)]
struct OwnedStmtBlock {
    owner_module: String,
    stmts: Vec<V2Stmt>,
}

#[derive(Debug, Clone)]
struct OwnedInteraction {
    owner_module: String,
    decl: InteractionDecl,
}

#[derive(Debug, Clone)]
struct OwnedFunctional {
    owner_module: String,
    decl: FunctionalDecl,
}

#[derive(Debug, Clone)]
struct ResolvedState {
    attr_index: usize,
    ty: ValueType,
    init: Option<StateInitValue>,
    span: Span,
    /// `true` for `state x @ N: T;` without an initializer: `init` leaves `my.attr[N]` untouched.
    skip_zero_init: bool,
}

#[derive(Debug, Clone)]
struct ResolvedSound {
    id: u16,
}

#[derive(Debug, Clone)]
struct ResolvedEnum {
    canonical_key: String,
    display_name: String,
    variants: BTreeMap<String, i16>,
}

#[derive(Debug, Clone)]
struct ResolvedBitflags {
    canonical_key: String,
    display_name: String,
    /// Flag name -> 1-based SimAntics bit position (`1..=16`).
    flags: BTreeMap<String, u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FunctionalGuardSummary {
    /// Functional declares no guard (always eligible if not in-use / disabled).
    None,
    /// Guard is a simple boolean check on `self.<bool_state>` (or `!self.<bool_state>`).
    AttrBool {
        attr_index: usize,
        expect_true: bool,
    },
    /// Guard has arbitrary logic that cannot be evaluated inline from another object.
    Complex,
}

#[derive(Debug, Clone)]
struct ResolvedFunctionalSig {
    params: Vec<ValueType>,
    return_type: Option<ValueType>,
    guard: FunctionalGuardSummary,
}

#[derive(Debug, Clone)]
struct ResolvedObjectSchema {
    canonical_key: String,
    display_name: String,
    /// Master GUID (used by `create_object::<T>()` to spawn single-tile or full multi-tile groups).
    master_guid: u32,
    /// Spawned lead-tile GUID (used by `objects_of::<T>()` and fallback searches; equals `master_guid` for single-tile objects).
    lead_guid: u32,
    /// All spawned tile GUIDs belonging to this object (used for runtime `test_object_type` checks).
    tile_guids: Vec<u32>,
    states: BTreeMap<String, (usize, ValueType)>,
    functionals: BTreeMap<String, ResolvedFunctionalSig>,
}

/// Resolved return ABI of a callable BHAV (`extern mod` fn or standalone `tree`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedReturnAbi {
    Void,
    Bool,
    Result,
    Value { ty: ValueType, temp: u8 },
    ResultValue { ty: ValueType, temp: u8 },
}

/// Resolved parameter of a callable BHAV.
#[derive(Debug, Clone)]
struct ResolvedAbiParam {
    name: String,
    ty: ValueType,
    slot: AbiParamSlot,
}

/// Fully resolved call signature of an `extern mod` fn or a sibling standalone `tree`.
#[derive(Debug, Clone)]
struct CallableSig {
    /// Human-readable name for diagnostics (`light_globals::init_floor`, `tree 'is_fridge'`).
    display: String,
    id: u16,
    /// `Some(true)` for Sim-on-object trees, `Some(false)` for object-self trees, `None` for externs.
    sim_context: Option<bool>,
    params: Vec<ResolvedAbiParam>,
    ret: ResolvedReturnAbi,
}

/// Private named BCON table (`bcon 4097 "LampTuning" { ... }`) resolved to operand addresses.
#[derive(Debug, Clone)]
struct ResolvedBconTable {
    id: u16,
    label: String,
    /// Entry name -> (operand address `(table << 7) | entry`, compile-time value).
    entries: BTreeMap<String, (u16, i16)>,
    /// Declaration-ordered values.
    values: Vec<(String, i16)>,
}

/// Standalone `tree` member ready for lowering.
#[derive(Debug, Clone)]
struct OwnedTree {
    owner_module: String,
    decl: TreeMemberDecl,
}

/// Generalized "value in `temp[N]`" return ABI used by typed functionals and value-returning trees.
#[derive(Debug, Clone)]
struct ValueReturnAbi {
    ty: ValueType,
    temp: u8,
    /// `true` when `return None;` / `return false;` (non-bool `T`) exits through `return false;`.
    allow_none: bool,
    /// `true` to lower compound arithmetic returns directly into `temp[N]` when alias/trap-safe.
    in_place: bool,
    /// Diagnostic noun (`functional` / `tree`).
    what: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedNamedType {
    Enum(String),
    Bitflags(String),
    Object(String),
}

/// Mutable STR# 301 string interning table shared across all trees in one object.
#[derive(Debug, Default, Clone)]
struct DialogStringTable {
    label: Option<String>,
    entries: Vec<(Span, String, String)>,
    by_text: BTreeMap<String, String>,
    used_names: BTreeSet<String>,
    next_auto_idx: usize,
}

impl DialogStringTable {
    fn from_existing(existing: Option<&StringsDecl>) -> Self {
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

    fn intern(&mut self, text: &str, span: Span) -> Result<String> {
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

fn builtin_bitflags() -> BTreeMap<String, ResolvedBitflags> {
    let mut map = BTreeMap::new();
    map.insert(
        "builtin::Height".to_owned(),
        ResolvedBitflags {
            canonical_key: "builtin::Height".to_owned(),
            display_name: "Height".to_owned(),
            flags: BTreeMap::from([
                ("Floor".to_owned(), 1),
                ("LowTable".to_owned(), 2),
                ("Table".to_owned(), 3),
                ("Counter".to_owned(), 4),
                ("NonStandard".to_owned(), 5),
                ("InHand".to_owned(), 6),
                ("Sitting".to_owned(), 7),
                ("EndTable".to_owned(), 8),
            ]),
        },
    );
    map.insert(
        "builtin::Placement".to_owned(),
        ResolvedBitflags {
            canonical_key: "builtin::Placement".to_owned(),
            display_name: "Placement".to_owned(),
            flags: BTreeMap::from([
                ("Floor".to_owned(), 1),
                ("Terrain".to_owned(), 2),
                ("Water".to_owned(), 3),
                ("Sloped".to_owned(), 9),
                ("Air".to_owned(), 10),
                ("Pool".to_owned(), 12),
                ("WaterOnly".to_owned(), 14),
            ]),
        },
    );
    map
}

fn bit_to_mask(bit: u8) -> i16 {
    debug_assert!((1..=16).contains(&bit));
    (1u16 << (bit - 1)) as i16
}

fn resolve_enum_decl(canonical_key: String, decl: &EnumDecl) -> ResolvedEnum {
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

fn resolve_bitflags_decl(canonical_key: String, decl: &BitflagsDecl) -> ResolvedBitflags {
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

/// Flattened, validated object environment ready to emit `.air` and `ObjectManifest`.
struct ObjectEnv<'a> {
    resolved: &'a ResolvedModule,
    name: String,
    i16_params: BTreeMap<String, i16>,
    i16_param_order: Vec<String>,
    bool_params: BTreeMap<String, bool>,
    states: BTreeMap<String, ResolvedState>,
    state_order: Vec<String>,
    sounds: BTreeMap<String, ResolvedSound>,
    /// Slot declarations paired with their routing-only ordinal (`Some(idx)` for `kind == 3`).
    slots: BTreeMap<String, (Option<u16>, SlotDecl)>,
    strings: BTreeMap<u16, StringsDecl>,
    string_names: BTreeMap<String, u16>,
    dialog_strings: RefCell<DialogStringTable>,
    local_fns: BTreeMap<String, (String, FnDecl)>,
    /// Object/template-local enums by short name -> canonical key.
    local_enums: BTreeMap<String, String>,
    /// Object/template-local bitflags by short name -> canonical key.
    local_bitflags: BTreeMap<String, String>,
    /// All resolved enums (built-in + module + object-local) keyed by canonical key.
    all_enums: BTreeMap<String, ResolvedEnum>,
    /// All resolved bitflags (built-in + module + object-local) keyed by canonical key.
    all_bitflags: BTreeMap<String, ResolvedBitflags>,
    /// All resolved object schemas (including `self_object_key` and imported objects) keyed by canonical key.
    object_schemas: BTreeMap<String, ResolvedObjectSchema>,
    /// Canonical schema key for the object currently being lowered (`Self`).
    self_object_key: String,
    /// Object/template-local `extern mod` blocks by name -> (owner module, decl).
    local_extern_mods: BTreeMap<String, (String, ExternModDecl)>,
    /// Object/template-local `extern bcon` blocks by name -> (owner module, decl).
    local_extern_bcons: BTreeMap<String, (String, ExternBconDecl)>,
    /// Object/template-local `const` items by name.
    local_consts: BTreeMap<String, i64>,
    /// Private named BCON tables by reference name (`LampTuning` and `lamp_tuning`).
    bcon_tables: BTreeMap<String, ResolvedBconTable>,
    /// Standalone sibling `tree` call signatures by call identifier.
    trees: BTreeMap<String, CallableSig>,
}

impl<'a> ObjectEnv<'a> {
    fn resolve_const_int(&self, expr: &ConstIntExpr, span: Span) -> Result<i16> {
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

    fn lookup_fn(
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
    fn lookup_named_type(
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
            "Height" => return Some(ResolvedNamedType::Bitflags("builtin::Height".to_owned())),
            "Placement" => {
                return Some(ResolvedNamedType::Bitflags("builtin::Placement".to_owned()));
            }
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

    fn lookup_enum(
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

    fn lookup_bitflags(
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

    fn lookup_object_schema(
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

    fn resolve_value_type(
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

    fn lookup_extern_mod(
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

    fn lookup_extern_bcon(
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
    fn lookup_const(&self, owner_module: &str, name: &str, allow_local: bool) -> Option<i64> {
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
    fn is_bcon_table_name(&self, owner_module: &str, name: &str, allow_local: bool) -> bool {
        (allow_local && self.bcon_tables.contains_key(name))
            || self
                .lookup_extern_bcon(owner_module, name, allow_local)
                .is_some()
    }

    /// Resolves `table.entry` to its `.air` constant operand and (for private tables) its
    /// compile-time value. Returns `None` if `table` is not a BCON table name in scope.
    fn lookup_bcon_entry(
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
    fn resolve_callable_sig(
        &self,
        display: String,
        id: u16,
        sim_context: Option<bool>,
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
        // `@ temp[N]` binding inside that window would be overwritten.
        for p in params {
            if let AbiParamSlot::Temp(t) = p.slot
                && usize::from(t) < positional
            {
                return Err(Error::at(
                    p.span,
                    format!(
                        "parameter '{}' '@ temp[{t}]' collides with positional parameter #{t} of {display}, which is passed in temp[{t}] by 'call(temps)' (bind explicit temps at or above temp[{positional}])",
                        p.name
                    ),
                ));
            }
        }
        let ret = match ret {
            ReturnAbi::Void => ResolvedReturnAbi::Void,
            ReturnAbi::Bool => ResolvedReturnAbi::Bool,
            ReturnAbi::Result => ResolvedReturnAbi::Result,
            ReturnAbi::Value { ty, temp } => ResolvedReturnAbi::Value {
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
            sim_context,
            params: out_params,
            ret,
        })
    }

    fn display_value_type(&self, ty: &ValueType) -> String {
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

fn lower_object(resolved: &ResolvedModule, obj: &ObjectDecl) -> Result<ExpandedV2Object> {
    // 1. Resolve optional `from Template`.
    let template_opt: Option<(String, TemplateDecl)> = match &obj.from_template {
        Some((tpl_name, tspan)) => {
            let found = resolved
                .templates
                .get(tpl_name)
                .cloned()
                .ok_or_else(|| Error::at(*tspan, format!("unknown template '{tpl_name}'")))?;
            Some(found)
        }
        None => None,
    };

    let slug = make_slug(&obj.name);

    // 2. Merge metadata, params, states, sounds, slots, strings, hooks, interactions, functionals, local fns.
    let mut guid: Option<u32> = None;
    let mut price: u16 = 100;
    let mut description = String::new();
    let mut catalog_sort_flags: u16 = 0;
    let mut object_type: u16 = 4;
    let mut objd_id: u16 = 128;
    let mut ttab_id: u16 = 128;
    let mut slot_id: u16 = 128;
    let mut ctss_id: u16 = 2000;
    let mut dgrp_id: u16 = 100;
    let mut spr2_id: u16 = 100;
    let mut palt_id: u16 = 100;
    let mut master_id: Option<u16> = None;
    let mut interaction_group: i16 = 0;
    let mut data_58: i16 = 0;
    let mut explicit_attributes: Option<u16> = None;
    let mut sprite: Option<SpriteDecl> = None;
    let mut graphics: Vec<SpriteDecl> = Vec::new();
    let mut tiles: Vec<TileDecl> = Vec::new();

    struct ParamSlot {
        span: Span,
        ty: ParamType,
        default: Option<ParamValue>,
    }
    let mut declared_params: BTreeMap<String, ParamSlot> = BTreeMap::new();
    let mut param_order: Vec<String> = Vec::new();
    let mut param_assigns: BTreeMap<String, (Span, ParamValue)> = BTreeMap::new();

    let mut raw_states: BTreeMap<String, StateDecl> = BTreeMap::new();
    let mut state_order: Vec<String> = Vec::new();

    let mut raw_sounds: BTreeMap<String, SoundDecl> = BTreeMap::new();
    let mut sound_order: Vec<String> = Vec::new();

    let mut raw_slots: BTreeMap<String, SlotDecl> = BTreeMap::new();
    let mut slot_order: Vec<String> = Vec::new();

    let mut strings: BTreeMap<u16, StringsDecl> = BTreeMap::new();
    let mut string_names: BTreeMap<String, u16> = BTreeMap::new();

    let mut init_blocks: Vec<OwnedStmtBlock> = Vec::new();
    let mut init_pinned_id: Option<u16> = None;
    let mut main_block: Option<OwnedStmtBlock> = None;
    let mut main_pinned_id: Option<u16> = None;
    let mut load_blocks: Vec<OwnedStmtBlock> = Vec::new();
    let mut load_pinned_id: Option<u16> = None;
    let mut reset_blocks: Vec<OwnedStmtBlock> = Vec::new();
    let mut reset_pinned_id: Option<u16> = None;
    let mut cleanup_blocks: Vec<OwnedStmtBlock> = Vec::new();
    let mut cleanup_pinned_id: Option<u16> = None;

    let mut local_enums_raw: BTreeMap<String, (String, EnumDecl)> = BTreeMap::new();
    let mut local_bitflags_raw: BTreeMap<String, (String, BitflagsDecl)> = BTreeMap::new();
    let mut state_owners: BTreeMap<String, String> = BTreeMap::new();

    let mut functionals: Vec<OwnedFunctional> = Vec::new();
    let mut interactions: Vec<OwnedInteraction> = Vec::new();
    let mut local_fns: BTreeMap<String, (String, FnDecl)> = BTreeMap::new();
    let mut local_extern_mods: BTreeMap<String, (String, ExternModDecl)> = BTreeMap::new();
    let mut local_extern_bcons: BTreeMap<String, (String, ExternBconDecl)> = BTreeMap::new();
    let mut local_consts: BTreeMap<String, i64> = BTreeMap::new();
    let mut bcon_decls: Vec<BconTableDecl> = Vec::new();
    let mut tree_members: Vec<OwnedTree> = Vec::new();
    // Every module that contributes members (templates + the object): helper-fn lookups from any
    // of their code must not be shadowed by an (inherited) tree ident.
    let mut member_modules: BTreeSet<String> = BTreeSet::new();

    let mut apply_members = |owner_mod: &str,
                             members: &[ObjectMember],
                             is_override: bool|
     -> Result<()> {
        member_modules.insert(owner_mod.to_owned());
        let mut seen_sounds_here = BTreeSet::new();
        let mut seen_slots_here = BTreeSet::new();
        let mut seen_functionals_here = BTreeSet::new();
        let mut seen_interactions_here = BTreeSet::new();
        let mut seen_fns_here = BTreeSet::new();
        let mut seen_types_here = BTreeSet::new();
        let mut seen_externs_here = BTreeSet::new();
        let mut seen_bcons_here = BTreeSet::new();
        let mut seen_trees_here = BTreeSet::new();
        let mut seen_consts_here = BTreeSet::new();
        for member in members {
            match member {
                ObjectMember::Const(c) => {
                    if !seen_consts_here.insert(c.name.clone()) {
                        return Err(Error::at(c.span, format!("duplicate const '{}'", c.name)));
                    }
                    local_consts.insert(c.name.clone(), c.value);
                }
                ObjectMember::Meta(MetaField { span, kind }) => match kind {
                    MetaFieldKind::Guid(v) => guid = Some(*v),
                    MetaFieldKind::Price(v) => price = *v,
                    MetaFieldKind::Description(s) => description = s.clone(),
                    MetaFieldKind::Category(v) => catalog_sort_flags = *v,
                    MetaFieldKind::ObjectType(v) => object_type = *v,
                    MetaFieldKind::ObjdId(v) => objd_id = *v,
                    MetaFieldKind::TtabId(v) => ttab_id = *v,
                    MetaFieldKind::SlotId(v) => slot_id = *v,
                    MetaFieldKind::CtssId(v) => ctss_id = *v,
                    MetaFieldKind::DgrpId(v) => dgrp_id = *v,
                    MetaFieldKind::Spr2Id(v) => spr2_id = *v,
                    MetaFieldKind::PaltId(v) => palt_id = *v,
                    MetaFieldKind::MasterId(v) => master_id = Some(*v),
                    MetaFieldKind::InteractionGroup(v) => interaction_group = *v,
                    MetaFieldKind::Data58(v) => data_58 = *v,
                    MetaFieldKind::Attributes(v) => explicit_attributes = Some(*v),
                    MetaFieldKind::Sprite(sp) => sprite = Some(sp.clone()),
                    MetaFieldKind::Graphics(list) => {
                        if list.is_empty() {
                            return Err(Error::at(*span, "'graphics' list cannot be empty"));
                        }
                        graphics = list.clone();
                    }
                    MetaFieldKind::Tiles(t) => tiles = t.clone(),
                },
                ObjectMember::Param(p) => {
                    if declared_params.contains_key(&p.name) {
                        return Err(Error::at(
                            p.span,
                            format!("duplicate parameter declaration '{}'", p.name),
                        ));
                    }
                    param_order.push(p.name.clone());
                    declared_params.insert(
                        p.name.clone(),
                        ParamSlot {
                            span: p.span,
                            ty: p.ty,
                            default: p.default,
                        },
                    );
                }
                ObjectMember::ParamAssign(pa) => {
                    if !is_override {
                        return Err(Error::at(
                            pa.span,
                            format!(
                                "template cannot assign parameter '{}' directly; declare 'param {}: ... = ...;' instead",
                                pa.name, pa.name
                            ),
                        ));
                    }
                    if param_assigns.contains_key(&pa.name) {
                        return Err(Error::at(
                            pa.span,
                            format!("duplicate parameter assignment '{}'", pa.name),
                        ));
                    }
                    param_assigns.insert(pa.name.clone(), (pa.span, pa.value));
                }
                ObjectMember::State(s) => {
                    if matches!(
                        s.name.as_str(),
                        "id" | "room"
                            | "graphic"
                            | "prep_score"
                            | "cook_score"
                            | "in_use"
                            | "allowed_heights"
                            | "placement"
                            | "attr"
                            | "data"
                            | "parts"
                    ) {
                        return Err(Error::at(
                            s.span,
                            format!(
                                "state name '{}' conflicts with built-in object property 'self.{}'",
                                s.name, s.name
                            ),
                        ));
                    }
                    if raw_states.contains_key(&s.name) {
                        return Err(Error::at(
                            s.span,
                            format!("duplicate state declaration '{}'", s.name),
                        ));
                    }
                    state_order.push(s.name.clone());
                    state_owners.insert(s.name.clone(), owner_mod.to_owned());
                    raw_states.insert(s.name.clone(), s.clone());
                }
                ObjectMember::Sound(s) => {
                    if !seen_sounds_here.insert(s.name.clone()) {
                        return Err(Error::at(
                            s.span,
                            format!("duplicate sound declaration '{}'", s.name),
                        ));
                    }
                    if let Some(existing) = raw_sounds.get_mut(&s.name) {
                        if !is_override {
                            return Err(Error::at(
                                s.span,
                                format!("duplicate sound declaration '{}'", s.name),
                            ));
                        }
                        *existing = s.clone();
                    } else {
                        sound_order.push(s.name.clone());
                        raw_sounds.insert(s.name.clone(), s.clone());
                    }
                }
                ObjectMember::Slot(s) => {
                    if !seen_slots_here.insert(s.name.clone()) {
                        return Err(Error::at(
                            s.span,
                            format!("duplicate slot declaration '{}'", s.name),
                        ));
                    }
                    if let Some(existing) = raw_slots.get_mut(&s.name) {
                        if !is_override {
                            return Err(Error::at(
                                s.span,
                                format!("duplicate slot declaration '{}'", s.name),
                            ));
                        }
                        *existing = s.clone();
                    } else {
                        slot_order.push(s.name.clone());
                        raw_slots.insert(s.name.clone(), s.clone());
                    }
                }
                ObjectMember::Strings(st) => {
                    if strings.contains_key(&st.id) {
                        return Err(Error::at(
                            st.span,
                            format!("duplicate strings table id {}", st.id),
                        ));
                    }
                    for (espan, ename, _) in &st.entries {
                        if string_names.insert(ename.clone(), st.id).is_some() {
                            return Err(Error::at(
                                *espan,
                                format!("duplicate string entry name '{ename}'"),
                            ));
                        }
                    }
                    strings.insert(st.id, st.clone());
                }
                ObjectMember::Enum(e) => {
                    if !seen_types_here.insert(e.name.clone())
                        || local_enums_raw.contains_key(&e.name)
                        || local_bitflags_raw.contains_key(&e.name)
                    {
                        return Err(Error::at(
                            e.span,
                            format!("duplicate local type declaration '{}'", e.name),
                        ));
                    }
                    local_enums_raw.insert(e.name.clone(), (owner_mod.to_owned(), e.clone()));
                }
                ObjectMember::Bitflags(b) => {
                    if !seen_types_here.insert(b.name.clone())
                        || local_enums_raw.contains_key(&b.name)
                        || local_bitflags_raw.contains_key(&b.name)
                    {
                        return Err(Error::at(
                            b.span,
                            format!("duplicate local type declaration '{}'", b.name),
                        ));
                    }
                    local_bitflags_raw.insert(b.name.clone(), (owner_mod.to_owned(), b.clone()));
                }
                ObjectMember::Init(b) => {
                    if let Some(pid) = b.pinned_id {
                        init_pinned_id = Some(pid);
                    }
                    init_blocks.push(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Main(b) => {
                    if let Some(pid) = b.pinned_id {
                        main_pinned_id = Some(pid);
                    }
                    main_block = Some(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Load(b) => {
                    if let Some(pid) = b.pinned_id {
                        load_pinned_id = Some(pid);
                    }
                    load_blocks.push(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Reset(b) => {
                    if let Some(pid) = b.pinned_id {
                        reset_pinned_id = Some(pid);
                    }
                    reset_blocks.push(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Cleanup(b) => {
                    if let Some(pid) = b.pinned_id {
                        cleanup_pinned_id = Some(pid);
                    }
                    cleanup_blocks.push(OwnedStmtBlock {
                        owner_module: owner_mod.to_owned(),
                        stmts: b.body.clone(),
                    });
                }
                ObjectMember::Functional(f) => {
                    if !seen_functionals_here.insert(f.kind.clone()) {
                        return Err(Error::at(
                            f.span,
                            format!("duplicate functional '{}'", f.kind),
                        ));
                    }
                    if let Some(pos) = functionals.iter().position(|x| x.decl.kind == f.kind) {
                        if !is_override {
                            return Err(Error::at(
                                f.span,
                                format!("duplicate functional '{}'", f.kind),
                            ));
                        }
                        functionals[pos] = OwnedFunctional {
                            owner_module: owner_mod.to_owned(),
                            decl: f.clone(),
                        };
                    } else {
                        functionals.push(OwnedFunctional {
                            owner_module: owner_mod.to_owned(),
                            decl: f.clone(),
                        });
                    }
                }
                ObjectMember::Interaction(inter) => {
                    if !seen_interactions_here.insert(inter.label.clone()) {
                        return Err(Error::at(
                            inter.span,
                            format!("duplicate interaction '{}'", inter.label),
                        ));
                    }
                    if let Some(pos) = interactions
                        .iter()
                        .position(|x| x.decl.label == inter.label)
                    {
                        if !is_override {
                            return Err(Error::at(
                                inter.span,
                                format!("duplicate interaction '{}'", inter.label),
                            ));
                        }
                        interactions[pos] = OwnedInteraction {
                            owner_module: owner_mod.to_owned(),
                            decl: inter.clone(),
                        };
                    } else {
                        interactions.push(OwnedInteraction {
                            owner_module: owner_mod.to_owned(),
                            decl: inter.clone(),
                        });
                    }
                }
                ObjectMember::Fn(f) => {
                    if !seen_fns_here.insert(f.name.clone())
                        || (local_fns.contains_key(&f.name) && !is_override)
                    {
                        return Err(Error::at(
                            f.span,
                            format!("duplicate helper function '{}'", f.name),
                        ));
                    }
                    local_fns.insert(f.name.clone(), (owner_mod.to_owned(), f.clone()));
                }
                ObjectMember::ExternMod(m) => {
                    if !seen_externs_here.insert(m.name.clone())
                        || (local_extern_mods.contains_key(&m.name) && !is_override)
                        || local_extern_bcons.contains_key(&m.name)
                    {
                        return Err(Error::at(
                            m.span,
                            format!("duplicate extern block '{}'", m.name),
                        ));
                    }
                    local_extern_mods.insert(m.name.clone(), (owner_mod.to_owned(), m.clone()));
                }
                ObjectMember::ExternBcon(b) => {
                    if !seen_externs_here.insert(b.name.clone())
                        || (local_extern_bcons.contains_key(&b.name) && !is_override)
                        || local_extern_mods.contains_key(&b.name)
                    {
                        return Err(Error::at(
                            b.span,
                            format!("duplicate extern block '{}'", b.name),
                        ));
                    }
                    local_extern_bcons.insert(b.name.clone(), (owner_mod.to_owned(), b.clone()));
                }
                ObjectMember::Bcon(t) => {
                    if !seen_bcons_here.insert(t.id) {
                        return Err(Error::at(
                            t.span,
                            format!("duplicate BCON table id {}", t.id),
                        ));
                    }
                    if let Some(pos) = bcon_decls.iter().position(|d| d.id == t.id) {
                        if !is_override {
                            return Err(Error::at(
                                t.span,
                                format!("duplicate BCON table id {}", t.id),
                            ));
                        }
                        bcon_decls[pos] = t.clone();
                    } else {
                        bcon_decls.push(t.clone());
                    }
                }
                ObjectMember::Tree(t) => {
                    if !seen_trees_here.insert(t.ident.clone()) {
                        return Err(Error::at(t.span, format!("duplicate tree '{}'", t.ident)));
                    }
                    let owned = OwnedTree {
                        owner_module: owner_mod.to_owned(),
                        decl: t.clone(),
                    };
                    if let Some(pos) = tree_members.iter().position(|x| x.decl.ident == t.ident) {
                        if !is_override {
                            return Err(Error::at(t.span, format!("duplicate tree '{}'", t.ident)));
                        }
                        tree_members[pos] = owned;
                    } else {
                        tree_members.push(owned);
                    }
                }
            }
        }
        Ok(())
    };

    if let Some((tpl_mod, tpl_decl)) = &template_opt {
        apply_members(tpl_mod, &tpl_decl.members, false)?;
    }
    apply_members(&resolved.module_id, &obj.members, true)?;

    // 3. Validate GUID & resolve all parameters (checking unknown, type mismatch, and missing required).
    let guid = guid.ok_or_else(|| {
        Error::at(
            obj.span,
            format!(
                "object '{}' is missing required 'guid: 0x...;' field",
                obj.name
            ),
        )
    })?;

    for (aname, (aspan, aval)) in &param_assigns {
        let Some(slot) = declared_params.get(aname) else {
            return Err(Error::at(
                *aspan,
                format!("unknown parameter '{aname}' in object '{}'", obj.name),
            ));
        };
        match (slot.ty, aval) {
            (ParamType::I16, ParamValue::I16(_)) | (ParamType::Bool, ParamValue::Bool(_)) => {}
            (ParamType::I16, ParamValue::Bool(_)) => {
                return Err(Error::at(
                    *aspan,
                    format!("parameter '{aname}' expects i16, found bool"),
                ));
            }
            (ParamType::Bool, ParamValue::I16(_)) => {
                return Err(Error::at(
                    *aspan,
                    format!("parameter '{aname}' expects bool, found integer"),
                ));
            }
        }
    }

    let mut i16_params = BTreeMap::new();
    let mut i16_param_order = Vec::new();
    let mut bool_params = BTreeMap::new();
    for pname in &param_order {
        let slot = &declared_params[pname];
        let val = match param_assigns.get(pname) {
            Some((_, v)) => *v,
            None => slot.default.ok_or_else(|| {
                Error::at(
                    obj.span,
                    format!(
                        "object '{}' is missing required parameter '{}' (declared at {})",
                        obj.name, pname, slot.span
                    ),
                )
            })?,
        };
        match val {
            ParamValue::I16(v) => {
                i16_param_order.push(pname.clone());
                i16_params.insert(pname.clone(), v);
            }
            ParamValue::Bool(b) => {
                bool_params.insert(pname.clone(), b);
            }
        }
    }
    if i16_param_order.len() > 128 {
        return Err(Error::at(
            obj.span,
            format!(
                "object '{}' declares {} i16 params, exceeding BCON 4096 limit of 128",
                obj.name,
                i16_param_order.len()
            ),
        ));
    }

    // 3b. Resolve all enums and bitflags (built-in + module-level + object/template-local).
    let mut all_enums: BTreeMap<String, ResolvedEnum> = BTreeMap::new();
    let mut all_bitflags: BTreeMap<String, ResolvedBitflags> = builtin_bitflags();
    for scope in resolved.module_scopes.values() {
        for (def_mod, edecl) in scope.enums.values() {
            let key = format!("enum::{def_mod}::{}", edecl.name);
            all_enums
                .entry(key.clone())
                .or_insert_with(|| resolve_enum_decl(key, edecl));
        }
        for (def_mod, bdecl) in scope.bitflags.values() {
            let key = format!("bitflags::{def_mod}::{}", bdecl.name);
            all_bitflags
                .entry(key.clone())
                .or_insert_with(|| resolve_bitflags_decl(key, bdecl));
        }
    }
    let mut local_enums: BTreeMap<String, String> = BTreeMap::new();
    for (ename, (owner_mod, edecl)) in &local_enums_raw {
        let key = format!("local_enum::{owner_mod}::{slug}::{}", edecl.name);
        all_enums.insert(key.clone(), resolve_enum_decl(key.clone(), edecl));
        local_enums.insert(ename.clone(), key);
    }
    let mut local_bitflags: BTreeMap<String, String> = BTreeMap::new();
    for (bname, (owner_mod, bdecl)) in &local_bitflags_raw {
        let key = format!("local_bitflags::{owner_mod}::{slug}::{}", bdecl.name);
        all_bitflags.insert(key.clone(), resolve_bitflags_decl(key.clone(), bdecl));
        local_bitflags.insert(bname.clone(), key);
    }

    // 3c. Resolve imported object schemas across all loaded modules.
    let mut object_schemas: BTreeMap<String, ResolvedObjectSchema> = BTreeMap::new();
    for scope in resolved.module_scopes.values() {
        for (def_mod, odecl) in scope.objects.values() {
            let type_name = object_type_ident(&odecl.name);
            let key = format!("object::{def_mod}::{type_name}");
            if let std::collections::btree_map::Entry::Vacant(slot) = object_schemas.entry(key) {
                let schema = build_object_schema(
                    resolved,
                    def_mod,
                    odecl,
                    slot.key(),
                    &type_name,
                    &mut all_enums,
                    &mut all_bitflags,
                )?;
                slot.insert(schema);
            }
        }
    }

    let self_type_name = object_type_ident(&obj.name);
    let self_object_key = format!("object::{}::{self_type_name}", resolved.module_id);

    // Helper closure to resolve a StateType / V2Type inside this object.
    let resolve_type_here = |ty: &V2Type, owner_mod: &str, span: Span| -> Result<ValueType> {
        match ty {
            V2Type::I16 => Ok(ValueType::I16),
            V2Type::Bool => Ok(ValueType::Bool),
            V2Type::ObjectId => Ok(ValueType::ObjectId),
            V2Type::Named(tname) => {
                if let Some(k) = local_enums.get(tname) {
                    return Ok(ValueType::Enum(k.clone()));
                }
                if let Some(k) = local_bitflags.get(tname) {
                    return Ok(ValueType::Bitflags(k.clone()));
                }
                match tname.as_str() {
                    "Height" => return Ok(ValueType::Bitflags("builtin::Height".to_owned())),
                    "Placement" => return Ok(ValueType::Bitflags("builtin::Placement".to_owned())),
                    "Self" => return Ok(ValueType::ObjectRef(self_object_key.clone())),
                    _ => {}
                }
                if let Some(sc) = resolved.module_scopes.get(owner_mod) {
                    if let Some((def_mod, ed)) = sc.enums.get(tname) {
                        return Ok(ValueType::Enum(format!("enum::{def_mod}::{}", ed.name)));
                    }
                    if let Some((def_mod, bd)) = sc.bitflags.get(tname) {
                        return Ok(ValueType::Bitflags(format!(
                            "bitflags::{def_mod}::{}",
                            bd.name
                        )));
                    }
                    if let Some((def_mod, od)) = sc.objects.get(tname) {
                        return Ok(ValueType::ObjectRef(format!(
                            "object::{def_mod}::{}",
                            object_type_ident(&od.name)
                        )));
                    }
                }
                Err(Error::at(span, format!("unknown type '{tname}'")))
            }
            other => Err(Error::at(
                span,
                format!("unsupported type '{}'", other.display_name()),
            )),
        }
    };

    // 4. Allocate state attribute indices (explicit `@ slot` states first, then the lowest free
    // slot in declaration order; without explicit slots this is exactly `0..N`).
    let state_slots = allocate_state_slots(
        state_order
            .iter()
            .map(|n| (n.as_str(), raw_states[n].slot, raw_states[n].span)),
    )?;
    let mut states = BTreeMap::new();
    let mut self_schema_states = BTreeMap::new();
    for (sname, &idx) in state_order.iter().zip(&state_slots) {
        let s = &raw_states[sname];
        let owner_mod = state_owners
            .get(sname)
            .map_or(resolved.module_id.as_str(), String::as_str);
        let resolved_ty = match &s.ty {
            StateType::Bool => ValueType::Bool,
            StateType::I16 => ValueType::I16,
            StateType::ObjectId => ValueType::ObjectId,
            StateType::Named(tname) => {
                resolve_type_here(&V2Type::Named(tname.clone()), owner_mod, s.span)?
            }
        };
        self_schema_states.insert(sname.clone(), (idx, resolved_ty.clone()));
        states.insert(
            sname.clone(),
            ResolvedState {
                attr_index: idx,
                ty: resolved_ty,
                init: s.init.clone(),
                span: s.span,
                skip_zero_init: s.slot.is_some() && s.init.is_none(),
            },
        );
    }

    // Register self schema (with its resolved states, functionals, and tile GUIDs) so `self.parts()`
    // and self-typed references resolve named states with full type safety!
    let mut self_schema_funcs = BTreeMap::new();
    for f in &functionals {
        let mut p_tys = Vec::with_capacity(f.decl.params.len());
        for fp in &f.decl.params {
            p_tys.push(resolve_type_here(&fp.ty, &f.owner_module, fp.span)?);
        }
        let r_ty = match &f.decl.return_type {
            Some(rt) => Some(resolve_type_here(rt, &f.owner_module, f.decl.span)?),
            None => None,
        };
        self_schema_funcs.insert(
            f.decl.kind.clone(),
            ResolvedFunctionalSig {
                params: p_tys,
                return_type: r_ty,
                guard: summarize_functional_guard(f.decl.guard.as_ref(), &self_schema_states),
            },
        );
    }
    let self_lead_guid = select_lead_tile_guid(&tiles, guid);
    let self_tile_guids = if tiles.is_empty() {
        vec![guid]
    } else {
        tiles.iter().map(|t| t.guid).collect()
    };
    let self_schema = ResolvedObjectSchema {
        canonical_key: self_object_key.clone(),
        display_name: self_type_name,
        master_guid: guid,
        lead_guid: self_lead_guid,
        tile_guids: self_tile_guids,
        states: self_schema_states,
        functionals: self_schema_funcs,
    };
    object_schemas.insert(self_object_key.clone(), self_schema);

    // 5. Assign FWAV sound IDs (`1..`, honoring explicit IDs).
    let mut used_sound_ids: BTreeSet<u16> = BTreeSet::new();
    for sname in &sound_order {
        if let Some(eid) = raw_sounds[sname].explicit_id {
            if eid == 0 {
                return Err(Error::at(
                    raw_sounds[sname].span,
                    format!("sound '{sname}' id must be >= 1"),
                ));
            }
            if !used_sound_ids.insert(eid) {
                return Err(Error::at(
                    raw_sounds[sname].span,
                    format!("duplicate sound id {eid}"),
                ));
            }
        }
    }
    let mut next_sound_id = 1u16;
    let mut sounds = BTreeMap::new();
    let mut sound_specs = Vec::with_capacity(sound_order.len());
    for sname in &sound_order {
        let s = &raw_sounds[sname];
        let id = if let Some(eid) = s.explicit_id {
            eid
        } else {
            while used_sound_ids.contains(&next_sound_id) {
                next_sound_id = next_sound_id
                    .checked_add(1)
                    .ok_or_else(|| Error::at(s.span, "FWAV sound id overflow"))?;
            }
            let assigned = next_sound_id;
            used_sound_ids.insert(assigned);
            next_sound_id = next_sound_id.saturating_add(1);
            assigned
        };
        let wav_name = s
            .wav_name
            .clone()
            .unwrap_or_else(|| format!("{}_{}", slug.replace('-', "_"), sname));
        crate::manifest::validate_relative_filename(&wav_name, "sound wav name")
            .map_err(|e| Error::at(s.span, e.message))?;
        sounds.insert(sname.clone(), ResolvedSound { id });
        sound_specs.push(SoundSpec {
            id,
            name: wav_name,
            preset: s.preset,
        });
    }

    // 6. Build slot table (routing_ordinal counts ONLY kind == 3 routing slots, matching sims-vm).
    let mut slots = BTreeMap::new();
    let mut slot_specs = Vec::with_capacity(slot_order.len());
    let mut routing_ordinal = 0u16;
    for sl_name in &slot_order {
        let sl = raw_slots[sl_name].clone();
        let routing_idx = if sl.kind == 3 {
            let idx = routing_ordinal;
            routing_ordinal = routing_ordinal
                .checked_add(1)
                .ok_or_else(|| Error::at(sl.span, "routing slot count overflow"))?;
            Some(idx)
        } else {
            None
        };
        slot_specs.push(SlotSpec {
            kind: sl.kind,
            offset: sl.offset,
            standing: sl.standing,
            sitting: sl.sitting,
            ground: sl.ground,
            flags: sl.flags,
            snap_target: sl.snap_target,
            proximity: sl.proximity,
            max_size: sl.max_size,
            gradient: sl.gradient,
            height: sl.height,
            facing: sl.facing,
            resolution: sl.resolution,
        });
        slots.insert(sl_name.clone(), (routing_idx, sl));
    }

    let bcon_tables = resolve_bcon_tables(&bcon_decls, &i16_param_order, &local_extern_bcons)?;

    let dialog_strings = RefCell::new(DialogStringTable::from_existing(strings.get(&301)));
    let env = ObjectEnv {
        resolved,
        name: obj.name.clone(),
        i16_params,
        i16_param_order,
        bool_params,
        states,
        state_order,
        sounds,
        slots,
        strings,
        string_names,
        dialog_strings,
        local_fns,
        local_enums,
        local_bitflags,
        all_enums,
        all_bitflags,
        object_schemas,
        self_object_key,
        local_extern_mods,
        local_extern_bcons,
        local_consts,
        bcon_tables,
        trees: BTreeMap::new(),
    };
    let env = {
        let mut env = env;
        let tree_sigs = resolve_tree_sigs(&env, &tree_members, &member_modules)?;
        env.trees = tree_sigs;
        env
    };

    // 7. Allocate deterministic private BHAV IDs (`4096..=8191`), respecting `@id` / `@guard_id`.
    let mut pinned_ids = BTreeSet::new();
    let mut register_pinned = |id: u16, span: Span| -> Result<()> {
        if !(4096..=8191).contains(&id) {
            return Err(Error::at(
                span,
                format!("@id({id}) must be in private BHAV range 4096..=8191"),
            ));
        }
        if !pinned_ids.insert(id) {
            return Err(Error::at(span, format!("duplicate pinned BHAV @id({id})")));
        }
        Ok(())
    };
    if let Some(id) = init_pinned_id {
        register_pinned(id, obj.span)?;
    }
    if let Some(id) = main_pinned_id {
        register_pinned(id, obj.span)?;
    }
    if let Some(id) = load_pinned_id {
        register_pinned(id, obj.span)?;
    }
    if let Some(id) = reset_pinned_id {
        register_pinned(id, obj.span)?;
    }
    if let Some(id) = cleanup_pinned_id {
        register_pinned(id, obj.span)?;
    }
    for f in &functionals {
        if let Some(id) = f.decl.pinned_guard_id {
            register_pinned(id, f.decl.span)?;
        }
        if let Some(id) = f.decl.pinned_id {
            register_pinned(id, f.decl.span)?;
        }
    }
    for inter in &interactions {
        if let Some(id) = inter.decl.pinned_guard_id {
            register_pinned(id, inter.decl.span)?;
        }
        if let Some(id) = inter.decl.pinned_id {
            register_pinned(id, inter.decl.span)?;
        }
    }
    for t in &tree_members {
        if let Some(id) = t.decl.pinned_id {
            register_pinned(id, t.decl.span)?;
        }
    }

    let mut used_tree_ids = pinned_ids;
    let mut next_tree_id = 4096u16;
    let mut alloc_tree_id = |pinned: Option<u16>,
                             preferred: Option<u16>,
                             span: Span|
     -> Result<u16> {
        if let Some(id) = pinned {
            return Ok(id);
        }
        if let Some(pref) = preferred
            && !used_tree_ids.contains(&pref)
        {
            used_tree_ids.insert(pref);
            return Ok(pref);
        }
        while used_tree_ids.contains(&next_tree_id) {
            next_tree_id = next_tree_id
                .checked_add(1)
                .filter(|&id| id <= 8191)
                .ok_or_else(|| Error::at(span, "exhausted private BHAV ID range 4096..=8191"))?;
        }
        let id = next_tree_id;
        used_tree_ids.insert(id);
        next_tree_id += 1;
        Ok(id)
    };

    let init_id = alloc_tree_id(init_pinned_id, Some(4096), obj.span)?;
    let main_id = alloc_tree_id(main_pinned_id, Some(4097), obj.span)?;
    // Standalone `tree` IDs are allocated up front so bodies (and siblings) can `call` them.
    let mut tree_ids: BTreeMap<String, u16> = BTreeMap::new();
    for t in &tree_members {
        let id = alloc_tree_id(t.decl.pinned_id, None, t.decl.span)?;
        tree_ids.insert(t.decl.ident.clone(), id);
    }
    let env = {
        let mut env = env;
        for (ident, sig) in &mut env.trees {
            sig.id = tree_ids[ident];
        }
        env
    };

    // 8. Emit all trees into `trees_air` first so inline dialog strings interned into
    // `env.dialog_strings` are complete before we emit the STR# 301 table at the top!
    let mut trees_air = String::new();

    // 8c. Init tree.
    {
        let mut ctx = TreeLowerer::new(&env, TreeContext::ObjectSelf, &resolved.module_id, false);
        // Set functional score data words first (e.g., `my.prep_score := ...`).
        for func in &functionals {
            if let Some(score_expr) = &func.decl.score {
                let (_, score_word) = functional_kind_info(&func.decl.kind, func.decl.kind_span)?;
                let Some(word_idx) = score_word else {
                    return Err(Error::at(
                        func.decl.span,
                        format!(
                            "functional '{}' uses a fixed score in Sims 1 and cannot set a score data word",
                            func.decl.kind
                        ),
                    ));
                };
                let rhs = match score_expr {
                    ConstIntExpr::Literal(v) => v.to_string(),
                    ConstIntExpr::Param { name, negate } => {
                        let resolved_v = env.resolve_const_int(score_expr, func.decl.span)?;
                        if *negate {
                            resolved_v.to_string()
                        } else {
                            format!("bcon.{name}")
                        }
                    }
                };
                ctx.line(format!("my.data[{word_idx}] := {rhs};"));
            }
        }
        // Auto-initialize all declared states in declaration order.
        for sname in &env.state_order {
            let st = &env.states[sname];
            if st.skip_zero_init {
                // `match` lowers the last enum arm as a plain `else`, so an uninitialized explicit
                // slot (0, or whatever a previous owner left) would silently dispatch to it.
                if let ValueType::Enum(ref key) = st.ty
                    && let Some(e) = env.all_enums.get(key)
                {
                    return Err(Error::at(
                        st.span,
                        format!(
                            "state '{sname}' @ {} of enum type '{}' requires an explicit initializer (init leaves an explicit slot without one untouched, so it may hold an undeclared value)",
                            st.attr_index, e.display_name
                        ),
                    ));
                }
                continue;
            }
            let owner_mod = state_owners
                .get(sname)
                .map_or(resolved.module_id.as_str(), String::as_str);
            let rhs = match &st.init {
                None => {
                    if let ValueType::Enum(ref key) = st.ty
                        && let Some(e) = env.all_enums.get(key)
                        && !e.variants.values().any(|&v| v == 0)
                    {
                        return Err(Error::at(
                            st.span,
                            format!(
                                "state '{sname}' of enum type '{}' requires an explicit initializer (0 is not a declared variant)",
                                e.display_name
                            ),
                        ));
                    }
                    "0".to_owned()
                }
                Some(init_val) => {
                    resolve_state_init_air(&env, owner_mod, sname, &st.ty, init_val, st.span)?
                }
            };
            ctx.line(format!("my.attr[{}] := {rhs};", st.attr_index));
        }
        // Append template + object `init { ... }` hooks, each in its own lexical/cleanup scope.
        for blk in &init_blocks {
            ctx.with_owner_module(&blk.owner_module, |c| {
                c.push_scope(ScopeFrame::block());
                let r = c.lower_stmts(&blk.stmts);
                let p = if stmts_always_return(&blk.stmts, &env, &blk.owner_module, true) {
                    c.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    c.pop_scope_and_run_cleanups()
                };
                r.and(p)
            })?;
        }
        if !init_blocks
            .last()
            .is_some_and(|b| stmts_always_return(&b.stmts, &env, &b.owner_module, true))
        {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        let _ = writeln!(trees_air, "tree {init_id} \"Init\" {{");
        trees_air.push_str(&ctx.finish());
        trees_air.push_str("}\n\n");
    }

    // 8d. Main tree.
    {
        let mut ctx = TreeLowerer::new(&env, TreeContext::ObjectSelf, &resolved.module_id, false);
        if let Some(blk) = &main_block {
            ctx.with_owner_module(&blk.owner_module, |c| {
                c.push_scope(ScopeFrame::block());
                let r = c.lower_stmts(&blk.stmts);
                let p = if stmts_always_return(&blk.stmts, &env, &blk.owner_module, true) {
                    c.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    c.pop_scope_and_run_cleanups()
                };
                r.and(p)
            })?;
            if !stmts_always_return(&blk.stmts, &env, &blk.owner_module, true) {
                ctx.emit_all_cleanups()?;
                ctx.line("return true;".to_owned());
            }
        } else {
            ctx.line("return true;".to_owned());
        }
        let _ = writeln!(trees_air, "tree {main_id} \"Main\" {{");
        trees_air.push_str(&ctx.finish());
        trees_air.push_str("}\n\n");
    }

    // 8d2. Optional OBJf lifecycle hooks: `load` (OBJf 2), `reset` (OBJf 3), `cleanup` (OBJf 4).
    let mut callback_specs = Vec::with_capacity(functionals.len() + 3);
    for (cb_index, tree_label, pinned_id, blocks) in [
        (2usize, "Load", load_pinned_id, &load_blocks),
        (3usize, "Reset", reset_pinned_id, &reset_blocks),
        (4usize, "Cleanup", cleanup_pinned_id, &cleanup_blocks),
    ] {
        if blocks.is_empty() {
            continue;
        }
        let tid = alloc_tree_id(pinned_id, None, obj.span)?;
        let mut ctx = TreeLowerer::new(&env, TreeContext::ObjectSelf, &resolved.module_id, false);
        for blk in blocks {
            ctx.with_owner_module(&blk.owner_module, |c| {
                c.push_scope(ScopeFrame::block());
                let r = c.lower_stmts(&blk.stmts);
                let p = if stmts_always_return(&blk.stmts, &env, &blk.owner_module, true) {
                    c.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    c.pop_scope_and_run_cleanups()
                };
                r.and(p)
            })?;
        }
        if !blocks
            .last()
            .is_some_and(|b| stmts_always_return(&b.stmts, &env, &b.owner_module, true))
        {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        let _ = writeln!(trees_air, "tree {tid} \"{tree_label}\" {{");
        trees_air.push_str(&ctx.finish());
        trees_air.push_str("}\n\n");
        callback_specs.push(CallbackSpec {
            index: cb_index,
            action: Some(TreeRef::Id(tid)),
            condition: None,
        });
    }

    // 8e. Functional callback trees (OBJf).
    for func in &functionals {
        let (cb_index, _) = functional_kind_info(&func.decl.kind, func.decl.kind_span)?;
        let guard_id = if let Some(guard) = &func.decl.guard {
            let gid = alloc_tree_id(func.decl.pinned_guard_id, None, guard.span)?;
            let label = format!("Can_{}", capitalize(&func.decl.kind));
            emit_guard_tree(
                &mut trees_air,
                &env,
                &func.owner_module,
                gid,
                &label,
                guard,
                &func.decl.params,
            )?;
            Some(gid)
        } else {
            None
        };
        let action_id = alloc_tree_id(func.decl.pinned_id, None, func.decl.span)?;
        let action_label = format!("Functional_{}", capitalize(&func.decl.kind));
        let func_ret_ty = match &func.decl.return_type {
            Some(rt) => {
                Some(env.resolve_value_type(rt, &func.owner_module, true, func.decl.span)?)
            }
            None => None,
        };
        if func_ret_ty.is_some()
            && !stmts_always_return(&func.decl.body, &env, &func.owner_module, true)
        {
            return Err(Error::at(
                func.decl.span,
                format!(
                    "functional '{}' declares a return type and must return a value on all control-flow paths",
                    func.decl.kind
                ),
            ));
        }
        let needs_self_save = stmts_may_clobber_stack_obj(
            &func.decl.body,
            &env,
            &func.owner_module,
            true,
            Some(&func.decl.sim_binding),
            0,
        );
        let mut ctx = TreeLowerer::new(
            &env,
            TreeContext::SimOnObject {
                sim_name: func.decl.sim_binding.clone(),
            },
            &func.owner_module,
            needs_self_save,
        );
        ctx.value_return = func_ret_ty.map(|ty| ValueReturnAbi {
            ty,
            temp: 0,
            allow_none: true,
            in_place: false,
            what: "functional",
        });
        if needs_self_save {
            ctx.line("let __self_obj = stack_obj;".to_owned());
        }
        for (pidx, fp) in func.decl.params.iter().enumerate() {
            ctx.bind_func_param(&fp.name, pidx, &fp.ty, fp.span)?;
        }
        ctx.lower_stmts(&func.decl.body)?;
        if !stmts_always_return(&func.decl.body, &env, &func.owner_module, true) {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        if func.decl.params.is_empty() {
            let _ = writeln!(trees_air, "tree {action_id} \"{action_label}\" {{");
        } else {
            let _ = writeln!(
                trees_air,
                "tree {action_id} \"{action_label}\" (args: {}) {{",
                func.decl.params.len()
            );
        }
        trees_air.push_str(&ctx.finish());
        trees_air.push_str("}\n\n");

        callback_specs.push(CallbackSpec {
            index: cb_index,
            action: Some(TreeRef::Id(action_id)),
            condition: guard_id.map(TreeRef::Id),
        });
    }

    // 8f. Interactions (guard + action trees + TTAB metadata).
    let mut interaction_specs = Vec::with_capacity(interactions.len());
    for (idx, inter) in interactions.iter().enumerate() {
        let base_ident = sanitize_tree_ident(&inter.decl.label, idx);

        // Validate optional declarative `lock: self.<bool_state>` and desugar into guard + `with lock(...)`.
        let (effective_guard, effective_run_body) = if let Some((lspan, ref field_name)) =
            inter.decl.lock
        {
            let st = env.states.get(field_name).ok_or_else(|| {
                Error::at(
                    lspan,
                    format!(
                        "interaction '{}' lock 'self.{field_name}' references unknown state '{field_name}'",
                        inter.decl.label
                    ),
                )
            })?;
            if st.ty != ValueType::Bool {
                let actual_ty = env.display_value_type(&st.ty);
                return Err(Error::at(
                    lspan,
                    format!(
                        "interaction '{}' lock 'self.{field_name}' requires a bool state, found '{actual_ty}'",
                        inter.decl.label
                    ),
                ));
            }
            let lock_expr = V2Expr::Field {
                span: lspan,
                base: Box::new(V2Expr::Ident(lspan, "self".to_owned())),
                field: field_name.clone(),
            };
            let not_locked = V2Expr::Unary {
                span: lspan,
                op: V2UnaryOp::Not,
                inner: Box::new(lock_expr.clone()),
            };
            let composed_guard = match &inter.decl.guard {
                None => GuardDecl {
                    span: lspan,
                    sim_binding: inter.decl.sim_binding.clone(),
                    autonomous_binding: None,
                    kind: GuardBody::Expr(not_locked),
                },
                Some(existing) => match &existing.kind {
                    GuardBody::Expr(expr) => {
                        let combined = if expr_starts_with_not_self_field(expr, field_name) {
                            expr.clone()
                        } else {
                            V2Expr::Binary {
                                span: lspan,
                                lhs: Box::new(not_locked),
                                op: V2BinOp::And,
                                rhs: Box::new(expr.clone()),
                            }
                        };
                        GuardDecl {
                            span: existing.span,
                            sim_binding: existing.sim_binding.clone(),
                            autonomous_binding: existing.autonomous_binding.clone(),
                            kind: GuardBody::Expr(combined),
                        }
                    }
                    GuardBody::Block(stmts) => {
                        let mut body = Vec::with_capacity(stmts.len() + 1);
                        body.push(V2Stmt::If {
                            span: lspan,
                            cond: lock_expr.clone(),
                            then_body: vec![V2Stmt::Return {
                                span: lspan,
                                value: Some(V2Expr::Bool(lspan, false)),
                            }],
                            else_body: Vec::new(),
                        });
                        body.extend(stmts.iter().cloned());
                        GuardDecl {
                            span: existing.span,
                            sim_binding: existing.sim_binding.clone(),
                            autonomous_binding: existing.autonomous_binding.clone(),
                            kind: GuardBody::Block(body),
                        }
                    }
                },
            };
            let wrapped_body = vec![V2Stmt::WithLock {
                span: lspan,
                lock_expr,
                body: inter.decl.run_body.clone(),
            }];
            (Some(composed_guard), wrapped_body)
        } else {
            (inter.decl.guard.clone(), inter.decl.run_body.clone())
        };

        let guard_id = if let Some(guard) = &effective_guard {
            let gid = alloc_tree_id(inter.decl.pinned_guard_id, None, guard.span)?;
            let glabel = format!("Can_{base_ident}");
            emit_guard_tree(
                &mut trees_air,
                &env,
                &inter.owner_module,
                gid,
                &glabel,
                guard,
                &[],
            )?;
            Some(gid)
        } else {
            None
        };
        let action_id = alloc_tree_id(inter.decl.pinned_id, None, inter.decl.span)?;
        let needs_self_save = stmts_may_clobber_stack_obj(
            &effective_run_body,
            &env,
            &inter.owner_module,
            true,
            Some(&inter.decl.sim_binding),
            0,
        );
        let mut ctx = TreeLowerer::new(
            &env,
            TreeContext::SimOnObject {
                sim_name: inter.decl.sim_binding.clone(),
            },
            &inter.owner_module,
            needs_self_save,
        );
        if needs_self_save {
            ctx.line("let __self_obj = stack_obj;".to_owned());
        }
        ctx.lower_stmts(&effective_run_body)?;
        if !stmts_always_return(&effective_run_body, &env, &inter.owner_module, true) {
            ctx.emit_all_cleanups()?;
            ctx.line("return true;".to_owned());
        }
        let _ = writeln!(trees_air, "interaction {action_id} \"{base_ident}\" {{");
        trees_air.push_str(&ctx.finish());
        trees_air.push_str("}\n\n");

        let mut motives = Vec::with_capacity(inter.decl.advertises.len());
        for ad in &inter.decl.advertises {
            let delta = env.resolve_const_int(&ad.delta, ad.span)?;
            let minimum = env.resolve_const_int(&ad.minimum, ad.span)?;
            motives.push(MotiveAdSpec {
                motive_index: ad.motive_index,
                minimum,
                delta,
                personality: ad.personality,
            });
        }
        interaction_specs.push(InteractionSpec {
            name: inter.decl.label.clone(),
            action: TreeRef::Id(action_id),
            test: guard_id.map(TreeRef::Id),
            flags: inter.decl.flags,
            flags2: inter.decl.flags2,
            autonomy: inter.decl.autonomy,
            attenuation_code: inter.decl.attenuation_code,
            attenuation: inter.decl.attenuation,
            joining_index: inter.decl.joining_index,
            motives,
        });
    }

    // 8h. Standalone `tree` declarations (non-inlined BHAVs callable via `.air` `call`).
    for t in &tree_members {
        emit_standalone_tree(&mut trees_air, &env, t)?;
    }

    // 8g. Assemble final `.air` output: header + BCON 4096 + STR# tables (including auto-interned 301) + trees.
    let mut air = String::new();
    let _ = writeln!(
        air,
        "// Generated .air (Antics IR) from .antics v2 object \"{}\" (GUID {guid:#010x})",
        escape_str(&env.name)
    );
    if !env.state_order.is_empty() {
        air.push_str("// Attributes: ");
        let mut by_index: Vec<(usize, &String)> = env
            .state_order
            .iter()
            .map(|n| (env.states[n].attr_index, n))
            .collect();
        by_index.sort();
        for (i, (idx, sname)) in by_index.into_iter().enumerate() {
            if i > 0 {
                air.push_str(", ");
            }
            let _ = write!(air, "attr[{idx}] = {sname}");
        }
        air.push('\n');
    }
    air.push('\n');

    if !env.i16_param_order.is_empty() {
        let _ = writeln!(air, "bcon 4096 \"{}Tuning\" {{", camel_label(&env.name));
        for pname in &env.i16_param_order {
            let val = env.i16_params[pname];
            let _ = writeln!(air, "    {pname} = {val},");
        }
        air.push_str("}\n\n");
    }
    // Named private BCON tables: emitted in list form (entries are referenced numerically as
    // `bcon[(table << 7) | entry]`, so they never enter `.air`'s global BCON name namespace).
    let mut emitted_tables = BTreeSet::new();
    for t in env.bcon_tables.values() {
        if !emitted_tables.insert(t.id) {
            continue;
        }
        let vals: Vec<String> = t.values.iter().map(|(_, v)| v.to_string()).collect();
        let _ = writeln!(
            air,
            "bcon {} \"{}\" = [{}];\n",
            t.id,
            escape_str(&t.label),
            vals.join(", ")
        );
    }

    // Emit STR# tables in deterministic ID order, merging any auto-interned STR# 301 dialog strings.
    let dlg_table = env.dialog_strings.borrow();
    let mut emitted_301 = false;
    for (&str_id, st) in &env.strings {
        if str_id == 301 {
            emitted_301 = true;
            if !dlg_table.entries.is_empty() {
                let lbl = dlg_table
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("{}Strings_301", camel_label(&env.name)));
                let _ = writeln!(air, "str 301 \"{}\" {{", escape_str(&lbl));
                for (_, ename, text) in &dlg_table.entries {
                    let _ = writeln!(air, "    {ename} = \"{}\",", escape_str(text));
                }
                air.push_str("}\n\n");
            }
            continue;
        }
        let lbl = st
            .label
            .clone()
            .unwrap_or_else(|| format!("{}Strings_{}", camel_label(&env.name), st.id));
        let _ = writeln!(air, "str {} \"{}\" {{", st.id, escape_str(&lbl));
        for (_, ename, text) in &st.entries {
            let _ = writeln!(air, "    {ename} = \"{}\",", escape_str(text));
        }
        air.push_str("}\n\n");
    }
    if !emitted_301 && !dlg_table.entries.is_empty() {
        let lbl = dlg_table
            .label
            .clone()
            .unwrap_or_else(|| format!("{}Dialogs", camel_label(&env.name)));
        let _ = writeln!(air, "str 301 \"{}\" {{", escape_str(&lbl));
        for (_, ename, text) in &dlg_table.entries {
            let _ = writeln!(air, "    {ename} = \"{}\",", escape_str(text));
        }
        air.push_str("}\n\n");
    }
    drop(dlg_table);
    air.push_str(&trees_air);

    // 9. Build SpriteSpec + optional extra graphics + multi-tile TileSpecs.
    let (base_sprite, extra_graphics) = if !graphics.is_empty() {
        let mut iter = graphics.into_iter().map(sprite_decl_to_spec);
        let first = iter.next().expect("non-empty graphics checked above");
        (first, iter.collect())
    } else if let Some(sp) = sprite {
        (sprite_decl_to_spec(sp), Vec::new())
    } else {
        (SpriteSpec::default(), Vec::new())
    };

    let tile_specs: Vec<TileSpec> = tiles
        .into_iter()
        .map(|t| TileSpec {
            guid: t.guid,
            name: t.name,
            objd_id: t.objd_id,
            sub_index: t.sub_index,
            level: t.level,
            lead: t.lead,
            interaction_group: t.interaction_group,
            dgrp_id: t.dgrp_id,
            shape: t.shape,
            slot_id: t.slot_id,
            ttab_id: t.ttab_id,
            init: None,
            main: None,
            attributes: t.attributes,
        })
        .collect();

    let state_count = env
        .states
        .values()
        .map(|st| st.attr_index + 1)
        .max()
        .unwrap_or(0);
    let state_count = u16::try_from(state_count)
        .map_err(|_| Error::at(obj.span, "object declares too many state attributes"))?;
    let num_attributes = match explicit_attributes {
        Some(explicit) => state_count.max(explicit),
        None => state_count.max(8),
    };

    let manifest = ObjectManifest {
        name: env.name.clone(),
        description,
        guid,
        price,
        object_type,
        catalog_sort_flags,
        objd_id,
        ttab_id,
        slot_id,
        ctss_id,
        dgrp_id,
        spr2_id,
        palt_id,
        master_id,
        attributes: num_attributes,
        interaction_group,
        data_58,
        init: Some(TreeRef::Id(init_id)),
        main: Some(TreeRef::Id(main_id)),
        scripts: Vec::new(),
        sprite: base_sprite,
        extra_graphics,
        interactions: interaction_specs,
        slots: slot_specs,
        sounds: sound_specs,
        constants: Vec::new(),
        callbacks: callback_specs,
        tiles: tile_specs,
        string_tables: Vec::new(),
    };

    Ok(ExpandedV2Object {
        name: env.name,
        slug,
        guid,
        manifest,
        air_source: air,
    })
}

/// Built-in v2 call names a standalone `tree` identifier may not shadow.
const BUILTIN_CALL_NAMES: &[&str] = &[
    "sleep",
    "idle",
    "play_sound",
    "stop_all_sounds",
    "debit_funds",
    "credit_funds",
    "destroy_self",
    "set_graphic",
    "route_to",
    "snap",
    "set_motive_change",
    "clear_motive_changes",
    "dialog",
    "random",
    "can_afford",
    "find_best_object",
    "create_object",
    "objects_of",
    "Some",
    "None",
];

/// How a primitive builtin argument is checked and spelled in `.air`.
#[derive(Clone, Copy)]
enum PrimArg {
    /// Compile-time integer in `lo..=hi`.
    Int(i64, i64),
    /// Compile-time `bool`.
    Bool,
    /// 32-bit GUID literal or `const`.
    Guid,
    /// A v2 `i16` local variable (spelled by its `.air` local name).
    Local,
    /// `self` / `stack_obj` (or `0` / `1`).
    RefreshTarget,
}

/// A v2 builtin that lowers one-to-one onto a named-argument `.air` primitive.
struct PrimBuiltin {
    v2: &'static str,
    air: &'static str,
    /// `(v2 key, .air key, kind)`, in `.air` emission order.
    keys: &'static [(&'static str, &'static str, PrimArg)],
}

const U8: PrimArg = PrimArg::Int(0, 255);
const U16: PrimArg = PrimArg::Int(0, 65535);
const I16: PrimArg = PrimArg::Int(-32768, 32767);

const PRIM_BUILTINS: &[PrimBuiltin] = &[
    PrimBuiltin {
        v2: "push_interaction",
        air: "push_interaction",
        keys: &[
            ("menu", "menu", U8),
            ("object", "object", U8),
            ("priority", "priority", U8),
            ("flags", "flags", U8),
            ("icon", "icon", PrimArg::Local),
        ],
    },
    PrimBuiltin {
        v2: "run_tree_by_name",
        air: "run_tree_by_name",
        keys: &[
            ("table", "table", U16),
            ("index", "index", U8),
            ("mode", "mode", U8),
            ("global", "global", PrimArg::Bool),
        ],
    },
    PrimBuiltin {
        v2: "go_to_relative",
        air: "go_to_relative",
        keys: &[
            ("location", "location", PrimArg::Int(-128, 127)),
            ("facing", "facing", U8),
            ("options", "options", U8),
        ],
    },
    PrimBuiltin {
        v2: "manage_inventory",
        air: "manage_inventory",
        keys: &[
            ("op", "op", U8),
            ("kind", "kind", U8),
            ("flags", "flags", U16),
            ("guid", "guid", PrimArg::Guid),
        ],
    },
    PrimBuiltin {
        v2: "refresh",
        air: "refresh",
        keys: &[
            ("target", "target", PrimArg::RefreshTarget),
            ("mode", "mode", U16),
        ],
    },
    PrimBuiltin {
        v2: "action_label",
        air: "action_string",
        keys: &[("table", "table", U16), ("index", "index", U8)],
    },
    PrimBuiltin {
        v2: "show_string",
        air: "show_string",
        keys: &[("table", "table", U16), ("index", "index", U16)],
    },
    PrimBuiltin {
        v2: "set_headline",
        air: "set_headline",
        keys: &[
            ("kind", "kind", U8),
            ("index", "index", U8),
            ("duration", "duration", I16),
            ("options", "options", U16),
            ("stack_obj", "stack_obj", PrimArg::Bool),
            ("local", "local", PrimArg::Local),
        ],
    },
    PrimBuiltin {
        v2: "special_effect",
        air: "special_effect",
        keys: &[
            ("duration", "duration", I16),
            ("kind", "kind", U8),
            ("size", "size", U8),
            ("flags", "flags", U8),
            ("str_index", "str_index", U8),
        ],
    },
];

/// Argless low-level opcodes exposed as `name()`; each lowers to `op 0xNNNN (0, ..., 0)` and
/// leaves `stack_obj` exactly as the primitive does (so `gosub_found_action()` sees
/// `find_best_interaction()`'s pick).
const OP_BUILTINS: &[(&str, u16)] = &[
    ("find_best_interaction", 0x0003),
    ("grab", 0x0004),
    ("drop", 0x0005),
    ("gosub_found_action", 0x001E),
    ("notify_out_of_idle", 0x0031),
];

/// Bitflags key of `self.flags`: an untyped bitfield that accepts any `bitflags` value.
const ANY_BITFLAGS: &str = "builtin::AnyFlags";

/// Builtins added after user helpers could already use these names: a user `fn`, `tree`, or
/// `extern` with the same name takes precedence.
fn is_prim_builtin_name(name: &str) -> bool {
    PRIM_BUILTINS.iter().any(|b| b.v2 == name)
        || OP_BUILTINS.iter().any(|(n, _)| *n == name)
        || matches!(name, "animate" | "breakpoint" | "random_into")
}
/// Allocates state attribute slots shared by object lowering and imported object schemas.
///
/// Explicit `state x @ N: T;` slots are reserved first (overlaps are errors), then every
/// auto-allocated state takes the lowest free slot in declaration order. Without explicit slots
/// this is exactly the historical `0..N` layout.
fn allocate_state_slots<'s>(
    states: impl IntoIterator<Item = (&'s str, Option<(Span, u16)>, Span)>,
) -> Result<Vec<usize>> {
    let states: Vec<_> = states.into_iter().collect();
    let mut used: BTreeMap<u16, &str> = BTreeMap::new();
    for &(name, slot, _) in &states {
        if let Some((sspan, n)) = slot
            && let Some(other) = used.insert(n, name)
        {
            return Err(Error::at(
                sspan,
                format!(
                    "state '{name}' @ {n} overlaps state '{other}' (both bind attribute slot {n})"
                ),
            ));
        }
    }
    let mut next = 0u16;
    let mut out = Vec::with_capacity(states.len());
    for &(name, slot, span) in &states {
        let idx = match slot {
            Some((_, n)) => n,
            None => {
                while used.contains_key(&next) {
                    next += 1;
                }
                if next > crate::v2::parser::MAX_STATE_SLOT {
                    return Err(Error::at(
                        span,
                        format!(
                            "state '{name}' does not fit in attribute slots 0..={}",
                            crate::v2::parser::MAX_STATE_SLOT
                        ),
                    ));
                }
                used.insert(next, name);
                next
            }
        };
        out.push(usize::from(idx));
    }
    Ok(out)
}

/// Resolves private named BCON tables into operand addresses, keyed by both reference spellings
/// (`LampTuning` and `lamp_tuning`).
fn resolve_bcon_tables(
    decls: &[BconTableDecl],
    i16_param_order: &[String],
    local_extern_bcons: &BTreeMap<String, (String, ExternBconDecl)>,
) -> Result<BTreeMap<String, ResolvedBconTable>> {
    let mut out: BTreeMap<String, ResolvedBconTable> = BTreeMap::new();
    for d in decls {
        if !(4096..=4159).contains(&d.id) {
            return Err(Error::at(
                d.span,
                format!(
                    "BCON table id {} must be a private BCON chunk id in 4096..=4159 (use 'extern bcon' for global/semi-global constants)",
                    d.id
                ),
            ));
        }
        if d.id == 4096 && !i16_param_order.is_empty() {
            return Err(Error::at(
                d.span,
                "BCON 4096 is reserved for this object's i16 params (pick 4097..=4159)",
            ));
        }
        if d.entries.len() > 128 {
            return Err(Error::at(
                d.span,
                format!(
                    "BCON table '{}' has {} entries (max 128)",
                    d.label,
                    d.entries.len()
                ),
            ));
        }
        let table = d.id - 4096;
        let mut entries = BTreeMap::new();
        let mut values = Vec::with_capacity(d.entries.len());
        for (i, (espan, ename, v)) in d.entries.iter().enumerate() {
            let entry = u16::try_from(i).expect("<= 128 entries");
            if entries
                .insert(ename.clone(), ((table << 7) | entry, *v))
                .is_some()
            {
                return Err(Error::at(
                    *espan,
                    format!("duplicate entry '{ename}' in BCON table '{}'", d.label),
                ));
            }
            values.push((ename.clone(), *v));
        }
        let resolved = ResolvedBconTable {
            id: d.id,
            label: d.label.clone(),
            entries,
            values,
        };
        let mut names = vec![object_type_ident(&d.label), snake_case_ident(&d.label)];
        names.dedup();
        for name in names {
            if local_extern_bcons.contains_key(&name) {
                return Err(Error::at(
                    d.span,
                    format!(
                        "BCON table '{}' reference name '{name}' collides with an 'extern bcon' block",
                        d.label
                    ),
                ));
            }
            if let Some(prev) = out.insert(name.clone(), resolved.clone()) {
                return Err(Error::at(
                    d.span,
                    format!(
                        "BCON table '{}' reference name '{name}' collides with BCON table '{}' (BCON {})",
                        d.label, prev.label, prev.id
                    ),
                ));
            }
        }
    }
    Ok(out)
}

/// Resolves standalone `tree` call signatures (IDs are filled in after allocation).
fn resolve_tree_sigs(
    env: &ObjectEnv<'_>,
    trees: &[OwnedTree],
    member_modules: &BTreeSet<String>,
) -> Result<BTreeMap<String, CallableSig>> {
    let mut out = BTreeMap::new();
    for t in trees {
        let d = &t.decl;
        if BUILTIN_CALL_NAMES.contains(&d.ident.as_str()) {
            return Err(Error::at(
                d.span,
                format!("tree '{}' shadows the built-in '{}()'", d.ident, d.ident),
            ));
        }
        if let Some(m) = std::iter::once(&t.owner_module)
            .chain(member_modules)
            .find(|m| env.lookup_fn(m, &d.ident, true).is_some())
        {
            return Err(Error::at(
                d.span,
                format!(
                    "tree '{}' collides with helper function '{}' in scope (module '{m}')",
                    d.ident, d.ident
                ),
            ));
        }
        let mut sig = env.resolve_callable_sig(
            format!("tree '{}'", d.ident),
            0,
            Some(d.sim_binding.is_some()),
            &d.params,
            &d.ret,
            &t.owner_module,
            true,
            d.span,
        )?;
        // A tree without `->` is a plain SimAntics check/action tree: it exits true/false
        // (`return true;` by default), so callers may test it like a `-> bool` tree.
        if sig.ret == ResolvedReturnAbi::Void {
            sig.ret = ResolvedReturnAbi::Bool;
        }
        out.insert(d.ident.clone(), sig);
    }
    Ok(out)
}

/// Lowers and emits one standalone `tree` member as a real (non-inlined) BHAV.
fn emit_standalone_tree(trees_air: &mut String, env: &ObjectEnv<'_>, t: &OwnedTree) -> Result<()> {
    let d = &t.decl;
    let sig = &env.trees[&d.ident];
    let must_return = matches!(
        d.ret,
        ReturnAbi::Bool | ReturnAbi::Value { .. } | ReturnAbi::ResultValue { .. }
    );
    if must_return && !stmts_always_return(&d.body, env, &t.owner_module, true) {
        return Err(Error::at(
            d.span,
            format!(
                "tree '{}' declares a return value and must return on all control-flow paths",
                d.ident
            ),
        ));
    }
    let (tree_ctx, needs_self_save) = match &d.sim_binding {
        Some(sim) => (
            TreeContext::SimOnObject {
                sim_name: sim.clone(),
            },
            stmts_may_clobber_stack_obj(&d.body, env, &t.owner_module, true, Some(sim), 0),
        ),
        None => (TreeContext::ObjectSelf, false),
    };
    let mut ctx = TreeLowerer::new(env, tree_ctx, &t.owner_module, needs_self_save);
    match &sig.ret {
        ResolvedReturnAbi::Value { ty, temp } | ResolvedReturnAbi::ResultValue { ty, temp } => {
            ctx.value_return = Some(ValueReturnAbi {
                ty: ty.clone(),
                temp: *temp,
                // `Result<T @ temp[N]>` trees take the false exit with `return None;` (or
                // `return false;` when T is not bool), so a caller's `?` runs its own cleanups.
                allow_none: matches!(sig.ret, ResolvedReturnAbi::ResultValue { .. }),
                in_place: true,
                what: "tree",
            });
        }
        _ => {}
    }
    if needs_self_save {
        ctx.line("let __self_obj = stack_obj;".to_owned());
    }
    // Temp-bound params are snapshotted into locals at entry (temps are shared with callees).
    let mut positional = 0usize;
    for p in &sig.params {
        match p.slot {
            AbiParamSlot::Positional => {
                ctx.bind_var(
                    p.name.clone(),
                    Binding::BhavParam {
                        index: positional,
                        ty: p.ty.clone(),
                    },
                );
                positional += 1;
            }
            AbiParamSlot::Temp(n) => {
                let local = ctx.fresh(&format!("p_{}", p.name));
                ctx.line(format!("let {local} = temp[{n}];"));
                ctx.bind_var(
                    p.name.clone(),
                    Binding::Local {
                        air_name: local,
                        ty: p.ty.clone(),
                    },
                );
            }
        }
    }
    ctx.lower_stmts(&d.body)?;
    if !stmts_always_return(&d.body, env, &t.owner_module, true) {
        ctx.emit_all_cleanups()?;
        ctx.line("return true;".to_owned());
    }
    let label = escape_str(&d.label);
    let mut header: Vec<String> = Vec::new();
    let frame = d.frame.unwrap_or_default();
    match frame.args {
        Some(args) if usize::from(args) < positional => {
            return Err(Error::at(
                frame.span,
                format!(
                    "@frame(args: {args}) on tree '{}' is below its {positional} positional parameter(s)",
                    d.ident
                ),
            ));
        }
        Some(args) => header.push(format!("args: {args}")),
        None if positional > 0 => header.push(format!("args: {positional}")),
        None => {}
    }
    if let Some(locals) = frame.locals {
        header.push(format!("locals: {locals}"));
    }
    if let Some(format) = frame.format {
        if !(0x8000..=0x8002).contains(&format) {
            return Err(Error::at(
                frame.span,
                format!(
                    "@frame(format: {format:#06x}) on tree '{}' is not a supported BHAV format (0x8000, 0x8001, or 0x8002)",
                    d.ident
                ),
            ));
        }
        header.push(format!("format: {format:#06x}"));
    }
    if let Some(revision) = frame.revision {
        header.push(format!("revision: {revision}"));
    }
    if header.is_empty() {
        let _ = writeln!(trees_air, "tree {} \"{label}\" {{", sig.id);
    } else {
        let _ = writeln!(
            trees_air,
            "tree {} \"{label}\" ({}) {{",
            sig.id,
            header.join(", ")
        );
    }
    trees_air.push_str(&ctx.finish());
    trees_air.push_str("}\n\n");
    Ok(())
}

fn sprite_decl_to_spec(sp: SpriteDecl) -> SpriteSpec {
    SpriteSpec {
        shape: sp.shape,
        primary_rgb: sp.primary_rgb,
        accent_rgb: sp.accent_rgb,
        trim_rgb: sp.trim_rgb,
    }
}

fn build_object_schema(
    resolved: &ResolvedModule,
    owner_mod: &str,
    obj: &ObjectDecl,
    canonical_key: &str,
    display_name: &str,
    all_enums: &mut BTreeMap<String, ResolvedEnum>,
    all_bitflags: &mut BTreeMap<String, ResolvedBitflags>,
) -> Result<ResolvedObjectSchema> {
    let mut guid: Option<u32> = None;
    let mut tiles: Vec<crate::v2::ast::TileDecl> = Vec::new();
    let mut raw_states: Vec<(String, String, StateDecl)> = Vec::new();
    let mut seen_states = BTreeSet::new();
    let mut local_enums: BTreeMap<String, String> = BTreeMap::new();
    let mut local_bitflags: BTreeMap<String, String> = BTreeMap::new();
    let mut raw_funcs: Vec<(String, FunctionalDecl)> = Vec::new();
    let slug = make_slug(&obj.name);

    let mut collect = |m_owner: &str, members: &[ObjectMember]| {
        for member in members {
            match member {
                ObjectMember::Meta(MetaField {
                    kind: MetaFieldKind::Guid(g),
                    ..
                }) => guid = Some(*g),
                ObjectMember::Meta(MetaField {
                    kind: MetaFieldKind::Tiles(t),
                    ..
                }) => tiles = t.clone(),
                ObjectMember::Enum(e) => {
                    let key = format!("local_enum::{m_owner}::{slug}::{}", e.name);
                    all_enums
                        .entry(key.clone())
                        .or_insert_with(|| resolve_enum_decl(key.clone(), e));
                    local_enums.insert(e.name.clone(), key);
                }
                ObjectMember::Bitflags(b) => {
                    let key = format!("local_bitflags::{m_owner}::{slug}::{}", b.name);
                    all_bitflags
                        .entry(key.clone())
                        .or_insert_with(|| resolve_bitflags_decl(key.clone(), b));
                    local_bitflags.insert(b.name.clone(), key);
                }
                ObjectMember::State(s) => {
                    if seen_states.insert(s.name.clone()) {
                        raw_states.push((m_owner.to_owned(), s.name.clone(), s.clone()));
                    }
                }
                ObjectMember::Functional(f) => {
                    if let Some(pos) = raw_funcs.iter().position(|(_, x)| x.kind == f.kind) {
                        raw_funcs[pos] = (m_owner.to_owned(), f.clone());
                    } else {
                        raw_funcs.push((m_owner.to_owned(), f.clone()));
                    }
                }
                _ => {}
            }
        }
    };

    if let Some((tpl_name, _)) = &obj.from_template
        && let Some(scope) = resolved.module_scopes.get(owner_mod)
        && let Some((tpl_mod, tpl_decl)) = scope.templates.get(tpl_name)
    {
        collect(tpl_mod, &tpl_decl.members);
    }
    collect(owner_mod, &obj.members);

    let resolve_ty = |ty: &V2Type, m_owner: &str, span: Span| -> Result<ValueType> {
        match ty {
            V2Type::I16 => Ok(ValueType::I16),
            V2Type::Bool => Ok(ValueType::Bool),
            V2Type::ObjectId => Ok(ValueType::ObjectId),
            V2Type::Named(tname) => {
                if let Some(k) = local_enums.get(tname) {
                    return Ok(ValueType::Enum(k.clone()));
                }
                if let Some(k) = local_bitflags.get(tname) {
                    return Ok(ValueType::Bitflags(k.clone()));
                }
                match tname.as_str() {
                    "Height" => return Ok(ValueType::Bitflags("builtin::Height".to_owned())),
                    "Placement" => return Ok(ValueType::Bitflags("builtin::Placement".to_owned())),
                    "Self" => return Ok(ValueType::ObjectRef(canonical_key.to_owned())),
                    _ => {}
                }
                if let Some(sc) = resolved.module_scopes.get(m_owner) {
                    if let Some((def_mod, ed)) = sc.enums.get(tname) {
                        return Ok(ValueType::Enum(format!("enum::{def_mod}::{}", ed.name)));
                    }
                    if let Some((def_mod, bd)) = sc.bitflags.get(tname) {
                        return Ok(ValueType::Bitflags(format!(
                            "bitflags::{def_mod}::{}",
                            bd.name
                        )));
                    }
                    if let Some((def_mod, od)) = sc.objects.get(tname) {
                        return Ok(ValueType::ObjectRef(format!(
                            "object::{def_mod}::{}",
                            object_type_ident(&od.name)
                        )));
                    }
                }
                Err(Error::at(
                    span,
                    format!("unknown type '{tname}' in imported object '{display_name}'"),
                ))
            }
            other => Err(Error::at(
                span,
                format!(
                    "unsupported type '{}' in imported object '{display_name}'",
                    other.display_name()
                ),
            )),
        }
    };

    let state_slots = allocate_state_slots(
        raw_states
            .iter()
            .map(|(_, n, d)| (n.as_str(), d.slot, d.span)),
    )?;
    let mut states = BTreeMap::new();
    for ((m_owner, sname, sdecl), idx) in raw_states.into_iter().zip(state_slots) {
        let vty = match &sdecl.ty {
            StateType::Bool => ValueType::Bool,
            StateType::I16 => ValueType::I16,
            StateType::ObjectId => ValueType::ObjectId,
            StateType::Named(tname) => {
                resolve_ty(&V2Type::Named(tname.clone()), &m_owner, sdecl.span)?
            }
        };
        states.insert(sname, (idx, vty));
    }

    let mut functionals = BTreeMap::new();
    for (m_owner, fdecl) in raw_funcs {
        let mut p_tys = Vec::with_capacity(fdecl.params.len());
        for fp in &fdecl.params {
            p_tys.push(resolve_ty(&fp.ty, &m_owner, fp.span)?);
        }
        let r_ty = match &fdecl.return_type {
            Some(rt) => Some(resolve_ty(rt, &m_owner, fdecl.span)?),
            None => None,
        };
        functionals.insert(
            fdecl.kind,
            ResolvedFunctionalSig {
                params: p_tys,
                return_type: r_ty,
                guard: summarize_functional_guard(fdecl.guard.as_ref(), &states),
            },
        );
    }

    let guid = guid.ok_or_else(|| {
        Error::at(
            obj.span,
            format!("imported object '{display_name}' is missing required 'guid' field"),
        )
    })?;
    let lead_guid = select_lead_tile_guid(&tiles, guid);
    let tile_guids = if tiles.is_empty() {
        vec![guid]
    } else {
        tiles.iter().map(|t| t.guid).collect()
    };

    Ok(ResolvedObjectSchema {
        canonical_key: canonical_key.to_owned(),
        display_name: display_name.to_owned(),
        master_guid: guid,
        lead_guid,
        tile_guids,
        states,
        functionals,
    })
}

/// Mirrors `compile.rs`'s multi-tile lead selection rule: explicit `lead: true` first,
/// otherwise the `(sub_index == 0, level == 0)` origin tile, falling back to the first tile.
fn select_lead_tile_guid(tiles: &[crate::v2::ast::TileDecl], master_guid: u32) -> u32 {
    if tiles.is_empty() {
        return master_guid;
    }
    if let Some(t) = tiles.iter().find(|t| t.lead) {
        return t.guid;
    }
    if let Some(t) = tiles.iter().find(|t| t.sub_index == 0 && t.level == 0) {
        return t.guid;
    }
    tiles[0].guid
}

fn summarize_functional_guard(
    guard: Option<&GuardDecl>,
    states: &BTreeMap<String, (usize, ValueType)>,
) -> FunctionalGuardSummary {
    let Some(g) = guard else {
        return FunctionalGuardSummary::None;
    };
    let GuardBody::Expr(expr) = &g.kind else {
        return FunctionalGuardSummary::Complex;
    };
    let (inner, expect_true) = match expr {
        V2Expr::Unary {
            op: V2UnaryOp::Not,
            inner,
            ..
        } => (inner.as_ref(), false),
        other => (other, true),
    };
    if let V2Expr::Field { base, field, .. } = inner
        && matches!(base.as_ref(), V2Expr::Ident(_, b) if b == "self")
        && let Some((idx, ValueType::Bool)) = states.get(field)
    {
        return FunctionalGuardSummary::AttrBool {
            attr_index: *idx,
            expect_true,
        };
    }
    FunctionalGuardSummary::Complex
}

fn resolve_state_init_air(
    env: &ObjectEnv<'_>,
    owner_mod: &str,
    sname: &str,
    ty: &ValueType,
    init: &StateInitValue,
    span: Span,
) -> Result<String> {
    match init {
        StateInitValue::Int(v) => {
            if !matches!(ty, ValueType::I16 | ValueType::ObjectId) {
                return Err(Error::at(
                    span,
                    format!(
                        "state '{sname}' of type '{}' cannot initialize from integer literal {v}",
                        env.display_value_type(ty)
                    ),
                ));
            }
            Ok(v.to_string())
        }
        StateInitValue::Bool(b) => {
            if *ty != ValueType::Bool {
                return Err(Error::at(
                    span,
                    format!(
                        "state '{sname}' of type '{}' cannot initialize from bool literal",
                        env.display_value_type(ty)
                    ),
                ));
            }
            Ok(if *b { "1" } else { "0" }.to_owned())
        }
        StateInitValue::Param(pname) => {
            if env.i16_params.contains_key(pname) {
                if !matches!(ty, ValueType::I16 | ValueType::ObjectId) {
                    return Err(Error::at(
                        span,
                        format!(
                            "state '{sname}' of type '{}' cannot initialize from i16 param '{pname}'",
                            env.display_value_type(ty)
                        ),
                    ));
                }
                Ok(format!("bcon.{pname}"))
            } else if let Some(&bval) = env.bool_params.get(pname) {
                if *ty != ValueType::Bool {
                    return Err(Error::at(
                        span,
                        format!(
                            "state '{sname}' of type '{}' cannot initialize from bool param '{pname}'",
                            env.display_value_type(ty)
                        ),
                    ));
                }
                Ok(if bval { "1" } else { "0" }.to_owned())
            } else {
                Err(Error::at(
                    span,
                    format!("state '{sname}' initializer references unknown param '{pname}'"),
                ))
            }
        }
        StateInitValue::Path(pspan, segs) => {
            if segs.len() != 2 {
                return Err(Error::at(
                    *pspan,
                    format!(
                        "expected 'Type::Variant' in state '{sname}' initializer, found '{}'",
                        segs.join("::")
                    ),
                ));
            }
            let (tname, vname) = (&segs[0], &segs[1]);
            if let Some(e) = env.lookup_enum(owner_mod, tname, true) {
                let Some(&val) = e.variants.get(vname) else {
                    return Err(Error::at(
                        *pspan,
                        format!("enum '{}' has no variant '{vname}'", e.display_name),
                    ));
                };
                if *ty != ValueType::Enum(e.canonical_key.clone()) {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "type mismatch: state '{sname}' has type '{}', cannot initialize from '{}'",
                            env.display_value_type(ty),
                            e.display_name
                        ),
                    ));
                }
                return Ok(val.to_string());
            }
            if let Some(bf) = env.lookup_bitflags(owner_mod, tname, true) {
                if *ty != ValueType::Bitflags(bf.canonical_key.clone()) {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "type mismatch: state '{sname}' has type '{}', cannot initialize from '{}'",
                            env.display_value_type(ty),
                            bf.display_name
                        ),
                    ));
                }
                let mask = if vname == "None" {
                    0i16
                } else if let Some(&bit) = bf.flags.get(vname) {
                    bit_to_mask(bit)
                } else {
                    return Err(Error::at(
                        *pspan,
                        format!("bitflags '{}' has no flag '{vname}'", bf.display_name),
                    ));
                };
                return Ok(mask.to_string());
            }
            Err(Error::at(
                *pspan,
                format!("unknown enum or bitflags type '{tname}' in state '{sname}' initializer"),
            ))
        }
        StateInitValue::BitOr(bspan, lhs, rhs) => {
            let ValueType::Bitflags(_) = ty else {
                return Err(Error::at(
                    *bspan,
                    format!(
                        "'|' in state '{sname}' initializer is only supported for bitflags types"
                    ),
                ));
            };
            let l_val: i16 = resolve_state_init_air(env, owner_mod, sname, ty, lhs, *bspan)?
                .parse()
                .map_err(|_| Error::at(*bspan, "bitflags initializer must be constant"))?;
            let r_val: i16 = resolve_state_init_air(env, owner_mod, sname, ty, rhs, *bspan)?
                .parse()
                .map_err(|_| Error::at(*bspan, "bitflags initializer must be constant"))?;
            Ok((l_val | r_val).to_string())
        }
    }
}

fn emit_guard_tree(
    air: &mut String,
    env: &ObjectEnv<'_>,
    owner_module: &str,
    tree_id: u16,
    label: &str,
    guard: &GuardDecl,
    func_params: &[crate::v2::ast::FnParam],
) -> Result<()> {
    let needs_self_save = match &guard.kind {
        GuardBody::Expr(e) => {
            expr_may_clobber_stack_obj(e, env, owner_module, true, Some(&guard.sim_binding), 0)
        }
        GuardBody::Block(stmts) => {
            stmts_may_clobber_stack_obj(stmts, env, owner_module, true, Some(&guard.sim_binding), 0)
        }
    };
    let mut ctx = TreeLowerer::new(
        env,
        TreeContext::SimOnObject {
            sim_name: guard.sim_binding.clone(),
        },
        owner_module,
        needs_self_save,
    );
    if needs_self_save {
        ctx.line("let __self_obj = stack_obj;".to_owned());
    }
    if let Some(ref auto_name) = guard.autonomous_binding {
        ctx.bind_var(auto_name.clone(), Binding::AutonomousFlag);
    }
    for (pidx, fp) in func_params.iter().enumerate() {
        ctx.bind_func_param(&fp.name, pidx, &fp.ty, fp.span)?;
    }
    match &guard.kind {
        GuardBody::Expr(expr) => {
            ctx.emit_return_bool_expr(expr)?;
        }
        GuardBody::Block(stmts) => {
            ctx.lower_stmts(stmts)?;
            if !stmts_always_return(stmts, env, owner_module, true) {
                ctx.emit_all_cleanups()?;
                ctx.line("return true;".to_owned());
            }
        }
    }
    let arg_count = if guard.autonomous_binding.is_some() {
        1
    } else {
        func_params.len()
    };
    if arg_count > 0 {
        let _ = writeln!(air, "tree {tree_id} \"{label}\" (args: {arg_count}) {{");
    } else {
        let _ = writeln!(air, "tree {tree_id} \"{label}\" {{");
    }
    air.push_str(&ctx.finish());
    air.push_str("}\n\n");
    Ok(())
}

/// Maps a v2 functional kind name to `(OBJf callback index, optional score object data word)`.
fn functional_kind_info(kind: &str, span: Span) -> Result<(usize, Option<usize>)> {
    match kind {
        "prep" => Ok((18, Some(31))),
        "cook" => Ok((19, Some(32))),
        "surface" => Ok((20, Some(33))),
        "dispose" => Ok((21, Some(36))),
        "eat" => Ok((24, Some(37))),
        "pick_up" => Ok((25, Some(38))),
        "sit" => Ok((26, None)),
        "clean" => Ok((14, Some(61))),
        "repair" => Ok((28, Some(39))),
        "serve" => Ok((16, Some(64))),
        "rest" => Ok((29, Some(15))),
        other => Err(Error::at(
            span,
            format!(
                "unknown functional kind '{other}' (expected prep, cook, surface, dispose, eat, pick_up, sit, clean, repair, serve, or rest)"
            ),
        )),
    }
}

#[derive(Debug, Clone)]
enum TreeContext {
    ObjectSelf,
    SimOnObject { sim_name: String },
}

/// Binding of a local variable or helper parameter during tree lowering.
#[derive(Debug, Clone)]
enum Binding {
    /// Regular local variable in `.air` (`local_name` is unique within the tree).
    Local { air_name: String, ty: ValueType },
    /// Immutable constant `i16` operand (literal or `bcon.<name>`) forwarded into an inline helper.
    ConstI16 { air: String, value: i16 },
    /// Immutable constant `bool` value forwarded into an inline helper.
    ConstBool(bool),
    /// Immutable constant `enum` value forwarded into an inline helper.
    ConstEnum { key: String, value: i16 },
    /// Immutable constant `bitflags` value forwarded into an inline helper (retains 1-based bit index for single flags!).
    ConstBitflag {
        key: String,
        mask: i16,
        single_bit: Option<u8>,
    },
    /// TTAB interaction guard autonomy probe parameter (`param[0] == 1`).
    AutonomousFlag,
    /// Immutable BHAV `param[idx]` (from a `functional` parameter).
    BhavParam { index: usize, ty: ValueType },
    /// Mutable reference (`&mut T`) bound to a caller lvalue during helper inlining.
    MutRef { target: LValue },
    /// Compile-time `Motive` index passed to a helper function.
    Motive(u8),
    /// Compile-time `Skill` person_data index passed to a helper function.
    Skill(u8),
    /// Compile-time `Sound` FWAV ID passed to a helper function.
    Sound(u16),
    /// Compile-time `Slot` index passed to a helper function.
    Slot(u16),
    /// `Sim` handle passed to a helper function.
    SimHandle,
    /// `self` Object handle passed to a helper function.
    SelfHandle,
    /// Foreign `ObjectId` passed as `Object` handle to a helper function.
    ForeignObject { air_name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ValueType {
    I16,
    Bool,
    ObjectId,
    Enum(String),
    Bitflags(String),
    ObjectRef(String),
}

impl ValueType {
    fn is_object_handle(&self) -> bool {
        matches!(self, Self::ObjectId | Self::ObjectRef(_))
    }
}

fn types_compatible_for_assign(target: &ValueType, value: &ValueType) -> bool {
    if target == value {
        return true;
    }
    match (target, value) {
        // `self.flags` takes (and yields) any bitflags type.
        (ValueType::Bitflags(t), ValueType::Bitflags(v))
            if t == ANY_BITFLAGS || v == ANY_BITFLAGS =>
        {
            true
        }
        // Typed ObjectRef implicitly upcasts to untyped ObjectId.
        (ValueType::ObjectId, ValueType::ObjectRef(_)) => true,
        // Preserve backwards compatibility for raw ObjectId <-> i16 temp/attr interop.
        (ValueType::ObjectId, ValueType::I16) | (ValueType::I16, ValueType::ObjectId) => true,
        _ => false,
    }
}

/// A resolved assignable storage location in `.air`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LValue {
    /// Direct `.air` operand (e.g., `stack.attr[0]`, `my.motive.energy`, `v_x_0`, `temp[2]`).
    Direct { air: String, ty: ValueType },
    /// Attribute on a foreign `ObjectId` local (`stack_obj := obj_air; stack.attr[idx] ...; stack_obj := __self_obj;`).
    ForeignAttr {
        obj_air: String,
        attr_index: i16,
        ty: ValueType,
    },
    /// Object data word on a foreign `ObjectId` local.
    ForeignData {
        obj_air: String,
        data_index: i16,
        ty: ValueType,
    },
}

impl LValue {
    fn value_type(&self) -> &ValueType {
        match self {
            LValue::Direct { ty, .. }
            | LValue::ForeignAttr { ty, .. }
            | LValue::ForeignData { ty, .. } => ty,
        }
    }
}

#[derive(Debug, Clone)]
enum CleanupAction {
    Unlock(LValue),
    Defer {
        owner_module: String,
        in_object_scope: bool,
        body: Vec<V2Stmt>,
        saved_bindings: Vec<BTreeMap<String, Binding>>,
    },
}

#[derive(Debug, Clone)]
struct ScopeFrame {
    cleanups: Vec<CleanupAction>,
    is_loop_body: bool,
    /// `true` when this frame is the root lexical scope of an inlined helper function.
    is_fn_root: bool,
    /// `true` when this frame corresponds to an emitted `.air` `while` loop
    /// (`repeat`, `while`, `for`, or wrapped inline helper `while true`).
    is_air_while: bool,
    /// Active `defer` token when this scope was entered (used to distinguish loop breaks inside
    /// a defer-local loop from breaks attempting to escape out of a `defer` block).
    owner_defer_id: Option<usize>,
    cancel_flag: Option<String>,
    repeat_counter: Option<String>,
}

impl ScopeFrame {
    fn block() -> Self {
        Self {
            cleanups: Vec::new(),
            is_loop_body: false,
            is_fn_root: false,
            is_air_while: false,
            owner_defer_id: None,
            cancel_flag: None,
            repeat_counter: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FnReturnKind {
    Void,
    Bool,
    Value(ValueType),
}

struct ActiveInlineFn {
    ret_flag: String,
    ret_val: String,
    ret_kind: FnReturnKind,
    /// Active `defer` token when this helper was entered (returns matching `current_defer_id`
    /// exit only this helper frame, whereas mismatched returns would escape an enclosing `defer`).
    owner_defer_id: Option<usize>,
}

struct TreeLowerer<'a> {
    env: &'a ObjectEnv<'a>,
    tree_ctx: TreeContext,
    owner_module: String,
    /// `true` when lowering code inside the object/template (can see `local_fns` / local types),
    /// `false` inside an imported/module-level helper function.
    in_object_scope: bool,
    has_self_obj_local: bool,
    /// Set to `true` when an `ObjectSelf` lifecycle tree (`init`/`main`/`load`/`reset`/`cleanup`)
    /// touches `stack_obj` for a foreign object access, so `finish()` preserves the incoming
    /// `stack_obj` (e.g. the cancelling Sim in `cleanup` / OBJf 4) into `__saved_stack_obj`.
    used_saved_stack_obj: bool,
    /// Non-`None` when lowering a body that returns a value in `temp[N]` (typed
    /// `functional kind(...) -> T` or a standalone `tree ... -> T @ temp[N]`).
    value_return: Option<ValueReturnAbi>,
    out: String,
    indent: usize,
    next_id: usize,
    var_scopes: Vec<BTreeMap<String, Binding>>,
    cleanup_scopes: Vec<ScopeFrame>,
    /// Locks currently held at this point in lowering (including during `defer` execution
    /// until the lock's own `Unlock` cleanup executes).
    held_locks: Vec<LValue>,
    defer_generation: usize,
    current_defer_id: Option<usize>,
    inline_call_stack: Vec<String>,
    inline_expansions: usize,
    active_inline_fn: Option<ActiveInlineFn>,
}

impl<'a> TreeLowerer<'a> {
    fn new(
        env: &'a ObjectEnv<'a>,
        tree_ctx: TreeContext,
        owner_module: &str,
        has_self_obj_local: bool,
    ) -> Self {
        let mut root_vars = BTreeMap::new();
        if let TreeContext::SimOnObject { ref sim_name } = tree_ctx {
            root_vars.insert(sim_name.clone(), Binding::SimHandle);
        }
        Self {
            env,
            tree_ctx,
            owner_module: owner_module.to_owned(),
            in_object_scope: true,
            has_self_obj_local,
            used_saved_stack_obj: false,
            value_return: None,
            out: String::new(),
            indent: 1,
            next_id: 0,
            var_scopes: vec![root_vars],
            cleanup_scopes: vec![ScopeFrame::block()],
            held_locks: Vec::new(),
            defer_generation: 0,
            current_defer_id: None,
            inline_call_stack: Vec::new(),
            inline_expansions: 0,
            active_inline_fn: None,
        }
    }

    fn in_defer(&self) -> bool {
        self.current_defer_id.is_some()
    }

    fn emit_restore_stack_obj(&mut self) {
        if self.has_self_obj_local {
            self.line("stack_obj := __self_obj;".to_owned());
        } else if matches!(self.tree_ctx, TreeContext::ObjectSelf) {
            self.used_saved_stack_obj = true;
            self.line("stack_obj := __saved_stack_obj;".to_owned());
        }
    }

    fn finish(self) -> String {
        if self.used_saved_stack_obj {
            format!("    let __saved_stack_obj = stack_obj;\n{}", self.out)
        } else {
            self.out
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        let id = self.next_id;
        self.next_id += 1;
        format!("__{prefix}_{id}")
    }

    fn line(&mut self, text: String) {
        for _ in 0..self.indent {
            self.out.push_str("    ");
        }
        self.out.push_str(&text);
        self.out.push('\n');
    }

    fn with_owner_module<T>(
        &mut self,
        module: &str,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let prev = std::mem::replace(&mut self.owner_module, module.to_owned());
        let res = f(self);
        self.owner_module = prev;
        res
    }

    /// Resolves a `tree_ident(...)` / `extern_mod::fn_name(...)` call target, if `callee` is one.
    fn resolve_callable(&self, callee: &V2Expr, span: Span) -> Result<Option<CallableSig>> {
        match callee {
            V2Expr::Ident(_, name) => {
                if self.in_object_scope
                    && let Some(sig) = self.env.trees.get(name)
                {
                    return Ok(Some(sig.clone()));
                }
                Ok(None)
            }
            V2Expr::Path(_, segs) if segs.len() == 2 => {
                let Some((decl_mod, is_local, decl)) =
                    self.env
                        .lookup_extern_mod(&self.owner_module, &segs[0], self.in_object_scope)
                else {
                    return Ok(None);
                };
                let f = decl.fns.iter().find(|f| f.name == segs[1]).ok_or_else(|| {
                    Error::at(
                        span,
                        format!("extern mod '{}' has no fn '{}'", segs[0], segs[1]),
                    )
                })?;
                self.env
                    .resolve_callable_sig(
                        format!("{}::{}", segs[0], segs[1]),
                        f.id,
                        None,
                        &f.params,
                        &f.ret,
                        &decl_mod,
                        is_local,
                        f.span,
                    )
                    .map(Some)
            }
            _ => Ok(None),
        }
    }

    /// Evaluates call arguments, writes `@ temp[N]` / `call(temps)` operands, and returns the
    /// `.air` call text (`call ID`, `call ID(1, 2)`, or `call ID(temps)`).
    fn prepare_callable_call(
        &mut self,
        sig: &CallableSig,
        args: &[CallArg],
        span: Span,
    ) -> Result<String> {
        let mut args = args;
        match sig.sim_context {
            Some(true) => {
                if !matches!(self.tree_ctx, TreeContext::SimOnObject { .. }) {
                    return Err(Error::at(
                        span,
                        format!(
                            "{} runs in the Sim-on-object context and can only be called from interactions, functionals, or other Sim trees",
                            sig.display
                        ),
                    ));
                }
                let first_is_sim = args.first().is_some_and(|a| {
                    a.name.is_none()
                        && matches!(&a.value, V2Expr::Ident(_, n) if self.is_sim_ident(n))
                });
                if !first_is_sim {
                    return Err(Error::at(
                        span,
                        format!(
                            "{} expects the Sim handle as its first argument",
                            sig.display
                        ),
                    ));
                }
                args = &args[1..];
            }
            Some(false) if !matches!(self.tree_ctx, TreeContext::ObjectSelf) => {
                return Err(Error::at(
                    span,
                    format!(
                        "{} runs in the object's own context (it has no Sim parameter) and can only be called from init/main/load/reset/cleanup or other object trees",
                        sig.display
                    ),
                ));
            }
            Some(false) | None => {}
        }

        // Match arguments to parameters (positional first, then by name).
        let n = sig.params.len();
        let mut slots: Vec<Option<&V2Expr>> = vec![None; n];
        // Parameter indices in source (evaluation) order.
        let mut source_order: Vec<usize> = Vec::with_capacity(n);
        let mut next_pos = 0usize;
        let mut seen_named = false;
        for a in args {
            match &a.name {
                None => {
                    if seen_named {
                        return Err(Error::at(
                            a.span,
                            format!(
                                "positional argument after named argument in call to {}",
                                sig.display
                            ),
                        ));
                    }
                    if next_pos >= n {
                        return Err(Error::at(
                            a.span,
                            format!("{} takes {n} argument(s), but more were given", sig.display),
                        ));
                    }
                    slots[next_pos] = Some(&a.value);
                    source_order.push(next_pos);
                    next_pos += 1;
                }
                Some(name) => {
                    seen_named = true;
                    let idx = sig
                        .params
                        .iter()
                        .position(|p| &p.name == name)
                        .ok_or_else(|| {
                            Error::at(
                                a.span,
                                format!("{} has no parameter named '{name}'", sig.display),
                            )
                        })?;
                    if slots[idx].is_some() {
                        return Err(Error::at(
                            a.span,
                            format!(
                                "duplicate argument for parameter '{name}' of {}",
                                sig.display
                            ),
                        ));
                    }
                    slots[idx] = Some(&a.value);
                    source_order.push(idx);
                }
            }
        }
        let mut values: Vec<&V2Expr> = Vec::with_capacity(n);
        for (i, slot) in slots.into_iter().enumerate() {
            values.push(slot.ok_or_else(|| {
                Error::at(
                    span,
                    format!(
                        "missing argument '{}' in call to {}",
                        sig.params[i].name, sig.display
                    ),
                )
            })?);
        }

        // Evaluate operands left to right in source order (named arguments may be reordered). An
        // operand is snapshotted into a fresh local when it reads a temp (about to be overwritten)
        // or a later-evaluated argument may run code (calls, foreign reads) that could change it.
        let mut operands: Vec<(String, Option<i16>)> = vec![(String::new(), None); n];
        for (k, &i) in source_order.iter().enumerate() {
            let expr = values[i];
            let (air, ty) = self.lower_value_expr(expr)?;
            let pty = &sig.params[i].ty;
            if !types_compatible_for_assign(pty, &ty) {
                return Err(Error::at(
                    expr.span(),
                    format!(
                        "argument '{}' of {} expects '{}', found '{}'",
                        sig.params[i].name,
                        sig.display,
                        self.env.display_value_type(pty),
                        self.env.display_value_type(&ty)
                    ),
                ));
            }
            let const_val = air.parse::<i16>().ok().or_else(|| {
                self.try_const_i16_air_operand(expr)
                    .ok()
                    .flatten()
                    .map(|(_, v)| v)
            });
            let later_impure = source_order[k + 1..]
                .iter()
                .any(|&j| !self.is_pure_single_operand(values[j]));
            // A runtime (`extern bcon`) read can fault on a missing chunk: read it now, in source
            // order, so a fault cannot follow a partial temp staging or an out-of-order read.
            let may_fault = air.starts_with("bcon[");
            operands[i] =
                if const_val.is_none() && (air.contains("temp[") || later_impure || may_fault) {
                    let snap = self.fresh("arg");
                    self.line(format!("let {snap} = {air};"));
                    (snap, None)
                } else {
                    (air, const_val)
                };
        }

        let positional: Vec<usize> = (0..n)
            .filter(|&i| sig.params[i].slot == AbiParamSlot::Positional)
            .collect();
        let literals: Option<Vec<i16>> = if positional.len() <= 4 {
            positional.iter().map(|&i| operands[i].1).collect()
        } else {
            None
        };
        let literals = literals.filter(|l| !(l.len() == 4 && l.iter().all(|&v| v == -1)));
        for (i, p) in sig.params.iter().enumerate() {
            let dst = match p.slot {
                AbiParamSlot::Temp(t) => usize::from(t),
                AbiParamSlot::Positional if literals.is_some() => continue,
                AbiParamSlot::Positional => positional
                    .iter()
                    .position(|&x| x == i)
                    .expect("positional index"),
            };
            let op = operands[i].0.clone();
            self.line(format!("temp[{dst}] := {op};"));
        }
        Ok(match literals {
            // An argless call is spelled `(temps)` (`[0xFF; 8]`), the authoring tools' encoding:
            // the callee sees the caller's temps as its arguments (the temps pass-through ABI).
            Some(l) if l.is_empty() => format!("call {}(temps)", sig.id),
            Some(l) => {
                let parts: Vec<String> = l.iter().map(i16::to_string).collect();
                format!("call {}({})", sig.id, parts.join(", "))
            }
            None => format!("call {}(temps)", sig.id),
        })
    }

    /// Emits `if !<call> { <all cleanups> return error; }` for `?` propagation.
    fn emit_try_fail_branch(&mut self, call: &str, span: Span) -> Result<()> {
        if self.in_defer() {
            return Err(Error::at(span, "'?' cannot exit out of a 'defer' block"));
        }
        self.line(format!("if !{call} {{"));
        self.indent += 1;
        self.emit_cleanups_down_to(0)?;
        self.line("return error;".to_owned());
        self.indent -= 1;
        self.line("}".to_owned());
        Ok(())
    }

    /// Lowers a callable used as a statement (`f(..);` or `f(..)?;`).
    fn emit_callable_stmt(
        &mut self,
        sig: &CallableSig,
        args: &[CallArg],
        span: Span,
        is_try: bool,
    ) -> Result<()> {
        match (&sig.ret, is_try) {
            (ResolvedReturnAbi::Result | ResolvedReturnAbi::ResultValue { .. }, false) => {
                Err(Error::at(
                    span,
                    format!(
                        "unused Result from {}: propagate failure with '?' or test it with 'if'",
                        sig.display
                    ),
                ))
            }
            (ResolvedReturnAbi::Void | ResolvedReturnAbi::Value { .. }, true) => Err(Error::at(
                span,
                format!(
                    "'?' needs a call returning 'bool' or 'Result', but {} does not",
                    sig.display
                ),
            )),
            (_, false) => {
                // Both exits continue: the result (if any) is ignored.
                let call = self.prepare_callable_call(sig, args, span)?;
                self.line(format!("if {call} {{"));
                self.line("}".to_owned());
                Ok(())
            }
            (_, true) => {
                let call = self.prepare_callable_call(sig, args, span)?;
                self.emit_try_fail_branch(&call, span)
            }
        }
    }

    /// Lowers a callable in value position. With `raw`, a `temp[N]` result is returned as-is
    /// (the caller must consume it before anything else can run).
    fn lower_callable_value(
        &mut self,
        sig: &CallableSig,
        args: &[CallArg],
        span: Span,
        is_try: bool,
        raw: bool,
    ) -> Result<(String, ValueType)> {
        match (&sig.ret, is_try) {
            (ResolvedReturnAbi::Value { ty, temp }, false) => {
                let (ty, temp) = (ty.clone(), *temp);
                let call = self.prepare_callable_call(sig, args, span)?;
                self.line(format!("if {call} {{"));
                self.line("}".to_owned());
                Ok((self.capture_temp(temp, raw), ty))
            }
            (ResolvedReturnAbi::ResultValue { ty, temp }, true) => {
                let (ty, temp) = (ty.clone(), *temp);
                let call = self.prepare_callable_call(sig, args, span)?;
                self.emit_try_fail_branch(&call, span)?;
                Ok((self.capture_temp(temp, raw), ty))
            }
            (ResolvedReturnAbi::Bool, false) => {
                let call = self.prepare_callable_call(sig, args, span)?;
                let tmp = self.fresh("call");
                self.line(format!("let {tmp} = 0;"));
                self.line(format!("if {call} {{"));
                self.indent += 1;
                self.line(format!("{tmp} := 1;"));
                self.indent -= 1;
                self.line("}".to_owned());
                Ok((tmp, ValueType::Bool))
            }
            (ResolvedReturnAbi::Result | ResolvedReturnAbi::ResultValue { .. }, false) => {
                Err(Error::at(
                    span,
                    format!(
                        "{} returns a Result: unwrap it with '?' or test it with 'if'",
                        sig.display
                    ),
                ))
            }
            (ResolvedReturnAbi::Void, false) => Err(Error::at(
                span,
                format!("{} does not return a value", sig.display),
            )),
            (_, true) => Err(Error::at(
                span,
                format!(
                    "'?' in value position needs a call returning 'Result<T @ temp[N]>', but {} does not",
                    sig.display
                ),
            )),
        }
    }

    fn capture_temp(&mut self, temp: u8, raw: bool) -> String {
        if raw {
            return format!("temp[{temp}]");
        }
        let tmp = self.fresh("xret");
        self.line(format!("let {tmp} = temp[{temp}];"));
        tmp
    }

    /// If `expr` is a value-returning callable (`f(..)` with `-> T @ temp[N]`, or `f(..)?` with
    /// `-> Result<T @ temp[N]>`), lowers the call and returns the raw `temp[N]` operand.
    fn try_lower_callable_value_raw(
        &mut self,
        expr: &V2Expr,
    ) -> Result<Option<(String, ValueType)>> {
        let (call_expr, is_try) = match expr {
            V2Expr::Try { inner, .. } => (inner.as_ref(), true),
            other => (other, false),
        };
        let V2Expr::Call {
            span,
            callee,
            type_args,
            args,
        } = call_expr
        else {
            return Ok(None);
        };
        if !type_args.is_empty() {
            return Ok(None);
        }
        let Some(sig) = self.resolve_callable(callee, *span)? else {
            return Ok(None);
        };
        if !matches!(
            (&sig.ret, is_try),
            (ResolvedReturnAbi::Value { .. }, false)
                | (ResolvedReturnAbi::ResultValue { .. }, true)
        ) {
            return Ok(None);
        }
        self.lower_callable_value(&sig, args, *span, is_try, true)
            .map(Some)
    }

    /// `true` if `name` names a BCON table / `extern bcon` block in scope. Table names take
    /// priority over same-named variables (matching the `stack_obj` clobber analysis); only the
    /// `self` / Sim handles are never treated as tables.
    fn is_bcon_table_ref(&self, name: &str) -> bool {
        !self.is_self_ident(name)
            && !self.is_sim_ident(name)
            && self
                .env
                .is_bcon_table_name(&self.owner_module, name, self.in_object_scope)
    }

    /// Resolves `Table.entry` to its `bcon[addr]` operand (and compile-time value for private
    /// tables). `None` if `expr` is not a BCON table entry reference.
    fn bcon_field_ref(&self, expr: &V2Expr) -> Option<Result<(String, Option<i16>)>> {
        let V2Expr::Field { span, base, field } = expr else {
            return None;
        };
        let V2Expr::Ident(_, bname) = base.as_ref() else {
            return None;
        };
        if !self.is_bcon_table_ref(bname) {
            return None;
        }
        self.env.lookup_bcon_entry(
            &self.owner_module,
            bname,
            field,
            self.in_object_scope,
            *span,
        )
    }

    /// Lowers `return [expr];` inside a body that returns its value in `temp[N]`.
    fn lower_value_return(
        &mut self,
        span: Span,
        value: Option<&V2Expr>,
        abi: &ValueReturnAbi,
    ) -> Result<()> {
        let expected_ty = abi.ty.clone();
        let dest = format!("temp[{}]", abi.temp);
        let Some(raw_expr) = value else {
            return Err(Error::at(
                span,
                if abi.allow_none {
                    format!(
                        "{} declaring '-> {}' must return a value (or 'return false;' / 'return None;' on failure)",
                        abi.what,
                        self.env.display_value_type(&expected_ty)
                    )
                } else {
                    format!(
                        "{} declaring '-> {} @ {dest}' must return a value",
                        abi.what,
                        self.env.display_value_type(&expected_ty)
                    )
                },
            ));
        };
        if abi.allow_none
            && (matches!(raw_expr, V2Expr::Ident(_, n) if n == "None" && self.lookup_var(n).is_none())
                || (matches!(raw_expr, V2Expr::Bool(_, false)) && expected_ty != ValueType::Bool))
        {
            self.emit_all_cleanups()?;
            self.line("return false;".to_owned());
            return Ok(());
        }
        let inner_expr = match raw_expr {
            V2Expr::Call {
                callee,
                type_args,
                args,
                ..
            } if abi.allow_none
                && type_args.is_empty()
                && args.len() == 1
                && args[0].name.is_none()
                && matches!(callee.as_ref(), V2Expr::Ident(_, s) if s == "Some") =>
            {
                &args[0].value
            }
            other => other,
        };
        let type_err = |this: &Self, val_ty: &ValueType| {
            Error::at(
                span,
                format!(
                    "{} returning '{}' cannot return '{}' expression",
                    abi.what,
                    this.env.display_value_type(&expected_ty),
                    this.env.display_value_type(val_ty)
                ),
            )
        };
        if abi.in_place
            && !self.has_active_cleanups()
            && is_compound_arith_or_neg(inner_expr)
            && self.can_lower_arith_in_place(&dest, inner_expr, false)
        {
            let val_ty = self.lower_arith_into_dest(inner_expr, &dest, false)?;
            if !types_compatible_for_assign(&expected_ty, &val_ty) {
                return Err(type_err(self, &val_ty));
            }
            self.line("return true;".to_owned());
            return Ok(());
        }
        let (val_air, val_ty) = self.lower_value_expr(inner_expr)?;
        if !types_compatible_for_assign(&expected_ty, &val_ty) {
            return Err(type_err(self, &val_ty));
        }
        if self.has_active_cleanups() {
            let ret_snap = self.fresh("fret");
            self.line(format!("let {ret_snap} = {val_air};"));
            self.emit_all_cleanups()?;
            self.line(format!("{dest} := {ret_snap};"));
        } else {
            self.line(format!("{dest} := {val_air};"));
        }
        self.line("return true;".to_owned());
        Ok(())
    }

    fn bind_var(&mut self, name: String, binding: Binding) {
        if let Some(top) = self.var_scopes.last_mut() {
            top.insert(name, binding);
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&Binding> {
        for scope in self.var_scopes.iter().rev() {
            if let Some(b) = scope.get(name) {
                return Some(b);
            }
        }
        None
    }

    fn bind_func_param(&mut self, name: &str, idx: usize, ty: &V2Type, span: Span) -> Result<()> {
        let vty = self
            .env
            .resolve_value_type(ty, &self.owner_module, self.in_object_scope, span)
            .map_err(|_| {
                Error::at(
                    span,
                    format!(
                        "functional parameter '{name}' has unsupported type '{}' (expected i16, bool, ObjectId, enum, bitflags, or object type)",
                        ty.display_name()
                    ),
                )
            })?;
        self.bind_var(
            name.to_owned(),
            Binding::BhavParam {
                index: idx,
                ty: vty,
            },
        );
        Ok(())
    }

    fn is_sim_ident(&self, name: &str) -> bool {
        matches!(self.lookup_var(name), Some(Binding::SimHandle))
    }

    fn is_self_ident(&self, name: &str) -> bool {
        if matches!(self.lookup_var(name), Some(Binding::SelfHandle)) {
            return true;
        }
        name == "self"
    }

    fn self_prefix(&self) -> &'static str {
        match self.tree_ctx {
            TreeContext::ObjectSelf => "my",
            TreeContext::SimOnObject { .. } => "stack",
        }
    }

    fn enclosing_cancel_flag(&self) -> Option<(usize, String)> {
        for (idx, frame) in self.cleanup_scopes.iter().enumerate().rev() {
            if let Some(flag) = &frame.cancel_flag {
                return Some((idx, flag.clone()));
            }
        }
        None
    }

    fn nearest_air_while_idx(&self) -> Option<usize> {
        self.cleanup_scopes.iter().rposition(|f| f.is_air_while)
    }

    fn push_scope(&mut self, frame: ScopeFrame) {
        self.var_scopes.push(BTreeMap::new());
        self.cleanup_scopes.push(frame);
    }

    fn pop_scope_and_run_cleanups(&mut self) -> Result<()> {
        let frame = self.cleanup_scopes.pop().expect("cleanup_scopes balanced");
        let res = self.run_frame_cleanups(&frame);
        self.var_scopes.pop();
        res
    }

    fn pop_scope_discard_cleanups(&mut self) {
        let frame = self.cleanup_scopes.pop().expect("cleanup_scopes balanced");
        for action in frame.cleanups.iter().rev() {
            if let CleanupAction::Unlock(lval) = action
                && let Some(pos) = self.held_locks.iter().rposition(|x| x == lval)
            {
                self.held_locks.remove(pos);
            }
        }
        self.var_scopes.pop();
    }

    fn fn_scope_floor(&self) -> usize {
        self.cleanup_scopes
            .iter()
            .rposition(|f| f.is_fn_root)
            .unwrap_or(0)
    }

    fn has_active_cleanups(&self) -> bool {
        let floor = self.fn_scope_floor();
        self.cleanup_scopes[floor..]
            .iter()
            .any(|f| !f.cleanups.is_empty())
    }

    fn has_any_active_cleanups(&self) -> bool {
        self.cleanup_scopes.iter().any(|f| !f.cleanups.is_empty())
    }

    fn emit_cleanups_down_to(&mut self, start: usize) -> Result<()> {
        let saved = self.cleanup_scopes.split_off(start);
        let saved_locks = self.held_locks.clone();
        let mut res = Ok(());
        for frame in saved.iter().rev() {
            if let Err(e) = self.run_frame_cleanups(frame) {
                res = Err(e);
                break;
            }
        }
        self.held_locks = saved_locks;
        self.cleanup_scopes.extend(saved);
        res
    }

    fn emit_all_cleanups(&mut self) -> Result<()> {
        let floor = self.fn_scope_floor();
        self.emit_cleanups_down_to(floor)
    }

    fn emit_loop_cleanups(&mut self, span: Span, is_continue: bool) -> Result<()> {
        let floor = self.fn_scope_floor();
        let loop_idx = self.cleanup_scopes[floor..]
            .iter()
            .rposition(|f| f.is_loop_body)
            .map(|rel| floor + rel)
            .ok_or_else(|| {
                Error::at(
                    span,
                    if self.in_defer() {
                        if is_continue {
                            "'continue' cannot exit out of a 'defer' block"
                        } else {
                            "'break' cannot exit out of a 'defer' block"
                        }
                    } else if is_continue {
                        "'continue' outside of a loop"
                    } else {
                        "'break' outside of a loop"
                    },
                )
            })?;
        if self.cleanup_scopes[loop_idx].owner_defer_id != self.current_defer_id {
            return Err(Error::at(
                span,
                if is_continue {
                    "'continue' cannot exit out of a 'defer' block"
                } else {
                    "'break' cannot exit out of a 'defer' block"
                },
            ));
        }
        self.emit_cleanups_down_to(loop_idx)?;
        if is_continue && let Some(counter) = self.cleanup_scopes[loop_idx].repeat_counter.clone() {
            self.line(format!("{counter} += 1;"));
        }
        Ok(())
    }

    fn emit_cancel_propagation(&mut self) -> Result<()> {
        if self.in_defer() {
            return Ok(());
        }
        let Some((_, flag)) = self.enclosing_cancel_flag() else {
            return Ok(());
        };
        let target_while = self
            .nearest_air_while_idx()
            .expect("enclosing cancel_flag implies an enclosing air while loop");
        self.line(format!("if {flag} != 0 {{"));
        self.indent += 1;
        self.emit_cleanups_down_to(target_while)?;
        self.line("break;".to_owned());
        self.indent -= 1;
        self.line("}".to_owned());
        Ok(())
    }

    fn run_frame_cleanups(&mut self, frame: &ScopeFrame) -> Result<()> {
        // LIFO order within each lexical scope!
        for action in frame.cleanups.iter().rev() {
            match action {
                CleanupAction::Unlock(lval) => {
                    self.emit_lvalue_assign(lval, V2AssignOp::Set, "0")?;
                    if let Some(pos) = self.held_locks.iter().rposition(|x| x == lval) {
                        self.held_locks.remove(pos);
                    }
                }
                CleanupAction::Defer {
                    owner_module,
                    in_object_scope,
                    body,
                    saved_bindings,
                } => {
                    self.defer_generation += 1;
                    let prev_defer_id = self.current_defer_id.replace(self.defer_generation);
                    let prev_in_obj = self.in_object_scope;
                    let prev_vars = std::mem::replace(&mut self.var_scopes, saved_bindings.clone());
                    self.in_object_scope = *in_object_scope;
                    let res = self.with_owner_module(owner_module, |c| {
                        c.push_scope(ScopeFrame::block());
                        let r = c.lower_stmts(body);
                        let p = c.pop_scope_and_run_cleanups();
                        r.and(p)
                    });
                    self.current_defer_id = prev_defer_id;
                    self.in_object_scope = prev_in_obj;
                    self.var_scopes = prev_vars;
                    res?;
                }
            }
        }
        Ok(())
    }

    fn lower_stmts(&mut self, stmts: &[V2Stmt]) -> Result<()> {
        for stmt in stmts {
            self.lower_stmt(stmt)?;
        }
        Ok(())
    }

    fn lower_stmt(&mut self, stmt: &V2Stmt) -> Result<()> {
        match stmt {
            V2Stmt::Let {
                span,
                name,
                ty,
                init,
            } => {
                let air_name = self.fresh(&format!("v_{name}"));
                let inferred_ty = if is_compound_arith_or_neg(init) {
                    self.lower_arith_into_dest(init, &air_name, true)?
                } else if let Some((val_air, inferred)) = self.try_lower_callable_value_raw(init)? {
                    self.line(format!("let {air_name} = {val_air};"));
                    inferred
                } else {
                    let (val_air, inferred) = self.lower_value_expr(init)?;
                    self.line(format!("let {air_name} = {val_air};"));
                    inferred
                };
                let final_ty = if let Some(ann) = ty {
                    let expected = self.env.resolve_value_type(
                        ann,
                        &self.owner_module,
                        self.in_object_scope,
                        *span,
                    )?;
                    if !types_compatible_for_assign(&expected, &inferred_ty) {
                        return Err(Error::at(
                            *span,
                            format!(
                                "cannot initialize '{}' variable '{name}' from {} expression",
                                self.env.display_value_type(&expected),
                                self.env.display_value_type(&inferred_ty)
                            ),
                        ));
                    }
                    expected
                } else {
                    inferred_ty
                };
                self.bind_var(
                    name.clone(),
                    Binding::Local {
                        air_name,
                        ty: final_ty,
                    },
                );
                Ok(())
            }
            V2Stmt::LetSome {
                span,
                name,
                ty,
                init,
                else_body,
            } => self.lower_let_some(*span, name, ty.as_ref(), init, else_body),
            V2Stmt::LetUninit { span, name, ty } => {
                let resolved = self.env.resolve_value_type(
                    ty,
                    &self.owner_module,
                    self.in_object_scope,
                    *span,
                )?;
                let air_name = self.fresh(&format!("v_{name}"));
                self.line(format!("let {air_name};"));
                self.bind_var(
                    name.clone(),
                    Binding::Local {
                        air_name,
                        ty: resolved,
                    },
                );
                Ok(())
            }
            V2Stmt::Animate {
                span: _,
                args,
                event,
                body,
            } => self.lower_animate_stmt(args, event.as_deref(), body),
            V2Stmt::Assign {
                span,
                target,
                op,
                value,
            } => {
                let lval = self.resolve_lvalue(target)?;
                let target_ty = lval.value_type().clone();
                // Handle bitflags `flag_set` / `flag_clear` explicitly (1-based bit index!).
                if matches!(op, V2AssignOp::FlagSet | V2AssignOp::FlagClear)
                    && let ValueType::Bitflags(ref key) = target_ty
                {
                    let Some((_, Some(bit))) = self.try_eval_const_bitflag(value, key)? else {
                        return Err(Error::at(
                            value.span(),
                            format!(
                                "'flag_set'/'flag_clear' on bitflags '{}' requires a single flag constant (use '.set(...)' / '.clear(...)' for masks)",
                                self.env.display_value_type(&target_ty)
                            ),
                        ));
                    };
                    return self.emit_lvalue_assign(&lval, *op, &bit.to_string());
                }
                // Compact in-place compound arithmetic lowering when target is Direct, alias-free,
                // side-effect-free, and cannot trap partway through!
                if *op == V2AssignOp::Set
                    && let LValue::Direct { ref air, .. } = lval
                    && is_compound_arith_or_neg(value)
                    && self.can_lower_arith_in_place(air, value, false)
                {
                    let val_ty = self.lower_arith_into_dest(value, air, false)?;
                    if !types_compatible_for_assign(&target_ty, &val_ty) {
                        return Err(Error::at(
                            *span,
                            format!(
                                "cannot assign '{}' expression to '{}' target",
                                self.env.display_value_type(&val_ty),
                                self.env.display_value_type(&target_ty)
                            ),
                        ));
                    }
                    return Ok(());
                }
                // A value-returning call into a plain (non-foreign) target reads `temp[N]` directly.
                let raw_call = if matches!(lval, LValue::Direct { .. }) {
                    self.try_lower_callable_value_raw(value)?
                } else {
                    None
                };
                let (val_air, val_ty) = match raw_call {
                    Some(r) => r,
                    None => self.lower_value_expr(value)?,
                };
                if *op != V2AssignOp::Set {
                    match &target_ty {
                        ValueType::Bool => {
                            return Err(Error::at(
                                *span,
                                "compound assignment is not allowed on a bool target",
                            ));
                        }
                        ValueType::Enum(_) | ValueType::ObjectRef(_) => {
                            return Err(Error::at(
                                *span,
                                format!(
                                    "compound assignment is not allowed on '{}' target",
                                    self.env.display_value_type(&target_ty)
                                ),
                            ));
                        }
                        ValueType::Bitflags(_) => {
                            if !matches!(
                                op,
                                V2AssignOp::BitAnd | V2AssignOp::BitOr | V2AssignOp::BitXor
                            ) {
                                return Err(Error::at(
                                    *span,
                                    format!(
                                        "arithmetic assignment is not allowed on bitflags '{}' (use |=, &=, ^=, .set(), or .clear())",
                                        self.env.display_value_type(&target_ty)
                                    ),
                                ));
                            }
                        }
                        ValueType::I16 | ValueType::ObjectId => {}
                    }
                }
                if !types_compatible_for_assign(&target_ty, &val_ty) {
                    return Err(Error::at(
                        *span,
                        format!(
                            "cannot assign '{}' expression to '{}' target",
                            self.env.display_value_type(&val_ty),
                            self.env.display_value_type(&target_ty)
                        ),
                    ));
                }
                self.emit_lvalue_assign(&lval, *op, &val_air)
            }
            V2Stmt::WithLock {
                span,
                lock_expr,
                body,
            } => {
                let lval = self.resolve_lvalue(lock_expr)?;
                if *lval.value_type() != ValueType::Bool {
                    return Err(Error::at(
                        *span,
                        "with lock(...) requires a bool state or &mut bool target",
                    ));
                }
                if self.held_locks.contains(&lval) {
                    return Err(Error::at(
                        *span,
                        "re-entrant 'with lock(...)' on an already-locked target in the same scope chain",
                    ));
                }
                self.emit_lvalue_assign(&lval, V2AssignOp::Set, "1")?;
                self.held_locks.push(lval.clone());
                self.push_scope(ScopeFrame {
                    cleanups: vec![CleanupAction::Unlock(lval)],
                    is_loop_body: false,
                    is_fn_root: false,
                    is_air_while: false,
                    owner_defer_id: self.current_defer_id,
                    cancel_flag: None,
                    repeat_counter: None,
                });
                let res = self.lower_stmts(body);
                let pop_res = if stmts_always_return(
                    body,
                    self.env,
                    &self.owner_module,
                    self.in_object_scope,
                ) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
                res.and(pop_res)
            }
            V2Stmt::Defer { span, body } => {
                if self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "nested 'defer' inside another 'defer' block is not allowed",
                    ));
                }
                let saved_bindings = self.var_scopes.clone();
                let owner_module = self.owner_module.clone();
                let in_object_scope = self.in_object_scope;
                if let Some(frame) = self.cleanup_scopes.last_mut() {
                    frame.cleanups.push(CleanupAction::Defer {
                        owner_module,
                        in_object_scope,
                        body: body.clone(),
                        saved_bindings,
                    });
                }
                Ok(())
            }
            V2Stmt::Repeat {
                span,
                count,
                cancel_on_abort,
                body,
            } => {
                if *cancel_on_abort && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' requires a Sim interaction context (not available in init/main)",
                    ));
                }
                if *cancel_on_abort && self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' is not allowed inside a 'defer' block",
                    ));
                }
                let (limit_air, _) = self.lower_value_expr(count)?;
                let cancel_flag = if *cancel_on_abort {
                    let f = self.fresh("abort");
                    self.line(format!("let {f} = 0;"));
                    Some(f)
                } else {
                    None
                };
                let counter = self.fresh("rep");
                self.line(format!("let {counter} = 0;"));
                self.line(format!("while {counter} < {limit_air} {{"));
                self.indent += 1;
                self.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: true,
                    is_fn_root: false,
                    is_air_while: true,
                    owner_defer_id: self.current_defer_id,
                    cancel_flag: cancel_flag.clone(),
                    repeat_counter: Some(counter.clone()),
                });
                if let Some(flag) = &cancel_flag {
                    self.line("if my.person.priority == 0 {".to_owned());
                    self.indent += 1;
                    self.line(format!("{flag} := 1;"));
                    self.line("break;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                let res = self.lower_stmts(body);
                let pop_res = self.pop_scope_and_run_cleanups();
                self.line(format!("{counter} += 1;"));
                self.indent -= 1;
                self.line("}".to_owned());
                res.and(pop_res)?;
                if let Some(inner_flag) = &cancel_flag
                    && let Some((_, outer_flag)) = self.enclosing_cancel_flag()
                {
                    self.line(format!("if {inner_flag} != 0 {{"));
                    self.indent += 1;
                    self.line(format!("{outer_flag} := 1;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                self.emit_inline_fn_return_propagation()?;
                self.emit_cancel_propagation()?;
                Ok(())
            }
            V2Stmt::While {
                span,
                cond,
                cancel_on_abort,
                body,
            } => {
                if *cancel_on_abort && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' requires a Sim interaction context (not available in init/main)",
                    ));
                }
                if *cancel_on_abort && self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' is not allowed inside a 'defer' block",
                    ));
                }
                let cancel_flag = if *cancel_on_abort {
                    let f = self.fresh("abort");
                    self.line(format!("let {f} = 0;"));
                    Some(f)
                } else {
                    None
                };
                // `loop { }` / `while true { }` lower to `.air` `while true`, which has no
                // condition node at all.
                let header = if self.try_eval_const_bool_inner(cond, true) == Some(true) {
                    Some("true".to_owned())
                } else if self.is_pure_simple_air_cond(cond) {
                    self.try_lower_simple_air_cond(cond)?
                } else {
                    None
                };
                if let Some(simple_cond) = header {
                    self.line(format!("while {simple_cond} {{"));
                    self.indent += 1;
                    self.push_scope(ScopeFrame {
                        cleanups: Vec::new(),
                        is_loop_body: true,
                        is_fn_root: false,
                        is_air_while: true,
                        owner_defer_id: self.current_defer_id,
                        cancel_flag: cancel_flag.clone(),
                        repeat_counter: None,
                    });
                    if let Some(flag) = &cancel_flag {
                        self.line("if my.person.priority == 0 {".to_owned());
                        self.indent += 1;
                        self.line(format!("{flag} := 1;"));
                        self.line("break;".to_owned());
                        self.indent -= 1;
                        self.line("}".to_owned());
                    }
                    let res = self.lower_stmts(body);
                    let pop_res = self.pop_scope_and_run_cleanups();
                    self.indent -= 1;
                    self.line("}".to_owned());
                    res.and(pop_res)?;
                } else {
                    self.line("while true {".to_owned());
                    self.indent += 1;
                    self.push_scope(ScopeFrame {
                        cleanups: Vec::new(),
                        is_loop_body: true,
                        is_fn_root: false,
                        is_air_while: true,
                        owner_defer_id: self.current_defer_id,
                        cancel_flag: cancel_flag.clone(),
                        repeat_counter: None,
                    });
                    if let Some(flag) = &cancel_flag {
                        self.line("if my.person.priority == 0 {".to_owned());
                        self.indent += 1;
                        self.line(format!("{flag} := 1;"));
                        self.line("break;".to_owned());
                        self.indent -= 1;
                        self.line("}".to_owned());
                    }
                    let cond_loc = self.fresh("wcond");
                    self.line(format!("let {cond_loc} = 0;"));
                    let cond_res = self.emit_bool_into_local(cond, &cond_loc);
                    self.line(format!("if {cond_loc} == 0 {{"));
                    self.indent += 1;
                    self.line("break;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    let body_res = cond_res.and_then(|()| self.lower_stmts(body));
                    let pop_res = self.pop_scope_and_run_cleanups();
                    self.indent -= 1;
                    self.line("}".to_owned());
                    body_res.and(pop_res)?;
                }
                if let Some(inner_flag) = &cancel_flag
                    && let Some((_, outer_flag)) = self.enclosing_cancel_flag()
                {
                    self.line(format!("if {inner_flag} != 0 {{"));
                    self.indent += 1;
                    self.line(format!("{outer_flag} := 1;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                self.emit_inline_fn_return_propagation()?;
                self.emit_cancel_propagation()?;
                Ok(())
            }
            V2Stmt::ForEach {
                span: _,
                binding,
                iter,
                body,
            } => {
                if let ForEachIter::Range {
                    start,
                    end,
                    inclusive,
                    rev,
                } = iter
                {
                    return self.lower_range_for(binding, start, end, *inclusive, *rev, body);
                }
                let iter_var = self.fresh(&format!("v_{binding}"));
                let mut parts_root_opt = None;
                let binding_ty = match iter {
                    ForEachIter::Range { .. } => unreachable!("handled above"),
                    ForEachIter::ObjectsByGuid(guid_expr) => {
                        let guid = self.resolve_guid_expr(guid_expr)?;
                        self.line(format!("let {iter_var} = 0;"));
                        self.line(format!(
                            "while set_to_next(guid, {guid:#010x}, dest: {iter_var}) {{"
                        ));
                        ValueType::ObjectId
                    }
                    ForEachIter::LotSims | ForEachIter::LotObjects => {
                        let mode = if matches!(iter, ForEachIter::LotSims) {
                            "person"
                        } else {
                            "all"
                        };
                        self.line(format!("let {iter_var} = 0;"));
                        self.line(format!("while set_to_next({mode}, dest: {iter_var}) {{"));
                        ValueType::ObjectId
                    }
                    ForEachIter::ObjectsOf { span, object_type } => {
                        let schema = self
                            .env
                            .lookup_object_schema(
                                &self.owner_module,
                                object_type,
                                self.in_object_scope,
                            )
                            .ok_or_else(|| {
                                Error::at(
                                    *span,
                                    format!(
                                        "unknown object type '{object_type}' in 'objects_of::<{object_type}>()' (import it with 'use ...::{object_type};')"
                                    ),
                                )
                            })?;
                        let guid = schema.lead_guid;
                        let key = schema.canonical_key.clone();
                        self.line(format!("let {iter_var} = 0;"));
                        self.line(format!(
                            "while set_to_next(guid, {guid:#010x}, dest: {iter_var}) {{"
                        ));
                        ValueType::ObjectRef(key)
                    }
                    ForEachIter::SelfParts => {
                        let root_expr = match self.tree_ctx {
                            TreeContext::ObjectSelf => {
                                let root_loc = self.fresh("parts_root");
                                self.line(format!("let {root_loc} = my.id;"));
                                root_loc
                            }
                            TreeContext::SimOnObject { .. } => {
                                if self.has_self_obj_local {
                                    "__self_obj".to_owned()
                                } else {
                                    let root_loc = self.fresh("parts_root");
                                    self.line(format!("let {root_loc} = stack_obj;"));
                                    root_loc
                                }
                            }
                        };
                        self.line(format!("let {iter_var} = {root_expr};"));
                        self.line(format!("while set_to_next(part, dest: {iter_var}) {{"));
                        parts_root_opt = Some(root_expr);
                        ValueType::ObjectRef(self.env.self_object_key.clone())
                    }
                };
                self.indent += 1;
                if let Some(root_expr) = parts_root_opt {
                    self.line(format!("if {iter_var} == {root_expr} {{"));
                    self.indent += 1;
                    self.line("break;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                self.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: true,
                    is_fn_root: false,
                    is_air_while: true,
                    owner_defer_id: self.current_defer_id,
                    cancel_flag: None,
                    repeat_counter: None,
                });
                self.bind_var(
                    binding.clone(),
                    Binding::Local {
                        air_name: iter_var,
                        ty: binding_ty,
                    },
                );
                let res = self.lower_stmts(body);
                let pop_res = self.pop_scope_and_run_cleanups();
                self.indent -= 1;
                self.line("}".to_owned());
                self.emit_restore_stack_obj();
                res.and(pop_res)?;
                self.emit_inline_fn_return_propagation()?;
                self.emit_cancel_propagation()?;
                Ok(())
            }
            V2Stmt::If {
                span: _,
                cond,
                then_body,
                else_body,
            } => {
                // Constant-fold literal boolean conditions (e.g. from `bool` params!).
                if let Some(const_b) = self.try_eval_const_bool(cond) {
                    let chosen = if const_b { then_body } else { else_body };
                    self.push_scope(ScopeFrame::block());
                    let r = self.lower_stmts(chosen);
                    let p = self.pop_scope_and_run_cleanups();
                    return r.and(p);
                }
                // Literal comparisons (`kind == Menu::Meal` with a constant helper argument) fold
                // too; the untaken branch is still lowered (and discarded) so it stays checked.
                if let Some(const_b) = self.try_fold_literal_cond(cond) {
                    let (chosen, dead) = if const_b {
                        (then_body, else_body)
                    } else {
                        (else_body, then_body)
                    };
                    self.check_dead_branch(dead)?;
                    self.push_scope(ScopeFrame::block());
                    let r = self.lower_stmts(chosen);
                    let p = self.pop_scope_and_run_cleanups();
                    return r.and(p);
                }

                let cond_text = if let Some(simple) = self.try_lower_simple_air_cond(cond)? {
                    simple
                } else {
                    let c = self.fresh("cond");
                    self.line(format!("let {c} = 0;"));
                    self.emit_bool_into_local(cond, &c)?;
                    format!("{c} != 0")
                };

                self.line(format!("if {cond_text} {{"));
                self.indent += 1;
                self.push_scope(ScopeFrame::block());
                let r1 = self.lower_stmts(then_body);
                let p1 = self.pop_scope_and_run_cleanups();
                self.indent -= 1;
                if else_body.is_empty() {
                    self.line("}".to_owned());
                    r1.and(p1)
                } else {
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.push_scope(ScopeFrame::block());
                    let r2 = self.lower_stmts(else_body);
                    let p2 = self.pop_scope_and_run_cleanups();
                    self.indent -= 1;
                    self.line("}".to_owned());
                    r1.and(p1).and(r2).and(p2)
                }
            }
            V2Stmt::Match {
                span,
                scrutinee,
                arms,
            } => self.lower_match(*span, scrutinee, arms),
            V2Stmt::Break { span } => {
                self.emit_loop_cleanups(*span, false)?;
                self.line("break;".to_owned());
                Ok(())
            }
            V2Stmt::Continue { span } => {
                self.emit_loop_cleanups(*span, true)?;
                self.line("continue;".to_owned());
                Ok(())
            }
            V2Stmt::Return { span, value } => {
                if let Some(active_fn) = self.active_inline_fn.as_ref() {
                    if active_fn.owner_defer_id != self.current_defer_id {
                        return Err(Error::at(
                            *span,
                            "'return' cannot exit out of a 'defer' block",
                        ));
                    }
                    let ret_flag = active_fn.ret_flag.clone();
                    let ret_val = active_fn.ret_val.clone();
                    let ret_kind = active_fn.ret_kind.clone();
                    match (ret_kind, value) {
                        (FnReturnKind::Void, Some(_)) => {
                            return Err(Error::at(
                                *span,
                                "void helper function cannot return a value",
                            ));
                        }
                        (FnReturnKind::Bool | FnReturnKind::Value(_), None) => {
                            return Err(Error::at(
                                *span,
                                "helper function with return type must return a value",
                            ));
                        }
                        (FnReturnKind::Void, None) => {}
                        (FnReturnKind::Bool, Some(expr)) => {
                            self.emit_bool_into_local(expr, &ret_val)?;
                        }
                        (FnReturnKind::Value(expected_ty), Some(expr)) => {
                            if is_compound_arith_or_neg(expr)
                                && self.can_lower_arith_in_place(&ret_val, expr, true)
                            {
                                let val_ty = self.lower_arith_into_dest(expr, &ret_val, false)?;
                                if !types_compatible_for_assign(&expected_ty, &val_ty) {
                                    return Err(Error::at(
                                        *span,
                                        format!(
                                            "helper function returning '{}' cannot return '{}' expression",
                                            self.env.display_value_type(&expected_ty),
                                            self.env.display_value_type(&val_ty)
                                        ),
                                    ));
                                }
                            } else {
                                let (val_air, val_ty) = self.lower_value_expr(expr)?;
                                if !types_compatible_for_assign(&expected_ty, &val_ty) {
                                    return Err(Error::at(
                                        *span,
                                        format!(
                                            "helper function returning '{}' cannot return '{}' expression",
                                            self.env.display_value_type(&expected_ty),
                                            self.env.display_value_type(&val_ty)
                                        ),
                                    ));
                                }
                                self.line(format!("{ret_val} := {val_air};"));
                            }
                        }
                    }
                    self.emit_all_cleanups()?;
                    self.line(format!("{ret_flag} := 1;"));
                    self.line("break;".to_owned());
                    return Ok(());
                }
                if self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "'return' cannot exit out of a 'defer' block",
                    ));
                }

                // Value-returning `functional kind(...) -> T` tree / `tree ... -> T @ temp[N]`:
                // `return None;` / `return false;` (functionals, non-bool T) exits with `return false;`
                // `return Some(expr);` / `return expr;` writes `temp[N] := val;` and exits `return true;`
                if let Some(abi) = self.value_return.clone() {
                    return self.lower_value_return(*span, value.as_ref(), &abi);
                }
                match value {
                    None => {
                        self.emit_all_cleanups()?;
                        self.line("return true;".to_owned());
                        Ok(())
                    }
                    Some(expr) => {
                        if !self.has_active_cleanups() {
                            self.emit_return_bool_expr(expr)
                        } else {
                            // Evaluate return expression BEFORE running cleanups (so cleanups cannot
                            // clobber values read by the return expression), then run LIFO cleanups!
                            if let Some(const_b) = self.try_eval_const_bool(expr) {
                                self.emit_all_cleanups()?;
                                self.line(if const_b {
                                    "return true;".to_owned()
                                } else {
                                    "return false;".to_owned()
                                });
                                Ok(())
                            } else {
                                let ret_loc = self.fresh("ret");
                                self.line(format!("let {ret_loc} = 0;"));
                                self.emit_bool_into_local(expr, &ret_loc)?;
                                self.emit_all_cleanups()?;
                                self.line(format!("if {ret_loc} != 0 {{"));
                                self.indent += 1;
                                self.line("return true;".to_owned());
                                self.indent -= 1;
                                self.line("} else {".to_owned());
                                self.indent += 1;
                                self.line("return false;".to_owned());
                                self.indent -= 1;
                                self.line("}".to_owned());
                                Ok(())
                            }
                        }
                    }
                }
            }
            V2Stmt::ReturnError { span, from_try } => {
                // A written `return error;` with `error` bound as a local/param/i16 or bool param
                // keeps its pre-v2-interop meaning (returns that value). `cond?;` always errors.
                if !*from_try
                    && (self.lookup_var("error").is_some()
                        || self.env.i16_params.contains_key("error")
                        || self.env.bool_params.contains_key("error"))
                {
                    return self.lower_stmt(&V2Stmt::Return {
                        span: *span,
                        value: Some(V2Expr::Ident(*span, "error".to_owned())),
                    });
                }
                if self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "'return error;' cannot exit out of a 'defer' block",
                    ));
                }
                // Exits the whole BHAV (even from an inlined helper), so every active cleanup runs.
                self.emit_cleanups_down_to(0)?;
                self.line("return error;".to_owned());
                Ok(())
            }
            V2Stmt::Expr { span: _, expr } => self.lower_expr_stmt(expr),
            V2Stmt::RawAir { span, code } => {
                if self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "'raw { ... }' is not allowed inside a 'defer' block",
                    ));
                }
                if self.has_any_active_cleanups()
                    || !self.inline_call_stack.is_empty()
                    || self.enclosing_cancel_flag().is_some()
                {
                    let wrapped = format!("tree 4096 \"__raw_check\" {{\n{code}\n}}\n");
                    let parsed = crate::parser::parse_module(&wrapped).map_err(|e| {
                        Error::at(
                            *span,
                            format!("invalid .air syntax in raw block: {}", e.message),
                        )
                    })?;
                    let stmts = match parsed.items.first() {
                        Some(crate::ast::Item::Tree(t)) => &t.body,
                        _ => unreachable!("synthetic wrapper always has one tree"),
                    };
                    let mut defined_labels = BTreeSet::new();
                    collect_raw_air_labels(stmts, &mut defined_labels);
                    validate_raw_air_stmts(stmts, false, &defined_labels, *span)?;
                }
                for raw_line in code.lines() {
                    let trimmed = raw_line.trim();
                    if !trimmed.is_empty() {
                        self.line(trimmed.to_owned());
                    }
                }
                Ok(())
            }
        }
    }

    /// `for i in a..b` → `i := a - 1; while i inc_lt b { ... }` and `(a..b).rev()` →
    /// `i := b; while i dec_gt a - 1 { ... }` (one node per iteration). Non-literal bounds are
    /// read once, before the loop; the loop variable is read-only.
    fn lower_range_for(
        &mut self,
        binding: &str,
        start: &V2Expr,
        end: &V2Expr,
        inclusive: bool,
        rev: bool,
        body: &[V2Stmt],
    ) -> Result<()> {
        if stmts_mutate_or_borrow_ident(body, binding) {
            return Err(Error::at(
                start.span(),
                format!("the range loop variable '{binding}' is read-only"),
            ));
        }
        // `start` is evaluated before `end` (Rust's order). Each bound becomes either a literal
        // (adjusted at compile time) or a fresh local (adjusted at run time).
        let bound = |c: &mut Self, expr: &V2Expr, delta: i16| -> Result<String> {
            if let Some((air, v)) = c.try_const_i16_air_operand(expr)?
                && air.parse::<i16>().is_ok()
            {
                return v
                    .checked_add(delta)
                    .map(|n| n.to_string())
                    .ok_or_else(|| Error::at(expr.span(), "range bound overflows i16"));
            }
            let (air, ty) = c.lower_value_expr(expr)?;
            if ty != ValueType::I16 {
                return Err(Error::at(
                    expr.span(),
                    format!(
                        "range bounds must be i16, found '{}'",
                        c.env.display_value_type(&ty)
                    ),
                ));
            }
            let local = c.fresh("range");
            c.line(format!("let {local} = {air};"));
            match delta {
                0 => {}
                d if d > 0 => c.line(format!("{local} += {d};")),
                d => c.line(format!("{local} -= {};", -d)),
            }
            Ok(local)
        };
        let iter_var = self.fresh(&format!("v_{binding}"));
        if rev {
            // Counts down from the last value; the cursor starts one past it.
            let lo = bound(self, start, -1)?;
            let hi = bound(self, end, i16::from(inclusive))?;
            self.line(format!("let {iter_var} = {hi};"));
            self.line(format!("while {iter_var} dec_gt {lo} {{"));
        } else {
            let lo = bound(self, start, -1)?;
            let hi = bound(self, end, i16::from(inclusive))?;
            self.line(format!("let {iter_var} = {lo};"));
            self.line(format!("while {iter_var} inc_lt {hi} {{"));
        }
        self.indent += 1;
        self.push_scope(ScopeFrame {
            cleanups: Vec::new(),
            is_loop_body: true,
            is_fn_root: false,
            is_air_while: true,
            owner_defer_id: self.current_defer_id,
            cancel_flag: None,
            repeat_counter: None,
        });
        self.bind_var(
            binding.to_owned(),
            Binding::Local {
                air_name: iter_var,
                ty: ValueType::I16,
            },
        );
        let res = self.lower_stmts(body);
        let pop_res = self.pop_scope_and_run_cleanups();
        self.indent -= 1;
        self.line("}".to_owned());
        res.and(pop_res)?;
        self.emit_inline_fn_return_propagation()?;
        self.emit_cancel_propagation()?;
        Ok(())
    }

    fn emit_inline_fn_return_propagation(&mut self) -> Result<()> {
        if let Some(active) = self.active_inline_fn.as_ref() {
            let ret_flag = active.ret_flag.clone();
            // If we're still inside an outer loop inside the inline fn (Wait: the inline fn's
            // own wrapper loop is at `fn_scope_depth`!), breaking out of an inner loop needs to
            // propagate upward until we exit the wrapper loop.
            self.line(format!("if {ret_flag} != 0 {{"));
            self.indent += 1;
            self.line("break;".to_owned());
            self.indent -= 1;
            self.line("}".to_owned());
        }
        Ok(())
    }

    fn lower_match(&mut self, span: Span, scrutinee: &V2Expr, arms: &[MatchArm]) -> Result<()> {
        if let V2Expr::Call {
            span: cspan,
            callee,
            type_args,
            args,
        } = scrutinee
            && type_args.is_empty()
            && matches!(callee.as_ref(), V2Expr::Ident(_, fname) if fname == "dialog")
        {
            return self.lower_match_dialog(span, *cspan, args, arms);
        }
        self.lower_match_value(span, scrutinee, arms)
    }

    fn lower_match_dialog(
        &mut self,
        match_span: Span,
        call_span: Span,
        args: &[CallArg],
        arms: &[MatchArm],
    ) -> Result<()> {
        let opts = self.parse_dialog_options(args, call_span, true)?;
        if (opts.effective_flags & 1) != 0 {
            return Err(Error::at(
                call_span,
                "'match dialog(...)' cannot set the inform bit ('inform: true' or 'flags' bit 0), because informational dialogs do not wait for a button choice",
            ));
        }

        let arms_have_third = arms.iter().any(|a| {
            matches!(
                &a.pattern,
                MatchPattern::Ident(_, n) if matches!(n.as_str(), "Button3" | "Cancel")
            ) || matches!(&a.pattern, MatchPattern::Int(_, 2))
        });
        let is_three_way = match opts.kind_num {
            Some(1) => false,
            Some(2) => true,
            Some(other) => {
                return Err(Error::at(
                    call_span,
                    format!(
                        "'match dialog(...)' only supports 2-button ('yes_no') or 3-button ('yes_no_cancel') dialogs, found kind {other}"
                    ),
                ));
            }
            None => opts.button_count == 3 || arms_have_third,
        };
        if let Some(tuple_len) = opts.buttons_tuple_len {
            let expected_len = if is_three_way { 3 } else { 2 };
            if tuple_len != expected_len {
                return Err(Error::at(
                    call_span,
                    format!(
                        "dialog 'buttons:' tuple has {tuple_len} entries, but {} dialog requires exactly {expected_len}",
                        if is_three_way {
                            "'yes_no_cancel'"
                        } else {
                            "'yes_no'"
                        }
                    ),
                ));
            }
        }

        // Map arm patterns to button indices: 0 (Button1/Yes/0), 1 (Button2/No/1), 2 (Button3/Cancel/2).
        let mut arm_bodies: [Option<&[V2Stmt]>; 3] = [None, None, None];
        let mut wildcard_body: Option<&[V2Stmt]> = None;
        for arm in arms {
            if wildcard_body.is_some() {
                return Err(Error::at(
                    arm.span,
                    "unreachable arm after '_' wildcard in 'match dialog'",
                ));
            }
            let slot: Option<usize> = match &arm.pattern {
                MatchPattern::Wildcard(_) => {
                    wildcard_body = Some(arm.body.as_slice());
                    None
                }
                MatchPattern::Range(pspan, ..) => {
                    return Err(Error::at(
                        *pspan,
                        "range patterns cannot match a 'dialog(...)' result",
                    ));
                }
                MatchPattern::Ident(pspan, name) => match name.as_str() {
                    "Button1" | "Yes" => Some(0),
                    "Button2" | "No" => Some(1),
                    "Button3" | "Cancel" => {
                        if !is_three_way {
                            return Err(Error::at(
                                *pspan,
                                format!(
                                    "arm '{name}' is invalid for 2-button 'yes_no' dialog (expected Button1/Yes or Button2/No)"
                                ),
                            ));
                        }
                        Some(2)
                    }
                    other => {
                        return Err(Error::at(
                            *pspan,
                            format!(
                                "invalid 'match dialog' arm '{other}' (expected Button1/Yes, Button2/No{}{})",
                                if is_three_way { ", Button3/Cancel" } else { "" },
                                ", 0/1/2, or _"
                            ),
                        ));
                    }
                },
                MatchPattern::Int(pspan, v) => match *v {
                    0 => Some(0),
                    1 => Some(1),
                    2 if is_three_way => Some(2),
                    other => {
                        return Err(Error::at(
                            *pspan,
                            format!(
                                "invalid 'match dialog' button index {other} (expected {})",
                                if is_three_way { "0, 1, or 2" } else { "0 or 1" }
                            ),
                        ));
                    }
                },
                MatchPattern::Path(pspan, segs) => {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "unexpected path pattern '{}' in 'match dialog' (expected Button1/Button2/Button3, Yes/No/Cancel, 0/1/2, or _)",
                            segs.join("::")
                        ),
                    ));
                }
            };
            if let Some(idx) = slot {
                if arm_bodies[idx].is_some() {
                    return Err(Error::at(
                        arm.span,
                        format!("duplicate arm for button {} in 'match dialog'", idx + 1),
                    ));
                }
                arm_bodies[idx] = Some(arm.body.as_slice());
            }
        }

        let required = if is_three_way { 3 } else { 2 };
        for (i, slot) in arm_bodies.iter_mut().enumerate().take(required) {
            if slot.is_none() {
                if let Some(wc) = wildcard_body {
                    *slot = Some(wc);
                } else {
                    return Err(Error::at(
                        match_span,
                        format!(
                            "non-exhaustive 'match dialog': missing arm for Button{} (add the missing arm or a '_' fallback)",
                            i + 1
                        ),
                    ));
                }
            }
        }

        // Build effective dialog() args (injecting kind: and temp: 0 for 3-way dialogs if omitted).
        let mut effective_args: Vec<CallArg> = args.to_vec();
        if opts.kind_num.is_none() {
            effective_args.insert(
                0,
                CallArg {
                    span: call_span,
                    name: Some("kind".to_owned()),
                    value: V2Expr::Ident(
                        call_span,
                        if is_three_way {
                            "yes_no_cancel".to_owned()
                        } else {
                            "yes_no".to_owned()
                        },
                    ),
                },
            );
        }
        let temp_slot = opts.effective_temp_slot;
        let save_implicit_temp = is_three_way && !opts.explicit_temp_specified;
        if save_implicit_temp {
            effective_args.push(CallArg {
                span: call_span,
                name: Some("temp".to_owned()),
                value: V2Expr::Int(call_span, i64::from(temp_slot)),
            });
        }

        let saved_temp_local = if save_implicit_temp {
            let s = self.fresh("dlg_t0");
            self.line(format!("let {s} = temp[{temp_slot}];"));
            Some(s)
        } else {
            None
        };

        let dlg_air = self.format_dialog(&effective_args, call_span)?;
        let body0 = arm_bodies[0].unwrap();
        self.line(format!("if {dlg_air} {{"));
        self.indent += 1;
        self.push_scope(ScopeFrame::block());
        let r0 = self.lower_stmts(body0);
        let p0 = if stmts_always_return(body0, self.env, &self.owner_module, self.in_object_scope) {
            self.pop_scope_discard_cleanups();
            Ok(())
        } else {
            self.pop_scope_and_run_cleanups()
        };
        self.indent -= 1;
        r0.and(p0)?;

        let body1 = arm_bodies[1].unwrap();
        if !is_three_way {
            self.line("} else {".to_owned());
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r1 = self.lower_stmts(body1);
            let p1 =
                if stmts_always_return(body1, self.env, &self.owner_module, self.in_object_scope) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
            self.indent -= 1;
            self.line("}".to_owned());
            r1.and(p1)
        } else if let Some(saved_t) = saved_temp_local {
            // Restore implicit temp[0] BEFORE executing Button2/Button3 user code or defers!
            self.line("} else {".to_owned());
            self.indent += 1;
            let choice = self.fresh("dlg_choice");
            self.line(format!("let {choice} = temp[{temp_slot}];"));
            self.line(format!("temp[{temp_slot}] := {saved_t};"));
            self.line(format!("if {choice} == 0 {{"));
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r1 = self.lower_stmts(body1);
            let p1 =
                if stmts_always_return(body1, self.env, &self.owner_module, self.in_object_scope) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
            self.indent -= 1;
            r1.and(p1)?;
            let body2 = arm_bodies[2].unwrap();
            self.line("} else {".to_owned());
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r2 = self.lower_stmts(body2);
            let p2 =
                if stmts_always_return(body2, self.env, &self.owner_module, self.in_object_scope) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
            self.indent -= 1;
            self.line("}".to_owned());
            self.indent -= 1;
            self.line("}".to_owned());
            r2.and(p2)
        } else {
            self.line(format!("}} else if temp[{temp_slot}] == 0 {{"));
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r1 = self.lower_stmts(body1);
            let p1 =
                if stmts_always_return(body1, self.env, &self.owner_module, self.in_object_scope) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
            self.indent -= 1;
            r1.and(p1)?;
            let body2 = arm_bodies[2].unwrap();
            self.line("} else {".to_owned());
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r2 = self.lower_stmts(body2);
            let p2 =
                if stmts_always_return(body2, self.env, &self.owner_module, self.in_object_scope) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
            self.indent -= 1;
            self.line("}".to_owned());
            r2.and(p2)
        }
    }

    fn lower_match_value(
        &mut self,
        span: Span,
        scrutinee: &V2Expr,
        arms: &[MatchArm],
    ) -> Result<()> {
        let (val_air, val_ty) = self.lower_value_expr(scrutinee)?;
        if !matches!(val_ty, ValueType::I16 | ValueType::Enum(_)) {
            return Err(Error::at(
                span,
                format!(
                    "'match' scrutinee must be 'dialog(...)', an enum, or 'i16', found '{}'",
                    self.env.display_value_type(&val_ty)
                ),
            ));
        }
        let scrut_local = self.fresh("match");
        self.line(format!("let {scrut_local} = {val_air};"));

        // Arms as inclusive ranges (`lo == hi` for a single value).
        let mut cases: Vec<(i16, i16, &[V2Stmt])> = Vec::new();
        let mut wildcard: Option<&[V2Stmt]> = None;
        let mut seen_vals = BTreeSet::new();

        for arm in arms {
            if wildcard.is_some() {
                return Err(Error::at(
                    arm.span,
                    "unreachable match arm after '_' wildcard",
                ));
            }
            match &arm.pattern {
                MatchPattern::Wildcard(_) => {
                    wildcard = Some(arm.body.as_slice());
                }
                MatchPattern::Int(pspan, v) => {
                    if val_ty != ValueType::I16 {
                        return Err(Error::at(
                            *pspan,
                            format!(
                                "integer pattern {v} cannot match '{}'",
                                self.env.display_value_type(&val_ty)
                            ),
                        ));
                    }
                    if !seen_vals.insert(*v)
                        || cases.iter().any(|&(lo, hi, _)| (lo..=hi).contains(v))
                    {
                        return Err(Error::at(
                            *pspan,
                            format!("duplicate match arm for value {v}"),
                        ));
                    }
                    cases.push((*v, *v, arm.body.as_slice()));
                }
                MatchPattern::Range(pspan, lo, hi) => {
                    if val_ty != ValueType::I16 {
                        return Err(Error::at(
                            *pspan,
                            format!(
                                "range pattern {lo}..={hi} cannot match '{}'",
                                self.env.display_value_type(&val_ty)
                            ),
                        ));
                    }
                    if cases.iter().any(|&(a, b, _)| a <= *hi && *lo <= b) {
                        return Err(Error::at(
                            *pspan,
                            format!("range pattern {lo}..={hi} overlaps an earlier match arm"),
                        ));
                    }
                    cases.push((*lo, *hi, arm.body.as_slice()));
                }
                MatchPattern::Path(pspan, segs) => {
                    let ValueType::Enum(ref expected_key) = val_ty else {
                        return Err(Error::at(
                            *pspan,
                            format!(
                                "enum pattern '{}' cannot match '{}'",
                                segs.join("::"),
                                self.env.display_value_type(&val_ty)
                            ),
                        ));
                    };
                    if segs.len() != 2 {
                        return Err(Error::at(
                            *pspan,
                            format!(
                                "expected 'Enum::Variant' pattern, found '{}'",
                                segs.join("::")
                            ),
                        ));
                    }
                    let e = self
                        .env
                        .lookup_enum(&self.owner_module, &segs[0], self.in_object_scope)
                        .ok_or_else(|| Error::at(*pspan, format!("unknown enum '{}'", segs[0])))?;
                    if &e.canonical_key != expected_key {
                        return Err(Error::at(
                            *pspan,
                            format!(
                                "mismatched enum pattern '{}': expected '{}'",
                                e.display_name,
                                self.env.display_value_type(&val_ty)
                            ),
                        ));
                    }
                    let Some(&v) = e.variants.get(&segs[1]) else {
                        return Err(Error::at(
                            *pspan,
                            format!("enum '{}' has no variant '{}'", e.display_name, segs[1]),
                        ));
                    };
                    if !seen_vals.insert(v) {
                        return Err(Error::at(
                            *pspan,
                            format!("duplicate match arm for '{}::{}'", e.display_name, segs[1]),
                        ));
                    }
                    cases.push((v, v, arm.body.as_slice()));
                }
                MatchPattern::Ident(pspan, name) => {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "unexpected identifier pattern '{name}' in match (use 'Enum::Variant', integer literal, or '_')"
                        ),
                    ));
                }
            }
        }

        // Enforce exhaustiveness for enum matches when no `_` fallback is provided.
        let mut exhaustive_enum_without_wildcard = false;
        if let ValueType::Enum(ref key) = val_ty
            && wildcard.is_none()
            && let Some(e) = self.env.all_enums.get(key)
        {
            for (vname, &vval) in &e.variants {
                if !seen_vals.contains(&vval) {
                    return Err(Error::at(
                        span,
                        format!(
                            "non-exhaustive match on enum '{}': missing variant '{}' (add the missing arm or a '_' fallback)",
                            e.display_name, vname
                        ),
                    ));
                }
            }
            exhaustive_enum_without_wildcard = true;
        }

        if let Some(wc) = wildcard
            && cases.is_empty()
        {
            self.push_scope(ScopeFrame::block());
            let r = self.lower_stmts(wc);
            let p = if stmts_always_return(wc, self.env, &self.owner_module, self.in_object_scope) {
                self.pop_scope_discard_cleanups();
                Ok(())
            } else {
                self.pop_scope_and_run_cleanups()
            };
            return r.and(p);
        }

        if exhaustive_enum_without_wildcard && cases.len() == 1 {
            let body = cases[0].2;
            self.push_scope(ScopeFrame::block());
            let r = self.lower_stmts(body);
            let p = if stmts_always_return(body, self.env, &self.owner_module, self.in_object_scope)
            {
                self.pop_scope_discard_cleanups();
                Ok(())
            } else {
                self.pop_scope_and_run_cleanups()
            };
            return r.and(p);
        }

        let last_idx = cases.len().saturating_sub(1);
        // A bounded range needs two compares: its test is computed into a flag inside the
        // previous arm's `else`, which nests (but never duplicates) the rest of the chain.
        let mut extra_closes = 0usize;
        for (idx, &(lo, hi, body)) in cases.iter().enumerate() {
            let single = if lo == hi {
                Some(format!("{scrut_local} == {lo}"))
            } else if lo == i16::MIN {
                Some(format!("{scrut_local} <= {hi}"))
            } else if hi == i16::MAX {
                Some(format!("{scrut_local} >= {lo}"))
            } else {
                None
            };
            match single {
                Some(cond) if idx == 0 => self.line(format!("if {cond} {{")),
                Some(_) if exhaustive_enum_without_wildcard && idx == last_idx => {
                    self.line("} else {".to_owned());
                }
                Some(cond) => self.line(format!("}} else if {cond} {{")),
                None => {
                    if idx > 0 {
                        self.line("} else {".to_owned());
                        self.indent += 1;
                        extra_closes += 1;
                    }
                    let flag = self.fresh("in_range");
                    self.line(format!("let {flag} = 0;"));
                    self.line(format!("if {scrut_local} >= {lo} {{"));
                    self.indent += 1;
                    self.line(format!("if {scrut_local} <= {hi} {{"));
                    self.indent += 1;
                    self.line(format!("{flag} := 1;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.line(format!("if {flag} != 0 {{"));
                }
            }
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r = self.lower_stmts(body);
            let p = if stmts_always_return(body, self.env, &self.owner_module, self.in_object_scope)
            {
                self.pop_scope_discard_cleanups();
                Ok(())
            } else {
                self.pop_scope_and_run_cleanups()
            };
            self.indent -= 1;
            r.and(p)?;
        }
        if let Some(wc) = wildcard {
            self.line("} else {".to_owned());
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r = self.lower_stmts(wc);
            let p = if stmts_always_return(wc, self.env, &self.owner_module, self.in_object_scope) {
                self.pop_scope_discard_cleanups();
                Ok(())
            } else {
                self.pop_scope_and_run_cleanups()
            };
            self.indent -= 1;
            self.line("}".to_owned());
            self.close_match_chain(extra_closes);
            r.and(p)
        } else {
            self.line("}".to_owned());
            self.close_match_chain(extra_closes);
            Ok(())
        }
    }

    fn close_match_chain(&mut self, extra_closes: usize) {
        for _ in 0..extra_closes {
            self.indent -= 1;
            self.line("}".to_owned());
        }
    }

    fn lower_let_some(
        &mut self,
        span: Span,
        name: &str,
        ty_ann: Option<&V2Type>,
        init: &V2Expr,
        else_body: &[V2Stmt],
    ) -> Result<()> {
        if !stmts_unconditionally_diverge(
            else_body,
            self.env,
            &self.owner_module,
            self.in_object_scope,
        ) {
            return Err(Error::at(
                span,
                format!(
                    "'let Some({name}) = ... else {{ ... }}' else-block must diverge (end with return, break, or continue)"
                ),
            ));
        }
        // Form 1: `let Some(val) = obj.run_functional(kind [, args...]) else { ... };`
        if let V2Expr::MethodCall {
            span: mspan,
            receiver,
            method,
            type_args,
            args,
        } = init
            && method == "run_functional"
        {
            let ret_ty = self.emit_run_functional_let_some(
                receiver, type_args, args, *mspan, name, ty_ann, else_body,
            )?;
            let air_name = self.fresh(&format!("v_{name}"));
            self.line(format!("let {air_name} = temp[0];"));
            self.emit_restore_stack_obj();
            self.bind_var(
                name.to_owned(),
                Binding::Local {
                    air_name,
                    ty: ret_ty,
                },
            );
            return Ok(());
        }

        // Form 2: `let Some(obj) = find_best_object[::<T>](...) | create_object[::<T>](...) else { ... };`
        let V2Expr::Call {
            callee,
            type_args,
            args,
            ..
        } = init
        else {
            return Err(Error::at(
                span,
                "'let Some(x) = ... else { ... }' requires find_best_object(...), create_object(...), or obj.run_functional(...)",
            ));
        };
        let V2Expr::Ident(_, fname) = callee.as_ref() else {
            return Err(Error::at(
                span,
                "'let Some(x) = ... else { ... }' requires find_best_object(...), create_object(...), or obj.run_functional(...)",
            ));
        };

        // Typed `find_best_object::<T>(kind [, flags: f])`: runtime-checked against T's GUIDs so
        // a higher-scoring foreign object can NEVER be bound as T, AND a lower-scoring T is still
        // found via fallback `set_to_next(guid, T::lead_guid)` scan!
        if fname == "find_best_object" && !type_args.is_empty() {
            let plan = self.parse_find_best_object(type_args, args, span)?;
            let schema = plan
                .typed_schema
                .expect("non-empty type_args yields schema");
            let inferred_ty = ValueType::ObjectRef(schema.canonical_key.clone());
            let final_ty = if let Some(ann) = ty_ann {
                let expected = self.env.resolve_value_type(
                    ann,
                    &self.owner_module,
                    self.in_object_scope,
                    span,
                )?;
                if !types_compatible_for_assign(&expected, &inferred_ty) {
                    return Err(Error::at(
                        span,
                        format!(
                            "'let Some({name}: {})' does not match '{}' result",
                            self.env.display_value_type(&expected),
                            self.env.display_value_type(&inferred_ty)
                        ),
                    ));
                }
                expected
            } else {
                inferred_ty
            };

            let found_var = self.fresh(&format!("v_{name}"));
            let raw_var = self.fresh("fbo_raw");
            let cand_var = self.fresh("fbo_cand");
            let score_field = match plan.kind_name.as_str() {
                "prep" => Some("stack.prep_score"),
                "cook" => Some("stack.cook_score"),
                _ => None,
            };
            let guard_summary = schema
                .functionals
                .get(&plan.kind_name)
                .map_or(FunctionalGuardSummary::Complex, |s| s.guard);
            let allow_fallback = !matches!(guard_summary, FunctionalGuardSummary::Complex);
            self.line(format!("let {found_var} = 0;"));
            self.line(format!("if {} {{", plan.prim_air));
            self.indent += 1;
            self.line(format!("let {raw_var} = stack_obj;"));
            self.line(format!("let {cand_var} = 0;"));
            let best_var = if allow_fallback && score_field.is_some() {
                let b = self.fresh("fbo_best");
                self.line(format!("let {b} = 0;"));
                Some(b)
            } else {
                None
            };
            self.line(format!(
                "while set_to_next(guid, {:#010x}, dest: {cand_var}) {{",
                schema.lead_guid
            ));
            self.indent += 1;
            self.line(format!("if {cand_var} == {raw_var} {{"));
            self.indent += 1;
            self.line(format!("{found_var} := {cand_var};"));
            self.line("break;".to_owned());
            self.indent -= 1;
            self.line("}".to_owned());
            if allow_fallback {
                self.line(format!("stack_obj := {cand_var};"));
                self.line("if stack.in_use == 0 {".to_owned());
                self.indent += 1;
                self.line("if !stack.data[8] flag_test 10 {".to_owned());
                self.indent += 1;
                let mut extra_blocks = 0usize;
                if (plan.flags & 4) != 0 {
                    self.line("if stack.room == my.room {".to_owned());
                    self.indent += 1;
                    extra_blocks += 1;
                }
                if let FunctionalGuardSummary::AttrBool {
                    attr_index,
                    expect_true,
                } = guard_summary
                {
                    let cond = if expect_true {
                        format!("stack.attr[{attr_index}] != 0")
                    } else {
                        format!("stack.attr[{attr_index}] == 0")
                    };
                    self.line(format!("if {cond} {{"));
                    self.indent += 1;
                    extra_blocks += 1;
                }
                if let (Some(sf), Some(bvar)) = (score_field, &best_var) {
                    self.line(format!("if {sf} > 0 {{"));
                    self.indent += 1;
                    self.line(format!("if {found_var} == 0 {{"));
                    self.indent += 1;
                    self.line(format!("{found_var} := {cand_var};"));
                    self.line(format!("{bvar} := {sf};"));
                    self.indent -= 1;
                    self.line(format!("}} else if {sf} > {bvar} {{"));
                    self.indent += 1;
                    self.line(format!("{found_var} := {cand_var};"));
                    self.line(format!("{bvar} := {sf};"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                } else {
                    self.line(format!("if {found_var} == 0 {{"));
                    self.indent += 1;
                    self.line(format!("{found_var} := {cand_var};"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                for _ in 0..extra_blocks {
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                self.indent -= 1;
                self.line("}".to_owned());
                self.indent -= 1;
                self.line("}".to_owned());
            }
            self.indent -= 1;
            self.line("}".to_owned());

            // Multi-tile object support: if the VM's winner was a non-lead tile of T, accept it too.
            for &tile_guid in &schema.tile_guids {
                if tile_guid == schema.lead_guid {
                    continue;
                }
                let tchk = self.fresh("fbo_tile");
                self.line(format!("if {found_var} != {raw_var} {{"));
                self.indent += 1;
                self.line(format!("let {tchk} = 0;"));
                self.line(format!(
                    "while set_to_next(guid, {tile_guid:#010x}, dest: {tchk}) {{"
                ));
                self.indent += 1;
                self.line(format!("if {tchk} == {raw_var} {{"));
                self.indent += 1;
                self.line(format!("{found_var} := {raw_var};"));
                self.line("break;".to_owned());
                self.indent -= 1;
                self.line("}".to_owned());
                self.indent -= 1;
                self.line("}".to_owned());
                self.indent -= 1;
                self.line("}".to_owned());
            }
            self.indent -= 1;
            self.line("}".to_owned());

            self.line(format!("if {found_var} == 0 {{"));
            self.indent += 1;
            self.emit_restore_stack_obj();
            self.push_scope(ScopeFrame::block());
            let r = self.lower_stmts(else_body);
            let p = self.pop_scope_and_run_cleanups();
            self.indent -= 1;
            self.line("}".to_owned());
            r.and(p)?;

            self.emit_restore_stack_obj();
            self.bind_var(
                name.to_owned(),
                Binding::Local {
                    air_name: found_var,
                    ty: final_ty,
                },
            );
            return Ok(());
        }

        let (prim_air, inferred_ty) = match fname.as_str() {
            "find_best_object" => {
                let plan = self.parse_find_best_object(type_args, args, span)?;
                (plan.prim_air, ValueType::ObjectId)
            }
            "create_object" => self.format_create_object(type_args, args, span)?,
            other => {
                return Err(Error::at(
                    span,
                    format!(
                        "'let Some({name}) = {other}(...)' is not supported (expected 'find_best_object', 'create_object', or 'obj.run_functional')"
                    ),
                ));
            }
        };
        let final_ty = if let Some(ann) = ty_ann {
            let expected =
                self.env
                    .resolve_value_type(ann, &self.owner_module, self.in_object_scope, span)?;
            if !types_compatible_for_assign(&expected, &inferred_ty) {
                return Err(Error::at(
                    span,
                    format!(
                        "'let Some({name}: {})' does not match '{}' result",
                        self.env.display_value_type(&expected),
                        self.env.display_value_type(&inferred_ty)
                    ),
                ));
            }
            expected
        } else {
            inferred_ty
        };

        self.line(format!("if !{prim_air} {{"));
        self.indent += 1;
        self.emit_restore_stack_obj();
        self.push_scope(ScopeFrame::block());
        let r = self.lower_stmts(else_body);
        let p = self.pop_scope_and_run_cleanups();
        self.indent -= 1;
        self.line("}".to_owned());
        r.and(p)?;

        let air_name = self.fresh(&format!("v_{name}"));
        self.line(format!("let {air_name} = stack_obj;"));
        self.emit_restore_stack_obj();
        self.bind_var(
            name.to_owned(),
            Binding::Local {
                air_name,
                ty: final_ty,
            },
        );
        Ok(())
    }

    fn lower_expr_stmt(&mut self, expr: &V2Expr) -> Result<()> {
        match expr {
            V2Expr::Try { span, inner } => {
                // Checked up front: the generic desugaring below may constant-fold its error
                // branch away before `return error;` would report it.
                if self.in_defer() {
                    return Err(Error::at(*span, "'?' cannot exit out of a 'defer' block"));
                }
                if let V2Expr::Call {
                    span: cspan,
                    callee,
                    type_args,
                    args,
                } = inner.as_ref()
                    && type_args.is_empty()
                    && let Some(sig) = self.resolve_callable(callee, *cspan)?
                {
                    return self.emit_callable_stmt(&sig, args, *cspan, true);
                }
                // Generic `cond?;` on a bool expression: `if !cond { return error; }`.
                let desugared = V2Stmt::If {
                    span: *span,
                    cond: V2Expr::Unary {
                        span: *span,
                        op: V2UnaryOp::Not,
                        inner: inner.clone(),
                    },
                    then_body: vec![V2Stmt::ReturnError {
                        span: *span,
                        from_try: true,
                    }],
                    else_body: Vec::new(),
                };
                self.lower_stmt(&desugared)
            }
            V2Expr::Call {
                span,
                callee,
                type_args,
                args,
            } => {
                if let V2Expr::Ident(_, fname) = callee.as_ref()
                    && fname == "create_object"
                {
                    // Both edges continue; the new object is dropped and `stack_obj` restored.
                    let (prim, _) = self.format_create_object(type_args, args, *span)?;
                    self.line(format!("{prim};"));
                    self.emit_restore_stack_obj();
                    return Ok(());
                }
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        "type arguments are only supported on 'create_object::<T>', 'find_best_object::<T>', and 'objects_of::<T>'",
                    ));
                }
                if let Some(sig) = self.resolve_callable(callee, *span)? {
                    return self.emit_callable_stmt(&sig, args, *span, false);
                }
                if let V2Expr::Ident(_, fname) = callee.as_ref() {
                    match fname.as_str() {
                        "sleep" | "idle" => return self.emit_sleep(args, *span),
                        "play_sound" => {
                            let s = self.format_play_sound(args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "stop_all_sounds" => {
                            let s = self.format_stop_all_sounds(args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "debit_funds" => {
                            let s = self.format_alter_budget(args, *span, false, false)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "credit_funds" => {
                            let s = self.format_alter_budget(args, *span, false, true)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "destroy_self" => {
                            let mut flags = 0u8;
                            for a in args {
                                let Some(aname) = &a.name else {
                                    return Err(Error::at(
                                        a.span,
                                        "destroy_self() only takes named options ('flags:', 'no_yield:', 'skip_resets:')",
                                    ));
                                };
                                match aname.as_str() {
                                    "flags" => {
                                        let v = self.eval_const_i16(&a.value)?;
                                        flags |= u8::try_from(v).map_err(|_| {
                                            Error::at(
                                                a.span,
                                                format!(
                                                    "destroy_self flags {v} out of range 0..=255"
                                                ),
                                            )
                                        })?;
                                    }
                                    "no_yield" => {
                                        if self.eval_const_bool_required(&a.value, a.span)? {
                                            flags |= 1;
                                        }
                                    }
                                    "skip_resets" => {
                                        if self.eval_const_bool_required(&a.value, a.span)? {
                                            flags |= 2;
                                        }
                                    }
                                    other => {
                                        return Err(Error::at(
                                            a.span,
                                            format!(
                                                "unknown destroy_self option '{other}' (expected 'flags', 'no_yield', or 'skip_resets')"
                                            ),
                                        ));
                                    }
                                }
                            }
                            let target = match self.tree_ctx {
                                TreeContext::ObjectSelf => "me",
                                TreeContext::SimOnObject { .. } => "stack_obj",
                            };
                            if flags == 0 {
                                self.line(format!("remove_object({target});"));
                            } else {
                                self.line(format!("remove_object({target}, flags: {flags});"));
                            }
                            return Ok(());
                        }
                        "set_graphic" => {
                            if args.len() != 1 || args[0].name.is_some() {
                                return Err(Error::at(
                                    *span,
                                    "set_graphic(index) expects exactly one positional argument",
                                ));
                            }
                            let (val, _) = self.lower_value_expr(&args[0].value)?;
                            let prefix = self.self_prefix();
                            self.line(format!("{prefix}.data[0] := {val};"));
                            return Ok(());
                        }
                        "route_to" => {
                            let s = self.format_slot_prim("route_to", args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "snap" => {
                            let s = self.format_slot_prim("snap", args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "set_motive_change" => {
                            let s = self.format_set_motive_change(args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "clear_motive_changes" => {
                            if !args.is_empty() {
                                return Err(Error::at(
                                    *span,
                                    "clear_motive_changes() takes no arguments",
                                ));
                            }
                            self.line("clear_motive_changes();".to_owned());
                            return Ok(());
                        }
                        "dialog" => {
                            let s = self.format_dialog(args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        _ => {
                            // User helper function call (discarding optional return value).
                            if let Some((fn_mod, fn_decl, is_local)) =
                                self.env
                                    .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                            {
                                let _ = self
                                    .inline_helper_call(&fn_mod, &fn_decl, is_local, args, *span)?;
                                return Ok(());
                            }
                            if let Some(prim) = self.format_prim_builtin(fname, args, *span)? {
                                self.line(format!("{prim};"));
                                return Ok(());
                            }
                            return Err(Error::at(*span, format!("unknown function '{fname}'")));
                        }
                    }
                }
                Err(Error::at(*span, "unsupported call target expression"))
            }
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } => match method.as_str() {
                "run_functional" => {
                    if !type_args.is_empty() {
                        return Err(Error::at(
                            *span,
                            "type arguments on run_functional are only valid inside 'let Some(x) = obj.run_functional::<T>(...)'",
                        ));
                    }
                    self.emit_run_functional(receiver, args, *span, None)?;
                    Ok(())
                }
                "set" => {
                    if !type_args.is_empty() {
                        return Err(Error::at(*span, "'.set()' does not accept type arguments"));
                    }
                    self.emit_bitflag_mutate(receiver, args, *span, true)
                }
                "clear" => {
                    if !type_args.is_empty() {
                        return Err(Error::at(
                            *span,
                            "'.clear()' does not accept type arguments",
                        ));
                    }
                    self.emit_bitflag_mutate(receiver, args, *span, false)
                }
                _ => Err(Error::at(*span, format!("unknown method '.{method}(...)'"))),
            },
            other => Err(Error::at(
                other.span(),
                "expression has no effect as a statement",
            )),
        }
    }

    fn maybe_auto_deref_mut_ref(&self, expr: &V2Expr) -> Option<V2Expr> {
        if let V2Expr::Ident(span, name) = expr
            && matches!(self.lookup_var(name), Some(Binding::MutRef { .. }))
        {
            return Some(V2Expr::Deref {
                span: *span,
                inner: Box::new(expr.clone()),
            });
        }
        None
    }

    fn emit_bitflag_mutate(
        &mut self,
        receiver: &V2Expr,
        args: &[CallArg],
        span: Span,
        is_set: bool,
    ) -> Result<()> {
        let method = if is_set { "set" } else { "clear" };
        if args.len() != 1 || args[0].name.is_some() {
            return Err(Error::at(
                span,
                format!("'.{method}(flag)' expects exactly 1 positional argument"),
            ));
        }
        let derefed = self.maybe_auto_deref_mut_ref(receiver);
        let lval = self.resolve_lvalue(derefed.as_ref().unwrap_or(receiver))?;
        let ValueType::Bitflags(key) = lval.value_type().clone() else {
            return Err(Error::at(
                span,
                format!(
                    "'.{method}(...)' requires a bitflags target, found '{}'",
                    self.env.display_value_type(lval.value_type())
                ),
            ));
        };
        if let Some((mask, single_bit)) = self.try_eval_const_bitflag(&args[0].value, &key)? {
            if let Some(bit) = single_bit {
                let op = if is_set {
                    V2AssignOp::FlagSet
                } else {
                    V2AssignOp::FlagClear
                };
                return self.emit_lvalue_assign(&lval, op, &bit.to_string());
            }
            if is_set {
                return self.emit_lvalue_assign(&lval, V2AssignOp::BitOr, &mask.to_string());
            }
            return self.emit_lvalue_assign(&lval, V2AssignOp::BitAnd, &(!mask).to_string());
        }
        let (rhs_air, rhs_ty) = self.lower_value_expr(&args[0].value)?;
        if !types_compatible_for_assign(&ValueType::Bitflags(key.clone()), &rhs_ty)
            || !matches!(rhs_ty, ValueType::Bitflags(_))
        {
            return Err(Error::at(
                args[0].span,
                format!(
                    "type mismatch in '.{method}(...)': expected '{}', found '{}'",
                    self.env.display_value_type(&ValueType::Bitflags(key)),
                    self.env.display_value_type(&rhs_ty)
                ),
            ));
        }
        if is_set {
            self.emit_lvalue_assign(&lval, V2AssignOp::BitOr, &rhs_air)
        } else {
            let inv = self.fresh("bf_inv");
            self.line(format!("let {inv} = -1;"));
            self.line(format!("{inv} ^= {rhs_air};"));
            self.emit_lvalue_assign(&lval, V2AssignOp::BitAnd, &inv)
        }
    }

    /// Compile-time integer argument for a primitive builtin (literal, enum, `const`, constant
    /// helper binding, or a BCON entry with a known value), checked against `lo..=hi`.
    fn eval_builtin_int(&self, expr: &V2Expr, what: &str, lo: i64, hi: i64) -> Result<i64> {
        let v: i64 = match expr {
            V2Expr::Int(_, v) => *v,
            V2Expr::Ident(_, name)
                if self.lookup_var(name).is_none() && self.const_item(name).is_some() =>
            {
                self.const_item(name).unwrap_or_default()
            }
            _ => match self.literal_compare_operand(expr) {
                Some((_, v)) => i64::from(v),
                None => i64::from(self.eval_const_i16(expr).map_err(|_| {
                    Error::at(
                        expr.span(),
                        format!("{what} must be a compile-time constant"),
                    )
                })?),
            },
        };
        if v < lo || v > hi {
            return Err(Error::at(
                expr.span(),
                format!("{what} {v} out of range {lo}..={hi}"),
            ));
        }
        Ok(v)
    }

    /// `.air` local name of a v2 `i16` local variable used as a primitive's local slot.
    fn builtin_local_arg(&self, expr: &V2Expr, what: &str) -> Result<String> {
        if let V2Expr::Ident(_, name) = expr
            && let Some(Binding::Local { air_name, ty }) = self.lookup_var(name)
            && matches!(ty, ValueType::I16 | ValueType::Enum(_))
        {
            return Ok(air_name.clone());
        }
        Err(Error::at(
            expr.span(),
            format!("{what} must name an i16 local variable (declare one with 'let x: i16;')"),
        ))
    }

    /// Collects `name: value` arguments, rejecting positional, unknown, and repeated keys.
    fn builtin_named_args<'e>(
        &self,
        prim: &str,
        args: &'e [CallArg],
        allowed: &[&str],
    ) -> Result<BTreeMap<String, &'e CallArg>> {
        let mut out = BTreeMap::new();
        for a in args {
            let Some(key) = &a.name else {
                return Err(Error::at(
                    a.span,
                    format!(
                        "{prim}() only takes named arguments ({})",
                        allowed
                            .iter()
                            .map(|k| format!("'{k}:'"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            };
            if !allowed.contains(&key.as_str()) {
                return Err(Error::at(
                    a.span,
                    format!(
                        "unknown {prim}() argument '{key}' (expected one of: {})",
                        allowed.join(", ")
                    ),
                ));
            }
            if out.insert(key.clone(), a).is_some() {
                return Err(Error::at(
                    a.span,
                    format!("duplicate {prim}() argument '{key}'"),
                ));
            }
        }
        Ok(out)
    }

    /// Formats `animate(clip:, table:, flags:, events:, local:)` and returns the `.air` primitive
    /// text plus the event local's `.air` name (`local:` sets flag `0x20`).
    fn format_animate(&self, args: &[CallArg]) -> Result<(String, Option<String>)> {
        let named = self.builtin_named_args(
            "animate",
            args,
            &["clip", "table", "flags", "events", "local"],
        )?;
        let int = |key: &str, lo: i64, hi: i64| -> Result<i64> {
            match named.get(key) {
                Some(a) => self.eval_builtin_int(&a.value, &format!("animate {key}"), lo, hi),
                None => Ok(0),
            }
        };
        let clip = int("clip", 0, 65535)?;
        let table = int("table", 0, 255)?;
        let mut flags = int("flags", 0, 255)?;
        let events = int("events", -128, 127)?;
        let local = match named.get("local") {
            Some(a) => {
                flags |= 0x20;
                Some(self.builtin_local_arg(&a.value, "animate(local:)")?)
            }
            None => None,
        };
        let mut text =
            format!("animate(clip: {clip}, table: {table}, flags: {flags:#04x}, events: {events}");
        if let Some(l) = &local {
            text.push_str(&format!(", local: {l}"));
        }
        text.push(')');
        Ok((text, local))
    }

    /// `animate(...)` as a statement (runs to completion) or `animate(...) |event| { ... }`
    /// (runs the body once per event). Both lower to `while !animate(...) { ... }`.
    fn lower_animate_stmt(
        &mut self,
        args: &[CallArg],
        event: Option<&str>,
        body: &[V2Stmt],
    ) -> Result<()> {
        let (prim, local) = self.format_animate(args)?;
        self.line(format!("while !{prim} {{"));
        self.indent += 1;
        self.push_scope(ScopeFrame {
            cleanups: Vec::new(),
            is_loop_body: true,
            is_fn_root: false,
            is_air_while: true,
            owner_defer_id: self.current_defer_id,
            cancel_flag: None,
            repeat_counter: None,
        });
        if let Some(name) = event {
            let binding = match local {
                Some(air_name) => Binding::Local {
                    air_name,
                    ty: ValueType::I16,
                },
                None => Binding::BhavParam {
                    index: 0,
                    ty: ValueType::I16,
                },
            };
            self.bind_var(name.to_owned(), binding);
        }
        let res = self.lower_stmts(body);
        let pop_res = self.pop_scope_and_run_cleanups();
        self.indent -= 1;
        self.line("}".to_owned());
        res.and(pop_res)?;
        self.emit_inline_fn_return_propagation()?;
        self.emit_cancel_propagation()?;
        Ok(())
    }

    /// Formats a primitive builtin (`PRIM_BUILTINS`, `OP_BUILTINS`, `animate`, `breakpoint`,
    /// `random_into`) as `.air` primitive text usable as a statement or a condition. Returns
    /// `None` for other names. Callers resolve user callables and helpers first.
    fn format_prim_builtin(
        &mut self,
        fname: &str,
        args: &[CallArg],
        span: Span,
    ) -> Result<Option<String>> {
        if let Some(&(_, opcode)) = OP_BUILTINS.iter().find(|(n, _)| *n == fname) {
            if !args.is_empty() {
                return Err(Error::at(span, format!("{fname}() takes no arguments")));
            }
            return Ok(Some(format!("op {opcode:#06x} (0, 0, 0, 0, 0, 0, 0, 0)")));
        }
        match fname {
            "animate" => return Ok(Some(self.format_animate(args)?.0)),
            "breakpoint" => {
                return match args {
                    [] => Ok(Some("breakpoint()".to_owned())),
                    [a] if a.name.is_none() => {
                        let (v, _) = self.lower_value_expr(&a.value)?;
                        Ok(Some(format!("breakpoint({v})")))
                    }
                    _ => Err(Error::at(
                        span,
                        "breakpoint() takes at most one positional argument",
                    )),
                };
            }
            "random_into" => {
                let [dst, range] = args else {
                    return Err(Error::at(
                        span,
                        "random_into(x, n) expects 2 positional arguments",
                    ));
                };
                if dst.name.is_some() || range.name.is_some() {
                    return Err(Error::at(
                        span,
                        "random_into(x, n) expects 2 positional arguments",
                    ));
                }
                let (range_air, range_ty) = self.lower_value_expr(&range.value)?;
                if range_ty != ValueType::I16 {
                    return Err(Error::at(
                        range.span,
                        "random_into(x, n): 'n' must be an i16",
                    ));
                }
                let lval = self.resolve_lvalue(&dst.value)?;
                let LValue::Direct { air, ty } = lval else {
                    return Err(Error::at(
                        dst.span,
                        "random_into(x, n): 'x' must be a local, attribute, or other direct i16 slot of this object",
                    ));
                };
                if ty != ValueType::I16 {
                    return Err(Error::at(
                        dst.span,
                        format!(
                            "random_into(x, n): 'x' must be an i16, found '{}'",
                            self.env.display_value_type(&ty)
                        ),
                    ));
                }
                return Ok(Some(format!("random({air}, {range_air})")));
            }
            _ => {}
        }
        let Some(spec) = PRIM_BUILTINS.iter().find(|b| b.v2 == fname) else {
            return Ok(None);
        };
        let allowed: Vec<&str> = spec.keys.iter().map(|(k, _, _)| *k).collect();
        let named = self.builtin_named_args(fname, args, &allowed)?;
        let mut parts = Vec::new();
        for &(key, air_key, kind) in spec.keys {
            let Some(a) = named.get(key) else {
                continue;
            };
            let what = format!("{fname} {key}");
            let text = match kind {
                PrimArg::Int(lo, hi) => self.eval_builtin_int(&a.value, &what, lo, hi)?.to_string(),
                PrimArg::Bool => self.eval_const_bool_required(&a.value, a.span)?.to_string(),
                PrimArg::Guid => format!("{:#010x}", self.resolve_guid_expr(&a.value)?),
                PrimArg::Local => self.builtin_local_arg(&a.value, &format!("{fname}({key}:)"))?,
                PrimArg::RefreshTarget => match &a.value {
                    V2Expr::Ident(_, n) if self.is_self_ident(n) => "me".to_owned(),
                    V2Expr::Ident(_, n) if n == "stack_obj" => "stack_obj".to_owned(),
                    other => self.eval_builtin_int(other, &what, 0, 1)?.to_string(),
                },
            };
            parts.push(format!("{air_key}: {text}"));
        }
        Ok(Some(format!("{}({})", spec.air, parts.join(", "))))
    }

    /// `x.is_type(GUID)` on an object handle: `test_object_type(guid, object: x)`.
    fn format_is_type(
        &mut self,
        receiver: &V2Expr,
        args: &[CallArg],
        span: Span,
    ) -> Result<String> {
        let [a] = args else {
            return Err(Error::at(
                span,
                "'.is_type(GUID)' expects exactly 1 argument",
            ));
        };
        if a.name.is_some() {
            return Err(Error::at(
                a.span,
                "'.is_type(GUID)' takes a positional GUID",
            ));
        }
        let guid = self.resolve_guid_expr(&a.value)?;
        let (obj, ty) = self.lower_value_expr(receiver)?;
        if !ty.is_object_handle() {
            return Err(Error::at(
                receiver.span(),
                format!(
                    "'.is_type(...)' needs an object handle, found '{}'",
                    self.env.display_value_type(&ty)
                ),
            ));
        }
        Ok(format!("test_object_type({guid:#010x}, object: {obj})"))
    }
    fn emit_sleep(&mut self, args: &[CallArg], span: Span) -> Result<()> {
        if args.len() != 1 || args[0].name.is_some() {
            return Err(Error::at(
                span,
                "sleep(ticks) expects 1 positional argument",
            ));
        }
        let (ticks_air, _) = self.lower_value_expr(&args[0].value)?;
        let cancel = if self.in_defer() {
            None
        } else {
            self.enclosing_cancel_flag()
        };
        if let Some((_, ref flag)) = cancel {
            let target_while = self
                .nearest_air_while_idx()
                .expect("enclosing cancel_flag implies an enclosing air while loop");
            self.line("if my.person.priority == 0 {".to_owned());
            self.indent += 1;
            self.line(format!("{flag} := 1;"));
            self.emit_cleanups_down_to(target_while)?;
            self.line("break;".to_owned());
            self.indent -= 1;
            self.line("}".to_owned());
        }
        self.line(format!("idle({ticks_air});"));
        if let Some((_, ref flag)) = cancel {
            let target_while = self
                .nearest_air_while_idx()
                .expect("enclosing cancel_flag implies an enclosing air while loop");
            self.line("if my.person.priority == 0 {".to_owned());
            self.indent += 1;
            self.line(format!("{flag} := 1;"));
            self.emit_cleanups_down_to(target_while)?;
            self.line("break;".to_owned());
            self.indent -= 1;
            self.line("}".to_owned());
        }
        Ok(())
    }

    fn prepare_run_functional(
        &mut self,
        receiver: &V2Expr,
        args: &[CallArg],
        span: Span,
    ) -> Result<(String, Option<ResolvedFunctionalSig>, String)> {
        if args.is_empty() {
            return Err(Error::at(
                span,
                "run_functional(kind [, args...]) requires a functional kind",
            ));
        }
        let V2Expr::Ident(kspan, kind_name) = &args[0].value else {
            return Err(Error::at(
                args[0].span,
                "first argument to run_functional() must be a functional kind identifier (e.g. prep, cook)",
            ));
        };
        let _ = functional_kind_info(kind_name, *kspan)?;
        let (target_air, target_ty) = self.lower_value_expr(receiver)?;
        if !matches!(
            target_ty,
            ValueType::ObjectId | ValueType::ObjectRef(_) | ValueType::I16
        ) {
            return Err(Error::at(
                receiver.span(),
                format!(
                    "run_functional() receiver must be an ObjectId or typed object reference, found '{}'",
                    self.env.display_value_type(&target_ty)
                ),
            ));
        }
        let sig_opt = if let ValueType::ObjectRef(ref schema_key) = target_ty {
            let schema = self
                .env
                .object_schemas
                .get(schema_key)
                .expect("ObjectRef schema must exist");
            let sig = schema.functionals.get(kind_name).cloned().ok_or_else(|| {
                Error::at(
                    *kspan,
                    format!(
                        "object '{}' does not declare 'functional {kind_name}'",
                        schema.display_name
                    ),
                )
            })?;
            Some(sig)
        } else {
            None
        };
        let staged_target = self.fresh("rf_obj");
        self.line(format!("let {staged_target} = {target_air};"));
        let mut change_icon = false;
        let mut staged_args: Vec<(String, ValueType, Span)> = Vec::new();
        for arg in &args[1..] {
            if let Some(aname) = &arg.name {
                if aname == "change_icon" {
                    change_icon = self.eval_const_bool_required(&arg.value, arg.span)?;
                } else {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown run_functional option '{aname}'"),
                    ));
                }
            } else {
                let (val, val_ty) = self.lower_value_expr(&arg.value)?;
                let staged = self.fresh("rf_arg");
                self.line(format!("let {staged} = {val};"));
                staged_args.push((staged, val_ty, arg.span));
            }
        }
        if staged_args.len() > 8 {
            return Err(Error::at(
                span,
                "run_functional() supports at most 8 temp arguments (temp[0..7])",
            ));
        }
        if let Some(ref sig) = sig_opt {
            if staged_args.len() != sig.params.len() {
                return Err(Error::at(
                    span,
                    format!(
                        "functional '{kind_name}' expects {} argument(s), found {}",
                        sig.params.len(),
                        staged_args.len()
                    ),
                ));
            }
            for (idx, ((_, actual_ty, aspan), expected_ty)) in
                staged_args.iter().zip(sig.params.iter()).enumerate()
            {
                if !types_compatible_for_assign(expected_ty, actual_ty) {
                    return Err(Error::at(
                        *aspan,
                        format!(
                            "argument {} to functional '{kind_name}' expects '{}', found '{}'",
                            idx + 1,
                            self.env.display_value_type(expected_ty),
                            self.env.display_value_type(actual_ty)
                        ),
                    ));
                }
            }
        }
        for (i, (val, _, _)) in staged_args.iter().enumerate() {
            self.line(format!("temp[{i}] := {val};"));
        }
        self.line(format!("stack_obj := {staged_target};"));
        let prim = if change_icon {
            format!("run_functional_tree({kind_name}, change_icon: true)")
        } else {
            format!("run_functional_tree({kind_name})")
        };
        Ok((prim, sig_opt, kind_name.clone()))
    }

    fn emit_run_functional(
        &mut self,
        receiver: &V2Expr,
        args: &[CallArg],
        span: Span,
        result_local: Option<&str>,
    ) -> Result<()> {
        let (prim, _, _) = self.prepare_run_functional(receiver, args, span)?;
        if let Some(dst) = result_local {
            self.line(format!("if {prim} {{"));
            self.indent += 1;
            self.line(format!("{dst} := 1;"));
            self.indent -= 1;
            self.line("} else {".to_owned());
            self.indent += 1;
            self.line(format!("{dst} := 0;"));
            self.indent -= 1;
            self.line("}".to_owned());
        } else {
            self.line(format!("{prim};"));
        }
        self.emit_restore_stack_obj();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_run_functional_let_some(
        &mut self,
        receiver: &V2Expr,
        type_args: &[V2Type],
        args: &[CallArg],
        span: Span,
        binding_name: &str,
        ty_ann: Option<&V2Type>,
        else_body: &[V2Stmt],
    ) -> Result<ValueType> {
        if type_args.len() > 1 {
            return Err(Error::at(
                span,
                "run_functional::<T>() accepts at most 1 return type argument",
            ));
        }
        let (prim, sig_opt, kind_name) = self.prepare_run_functional(receiver, args, span)?;
        let is_untyped_receiver = sig_opt.is_none();
        let inferred_ty = match sig_opt {
            Some(sig) => sig.return_type.ok_or_else(|| {
                Error::at(
                    span,
                    format!(
                        "functional '{kind_name}' does not declare a return type (use 'if !obj.run_functional({kind_name})' instead of 'let Some(...)')"
                    ),
                )
            })?,
            None => {
                if let Some(t) = type_args.first() {
                    self.env
                        .resolve_value_type(t, &self.owner_module, self.in_object_scope, span)?
                } else {
                    ValueType::I16
                }
            }
        };
        let after_generic_ty = if let Some(t) = type_args.first() {
            let expected =
                self.env
                    .resolve_value_type(t, &self.owner_module, self.in_object_scope, span)?;
            if !types_compatible_for_assign(&expected, &inferred_ty) {
                return Err(Error::at(
                    span,
                    format!(
                        "run_functional::<{}> does not match functional '{kind_name}' return type '{}'",
                        self.env.display_value_type(&expected),
                        self.env.display_value_type(&inferred_ty)
                    ),
                ));
            }
            expected
        } else {
            inferred_ty
        };
        let final_ty = if let Some(ann) = ty_ann {
            let expected =
                self.env
                    .resolve_value_type(ann, &self.owner_module, self.in_object_scope, span)?;
            if !types_compatible_for_assign(&expected, &after_generic_ty) {
                return Err(Error::at(
                    span,
                    format!(
                        "'let Some({binding_name}: {})' does not match functional '{kind_name}' return type '{}'",
                        self.env.display_value_type(&expected),
                        self.env.display_value_type(&after_generic_ty)
                    ),
                ));
            }
            expected
        } else {
            after_generic_ty
        };
        if is_untyped_receiver && !matches!(final_ty, ValueType::I16 | ValueType::ObjectId) {
            return Err(Error::at(
                span,
                format!(
                    "untyped ObjectId.run_functional({kind_name}) cannot safely manufacture nominal type '{}' (bind the target via a typed object reference such as 'find_best_object::<T>' or 'objects_of::<T>')",
                    self.env.display_value_type(&final_ty)
                ),
            ));
        }

        self.line(format!("if !{prim} {{"));
        self.indent += 1;
        self.emit_restore_stack_obj();
        self.push_scope(ScopeFrame::block());
        let r = self.lower_stmts(else_body);
        let p = self.pop_scope_and_run_cleanups();
        self.indent -= 1;
        self.line("}".to_owned());
        r.and(p)?;
        Ok(final_ty)
    }

    fn bind_helper_arg(
        &mut self,
        fn_mod: &str,
        is_local_fn: bool,
        fn_decl: &FnDecl,
        param: &crate::v2::ast::FnParam,
        arg_expr: &V2Expr,
        arg_span: Span,
    ) -> Result<Binding> {
        let binding = match &param.ty {
            V2Type::I16 => {
                if !stmts_mutate_or_borrow_ident(&fn_decl.body, &param.name)
                    && let Some((const_air, const_val)) =
                        self.try_const_i16_air_operand(arg_expr)?
                {
                    Binding::ConstI16 {
                        air: const_air,
                        value: const_val,
                    }
                } else {
                    let (air, ty) = self.lower_value_expr(arg_expr)?;
                    if !types_compatible_for_assign(&ValueType::I16, &ty) {
                        return Err(Error::at(
                            arg_span,
                            format!(
                                "parameter '{}' expects i16, found '{}'",
                                param.name,
                                self.env.display_value_type(&ty)
                            ),
                        ));
                    }
                    let loc = self.fresh(&format!("arg_{}", param.name));
                    self.line(format!("let {loc} = {air};"));
                    Binding::Local {
                        air_name: loc,
                        ty: ValueType::I16,
                    }
                }
            }
            V2Type::Bool => {
                if !stmts_mutate_or_borrow_ident(&fn_decl.body, &param.name)
                    && let Some(const_b) = self.try_const_bool_arg(arg_expr)
                {
                    Binding::ConstBool(const_b)
                } else {
                    let (air, ty) = self.lower_value_expr(arg_expr)?;
                    if ty != ValueType::Bool {
                        return Err(Error::at(
                            arg_span,
                            format!("parameter '{}' expects bool", param.name),
                        ));
                    }
                    let loc = self.fresh(&format!("arg_{}", param.name));
                    self.line(format!("let {loc} = {air};"));
                    Binding::Local {
                        air_name: loc,
                        ty: ValueType::Bool,
                    }
                }
            }
            V2Type::ObjectId => {
                let (air, ty) = self.lower_value_expr(arg_expr)?;
                let is_null_zero = matches!(arg_expr, V2Expr::Int(_, 0));
                if !ty.is_object_handle() && !is_null_zero {
                    return Err(Error::at(
                        arg_span,
                        format!(
                            "parameter '{}' expects ObjectId (or 0), found '{}' (convert numeric temps explicitly via 'let id: ObjectId = ...;')",
                            param.name,
                            self.env.display_value_type(&ty)
                        ),
                    ));
                }
                let loc = self.fresh(&format!("arg_{}", param.name));
                self.line(format!("let {loc} = {air};"));
                Binding::Local {
                    air_name: loc,
                    ty: ValueType::ObjectId,
                }
            }
            V2Type::Named(_) => {
                let expected =
                    self.env
                        .resolve_value_type(&param.ty, fn_mod, is_local_fn, param.span)?;
                if !stmts_mutate_or_borrow_ident(&fn_decl.body, &param.name) {
                    if let ValueType::Bitflags(ref bf_key) = expected
                        && let Some((mask, single_bit)) =
                            self.try_eval_const_bitflag(arg_expr, bf_key)?
                    {
                        return Ok(Binding::ConstBitflag {
                            key: bf_key.clone(),
                            mask,
                            single_bit,
                        });
                    }
                    if let ValueType::Enum(ref e_key) = expected
                        && let Some(val) = self.try_eval_const_enum(arg_expr, e_key)?
                    {
                        return Ok(Binding::ConstEnum {
                            key: e_key.clone(),
                            value: val,
                        });
                    }
                }
                let (air, ty) = self.lower_value_expr(arg_expr)?;
                if !types_compatible_for_assign(&expected, &ty) {
                    return Err(Error::at(
                        arg_span,
                        format!(
                            "parameter '{}' expects '{}', found '{}'",
                            param.name,
                            self.env.display_value_type(&expected),
                            self.env.display_value_type(&ty)
                        ),
                    ));
                }
                let loc = self.fresh(&format!("arg_{}", param.name));
                self.line(format!("let {loc} = {air};"));
                Binding::Local {
                    air_name: loc,
                    ty: expected,
                }
            }
            V2Type::MutRef(inner) => {
                let mut lval = match arg_expr {
                    V2Expr::MutRef { inner: place, .. } => self.resolve_lvalue(place)?,
                    V2Expr::Ident(_, pname) => match self.lookup_var(pname).cloned() {
                        Some(Binding::MutRef { target }) => target,
                        _ => {
                            return Err(Error::at(
                                arg_span,
                                format!(
                                    "parameter '{}' expects '&mut {}'",
                                    param.name,
                                    inner.display_name()
                                ),
                            ));
                        }
                    },
                    _ => {
                        return Err(Error::at(
                            arg_span,
                            format!(
                                "parameter '{}' expects '&mut {}'",
                                param.name,
                                inner.display_name()
                            ),
                        ));
                    }
                };
                let expected_inner =
                    self.env
                        .resolve_value_type(inner, fn_mod, is_local_fn, param.span)?;
                if lval.value_type() != &expected_inner {
                    return Err(Error::at(
                        arg_span,
                        format!(
                            "type mismatch for '&mut {}' parameter '{}' (found '{}')",
                            self.env.display_value_type(&expected_inner),
                            param.name,
                            self.env.display_value_type(lval.value_type())
                        ),
                    ));
                }
                // Snapshot foreign object handle so later reassignment of caller local cannot alias.
                match &mut lval {
                    LValue::ForeignAttr { obj_air, .. } | LValue::ForeignData { obj_air, .. } => {
                        let snap = self.fresh(&format!("ref_obj_{}", param.name));
                        self.line(format!("let {snap} = {obj_air};"));
                        *obj_air = snap;
                    }
                    LValue::Direct { .. } => {}
                }
                Binding::MutRef { target: lval }
            }
            V2Type::Sim => {
                let V2Expr::Ident(ispan, ident) = arg_expr else {
                    return Err(Error::at(
                        arg_span,
                        format!("parameter '{}' expects Sim handle", param.name),
                    ));
                };
                if !self.is_sim_ident(ident) {
                    return Err(Error::at(
                        *ispan,
                        format!(
                            "expected Sim handle for parameter '{}', found '{ident}'",
                            param.name
                        ),
                    ));
                }
                Binding::SimHandle
            }
            V2Type::Object => {
                if let V2Expr::Ident(_, ident) = arg_expr
                    && self.is_self_ident(ident)
                {
                    Binding::SelfHandle
                } else {
                    let (air, ty) = self.lower_value_expr(arg_expr)?;
                    if !ty.is_object_handle() {
                        return Err(Error::at(
                            arg_span,
                            format!(
                                "parameter '{}' expects 'self' or ObjectId, found '{}'",
                                param.name,
                                self.env.display_value_type(&ty)
                            ),
                        ));
                    }
                    let loc = self.fresh(&format!("obj_{}", param.name));
                    self.line(format!("let {loc} = {air};"));
                    Binding::ForeignObject { air_name: loc }
                }
            }
            V2Type::Motive => {
                let m = self.resolve_motive_expr(arg_expr)?;
                Binding::Motive(m)
            }
            V2Type::Skill => {
                let sk = self.resolve_skill_expr(arg_expr)?;
                Binding::Skill(sk)
            }
            V2Type::Sound => {
                let sid = self.resolve_sound_expr(arg_expr)?;
                Binding::Sound(sid)
            }
            V2Type::Slot => {
                let slot_idx = self.resolve_slot_expr(arg_expr)?;
                Binding::Slot(slot_idx)
            }
        };
        Ok(binding)
    }

    fn inline_helper_call(
        &mut self,
        fn_mod: &str,
        fn_decl: &FnDecl,
        is_local_fn: bool,
        args: &[CallArg],
        call_span: Span,
    ) -> Result<Option<(String, ValueType)>> {
        self.inline_helper_call_mode(fn_mod, fn_decl, is_local_fn, args, call_span, false)
    }

    /// Whether a `-> bool` helper is straight-line code ending in one `return <cond>;`, which
    /// inlines as a branch condition without the return-flag loop wrapper.
    fn is_tail_return_bool_helper(&self, fn_decl: &FnDecl) -> bool {
        let Some((V2Stmt::Return { value: Some(_), .. }, prefix)) = fn_decl.body.split_last()
        else {
            return false;
        };
        fn_decl.return_type == Some(V2Type::Bool)
            && !stmts_have_return(prefix)
            && !prefix.iter().any(|s| matches!(s, V2Stmt::Defer { .. }))
            && (self.in_defer() || self.enclosing_cancel_flag().is_none())
    }

    /// Inlines a helper; with `tail_cond` (only for [`Self::is_tail_return_bool_helper`]
    /// helpers) the prefix is emitted and the returned operand is the tail condition's `.air`
    /// text, to be used directly as the caller's branch condition.
    fn inline_helper_call_mode(
        &mut self,
        fn_mod: &str,
        fn_decl: &FnDecl,
        is_local_fn: bool,
        args: &[CallArg],
        call_span: Span,
        tail_cond: bool,
    ) -> Result<Option<(String, ValueType)>> {
        let fqn = format!(
            "{}::{fn_mod}::{}",
            if is_local_fn { "local" } else { "mod" },
            fn_decl.name
        );
        if let Some(pos) = self.inline_call_stack.iter().position(|n| n == &fqn) {
            let mut chain: Vec<&str> = self.inline_call_stack[pos..]
                .iter()
                .map(String::as_str)
                .collect();
            chain.push(&fqn);
            return Err(Error::at(
                call_span,
                format!(
                    "recursive function call cycle detected: {}",
                    chain.join(" -> ")
                ),
            ));
        }
        if self.inline_call_stack.len() >= MAX_INLINE_DEPTH {
            return Err(Error::at(
                call_span,
                format!("helper function nesting exceeds maximum depth {MAX_INLINE_DEPTH}"),
            ));
        }
        self.inline_expansions += 1;
        if self.inline_expansions > MAX_INLINE_EXPANSIONS {
            return Err(Error::at(
                call_span,
                format!("helper function expansion budget ({MAX_INLINE_EXPANSIONS}) exceeded"),
            ));
        }

        // Resolve positional + named arguments against `fn_decl.params` (supporting default values).
        let mut supplied = vec![false; fn_decl.params.len()];
        let mut call_order: Vec<(usize, &CallArg)> = Vec::with_capacity(args.len());
        let mut seen_named = false;
        let mut positional_idx = 0usize;
        for arg in args {
            if let Some(ref arg_name) = arg.name {
                seen_named = true;
                let Some(param_idx) = fn_decl.params.iter().position(|p| &p.name == arg_name)
                else {
                    return Err(Error::at(
                        arg.span,
                        format!(
                            "function '{}' has no parameter named '{arg_name}'",
                            fn_decl.name
                        ),
                    ));
                };
                if supplied[param_idx] {
                    return Err(Error::at(
                        arg.span,
                        format!(
                            "duplicate argument for parameter '{arg_name}' in call to '{}'",
                            fn_decl.name
                        ),
                    ));
                }
                supplied[param_idx] = true;
                call_order.push((param_idx, arg));
            } else {
                if seen_named {
                    return Err(Error::at(
                        arg.span,
                        format!(
                            "positional argument cannot follow a named argument in call to '{}'",
                            fn_decl.name
                        ),
                    ));
                }
                if positional_idx >= fn_decl.params.len() {
                    return Err(Error::at(
                        call_span,
                        format!(
                            "function '{}' expects at most {} argument(s), found {}",
                            fn_decl.name,
                            fn_decl.params.len(),
                            args.len()
                        ),
                    ));
                }
                supplied[positional_idx] = true;
                call_order.push((positional_idx, arg));
                positional_idx += 1;
            }
        }
        for (i, param) in fn_decl.params.iter().enumerate() {
            if !supplied[i] && param.default.is_none() {
                return Err(Error::at(
                    call_span,
                    format!(
                        "missing required argument '{}' in call to '{}'",
                        param.name, fn_decl.name
                    ),
                ));
            }
        }

        // Evaluate caller-supplied arguments in call-site order BEFORE registering `fqn` on
        // `inline_call_stack` (so finite explicit nesting like `f(f(1))` succeeds).
        let mut bound_params: BTreeMap<String, Binding> = BTreeMap::new();
        for (param_idx, arg) in call_order {
            let param = &fn_decl.params[param_idx];
            let binding =
                self.bind_helper_arg(fn_mod, is_local_fn, fn_decl, param, &arg.value, arg.span)?;
            bound_params.insert(param.name.clone(), binding);
        }

        // Register `fqn` on `inline_call_stack` BEFORE evaluating omitted default expressions
        // so recursive default parameters (e.g. `fn f(x: i16 = f())`) trigger cycle/depth detection.
        self.inline_call_stack.push(fqn);

        // Evaluate omitted defaulted parameters hygienically in the callee's module/object scope
        // (never capturing caller local variables or caller `run(actor)` Sim bindings).
        if supplied.iter().any(|&s| !s) {
            let prev_vars = std::mem::replace(&mut self.var_scopes, vec![BTreeMap::new()]);
            let prev_in_obj = self.in_object_scope;
            self.in_object_scope = is_local_fn;
            let default_res = self.with_owner_module(fn_mod, |c| {
                for (i, param) in fn_decl.params.iter().enumerate() {
                    if !supplied[i] {
                        let def_expr = param
                            .default
                            .as_ref()
                            .expect("missing required args rejected above");
                        let binding = c.bind_helper_arg(
                            fn_mod,
                            is_local_fn,
                            fn_decl,
                            param,
                            def_expr,
                            param.span,
                        )?;
                        bound_params.insert(param.name.clone(), binding);
                    }
                }
                Ok(())
            });
            self.in_object_scope = prev_in_obj;
            self.var_scopes = prev_vars;
            if let Err(e) = default_res {
                self.inline_call_stack.pop();
                return Err(e);
            }
        }

        let ret_kind = match &fn_decl.return_type {
            None => FnReturnKind::Void,
            Some(V2Type::Bool) => FnReturnKind::Bool,
            Some(other) => {
                match self
                    .env
                    .resolve_value_type(other, fn_mod, is_local_fn, fn_decl.span)
                {
                    Ok(vty) => FnReturnKind::Value(vty),
                    Err(_) => {
                        self.inline_call_stack.pop();
                        return Err(Error::at(
                            fn_decl.span,
                            format!(
                                "helper function '{}' has unsupported return type '{}'",
                                fn_decl.name,
                                other.display_name()
                            ),
                        ));
                    }
                }
            }
        };
        if ret_kind != FnReturnKind::Void
            && !stmts_always_return(&fn_decl.body, self.env, fn_mod, is_local_fn)
        {
            self.inline_call_stack.pop();
            return Err(Error::at(
                fn_decl.span,
                format!(
                    "helper function '{}' declares '-> {}' but does not return on all control-flow paths",
                    fn_decl.name,
                    fn_decl
                        .return_type
                        .as_ref()
                        .expect("non-void has return_type")
                        .display_name()
                ),
            ));
        }

        if tail_cond {
            let prev_vars = std::mem::replace(&mut self.var_scopes, vec![bound_params]);
            let prev_active_fn = self.active_inline_fn.take();
            let prev_in_obj = self.in_object_scope;
            self.in_object_scope = is_local_fn;
            let res = self.with_owner_module(fn_mod, |c| {
                let Some((
                    V2Stmt::Return {
                        value: Some(tail), ..
                    },
                    prefix,
                )) = fn_decl.body.split_last()
                else {
                    unreachable!("checked by is_tail_return_bool_helper");
                };
                c.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: false,
                    is_fn_root: true,
                    is_air_while: false,
                    owner_defer_id: c.current_defer_id,
                    cancel_flag: None,
                    repeat_counter: None,
                });
                let r = c.lower_stmts(prefix).and_then(|()| {
                    if let Some(text) = c.try_lower_simple_air_cond(tail)? {
                        return Ok(text);
                    }
                    let cond = c.fresh("cond");
                    c.line(format!("let {cond} = 0;"));
                    c.emit_bool_into_local(tail, &cond)?;
                    Ok(format!("{cond} != 0"))
                });
                let p = c.pop_scope_and_run_cleanups();
                let text = r?;
                p?;
                Ok(text)
            });
            self.in_object_scope = prev_in_obj;
            self.active_inline_fn = prev_active_fn;
            self.var_scopes = prev_vars;
            self.inline_call_stack.pop();
            return Ok(Some((res?, ValueType::Bool)));
        }

        let has_returns = stmts_have_return(&fn_decl.body);
        let has_enclosing_cancel = !self.in_defer() && self.enclosing_cancel_flag().is_some();
        let needs_wrapper = has_returns || has_enclosing_cancel;
        let ret_val_local = if ret_kind != FnReturnKind::Void {
            let rv = self.fresh(&format!("ret_{}", fn_decl.name));
            self.line(format!("let {rv} = 0;"));
            Some(rv)
        } else {
            None
        };

        let prev_vars = std::mem::replace(&mut self.var_scopes, vec![bound_params]);
        let prev_active_fn = self.active_inline_fn.take();
        let prev_in_obj = self.in_object_scope;
        self.in_object_scope = is_local_fn;

        let res = self.with_owner_module(fn_mod, |c| {
            if needs_wrapper {
                if has_returns {
                    let ret_flag = c.fresh("fn_ret");
                    let rv_name = ret_val_local
                        .clone()
                        .unwrap_or_else(|| c.fresh("fn_void_ret"));
                    c.line(format!("let {ret_flag} = 0;"));
                    c.active_inline_fn = Some(ActiveInlineFn {
                        ret_flag,
                        ret_val: rv_name,
                        ret_kind: ret_kind.clone(),
                        owner_defer_id: c.current_defer_id,
                    });
                }
                c.line("while true {".to_owned());
                c.indent += 1;
                c.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: false,
                    is_fn_root: true,
                    is_air_while: true,
                    owner_defer_id: c.current_defer_id,
                    cancel_flag: None,
                    repeat_counter: None,
                });
                let r = c.lower_stmts(&fn_decl.body);
                let p = c.pop_scope_and_run_cleanups();
                c.line("break;".to_owned());
                c.indent -= 1;
                c.line("}".to_owned());
                r.and(p)
            } else {
                c.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: false,
                    is_fn_root: true,
                    is_air_while: false,
                    owner_defer_id: c.current_defer_id,
                    cancel_flag: None,
                    repeat_counter: None,
                });
                let r = c.lower_stmts(&fn_decl.body);
                let p = c.pop_scope_and_run_cleanups();
                r.and(p)
            }
        });

        self.in_object_scope = prev_in_obj;
        self.active_inline_fn = prev_active_fn;
        self.var_scopes = prev_vars;
        self.inline_call_stack.pop();
        res?;
        if needs_wrapper {
            self.emit_cancel_propagation()?;
        }
        Ok(ret_val_local.map(|rv| {
            let vty = match ret_kind {
                FnReturnKind::Bool => ValueType::Bool,
                FnReturnKind::Value(v) => v,
                FnReturnKind::Void => unreachable!("ret_val_local only set for non-void"),
            };
            (rv, vty)
        }))
    }

    fn resolve_lvalue(&mut self, expr: &V2Expr) -> Result<LValue> {
        match expr {
            V2Expr::Deref { span, inner } => {
                let V2Expr::Ident(_, pname) = inner.as_ref() else {
                    return Err(Error::at(
                        *span,
                        "only '*param' dereference on '&mut' parameters is supported",
                    ));
                };
                match self.lookup_var(pname).cloned() {
                    Some(Binding::MutRef { target, .. }) => Ok(target),
                    Some(_) => Err(Error::at(
                        *span,
                        format!("cannot dereference non-'&mut' variable '{pname}'"),
                    )),
                    None => Err(Error::at(*span, format!("unknown variable '{pname}'"))),
                }
            }
            V2Expr::Ident(span, name) => match self.lookup_var(name).cloned() {
                Some(Binding::Local { air_name, ty }) => Ok(LValue::Direct { air: air_name, ty }),
                Some(Binding::BhavParam { .. }) => Err(Error::at(
                    *span,
                    format!("cannot assign directly to immutable parameter '{name}'"),
                )),
                Some(Binding::MutRef { .. }) => Err(Error::at(
                    *span,
                    format!("use '*{name} = ...' to assign through '&mut' parameter '{name}'"),
                )),
                Some(_) => Err(Error::at(*span, format!("'{name}' is not assignable"))),
                None if name == "stack_obj" => Ok(LValue::Direct {
                    air: "stack_obj".to_owned(),
                    ty: ValueType::ObjectId,
                }),
                None => {
                    if self.env.i16_params.contains_key(name)
                        || self.env.bool_params.contains_key(name)
                    {
                        Err(Error::at(
                            *span,
                            format!("cannot assign to compile-time param '{name}'"),
                        ))
                    } else {
                        Err(Error::at(*span, format!("unknown variable '{name}'")))
                    }
                }
            },
            V2Expr::Field { span, base, field } => {
                if let Some(r) = self.bcon_field_ref(expr) {
                    r?;
                    return Err(Error::at(
                        *span,
                        "BCON constants are read-only (cannot assign to a BCON table entry)",
                    ));
                }
                // `sim.motive.energy`, `sim.skill.cooking`, `self.<state>`, `self.graphic`, etc.
                if let V2Expr::Field {
                    base: root,
                    field: group,
                    ..
                } = base.as_ref()
                    && let V2Expr::Ident(rspan, rname) = root.as_ref()
                {
                    if rname == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                        return Err(Error::at(
                            *rspan,
                            "'sim' is not available in init/main (object lifecycle context)",
                        ));
                    }
                    if self.is_sim_ident(rname) {
                        return match group.as_str() {
                            "motive" => {
                                let _ = motive_by_name(field).ok_or_else(|| {
                                    Error::at(*span, format!("unknown motive '{field}'"))
                                })?;
                                Ok(LValue::Direct {
                                    air: format!("my.motive.{field}"),
                                    ty: ValueType::I16,
                                })
                            }
                            "skill" => {
                                let _ = skill_by_name(field).ok_or_else(|| {
                                    Error::at(*span, format!("unknown skill '{field}'"))
                                })?;
                                Ok(LValue::Direct {
                                    air: format!("my.person.{field}"),
                                    ty: ValueType::I16,
                                })
                            }
                            other => Err(Error::at(
                                *span,
                                format!("unknown Sim member '{rname}.{other}.{field}'"),
                            )),
                        };
                    }
                    if self.is_self_ident(rname) {
                        return Err(Error::at(
                            *span,
                            format!(
                                "'self' is an Object, not a Sim (cannot access 'self.{group}.{field}')"
                            ),
                        ));
                    }
                }

                if let V2Expr::Ident(bspan, bname) = base.as_ref() {
                    if self.is_self_ident(bname) {
                        if let Some(st) = self.env.states.get(field) {
                            return Ok(LValue::Direct {
                                air: format!("{}.attr[{}]", self.self_prefix(), st.attr_index),
                                ty: st.ty.clone(),
                            });
                        }
                        return match field.as_str() {
                            "graphic" => Ok(LValue::Direct {
                                air: format!("{}.data[0]", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            "flags" => Ok(LValue::Direct {
                                air: format!("{}.data[8]", self.self_prefix()),
                                ty: ValueType::Bitflags(ANY_BITFLAGS.to_owned()),
                            }),
                            "allowed_heights" => Ok(LValue::Direct {
                                air: format!("{}.data[4]", self.self_prefix()),
                                ty: ValueType::Bitflags("builtin::Height".to_owned()),
                            }),
                            "placement" => Ok(LValue::Direct {
                                air: format!("{}.data[42]", self.self_prefix()),
                                ty: ValueType::Bitflags("builtin::Placement".to_owned()),
                            }),
                            "prep_score" => Ok(LValue::Direct {
                                air: format!("{}.prep_score", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            "cook_score" => Ok(LValue::Direct {
                                air: format!("{}.cook_score", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            "in_use" => Ok(LValue::Direct {
                                air: format!("{}.in_use", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            "motive" | "skill" | "priority" => Err(Error::at(
                                *span,
                                format!(
                                    "'self' is an Object, not a Sim (cannot access 'self.{field}')"
                                ),
                            )),
                            other => Err(Error::at(
                                *span,
                                format!("unknown object state or field 'self.{other}'"),
                            )),
                        };
                    }
                    if self.is_sim_ident(bname) {
                        return match field.as_str() {
                            "hand" => Ok(LValue::Direct {
                                air: "my.slot[0]".to_owned(),
                                ty: ValueType::ObjectId,
                            }),
                            "age" => Ok(LValue::Direct {
                                air: "my.person[58]".to_owned(),
                                ty: ValueType::I16,
                            }),
                            _ => Err(Error::at(
                                *span,
                                format!(
                                    "cannot assign to '{bname}.{field}' (use '{bname}.motive.<name>', '{bname}.skill.<name>', '{bname}.hand', '{bname}.age', '{bname}.slot[i]', or '{bname}.person[i]'; use 'self.<state>' for object state)"
                                ),
                            )),
                        };
                    }
                    if bname == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                        return Err(Error::at(
                            *bspan,
                            "'sim' is not available in init/main (object lifecycle context)",
                        ));
                    }
                    // Typed foreign object field access (`fridge.stock`, `plate.nutrition`, `part.occupancy`, etc.)
                    if let Some(binding) = self.lookup_var(bname).cloned() {
                        let foreign_info = match binding {
                            Binding::Local {
                                air_name,
                                ty: ValueType::ObjectRef(schema_key),
                            } => Some((air_name, schema_key)),
                            Binding::BhavParam {
                                index,
                                ty: ValueType::ObjectRef(schema_key),
                            } => Some((format!("param[{index}]"), schema_key)),
                            _ => None,
                        };
                        if let Some((obj_air, schema_key)) = foreign_info {
                            let schema = self
                                .env
                                .object_schemas
                                .get(&schema_key)
                                .expect("ObjectRef schema must exist");
                            if let Some((attr_idx, state_ty)) = schema.states.get(field) {
                                return Ok(LValue::ForeignAttr {
                                    obj_air,
                                    attr_index: *attr_idx as i16,
                                    ty: state_ty.clone(),
                                });
                            }
                            return match field.as_str() {
                                "graphic" => Ok(LValue::ForeignData {
                                    obj_air,
                                    data_index: 0,
                                    ty: ValueType::I16,
                                }),
                                "allowed_heights" => Ok(LValue::ForeignData {
                                    obj_air,
                                    data_index: 4,
                                    ty: ValueType::Bitflags("builtin::Height".to_owned()),
                                }),
                                "placement" => Ok(LValue::ForeignData {
                                    obj_air,
                                    data_index: 42,
                                    ty: ValueType::Bitflags("builtin::Placement".to_owned()),
                                }),
                                other => Err(Error::at(
                                    *span,
                                    format!(
                                        "object '{}' has no state field '{other}'",
                                        schema.display_name
                                    ),
                                )),
                            };
                        }
                    }
                }
                Err(Error::at(
                    *span,
                    "expression is not a valid assignment target",
                ))
            }
            V2Expr::Index { span, base, index } => {
                // `temp[i]`, `global[i]`, `self.attr[i]`, `self.data[i]`, `sim.motive[m]`, `foreign.attr[i]`
                if let V2Expr::Ident(_, bname) = base.as_ref() {
                    match bname.as_str() {
                        "temp" => {
                            let idx = self.eval_const_i16(index)?;
                            return Ok(LValue::Direct {
                                air: format!("temp[{idx}]"),
                                ty: ValueType::I16,
                            });
                        }
                        "global" => {
                            let idx = self.eval_const_i16(index)?;
                            return Ok(LValue::Direct {
                                air: format!("global[{idx}]"),
                                ty: ValueType::I16,
                            });
                        }
                        _ => {}
                    }
                }
                if let V2Expr::Field {
                    base: root, field, ..
                } = base.as_ref()
                    && let V2Expr::Ident(rspan, rname) = root.as_ref()
                {
                    if rname == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                        return Err(Error::at(
                            *rspan,
                            "'sim' is not available in init/main (object lifecycle context)",
                        ));
                    }
                    if self.is_sim_ident(rname) && field == "motive" {
                        let m = self.resolve_motive_expr(index)?;
                        return Ok(LValue::Direct {
                            air: format!("my.motive[{m}]"),
                            ty: ValueType::I16,
                        });
                    }
                    if self.is_sim_ident(rname) && field == "skill" {
                        let sk = self.resolve_skill_expr(index)?;
                        return Ok(LValue::Direct {
                            air: format!("my.person[{sk}]"),
                            ty: ValueType::I16,
                        });
                    }
                    if self.is_sim_ident(rname) && matches!(field.as_str(), "slot" | "person") {
                        let idx = self.eval_builtin_int(
                            index,
                            &format!("{rname}.{field} index"),
                            0,
                            255,
                        )?;
                        return Ok(LValue::Direct {
                            air: format!("my.{field}[{idx}]"),
                            ty: if field == "slot" {
                                ValueType::ObjectId
                            } else {
                                ValueType::I16
                            },
                        });
                    }
                    if rname == "advert"
                        && self.lookup_var(rname).is_none()
                        && let Some(scope) = match field.as_str() {
                            "delta" => Some(12),
                            "personality" => Some(28),
                            "minimum" => Some(29),
                            _ => None,
                        }
                    {
                        // Motive indices are 0..16, which is also the advertisement range.
                        let m = self.resolve_motive_expr(index)?;
                        return Ok(LValue::Direct {
                            air: format!("scope({scope}, {m})"),
                            ty: ValueType::I16,
                        });
                    }
                    if self.is_self_ident(rname) {
                        let idx = self.eval_const_i16(index)?;
                        return match field.as_str() {
                            "attr" => Ok(LValue::Direct {
                                air: format!("{}.attr[{idx}]", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            "data" => Ok(LValue::Direct {
                                air: format!("{}.data[{idx}]", self.self_prefix()),
                                ty: ValueType::I16,
                            }),
                            other => Err(Error::at(
                                *span,
                                format!("cannot index 'self.{other}[...]'"),
                            )),
                        };
                    }
                    // Foreign ObjectId / ObjectRef variable: `plate.attr[0] = nutrition;`
                    if let Some(binding) = self.lookup_var(rname).cloned() {
                        let obj_air = match binding {
                            Binding::Local { air_name, ty } if ty.is_object_handle() => air_name,
                            Binding::ForeignObject { air_name } => air_name,
                            Binding::BhavParam { index, ty } if ty.is_object_handle() => {
                                format!("param[{index}]")
                            }
                            Binding::Local { ty, .. } | Binding::BhavParam { ty, .. } => {
                                return Err(Error::at(
                                    *span,
                                    format!(
                                        "cannot index '{rname}.{field}[...]': '{rname}' has type '{}', expected ObjectId",
                                        self.env.display_value_type(&ty)
                                    ),
                                ));
                            }
                            _ => {
                                return Err(Error::at(
                                    *span,
                                    format!("cannot index '{rname}.{field}[...]'"),
                                ));
                            }
                        };
                        let idx = self.eval_const_i16(index)?;
                        return match field.as_str() {
                            "attr" => Ok(LValue::ForeignAttr {
                                obj_air,
                                attr_index: idx,
                                ty: ValueType::I16,
                            }),
                            "data" => Ok(LValue::ForeignData {
                                obj_air,
                                data_index: idx,
                                ty: ValueType::I16,
                            }),
                            other => Err(Error::at(
                                *span,
                                format!("cannot index '{rname}.{other}[...]'"),
                            )),
                        };
                    }
                }
                Err(Error::at(*span, "unsupported indexed assignment target"))
            }
            other => Err(Error::at(
                other.span(),
                "expression is not a valid assignment target",
            )),
        }
    }

    fn emit_lvalue_assign(&mut self, lval: &LValue, op: V2AssignOp, rhs_air: &str) -> Result<()> {
        let op_str = match op {
            V2AssignOp::Set => ":=",
            V2AssignOp::Add => "+=",
            V2AssignOp::Sub => "-=",
            V2AssignOp::Mul => "*=",
            V2AssignOp::Div => "/=",
            V2AssignOp::Mod => "%=",
            V2AssignOp::BitAnd => "&=",
            V2AssignOp::BitOr => "|=",
            V2AssignOp::BitXor => "^=",
            V2AssignOp::FlagSet => "flag_set",
            V2AssignOp::FlagClear => "flag_clear",
        };
        match lval {
            LValue::Direct { air, .. } => {
                self.line(format!("{air} {op_str} {rhs_air};"));
            }
            LValue::ForeignAttr {
                obj_air,
                attr_index,
                ..
            } => {
                let effective_rhs = if rhs_air.starts_with("stack.") || rhs_air == "stack_obj" {
                    let snap = self.fresh("frhs");
                    self.line(format!("let {snap} = {rhs_air};"));
                    snap
                } else {
                    rhs_air.to_owned()
                };
                self.line(format!("stack_obj := {obj_air};"));
                self.line(format!(
                    "stack.attr[{attr_index}] {op_str} {effective_rhs};"
                ));
                self.emit_restore_stack_obj();
            }
            LValue::ForeignData {
                obj_air,
                data_index,
                ..
            } => {
                let effective_rhs = if rhs_air.starts_with("stack.") || rhs_air == "stack_obj" {
                    let snap = self.fresh("frhs");
                    self.line(format!("let {snap} = {rhs_air};"));
                    snap
                } else {
                    rhs_air.to_owned()
                };
                self.line(format!("stack_obj := {obj_air};"));
                self.line(format!(
                    "stack.data[{data_index}] {op_str} {effective_rhs};"
                ));
                self.emit_restore_stack_obj();
            }
        }
        Ok(())
    }

    /// Resolves a compile-time `Enum` constant (`Enum::Variant` or `Binding::ConstEnum`).
    fn try_eval_const_enum(&self, expr: &V2Expr, expected_key: &str) -> Result<Option<i16>> {
        match expr {
            V2Expr::Path(span, segs) if segs.len() == 2 => {
                if let Some(e) =
                    self.env
                        .lookup_enum(&self.owner_module, &segs[0], self.in_object_scope)
                {
                    if e.canonical_key != expected_key {
                        return Err(Error::at(
                            *span,
                            format!(
                                "type mismatch: expected '{}', found '{}'",
                                self.env
                                    .display_value_type(&ValueType::Enum(expected_key.to_owned())),
                                e.display_name
                            ),
                        ));
                    }
                    let &val = e.variants.get(&segs[1]).ok_or_else(|| {
                        Error::at(
                            *span,
                            format!("enum '{}' has no variant '{}'", e.display_name, segs[1]),
                        )
                    })?;
                    return Ok(Some(val));
                }
                Ok(None)
            }
            V2Expr::Ident(span, name) => {
                if let Some(Binding::ConstEnum { key, value }) = self.lookup_var(name) {
                    if key != expected_key {
                        return Err(Error::at(
                            *span,
                            format!(
                                "type mismatch: expected '{}', found '{}'",
                                self.env
                                    .display_value_type(&ValueType::Enum(expected_key.to_owned())),
                                self.env.display_value_type(&ValueType::Enum(key.clone()))
                            ),
                        ));
                    }
                    return Ok(Some(*value));
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Resolves a compile-time `Bitflags` constant expression (single flag, `None`, or `|`/`&`/`^` of constants),
    /// returning `(mask, single_bit_1_based)` when constant!
    fn try_eval_const_bitflag(
        &self,
        expr: &V2Expr,
        expected_key: &str,
    ) -> Result<Option<(i16, Option<u8>)>> {
        match expr {
            V2Expr::Path(span, segs) if segs.len() == 2 => {
                if let Some(bf) =
                    self.env
                        .lookup_bitflags(&self.owner_module, &segs[0], self.in_object_scope)
                {
                    if bf.canonical_key != expected_key && expected_key != ANY_BITFLAGS {
                        return Err(Error::at(
                            *span,
                            format!(
                                "type mismatch: expected bitflags '{}', found '{}'",
                                self.env.display_value_type(&ValueType::Bitflags(
                                    expected_key.to_owned()
                                )),
                                bf.display_name
                            ),
                        ));
                    }
                    if segs[1] == "None" {
                        return Ok(Some((0, None)));
                    }
                    let &bit = bf.flags.get(&segs[1]).ok_or_else(|| {
                        Error::at(
                            *span,
                            format!("bitflags '{}' has no flag '{}'", bf.display_name, segs[1]),
                        )
                    })?;
                    return Ok(Some((bit_to_mask(bit), Some(bit))));
                }
                Ok(None)
            }
            V2Expr::Ident(span, name) => {
                if let Some(Binding::ConstBitflag {
                    key,
                    mask,
                    single_bit,
                }) = self.lookup_var(name)
                {
                    if key != expected_key && expected_key != ANY_BITFLAGS {
                        return Err(Error::at(
                            *span,
                            format!(
                                "type mismatch: expected bitflags '{}', found '{}'",
                                self.env.display_value_type(&ValueType::Bitflags(
                                    expected_key.to_owned()
                                )),
                                self.env
                                    .display_value_type(&ValueType::Bitflags(key.clone()))
                            ),
                        ));
                    }
                    return Ok(Some((*mask, *single_bit)));
                }
                Ok(None)
            }
            V2Expr::Binary {
                lhs,
                op: op @ (V2BinOp::BitOr | V2BinOp::BitAnd | V2BinOp::BitXor),
                rhs,
                ..
            } => {
                let Some((lmask, _)) = self.try_eval_const_bitflag(lhs, expected_key)? else {
                    return Ok(None);
                };
                let Some((rmask, _)) = self.try_eval_const_bitflag(rhs, expected_key)? else {
                    return Ok(None);
                };
                let mask = match op {
                    V2BinOp::BitOr => lmask | rmask,
                    V2BinOp::BitAnd => lmask & rmask,
                    V2BinOp::BitXor => lmask ^ rmask,
                    _ => unreachable!(),
                };
                let single_bit = single_bit_of_mask(mask);
                Ok(Some((mask, single_bit)))
            }
            _ => Ok(None),
        }
    }

    /// Detects if `expr` is a compile-time constant `Bitflags` expression of ANY bitflags type.
    fn try_eval_any_const_bitflag(
        &self,
        expr: &V2Expr,
    ) -> Result<Option<(String, i16, Option<u8>)>> {
        match expr {
            V2Expr::Path(span, segs) if segs.len() == 2 => {
                if let Some(bf) =
                    self.env
                        .lookup_bitflags(&self.owner_module, &segs[0], self.in_object_scope)
                {
                    let key = bf.canonical_key.clone();
                    if segs[1] == "None" {
                        return Ok(Some((key, 0, None)));
                    }
                    let &bit = bf.flags.get(&segs[1]).ok_or_else(|| {
                        Error::at(
                            *span,
                            format!("bitflags '{}' has no flag '{}'", bf.display_name, segs[1]),
                        )
                    })?;
                    return Ok(Some((key, bit_to_mask(bit), Some(bit))));
                }
                Ok(None)
            }
            V2Expr::Ident(_, name) => {
                if let Some(Binding::ConstBitflag {
                    key,
                    mask,
                    single_bit,
                }) = self.lookup_var(name)
                {
                    return Ok(Some((key.clone(), *mask, *single_bit)));
                }
                Ok(None)
            }
            V2Expr::Binary {
                lhs,
                op: op @ (V2BinOp::BitOr | V2BinOp::BitAnd | V2BinOp::BitXor),
                rhs,
                ..
            } => {
                let Some((key, lmask, _)) = self.try_eval_any_const_bitflag(lhs)? else {
                    return Ok(None);
                };
                let Some((rmask, _)) = self.try_eval_const_bitflag(rhs, &key)? else {
                    return Ok(None);
                };
                let mask = match op {
                    V2BinOp::BitOr => lmask | rmask,
                    V2BinOp::BitAnd => lmask & rmask,
                    V2BinOp::BitXor => lmask ^ rmask,
                    _ => unreachable!(),
                };
                Ok(Some((key, mask, single_bit_of_mask(mask))))
            }
            _ => Ok(None),
        }
    }

    /// Returns `true` iff `expr` contains a division or remainder whose divisor is not a known
    /// non-zero compile-time constant (and therefore could trap partway through evaluation).
    fn expr_may_trap_arith(&self, expr: &V2Expr) -> bool {
        match expr {
            V2Expr::Binary { lhs, op, rhs, .. } => {
                if matches!(op, V2BinOp::Div | V2BinOp::Mod)
                    && !self.eval_const_i16(rhs).is_ok_and(|v| v != 0)
                {
                    return true;
                }
                self.expr_may_trap_arith(lhs) || self.expr_may_trap_arith(rhs)
            }
            V2Expr::Unary { inner, .. }
            | V2Expr::Deref { inner, .. }
            | V2Expr::MutRef { inner, .. } => self.expr_may_trap_arith(inner),
            V2Expr::Index { base, index, .. } => {
                self.expr_may_trap_arith(base) || self.expr_may_trap_arith(index)
            }
            V2Expr::Field { base, .. } => match self.bcon_field_ref(expr) {
                // `extern bcon` entries are read at runtime and trap when the chunk/entry is
                // missing; private tables are compile-time constants.
                Some(Ok((_, Some(_)))) => false,
                Some(_) => true,
                None => self.expr_may_trap_arith(base),
            },
            V2Expr::Tuple(_, elems) => elems.iter().any(|e| self.expr_may_trap_arith(e)),
            V2Expr::Call { .. } | V2Expr::MethodCall { .. } | V2Expr::Try { .. } => true,
            V2Expr::Int(..)
            | V2Expr::Bool(..)
            | V2Expr::Str(..)
            | V2Expr::Ident(..)
            | V2Expr::Path(..) => false,
        }
    }

    /// Returns `true` iff `expr` can be lowered directly in-place into `dst_air` without aliasing,
    /// partial writes on trap, observable VM write side effects, or premature motive/skill clamping!
    fn can_lower_arith_in_place(&self, dst_air: &str, expr: &V2Expr, is_fresh_local: bool) -> bool {
        let canon_dst = canonical_air_cell(dst_air);
        if canon_dst.starts_with("my.motive")
            || canon_dst.starts_with("stack.motive")
            || canon_dst.starts_with("my.person")
            || canon_dst.starts_with("stack.person")
            || canon_dst.starts_with("my.data[")
            || canon_dst.starts_with("stack.data[")
            || canon_dst.starts_with("global[")
        {
            return false;
        }
        if !is_fresh_local && self.expr_may_trap_arith(expr) {
            return false;
        }
        let (base, ops) = flatten_arith_chain(expr);
        if ops.is_empty() {
            if let V2Expr::Unary {
                op: V2UnaryOp::Neg,
                inner,
                ..
            } = base
            {
                return !self.expr_may_read_or_clobber_air(inner, &canon_dst);
            }
            return true;
        }
        if ops
            .iter()
            .any(|(_, rhs)| self.expr_may_read_or_clobber_air(rhs, &canon_dst))
        {
            return false;
        }
        // If `base` itself is unary `-inner`, emitting `dst := 0; dst -= inner;` would overwrite
        // `dst` before reading `inner` if `inner` reads `dst`!
        if let V2Expr::Unary {
            op: V2UnaryOp::Neg,
            inner,
            ..
        } = base
        {
            return !self.expr_may_read_or_clobber_air(inner, &canon_dst);
        }
        true
    }

    /// Conservatively checks whether evaluating `expr` could read or mutate `canon_target`
    /// (already canonicalized via [`canonical_air_cell`]).
    fn expr_may_read_or_clobber_air(&self, expr: &V2Expr, canon_target: &str) -> bool {
        match expr {
            V2Expr::Int(_, _) | V2Expr::Bool(_, _) | V2Expr::Str(_, _) | V2Expr::Path(_, _) => {
                false
            }
            V2Expr::Ident(_, name) => match self.lookup_var(name) {
                Some(Binding::Local { air_name, .. }) => {
                    canonical_air_cell(air_name) == canon_target
                }
                Some(Binding::BhavParam { index, .. }) => format!("param[{index}]") == canon_target,
                Some(Binding::MutRef {
                    target: LValue::Direct { air, .. },
                }) => canonical_air_cell(air) == canon_target,
                Some(Binding::MutRef { .. }) => true,
                Some(Binding::ConstI16 { air, .. }) => canonical_air_cell(air) == canon_target,
                _ => false,
            },
            V2Expr::Deref { inner, .. } => {
                if let V2Expr::Ident(_, pname) = inner.as_ref() {
                    match self.lookup_var(pname) {
                        Some(Binding::MutRef {
                            target: LValue::Direct { air, .. },
                        }) => canonical_air_cell(air) == canon_target,
                        _ => true,
                    }
                } else {
                    true
                }
            }
            V2Expr::Field { base, field, .. } => {
                if let V2Expr::Ident(_, bname) = base.as_ref() {
                    if self.is_bcon_table_ref(bname) {
                        // Read-only BCON constant: never aliases a writable cell.
                        return false;
                    }
                    if self.is_self_ident(bname) {
                        if let Some(st) = self.env.states.get(field) {
                            return format!("{}.attr[{}]", self.self_prefix(), st.attr_index)
                                == canon_target;
                        }
                        let prop_air = match field.as_str() {
                            "graphic" => format!("{}.data[0]", self.self_prefix()),
                            "flags" => format!("{}.data[8]", self.self_prefix()),
                            "allowed_heights" => format!("{}.data[4]", self.self_prefix()),
                            "placement" => format!("{}.data[42]", self.self_prefix()),
                            "prep_score" => format!("{}.data[31]", self.self_prefix()),
                            "cook_score" => format!("{}.data[32]", self.self_prefix()),
                            "in_use" => format!("{}.data[25]", self.self_prefix()),
                            "id" => format!("{}.data[11]", self.self_prefix()),
                            "room" => format!("{}.data[29]", self.self_prefix()),
                            _ => return true,
                        };
                        return prop_air == canon_target;
                    }
                    if self.is_sim_ident(bname) {
                        let sim_air = match field.as_str() {
                            "id" => "my.data[11]",
                            "priority" => "my.person[33]",
                            "room" => "my.data[29]",
                            "hand" => "my.slot[0]",
                            "age" => "my.person[58]",
                            _ => return true,
                        };
                        return sim_air == canon_target;
                    }
                    // Foreign object field access swaps `stack_obj` AND the foreign handle may
                    // alias the current object (`stack.*` in interactions OR `my.attr`/`my.data`
                    // in `ObjectSelf` lifecycle hooks such as `for p in objects_of::<Self>()`)!
                    return canon_target.starts_with("stack.")
                        || canon_target == "stack_obj"
                        || canon_target.starts_with("my.attr[")
                        || canon_target.starts_with("my.data[");
                }
                if let V2Expr::Field {
                    base: root,
                    field: group,
                    ..
                } = base.as_ref()
                    && let V2Expr::Ident(_, rname) = root.as_ref()
                    && self.is_sim_ident(rname)
                {
                    let sim_air = match group.as_str() {
                        "motive" => canonical_air_cell(&format!("my.motive.{field}")),
                        "skill" => canonical_air_cell(&format!("my.skill.{field}")),
                        _ => return true,
                    };
                    return sim_air == canon_target;
                }
                true
            }
            V2Expr::Index { base, index, .. } => {
                if self.expr_may_read_or_clobber_air(index, canon_target) {
                    return true;
                }
                if let V2Expr::Ident(_, bname) = base.as_ref()
                    && matches!(bname.as_str(), "temp" | "global")
                {
                    return if let Ok(idx) = self.eval_const_i16(index) {
                        format!("{bname}[{idx}]") == canon_target
                    } else {
                        true
                    };
                }
                if let V2Expr::Field {
                    base: root, field, ..
                } = base.as_ref()
                    && let V2Expr::Ident(_, rname) = root.as_ref()
                    && self.is_self_ident(rname)
                    && matches!(field.as_str(), "attr" | "data")
                {
                    return if let Ok(idx) = self.eval_const_i16(index) {
                        format!("{}.{field}[{idx}]", self.self_prefix()) == canon_target
                    } else {
                        true
                    };
                }
                true
            }
            V2Expr::Unary { inner, .. } | V2Expr::MutRef { inner, .. } => {
                self.expr_may_read_or_clobber_air(inner, canon_target)
            }
            V2Expr::Binary { lhs, rhs, .. } => {
                self.expr_may_read_or_clobber_air(lhs, canon_target)
                    || self.expr_may_read_or_clobber_air(rhs, canon_target)
            }
            V2Expr::Tuple(_, elems) => elems
                .iter()
                .any(|e| self.expr_may_read_or_clobber_air(e, canon_target)),
            // Calls/method calls (and `?`-propagated calls) may have arbitrary side effects.
            V2Expr::Call { .. } | V2Expr::MethodCall { .. } | V2Expr::Try { .. } => true,
        }
    }

    /// Lowers a compound arithmetic/bitwise/unary-negation expression directly into `dst_air`,
    /// collapsing left-associative chains `(a * 24) / 5 + 1` into zero extra scratch locals!
    fn lower_arith_into_dest(
        &mut self,
        expr: &V2Expr,
        dst_air: &str,
        declare_let: bool,
    ) -> Result<ValueType> {
        // Constant-fold compile-time `Bitflags` expressions (e.g. `Placement::Floor | Placement::Terrain` -> `3`).
        if let Some((key, mask, _)) = self.try_eval_any_const_bitflag(expr)? {
            if declare_let {
                self.line(format!("let {dst_air} = {mask};"));
            } else {
                self.line(format!("{dst_air} := {mask};"));
            }
            return Ok(ValueType::Bitflags(key));
        }

        let (base, ops) = flatten_arith_chain(expr);

        // Emit base into `dst_air`.
        let base_ty = match base {
            V2Expr::Unary {
                span,
                op: V2UnaryOp::Neg,
                inner,
            } => {
                if let V2Expr::Int(_, v) = inner.as_ref() {
                    let n = v
                        .checked_neg()
                        .and_then(|x| i16::try_from(x).ok())
                        .ok_or_else(|| {
                            Error::at(*span, format!("negated integer -{v} out of i16 range"))
                        })?;
                    if declare_let {
                        self.line(format!("let {dst_air} = {n};"));
                    } else {
                        self.line(format!("{dst_air} := {n};"));
                    }
                    ValueType::I16
                } else {
                    let (v_air, v_ty) = self.lower_value_expr(inner)?;
                    if v_ty != ValueType::I16 {
                        return Err(Error::at(
                            *span,
                            format!(
                                "cannot apply unary '-' to '{}' expression",
                                self.env.display_value_type(&v_ty)
                            ),
                        ));
                    }
                    if declare_let {
                        self.line(format!("let {dst_air} = 0;"));
                    } else {
                        self.line(format!("{dst_air} := 0;"));
                    }
                    self.line(format!("{dst_air} -= {v_air};"));
                    ValueType::I16
                }
            }
            other => {
                let (b_air, b_ty) = self.lower_value_expr(other)?;
                if !ops.is_empty() && !matches!(b_ty, ValueType::I16 | ValueType::Bitflags(_)) {
                    return Err(Error::at(
                        other.span(),
                        format!(
                            "arithmetic/bitwise operator cannot be applied to '{}' operand",
                            self.env.display_value_type(&b_ty)
                        ),
                    ));
                }
                if declare_let {
                    self.line(format!("let {dst_air} = {b_air};"));
                } else if b_air != dst_air {
                    self.line(format!("{dst_air} := {b_air};"));
                }
                b_ty
            }
        };

        for (op, rhs) in ops {
            if matches!(base_ty, ValueType::Bitflags(_))
                && !matches!(op, V2BinOp::BitAnd | V2BinOp::BitOr | V2BinOp::BitXor)
            {
                return Err(Error::at(
                    rhs.span(),
                    format!(
                        "arithmetic operator cannot be applied to bitflags '{}' (only |, &, ^ are allowed)",
                        self.env.display_value_type(&base_ty)
                    ),
                ));
            }
            let (r_air, r_ty) = self.lower_value_expr(rhs)?;
            if r_ty != base_ty {
                return Err(Error::at(
                    rhs.span(),
                    format!(
                        "mismatched operand types in arithmetic/bitwise expression: '{}' vs '{}'",
                        self.env.display_value_type(&base_ty),
                        self.env.display_value_type(&r_ty)
                    ),
                ));
            }
            let aop = match op {
                V2BinOp::Add => "+=",
                V2BinOp::Sub => "-=",
                V2BinOp::Mul => "*=",
                V2BinOp::Div => "/=",
                V2BinOp::Mod => "%=",
                V2BinOp::BitAnd => "&=",
                V2BinOp::BitOr => "|=",
                V2BinOp::BitXor => "^=",
                _ => unreachable!(),
            };
            self.line(format!("{dst_air} {aop} {r_air};"));
        }

        Ok(base_ty)
    }

    /// Lowers a value-producing expression into an `.air` operand string + its [`ValueType`].
    fn lower_value_expr(&mut self, expr: &V2Expr) -> Result<(String, ValueType)> {
        match expr {
            V2Expr::Int(span, v) => {
                let n = i16::try_from(*v)
                    .map_err(|_| Error::at(*span, format!("integer {v} out of i16 range")))?;
                Ok((n.to_string(), ValueType::I16))
            }
            V2Expr::Bool(_, b) => Ok((if *b { "1" } else { "0" }.to_owned(), ValueType::Bool)),
            V2Expr::Path(span, segs) => {
                if segs.len() != 2 {
                    return Err(Error::at(
                        *span,
                        format!("unsupported path expression '{}'", segs.join("::")),
                    ));
                }
                let (tname, vname) = (&segs[0], &segs[1]);
                if let Some(e) =
                    self.env
                        .lookup_enum(&self.owner_module, tname, self.in_object_scope)
                {
                    let &val = e.variants.get(vname).ok_or_else(|| {
                        Error::at(
                            *span,
                            format!("enum '{}' has no variant '{vname}'", e.display_name),
                        )
                    })?;
                    return Ok((val.to_string(), ValueType::Enum(e.canonical_key.clone())));
                }
                if let Some(bf) =
                    self.env
                        .lookup_bitflags(&self.owner_module, tname, self.in_object_scope)
                {
                    let mask = if vname == "None" {
                        0i16
                    } else if let Some(&bit) = bf.flags.get(vname) {
                        bit_to_mask(bit)
                    } else {
                        return Err(Error::at(
                            *span,
                            format!("bitflags '{}' has no flag '{vname}'", bf.display_name),
                        ));
                    };
                    return Ok((
                        mask.to_string(),
                        ValueType::Bitflags(bf.canonical_key.clone()),
                    ));
                }
                Err(Error::at(
                    *span,
                    format!("unknown enum or bitflags type '{tname}'"),
                ))
            }
            V2Expr::Ident(span, name) => {
                if let Some(binding) = self.lookup_var(name).cloned() {
                    return match binding {
                        Binding::Local { air_name, ty } => Ok((air_name, ty)),
                        Binding::ConstI16 { air, .. } => Ok((air, ValueType::I16)),
                        Binding::ConstBool(b) => {
                            Ok((if b { "1" } else { "0" }.to_owned(), ValueType::Bool))
                        }
                        Binding::ConstEnum { key, value } => {
                            Ok((value.to_string(), ValueType::Enum(key)))
                        }
                        Binding::ConstBitflag { key, mask, .. } => {
                            Ok((mask.to_string(), ValueType::Bitflags(key)))
                        }
                        Binding::AutonomousFlag => {
                            let tmp = self.fresh("auto");
                            self.line(format!("let {tmp} = 0;"));
                            self.line("if param[0] == 1 {".to_owned());
                            self.indent += 1;
                            self.line(format!("{tmp} := 1;"));
                            self.indent -= 1;
                            self.line("}".to_owned());
                            Ok((tmp, ValueType::Bool))
                        }
                        Binding::BhavParam { index, ty } => Ok((format!("param[{index}]"), ty)),
                        Binding::ForeignObject { air_name } => Ok((air_name, ValueType::ObjectId)),
                        Binding::MutRef { .. } => Err(Error::at(
                            *span,
                            format!("use '*{name}' to read through '&mut' parameter '{name}'"),
                        )),
                        Binding::SelfHandle => match self.tree_ctx {
                            TreeContext::ObjectSelf => {
                                Ok(("my.id".to_owned(), ValueType::ObjectId))
                            }
                            TreeContext::SimOnObject { .. } => Ok((
                                if self.has_self_obj_local {
                                    "__self_obj".to_owned()
                                } else {
                                    "stack_obj".to_owned()
                                },
                                ValueType::ObjectId,
                            )),
                        },
                        Binding::SimHandle => Ok(("my.id".to_owned(), ValueType::ObjectId)),
                        Binding::Motive(_)
                        | Binding::Skill(_)
                        | Binding::Sound(_)
                        | Binding::Slot(_) => Err(Error::at(
                            *span,
                            format!("'{name}' cannot be used directly as a numeric value"),
                        )),
                    };
                }
                if self.is_self_ident(name) {
                    return match self.tree_ctx {
                        TreeContext::ObjectSelf => Ok(("my.id".to_owned(), ValueType::ObjectId)),
                        TreeContext::SimOnObject { .. } => Ok((
                            if self.has_self_obj_local {
                                "__self_obj".to_owned()
                            } else {
                                "stack_obj".to_owned()
                            },
                            ValueType::ObjectId,
                        )),
                    };
                }
                if name == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                    return Err(Error::at(
                        *span,
                        "'sim' is not available in init/main (object lifecycle context)",
                    ));
                }
                if let Some(v) = self.const_item(name) {
                    let n = i16::try_from(v).map_err(|_| {
                        Error::at(
                            *span,
                            format!(
                                "const '{name}' ({v:#x}) does not fit in i16; use it where a GUID is expected"
                            ),
                        )
                    })?;
                    return Ok((n.to_string(), ValueType::I16));
                }
                if name == "stack_obj" {
                    return Ok(("stack_obj".to_owned(), ValueType::ObjectId));
                }
                if self.env.i16_params.contains_key(name) {
                    return Ok((format!("bcon.{name}"), ValueType::I16));
                }
                if let Some(&b) = self.env.bool_params.get(name) {
                    return Ok((if b { "1" } else { "0" }.to_owned(), ValueType::Bool));
                }
                Err(Error::at(*span, format!("unknown identifier '{name}'")))
            }
            V2Expr::Deref { span, inner } => {
                let lval = self.resolve_lvalue(&V2Expr::Deref {
                    span: *span,
                    inner: inner.clone(),
                })?;
                self.read_lvalue(&lval)
            }
            V2Expr::Field { span, base, field } => {
                if let Some(r) = self.bcon_field_ref(expr) {
                    return Ok((r?.0, ValueType::I16));
                }
                // Handle read-only properties like `sim.id`, `sim.priority`, `self.id`, `self.room`
                if let V2Expr::Ident(bspan, bname) = base.as_ref() {
                    if self.is_sim_ident(bname) {
                        return match field.as_str() {
                            "id" => Ok(("my.id".to_owned(), ValueType::ObjectId)),
                            "priority" => Ok(("my.person.priority".to_owned(), ValueType::I16)),
                            "room" => Ok(("my.room".to_owned(), ValueType::I16)),
                            "hand" => Ok(("my.slot[0]".to_owned(), ValueType::ObjectId)),
                            "age" => Ok(("my.person[58]".to_owned(), ValueType::I16)),
                            other => Err(Error::at(
                                *span,
                                format!(
                                    "unknown Sim property '{bname}.{other}' (expected motive, skill, id, priority, room, hand, age, slot[i], or person[i]; use 'self.<state>' for object state)"
                                ),
                            )),
                        };
                    }
                    if bname == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                        return Err(Error::at(
                            *bspan,
                            "'sim' is not available in init/main (object lifecycle context)",
                        ));
                    }
                    if self.is_self_ident(bname) {
                        match field.as_str() {
                            "id" => {
                                return Ok((
                                    format!("{}.id", self.self_prefix()),
                                    ValueType::ObjectId,
                                ));
                            }
                            "room" => {
                                return Ok((
                                    format!("{}.room", self.self_prefix()),
                                    ValueType::I16,
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                let lval = self.resolve_lvalue(expr)?;
                self.read_lvalue(&lval)
            }
            V2Expr::Index { .. } => {
                let lval = self.resolve_lvalue(expr)?;
                self.read_lvalue(&lval)
            }
            V2Expr::Unary { span, op, inner } => match op {
                V2UnaryOp::Not => {
                    let tmp = self.fresh("not");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(expr, &tmp)?;
                    Ok((tmp, ValueType::Bool))
                }
                V2UnaryOp::Neg => {
                    if let V2Expr::Int(_, v) = inner.as_ref() {
                        let n = v
                            .checked_neg()
                            .and_then(|x| i16::try_from(x).ok())
                            .ok_or_else(|| {
                                Error::at(*span, format!("negated integer -{v} out of i16 range"))
                            })?;
                        return Ok((n.to_string(), ValueType::I16));
                    }
                    let tmp = self.fresh("neg");
                    let ty = self.lower_arith_into_dest(expr, &tmp, true)?;
                    Ok((tmp, ty))
                }
            },
            V2Expr::Binary { op, .. } => match op {
                V2BinOp::And
                | V2BinOp::Or
                | V2BinOp::Eq
                | V2BinOp::Ne
                | V2BinOp::Lt
                | V2BinOp::Le
                | V2BinOp::Gt
                | V2BinOp::Ge
                | V2BinOp::FlagTest => {
                    let tmp = self.fresh("bool");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(expr, &tmp)?;
                    Ok((tmp, ValueType::Bool))
                }
                V2BinOp::Add
                | V2BinOp::Sub
                | V2BinOp::Mul
                | V2BinOp::Div
                | V2BinOp::Mod
                | V2BinOp::BitAnd
                | V2BinOp::BitOr
                | V2BinOp::BitXor => {
                    // Constant-fold pure Bitflags expressions into an immediate literal!
                    if let Some((key, mask, _)) = self.try_eval_any_const_bitflag(expr)? {
                        return Ok((mask.to_string(), ValueType::Bitflags(key)));
                    }
                    let tmp = self.fresh("arith");
                    let ty = self.lower_arith_into_dest(expr, &tmp, true)?;
                    Ok((tmp, ty))
                }
            },
            V2Expr::Call {
                span,
                callee,
                type_args,
                args,
            } => {
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        "type arguments are only supported on 'create_object::<T>', 'find_best_object::<T>', and 'objects_of::<T>'",
                    ));
                }
                if let Some(sig) = self.resolve_callable(callee, *span)? {
                    return self.lower_callable_value(&sig, args, *span, false, false);
                }
                if let V2Expr::Ident(_, fname) = callee.as_ref() {
                    if fname == "random" {
                        if args.len() != 1 || args[0].name.is_some() {
                            return Err(Error::at(*span, "random(range) expects 1 argument"));
                        }
                        let (range_air, _) = self.lower_value_expr(&args[0].value)?;
                        let tmp = self.fresh("rnd");
                        self.line(format!("let {tmp} = 0;"));
                        self.line(format!("random({tmp}, {range_air});"));
                        return Ok((tmp, ValueType::I16));
                    }
                    if matches!(
                        fname.as_str(),
                        "can_afford"
                            | "debit_funds"
                            | "credit_funds"
                            | "dialog"
                            | "route_to"
                            | "snap"
                            | "find_best_object"
                            | "create_object"
                    ) {
                        let tmp = self.fresh("call");
                        self.line(format!("let {tmp} = 0;"));
                        self.emit_bool_into_local(expr, &tmp)?;
                        return Ok((tmp, ValueType::Bool));
                    }
                    if let Some((fn_mod, fn_decl, is_local)) =
                        self.env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                    {
                        let ret =
                            self.inline_helper_call(&fn_mod, &fn_decl, is_local, args, *span)?;
                        let (rv, vty) = ret.ok_or_else(|| {
                            Error::at(
                                *span,
                                format!("void helper function '{fname}' does not return a value"),
                            )
                        })?;
                        return Ok((rv, vty));
                    }
                    if is_prim_builtin_name(fname) {
                        let tmp = self.fresh("call");
                        self.line(format!("let {tmp} = 0;"));
                        self.emit_bool_into_local(expr, &tmp)?;
                        return Ok((tmp, ValueType::Bool));
                    }
                }
                Err(Error::at(*span, "unsupported value call expression"))
            }
            V2Expr::MethodCall {
                method, type_args, ..
            } if method == "is_type" && type_args.is_empty() => {
                let tmp = self.fresh("call");
                self.line(format!("let {tmp} = 0;"));
                self.emit_bool_into_local(expr, &tmp)?;
                Ok((tmp, ValueType::Bool))
            }
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if method == "run_functional" => {
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        "type arguments on run_functional are only valid inside 'let Some(x) = obj.run_functional::<T>(...)'",
                    ));
                }
                let tmp = self.fresh("rf");
                self.line(format!("let {tmp} = 0;"));
                self.emit_run_functional(receiver, args, *span, Some(&tmp))?;
                Ok((tmp, ValueType::Bool))
            }
            V2Expr::MethodCall { method, .. }
                if matches!(method.as_str(), "contains" | "intersects" | "is_empty") =>
            {
                let tmp = self.fresh("bf");
                self.line(format!("let {tmp} = 0;"));
                self.emit_bool_into_local(expr, &tmp)?;
                Ok((tmp, ValueType::Bool))
            }
            V2Expr::Try { span, inner } => {
                if let V2Expr::Call {
                    span: cspan,
                    callee,
                    type_args,
                    args,
                } = inner.as_ref()
                    && type_args.is_empty()
                    && let Some(sig) = self.resolve_callable(callee, *cspan)?
                {
                    return self.lower_callable_value(&sig, args, *cspan, true, false);
                }
                Err(Error::at(
                    *span,
                    "'?' in an expression needs a call to an 'extern mod' fn or 'tree' (use 'cond?;' as a statement for plain bool checks)",
                ))
            }
            other => Err(Error::at(
                other.span(),
                "expression cannot be evaluated as a value",
            )),
        }
    }

    fn read_lvalue(&mut self, lval: &LValue) -> Result<(String, ValueType)> {
        match lval {
            LValue::Direct { air, ty } => Ok((air.clone(), ty.clone())),
            LValue::ForeignAttr {
                obj_air,
                attr_index,
                ty,
            } => {
                let tmp = self.fresh("fattr");
                self.line(format!("stack_obj := {obj_air};"));
                self.line(format!("let {tmp} = stack.attr[{attr_index}];"));
                self.emit_restore_stack_obj();
                Ok((tmp, ty.clone()))
            }
            LValue::ForeignData {
                obj_air,
                data_index,
                ty,
            } => {
                let tmp = self.fresh("fdata");
                self.line(format!("stack_obj := {obj_air};"));
                self.line(format!("let {tmp} = stack.data[{data_index}];"));
                self.emit_restore_stack_obj();
                Ok((tmp, ty.clone()))
            }
        }
    }

    /// A `const` item named `name`, unless a local binding shadows it.
    fn const_item(&self, name: &str) -> Option<i64> {
        if self.lookup_var(name).is_some() {
            return None;
        }
        self.env
            .lookup_const(&self.owner_module, name, self.in_object_scope)
    }

    /// A 32-bit GUID operand: an integer literal or a `const` item.
    fn resolve_guid_expr(&self, expr: &V2Expr) -> Result<u32> {
        let (span, value) = match expr {
            V2Expr::Int(span, v) => (*span, *v),
            V2Expr::Ident(span, name) => match self.const_item(name) {
                Some(v) => (*span, v),
                None => {
                    return Err(Error::at(
                        *span,
                        format!("expected a GUID literal or const, found '{name}'"),
                    ));
                }
            },
            other => {
                return Err(Error::at(
                    other.span(),
                    "expected a GUID literal (0x...) or a const",
                ));
            }
        };
        u32::try_from(value).map_err(|_| Error::at(span, format!("GUID {value} out of u32 range")))
    }

    fn try_const_i16_air_operand(&self, expr: &V2Expr) -> Result<Option<(String, i16)>> {
        match expr {
            V2Expr::Int(span, v) => {
                let n = i16::try_from(*v)
                    .map_err(|_| Error::at(*span, format!("integer {v} out of i16 range")))?;
                Ok(Some((n.to_string(), n)))
            }
            V2Expr::Unary {
                span,
                op: V2UnaryOp::Neg,
                inner,
            } => {
                if let V2Expr::Int(_, v) = inner.as_ref() {
                    let n = v
                        .checked_neg()
                        .and_then(|x| i16::try_from(x).ok())
                        .ok_or_else(|| {
                            Error::at(*span, format!("negated integer -{v} out of i16 range"))
                        })?;
                    Ok(Some((n.to_string(), n)))
                } else {
                    Ok(None)
                }
            }
            V2Expr::Ident(_, name) => {
                if let Some(binding) = self.lookup_var(name) {
                    return match binding {
                        Binding::ConstI16 { air, value } => Ok(Some((air.clone(), *value))),
                        _ => Ok(None),
                    };
                }
                if self.is_self_ident(name)
                    || self.is_sim_ident(name)
                    || (name == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf))
                {
                    return Ok(None);
                }
                if let Some(v) = self.const_item(name)
                    && let Ok(v) = i16::try_from(v)
                {
                    return Ok(Some((v.to_string(), v)));
                }
                if let Some(&val) = self.env.i16_params.get(name) {
                    return Ok(Some((format!("bcon.{name}"), val)));
                }
                Ok(None)
            }
            V2Expr::Field { .. } => match self.bcon_field_ref(expr) {
                Some(r) => {
                    let (air, val) = r?;
                    Ok(val.map(|v| (air, v)))
                }
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// Resolves a compile-time bool for helper argument substitution and built-in compile-time
    /// options (`inform:`, `non_modal:`, `change_icon:`), including `Binding::ConstBool`.
    /// Unlike `try_eval_const_bool`, this is NEVER used to prune `if` branches inside helpers,
    /// so untaken branches remain type- and symbol-checked.
    fn try_const_bool_arg(&self, expr: &V2Expr) -> Option<bool> {
        self.try_eval_const_bool_inner(expr, true)
    }

    /// Folds a condition built only from literal comparisons (integer literals, enum variants,
    /// constant helper bindings), bool literals, constant helper bools, and `!`/`&&`/`||`. Object
    /// `i16` params are excluded: they are runtime `bcon` reads.
    fn try_fold_literal_cond(&self, expr: &V2Expr) -> Option<bool> {
        match expr {
            V2Expr::Bool(_, b) => Some(*b),
            V2Expr::Ident(_, name) => match self.lookup_var(name) {
                Some(Binding::ConstBool(b)) => Some(*b),
                _ => None,
            },
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self.try_fold_literal_cond(inner).map(|b| !b),
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => match (
                self.try_fold_literal_cond(lhs),
                self.try_fold_literal_cond(rhs),
            ) {
                (Some(a), Some(b)) => Some(a && b),
                _ => None,
            },
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => match (
                self.try_fold_literal_cond(lhs),
                self.try_fold_literal_cond(rhs),
            ) {
                (Some(a), Some(b)) => Some(a || b),
                _ => None,
            },
            V2Expr::Binary { lhs, op, rhs, .. } => {
                let (lk, l) = self.literal_compare_operand(lhs)?;
                let (rk, r) = self.literal_compare_operand(rhs)?;
                // Mixed kinds (enum vs integer, two enums) take the normal, type-checked path.
                if lk != rk {
                    return None;
                }
                match op {
                    V2BinOp::Eq => Some(l == r),
                    V2BinOp::Ne => Some(l != r),
                    V2BinOp::Lt => Some(l < r),
                    V2BinOp::Le => Some(l <= r),
                    V2BinOp::Gt => Some(l > r),
                    V2BinOp::Ge => Some(l >= r),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// `(enum key, value)` of a compile-time comparison operand (`None` key for plain integers).
    fn literal_compare_operand(&self, expr: &V2Expr) -> Option<(Option<String>, i16)> {
        match expr {
            V2Expr::Path(_, segs) if segs.len() == 2 => {
                let e = self
                    .env
                    .lookup_enum(&self.owner_module, &segs[0], self.in_object_scope)?;
                let &v = e.variants.get(&segs[1])?;
                Some((Some(e.canonical_key.clone()), v))
            }
            V2Expr::Ident(_, name)
                if let Some(Binding::ConstEnum { key, value }) = self.lookup_var(name) =>
            {
                Some((Some(key.clone()), *value))
            }
            _ => {
                let (air, v) = self.try_const_i16_air_operand(expr).ok().flatten()?;
                air.parse::<i16>().ok().map(|_| (None, v))
            }
        }
    }

    /// Lowers a constant-folded-away branch into a discarded buffer so it is still type- and
    /// symbol-checked, then restores the lowering state.
    fn check_dead_branch(&mut self, stmts: &[V2Stmt]) -> Result<()> {
        if stmts.is_empty() {
            return Ok(());
        }
        let out_len = self.out.len();
        let indent = self.indent;
        let used_saved = self.used_saved_stack_obj;
        let expansions = self.inline_expansions;
        let strings = self.env.dialog_strings.borrow().clone();
        self.push_scope(ScopeFrame::block());
        let r = self.lower_stmts(stmts);
        let p = self.pop_scope_and_run_cleanups();
        self.out.truncate(out_len);
        self.indent = indent;
        self.used_saved_stack_obj = used_saved;
        self.inline_expansions = expansions;
        *self.env.dialog_strings.borrow_mut() = strings;
        r.and(p)
    }

    fn try_eval_const_bool(&self, expr: &V2Expr) -> Option<bool> {
        self.try_eval_const_bool_inner(expr, false)
    }

    fn try_eval_const_bool_inner(&self, expr: &V2Expr, allow_const_binding: bool) -> Option<bool> {
        match expr {
            V2Expr::Bool(_, b) => Some(*b),
            V2Expr::Ident(_, name) => {
                if let Some(binding) = self.lookup_var(name) {
                    if allow_const_binding && let Binding::ConstBool(b) = binding {
                        return Some(*b);
                    }
                    return None;
                }
                if self.is_self_ident(name)
                    || self.is_sim_ident(name)
                    || (name == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf))
                {
                    None
                } else {
                    self.env.bool_params.get(name).copied()
                }
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self
                .try_eval_const_bool_inner(inner, allow_const_binding)
                .map(|b| !b),
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => match self.try_eval_const_bool_inner(lhs, allow_const_binding) {
                Some(false) => Some(false),
                Some(true) => self.try_eval_const_bool_inner(rhs, allow_const_binding),
                None => None,
            },
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => match self.try_eval_const_bool_inner(lhs, allow_const_binding) {
                Some(true) => Some(true),
                Some(false) => self.try_eval_const_bool_inner(rhs, allow_const_binding),
                None => None,
            },
            _ => None,
        }
    }

    fn eval_const_bool_required(&self, expr: &V2Expr, span: Span) -> Result<bool> {
        self.try_const_bool_arg(expr).ok_or_else(|| {
            Error::at(
                span,
                "expected compile-time boolean ('true', 'false', or bool param)",
            )
        })
    }

    fn eval_const_i16(&self, expr: &V2Expr) -> Result<i16> {
        match expr {
            V2Expr::Int(span, v) => i16::try_from(*v)
                .map_err(|_| Error::at(*span, format!("integer {v} out of i16 range"))),
            V2Expr::Unary {
                span,
                op: V2UnaryOp::Neg,
                inner,
            } => {
                let inner_v = self.eval_const_i16(inner)?;
                inner_v
                    .checked_neg()
                    .ok_or_else(|| Error::at(*span, "i16 negation overflow"))
            }
            V2Expr::Ident(span, name) => {
                if let Some(binding) = self.lookup_var(name) {
                    return match binding {
                        Binding::ConstI16 { value, .. } => Ok(*value),
                        _ => Err(Error::at(
                            *span,
                            format!("expected constant i16, found non-constant variable '{name}'"),
                        )),
                    };
                }
                if self.is_self_ident(name)
                    || self.is_sim_ident(name)
                    || (name == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf))
                {
                    return Err(Error::at(
                        *span,
                        format!("expected constant i16, found '{name}'"),
                    ));
                }
                if let Some(v) = self.const_item(name) {
                    return i16::try_from(v).map_err(|_| {
                        Error::at(
                            *span,
                            format!("const '{name}' ({v:#x}) is a GUID, not an i16"),
                        )
                    });
                }
                self.env.i16_params.get(name).copied().ok_or_else(|| {
                    Error::at(*span, format!("expected constant i16, found '{name}'"))
                })
            }
            V2Expr::Field { span, .. } => match self.bcon_field_ref(expr) {
                Some(r) => r?.1.ok_or_else(|| {
                    Error::at(
                        *span,
                        "expected constant i16, found an 'extern bcon' entry (its value is only known at runtime)",
                    )
                }),
                None => Err(Error::at(*span, "expected constant i16 index")),
            },
            other => Err(Error::at(other.span(), "expected constant i16 index")),
        }
    }

    /// Returns `true` iff lowering `expr` emits zero setup statements and has zero side effects.
    fn is_pure_single_operand(&self, expr: &V2Expr) -> bool {
        match expr {
            V2Expr::Int(_, _) | V2Expr::Bool(_, _) | V2Expr::Path(_, _) => true,
            V2Expr::Ident(_, name) => !matches!(
                self.lookup_var(name),
                Some(
                    Binding::AutonomousFlag
                        | Binding::MutRef {
                            target: LValue::ForeignAttr { .. } | LValue::ForeignData { .. }
                        }
                )
            ),
            V2Expr::Unary {
                op: V2UnaryOp::Neg,
                inner,
                ..
            } => matches!(inner.as_ref(), V2Expr::Int(_, _)),
            V2Expr::Deref { inner, .. } => {
                if let V2Expr::Ident(_, pname) = inner.as_ref() {
                    matches!(
                        self.lookup_var(pname),
                        Some(Binding::MutRef {
                            target: LValue::Direct { .. }
                        })
                    )
                } else {
                    false
                }
            }
            V2Expr::Field { base, .. } => match base.as_ref() {
                V2Expr::Ident(_, bname) => {
                    self.is_self_ident(bname)
                        || self.is_sim_ident(bname)
                        || self.is_bcon_table_ref(bname)
                }
                V2Expr::Field { base: root, .. } => {
                    matches!(root.as_ref(), V2Expr::Ident(_, rname) if self.is_sim_ident(rname))
                }
                _ => false,
            },
            V2Expr::Index { base, index, .. } => {
                if self.eval_const_i16(index).is_err() {
                    return false;
                }
                match base.as_ref() {
                    V2Expr::Ident(_, bname) => matches!(bname.as_str(), "temp" | "global"),
                    V2Expr::Field { base: root, .. } => {
                        matches!(
                            root.as_ref(),
                            V2Expr::Ident(_, rname) if self.is_self_ident(rname) || self.is_sim_ident(rname)
                        )
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Returns `true` iff `expr` can be lowered directly into an `.air` `while <cond>` header
    /// without hoisting any setup statements before the loop!
    fn is_pure_simple_air_cond(&self, expr: &V2Expr) -> bool {
        match expr {
            V2Expr::Bool(_, _) => true,
            V2Expr::Ident(_, name)
                if matches!(self.lookup_var(name), Some(Binding::AutonomousFlag)) =>
            {
                true
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self.is_pure_simple_air_cond(inner),
            V2Expr::Binary { lhs, op, rhs, .. } => match op {
                V2BinOp::Eq
                | V2BinOp::Ne
                | V2BinOp::Lt
                | V2BinOp::Le
                | V2BinOp::Gt
                | V2BinOp::Ge
                | V2BinOp::FlagTest => {
                    self.is_pure_single_operand(lhs) && self.is_pure_single_operand(rhs)
                }
                _ => false,
            },
            V2Expr::MethodCall {
                receiver,
                method,
                args,
                ..
            } if method == "is_empty" && args.is_empty() => self.is_pure_single_operand(receiver),
            V2Expr::Call { .. } | V2Expr::MethodCall { .. } => false,
            _ => self.is_pure_single_operand(expr),
        }
    }

    fn lower_cmp_operands(
        &mut self,
        lhs: &V2Expr,
        op: V2BinOp,
        rhs: &V2Expr,
        span: Span,
    ) -> Result<(String, String)> {
        let (l, l_ty) = self.lower_value_expr(lhs)?;
        if op == V2BinOp::FlagTest
            && let ValueType::Bitflags(ref key) = l_ty
        {
            let Some((_, Some(bit))) = self.try_eval_const_bitflag(rhs, key)? else {
                return Err(Error::at(
                    rhs.span(),
                    format!(
                        "'flag_test' on bitflags '{}' requires a single flag constant (use '.contains(...)' for masks)",
                        self.env.display_value_type(&l_ty)
                    ),
                ));
            };
            return Ok((l, bit.to_string()));
        }
        let (l_eff, r, r_ty) = if self.is_pure_single_operand(rhs) {
            let (r, r_ty) = self.lower_value_expr(rhs)?;
            (l, r, r_ty)
        } else {
            let snap_l = self.fresh("cmp_l");
            self.line(format!("let {snap_l} = {l};"));
            let (r, r_ty) = self.lower_value_expr(rhs)?;
            (snap_l, r, r_ty)
        };
        match op {
            V2BinOp::Eq | V2BinOp::Ne => {
                if !types_compatible_for_assign(&l_ty, &r_ty)
                    && !types_compatible_for_assign(&r_ty, &l_ty)
                {
                    return Err(Error::at(
                        span,
                        format!(
                            "cannot compare '{}' with '{}'",
                            self.env.display_value_type(&l_ty),
                            self.env.display_value_type(&r_ty)
                        ),
                    ));
                }
            }
            V2BinOp::Lt | V2BinOp::Le | V2BinOp::Gt | V2BinOp::Ge => {
                if !matches!(l_ty, ValueType::I16 | ValueType::ObjectId)
                    || !matches!(r_ty, ValueType::I16 | ValueType::ObjectId)
                {
                    return Err(Error::at(
                        span,
                        format!(
                            "ordering comparison is not supported between '{}' and '{}'",
                            self.env.display_value_type(&l_ty),
                            self.env.display_value_type(&r_ty)
                        ),
                    ));
                }
            }
            V2BinOp::FlagTest if l_ty != ValueType::I16 || r_ty != ValueType::I16 => {
                return Err(Error::at(
                    span,
                    format!(
                        "'flag_test' requires matching bitflags or i16 operands, found '{}' and '{}'",
                        self.env.display_value_type(&l_ty),
                        self.env.display_value_type(&r_ty)
                    ),
                ));
            }
            _ => {}
        }
        Ok((l_eff, r))
    }

    fn lower_bitflag_predicate(
        &mut self,
        receiver: &V2Expr,
        method: &str,
        args: &[CallArg],
        span: Span,
        negated: bool,
    ) -> Result<String> {
        let derefed = self.maybe_auto_deref_mut_ref(receiver);
        let (lhs_air, lhs_ty) = self.lower_value_expr(derefed.as_ref().unwrap_or(receiver))?;
        let ValueType::Bitflags(key) = lhs_ty else {
            return Err(Error::at(
                span,
                format!(
                    "'.{method}(...)' requires a bitflags receiver, found '{}'",
                    self.env.display_value_type(&lhs_ty)
                ),
            ));
        };
        if method == "is_empty" {
            if !args.is_empty() {
                return Err(Error::at(span, "'.is_empty()' takes no arguments"));
            }
            return Ok(if negated {
                format!("{lhs_air} != 0")
            } else {
                format!("{lhs_air} == 0")
            });
        }
        if args.len() != 1 || args[0].name.is_some() {
            return Err(Error::at(
                span,
                format!("'.{method}(flag)' expects exactly 1 positional argument"),
            ));
        }
        if let Some((mask, single_bit)) = self.try_eval_const_bitflag(&args[0].value, &key)? {
            if let Some(bit) = single_bit {
                return Ok(if negated {
                    format!("!{lhs_air} flag_test {bit}")
                } else {
                    format!("{lhs_air} flag_test {bit}")
                });
            }
            if mask == 0 {
                let res = method == "contains";
                let effective = if negated { !res } else { res };
                return Ok(if effective { "1 == 1" } else { "1 == 0" }.to_owned());
            }
            let tmp = self.fresh("bf_tst");
            self.line(format!("let {tmp} = {lhs_air};"));
            self.line(format!("{tmp} &= {mask};"));
            return Ok(if method == "contains" {
                if negated {
                    format!("{tmp} != {mask}")
                } else {
                    format!("{tmp} == {mask}")
                }
            } else if negated {
                format!("{tmp} == 0")
            } else {
                format!("{tmp} != 0")
            });
        }
        let lhs_snap = if self.is_pure_single_operand(&args[0].value) {
            lhs_air
        } else {
            let s = self.fresh("bf_lhs");
            self.line(format!("let {s} = {lhs_air};"));
            s
        };
        let (rhs_air, rhs_ty) = self.lower_value_expr(&args[0].value)?;
        if !types_compatible_for_assign(&ValueType::Bitflags(key.clone()), &rhs_ty)
            || !matches!(rhs_ty, ValueType::Bitflags(_))
        {
            return Err(Error::at(
                args[0].span,
                format!(
                    "type mismatch in '.{method}(...)': expected '{}', found '{}'",
                    self.env.display_value_type(&ValueType::Bitflags(key)),
                    self.env.display_value_type(&rhs_ty)
                ),
            ));
        }
        let tmp = self.fresh("bf_tst");
        self.line(format!("let {tmp} = {lhs_snap};"));
        self.line(format!("{tmp} &= {rhs_air};"));
        Ok(if method == "contains" {
            if negated {
                format!("{tmp} != {rhs_air}")
            } else {
                format!("{tmp} == {rhs_air}")
            }
        } else if negated {
            format!("{tmp} == 0")
        } else {
            format!("{tmp} != 0")
        })
    }

    /// Tries to lower a boolean condition into a single `.air` `Condition` string (no `&&`/`||`).
    fn try_lower_simple_air_cond(&mut self, expr: &V2Expr) -> Result<Option<String>> {
        match expr {
            V2Expr::Bool(_, true) => Ok(Some("1 == 1".to_owned())),
            V2Expr::Bool(_, false) => Ok(Some("1 == 0".to_owned())),
            V2Expr::Ident(_, name)
                if matches!(self.lookup_var(name), Some(Binding::AutonomousFlag)) =>
            {
                Ok(Some("param[0] == 1".to_owned()))
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => {
                if let Some(neg) = self.try_lower_negated_simple_air_cond(inner)? {
                    return Ok(Some(neg));
                }
                Ok(None)
            }
            V2Expr::Binary { span, lhs, op, rhs } => match op {
                V2BinOp::And | V2BinOp::Or => Ok(None),
                V2BinOp::Eq
                | V2BinOp::Ne
                | V2BinOp::Lt
                | V2BinOp::Le
                | V2BinOp::Gt
                | V2BinOp::Ge
                | V2BinOp::FlagTest => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    let sym = cmp_op_str(*op);
                    Ok(Some(format!("{l} {sym} {r}")))
                }
                _ => Err(Error::at(
                    expr.span(),
                    "arithmetic expression cannot be used directly as a boolean condition",
                )),
            },
            V2Expr::Call {
                span,
                callee,
                type_args,
                args,
            } => {
                if !type_args.is_empty() {
                    return Ok(None);
                }
                if let Some(sig) = self.resolve_callable(callee, *span)? {
                    return match sig.ret {
                        ResolvedReturnAbi::Bool | ResolvedReturnAbi::Result => {
                            Ok(Some(self.prepare_callable_call(&sig, args, *span)?))
                        }
                        _ => Err(Error::at(
                            *span,
                            format!(
                                "{} cannot be used as a condition (only '-> bool' and '-> Result' calls can)",
                                sig.display
                            ),
                        )),
                    };
                }
                if let V2Expr::Ident(_, fname) = callee.as_ref() {
                    match fname.as_str() {
                        "can_afford" => {
                            return Ok(Some(self.format_alter_budget(args, *span, true, false)?));
                        }
                        "debit_funds" => {
                            return Ok(Some(self.format_alter_budget(args, *span, false, false)?));
                        }
                        "credit_funds" => {
                            return Ok(Some(self.format_alter_budget(args, *span, false, true)?));
                        }
                        "dialog" => return Ok(Some(self.format_dialog(args, *span)?)),
                        "route_to" => {
                            return Ok(Some(self.format_slot_prim("route_to", args, *span)?));
                        }
                        "snap" => return Ok(Some(self.format_slot_prim("snap", args, *span)?)),
                        _ => {}
                    }
                    if let Some((fn_mod, fn_decl, is_local)) =
                        self.env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                        && self.is_tail_return_bool_helper(&fn_decl)
                    {
                        let (text, _) = self
                            .inline_helper_call_mode(
                                &fn_mod, &fn_decl, is_local, args, *span, true,
                            )?
                            .expect("tail bool helpers return a condition");
                        return Ok(Some(text));
                    }
                    if self
                        .env
                        .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                        .is_none()
                    {
                        return self.format_prim_builtin(fname, args, *span);
                    }
                    return Ok(None);
                }
                Ok(None)
            }
            V2Expr::MethodCall { method, .. } if method == "run_functional" => Ok(None),
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if method == "is_type" && type_args.is_empty() => {
                Ok(Some(self.format_is_type(receiver, args, *span)?))
            }
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if matches!(method.as_str(), "contains" | "intersects" | "is_empty") => {
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        format!("'.{method}()' does not accept type arguments"),
                    ));
                }
                Ok(Some(self.lower_bitflag_predicate(
                    receiver, method, args, *span, false,
                )?))
            }
            _ => {
                let (val, ty) = self.lower_value_expr(expr)?;
                if ty != ValueType::Bool {
                    return Err(Error::at(
                        expr.span(),
                        "expected boolean condition (use an explicit comparison like 'x != 0')",
                    ));
                }
                Ok(Some(format!("{val} != 0")))
            }
        }
    }

    fn try_lower_negated_simple_air_cond(&mut self, expr: &V2Expr) -> Result<Option<String>> {
        match expr {
            V2Expr::Bool(_, true) => Ok(Some("1 == 0".to_owned())),
            V2Expr::Bool(_, false) => Ok(Some("1 == 1".to_owned())),
            V2Expr::Ident(_, name)
                if matches!(self.lookup_var(name), Some(Binding::AutonomousFlag)) =>
            {
                Ok(Some("param[0] != 1".to_owned()))
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self.try_lower_simple_air_cond(inner),
            V2Expr::Binary { span, lhs, op, rhs } => match op {
                V2BinOp::And | V2BinOp::Or => Ok(None),
                V2BinOp::Eq => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} != {r}")))
                }
                V2BinOp::Ne => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} == {r}")))
                }
                V2BinOp::Lt => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} >= {r}")))
                }
                V2BinOp::Le => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} > {r}")))
                }
                V2BinOp::Gt => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} <= {r}")))
                }
                V2BinOp::Ge => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} < {r}")))
                }
                V2BinOp::FlagTest => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("!{l} flag_test {r}")))
                }
                _ => Err(Error::at(
                    expr.span(),
                    "cannot apply '!' to arithmetic expression",
                )),
            },
            V2Expr::Call { .. } => {
                if let Some(pos) = self.try_lower_simple_air_cond(expr)? {
                    return Ok(Some(format!("!{pos}")));
                }
                Ok(None)
            }
            V2Expr::MethodCall { method, .. } if method == "run_functional" => Ok(None),
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if method == "is_type" && type_args.is_empty() => Ok(Some(format!(
                "!{}",
                self.format_is_type(receiver, args, *span)?
            ))),
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if matches!(method.as_str(), "contains" | "intersects" | "is_empty") => {
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        format!("'.{method}()' does not accept type arguments"),
                    ));
                }
                Ok(Some(self.lower_bitflag_predicate(
                    receiver, method, args, *span, true,
                )?))
            }
            _ => {
                let (val, ty) = self.lower_value_expr(expr)?;
                if ty != ValueType::Bool {
                    return Err(Error::at(
                        expr.span(),
                        "cannot apply '!' to non-bool expression",
                    ));
                }
                Ok(Some(format!("{val} == 0")))
            }
        }
    }

    /// Evaluates an arbitrary boolean expression (including short-circuit `&&`, `||`, `!`,
    /// helper calls, and `run_functional`) into `dst_local` (`0` or `1`).
    fn emit_bool_into_local(&mut self, expr: &V2Expr, dst_local: &str) -> Result<()> {
        if let Some(b) = self.try_eval_const_bool(expr) {
            self.line(format!("{dst_local} := {};", if b { 1 } else { 0 }));
            return Ok(());
        }
        match expr {
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => {
                // Short-circuit `lhs && rhs`: evaluate rhs ONLY when lhs is true!
                self.emit_bool_into_local(lhs, dst_local)?;
                self.line(format!("if {dst_local} != 0 {{"));
                self.indent += 1;
                self.emit_bool_into_local(rhs, dst_local)?;
                self.indent -= 1;
                self.line("}".to_owned());
                Ok(())
            }
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => {
                // Short-circuit `lhs || rhs`: evaluate rhs ONLY when lhs is false!
                self.emit_bool_into_local(lhs, dst_local)?;
                self.line(format!("if {dst_local} == 0 {{"));
                self.indent += 1;
                self.emit_bool_into_local(rhs, dst_local)?;
                self.indent -= 1;
                self.line("}".to_owned());
                Ok(())
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => {
                if let Some(neg) = self.try_lower_negated_simple_air_cond(inner)? {
                    self.line(format!("if {neg} {{"));
                    self.indent += 1;
                    self.line(format!("{dst_local} := 1;"));
                    self.indent -= 1;
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.line(format!("{dst_local} := 0;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                } else {
                    self.emit_bool_into_local(inner, dst_local)?;
                    self.line(format!("if {dst_local} == 0 {{"));
                    self.indent += 1;
                    self.line(format!("{dst_local} := 1;"));
                    self.indent -= 1;
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.line(format!("{dst_local} := 0;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                }
                Ok(())
            }
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if method == "run_functional" => {
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        "type arguments on run_functional are only valid inside 'let Some(x) = obj.run_functional::<T>(...)'",
                    ));
                }
                self.emit_run_functional(receiver, args, *span, Some(dst_local))
            }
            V2Expr::Call {
                span,
                callee,
                type_args,
                args,
            } => {
                if let V2Expr::Ident(_, fname) = callee.as_ref()
                    && fname == "create_object"
                    && self.resolve_callable(callee, *span)?.is_none()
                {
                    // `if create_object(...)`: success flag only; `stack_obj` is restored on
                    // both edges (bind the new object with `let Some(x) = create_object(...)`).
                    let (prim, _) = self.format_create_object(type_args, args, *span)?;
                    self.line(format!("if {prim} {{"));
                    self.indent += 1;
                    self.line(format!("{dst_local} := 1;"));
                    self.indent -= 1;
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.line(format!("{dst_local} := 0;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.emit_restore_stack_obj();
                    return Ok(());
                }
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        "type arguments are only supported on 'create_object::<T>', 'find_best_object::<T>', and 'objects_of::<T>'",
                    ));
                }
                if let Some(simple) = self.try_lower_simple_air_cond(expr)? {
                    self.line(format!("if {simple} {{"));
                    self.indent += 1;
                    self.line(format!("{dst_local} := 1;"));
                    self.indent -= 1;
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.line(format!("{dst_local} := 0;"));
                    self.indent -= 1;
                    self.line("}".to_owned());
                    return Ok(());
                }
                if let V2Expr::Ident(_, fname) = callee.as_ref()
                    && let Some((fn_mod, fn_decl, is_local)) =
                        self.env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                {
                    let (rv, vty) = self
                        .inline_helper_call(&fn_mod, &fn_decl, is_local, args, *span)?
                        .ok_or_else(|| {
                            Error::at(
                                *span,
                                format!("void helper function '{fname}' cannot be used as bool"),
                            )
                        })?;
                    if vty != ValueType::Bool {
                        let actual = self.env.display_value_type(&vty);
                        return Err(Error::at(
                            *span,
                            format!(
                                "helper function '{fname}' returns '{actual}' and cannot be used directly as a bool condition (use an explicit comparison)"
                            ),
                        ));
                    }
                    self.line(format!("{dst_local} := {rv};"));
                    return Ok(());
                }
                Err(Error::at(*span, "unsupported boolean call expression"))
            }
            _ => {
                let simple = self
                    .try_lower_simple_air_cond(expr)?
                    .ok_or_else(|| Error::at(expr.span(), "expected boolean expression"))?;
                self.line(format!("if {simple} {{"));
                self.indent += 1;
                self.line(format!("{dst_local} := 1;"));
                self.indent -= 1;
                self.line("} else {".to_owned());
                self.indent += 1;
                self.line(format!("{dst_local} := 0;"));
                self.indent -= 1;
                self.line("}".to_owned());
                Ok(())
            }
        }
    }

    /// Emits direct, zero-temporary short-circuit `return true` / `return false` for
    /// `guard(sim) => <expr>` and cleanup-free `return <expr>;`.
    fn emit_return_bool_expr(&mut self, expr: &V2Expr) -> Result<()> {
        if let Some(b) = self.try_eval_const_bool(expr) {
            self.line(if b {
                "return true;".to_owned()
            } else {
                "return false;".to_owned()
            });
            return Ok(());
        }
        match expr {
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => {
                if let Some(neg_lhs) = self.try_lower_negated_simple_air_cond(lhs)? {
                    self.line(format!("if {neg_lhs} {{"));
                    self.indent += 1;
                    self.line("return false;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                } else {
                    let tmp = self.fresh("and");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(lhs, &tmp)?;
                    self.line(format!("if {tmp} == 0 {{"));
                    self.indent += 1;
                    self.line("return false;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                }
            }
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => {
                if let Some(pos_lhs) = self.try_lower_simple_air_cond(lhs)? {
                    self.line(format!("if {pos_lhs} {{"));
                    self.indent += 1;
                    self.line("return true;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                } else {
                    let tmp = self.fresh("or");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(lhs, &tmp)?;
                    self.line(format!("if {tmp} != 0 {{"));
                    self.indent += 1;
                    self.line("return true;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                }
            }
            _ => {
                if let Some(neg) = self.try_lower_negated_simple_air_cond(expr)? {
                    self.line(format!("if {neg} {{"));
                    self.indent += 1;
                    self.line("return false;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    self.line("return true;".to_owned());
                    Ok(())
                } else {
                    let tmp = self.fresh("ret");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(expr, &tmp)?;
                    self.line(format!("if {tmp} != 0 {{"));
                    self.indent += 1;
                    self.line("return true;".to_owned());
                    self.indent -= 1;
                    self.line("} else {".to_owned());
                    self.indent += 1;
                    self.line("return false;".to_owned());
                    self.indent -= 1;
                    self.line("}".to_owned());
                    Ok(())
                }
            }
        }
    }

    fn resolve_motive_expr(&self, expr: &V2Expr) -> Result<u8> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(binding) = self.lookup_var(name) {
                    return match binding {
                        Binding::Motive(m) => Ok(*m),
                        _ => Err(Error::at(
                            *span,
                            format!("variable '{name}' is not a Motive"),
                        )),
                    };
                }
                if self.env.i16_params.contains_key(name) || self.env.bool_params.contains_key(name)
                {
                    return Err(Error::at(
                        *span,
                        format!(
                            "parameter '{name}' is not a Motive (use 'Motive::...' to refer to a motive explicitly)"
                        ),
                    ));
                }
                motive_by_name(name)
                    .ok_or_else(|| Error::at(*span, format!("unknown motive '{name}'")))
            }
            V2Expr::Path(span, segs) if segs.len() == 2 && segs[0] == "Motive" => {
                let lower = segs[1].to_ascii_lowercase();
                motive_by_name(&lower)
                    .ok_or_else(|| Error::at(*span, format!("unknown Motive::{}", segs[1])))
            }
            V2Expr::Int(span, v) => u8::try_from(*v)
                .ok()
                .filter(|&i| i < 16)
                .ok_or_else(|| Error::at(*span, format!("motive index {v} out of 0..16 range"))),
            other => Err(Error::at(
                other.span(),
                "expected motive name (e.g. energy, Motive::Energy) or index",
            )),
        }
    }

    fn resolve_skill_expr(&self, expr: &V2Expr) -> Result<u8> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(binding) = self.lookup_var(name) {
                    return match binding {
                        Binding::Skill(sk) => Ok(*sk),
                        _ => Err(Error::at(
                            *span,
                            format!("variable '{name}' is not a Skill"),
                        )),
                    };
                }
                if self.env.i16_params.contains_key(name) || self.env.bool_params.contains_key(name)
                {
                    return Err(Error::at(
                        *span,
                        format!(
                            "parameter '{name}' is not a Skill (use 'Skill::...' to refer to a skill explicitly)"
                        ),
                    ));
                }
                skill_by_name(name)
                    .ok_or_else(|| Error::at(*span, format!("unknown skill '{name}'")))
            }
            V2Expr::Path(span, segs) if segs.len() == 2 && segs[0] == "Skill" => {
                let lower = segs[1].to_ascii_lowercase();
                skill_by_name(&lower)
                    .ok_or_else(|| Error::at(*span, format!("unknown Skill::{}", segs[1])))
            }
            other => Err(Error::at(
                other.span(),
                "expected skill (e.g. Skill::Cooking, Skill::Mechanical, Skill::Charisma, Skill::Body, Skill::Logic, Skill::Creativity)",
            )),
        }
    }

    fn resolve_sound_expr(&self, expr: &V2Expr) -> Result<u16> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(Binding::Sound(id)) = self.lookup_var(name) {
                    return Ok(*id);
                }
                self.env
                    .sounds
                    .get(name)
                    .map(|s| s.id)
                    .ok_or_else(|| Error::at(*span, format!("unknown sound '{name}'")))
            }
            other => Err(Error::at(
                other.span(),
                "expected typed sound identifier (declared with 'sound name = preset(...);')",
            )),
        }
    }

    fn resolve_slot_expr(&self, expr: &V2Expr) -> Result<u16> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(Binding::Slot(idx)) = self.lookup_var(name) {
                    return Ok(*idx);
                }
                match self.env.slots.get(name) {
                    Some((Some(idx), _)) => Ok(*idx),
                    Some((None, _)) => Err(Error::at(
                        *span,
                        format!(
                            "slot '{name}' is not a routing slot (only 'kind: routing' slots can be used with route_to/snap/Slot)"
                        ),
                    )),
                    None => Err(Error::at(*span, format!("unknown slot '{name}'"))),
                }
            }
            V2Expr::Int(span, v) => u16::try_from(*v)
                .map_err(|_| Error::at(*span, format!("slot index {v} out of u16 range"))),
            other => Err(Error::at(other.span(), "expected slot identifier or index")),
        }
    }

    fn format_play_sound(&self, args: &[CallArg], span: Span) -> Result<String> {
        if args.is_empty() || args.len() > 2 || args.iter().any(|a| a.name.is_some()) {
            return Err(Error::at(
                span,
                "play_sound(sound [, target]) expects 1 or 2 positional arguments",
            ));
        }
        let sound_id = self.resolve_sound_expr(&args[0].value)?;
        let use_stack_obj = if let Some(target_arg) = args.get(1) {
            match &target_arg.value {
                V2Expr::Ident(tspan, name) => {
                    if self.is_self_ident(name) {
                        matches!(self.tree_ctx, TreeContext::SimOnObject { .. })
                    } else if self.is_sim_ident(name) {
                        false
                    } else {
                        return Err(Error::at(
                            *tspan,
                            format!("play_sound target must be 'self' or 'sim', found '{name}'"),
                        ));
                    }
                }
                other => {
                    return Err(Error::at(
                        other.span(),
                        "play_sound target must be 'self' or 'sim'",
                    ));
                }
            }
        } else {
            matches!(self.tree_ctx, TreeContext::SimOnObject { .. })
        };
        if use_stack_obj {
            Ok(format!("play_sound({sound_id}, stack_obj)"))
        } else {
            Ok(format!("play_sound({sound_id}, false)"))
        }
    }

    fn format_stop_all_sounds(&self, args: &[CallArg], span: Span) -> Result<String> {
        let use_stack = if let Some(arg) = args.first() {
            match &arg.value {
                V2Expr::Ident(_, n) if self.is_self_ident(n) => {
                    matches!(self.tree_ctx, TreeContext::SimOnObject { .. })
                }
                V2Expr::Ident(_, n) if self.is_sim_ident(n) => false,
                _ => {
                    return Err(Error::at(
                        span,
                        "stop_all_sounds target must be 'self' or 'sim'",
                    ));
                }
            }
        } else {
            matches!(self.tree_ctx, TreeContext::SimOnObject { .. })
        };
        Ok(format!("stop_all_sounds({use_stack})"))
    }

    fn format_alter_budget(
        &mut self,
        args: &[CallArg],
        span: Span,
        test_only: bool,
        income: bool,
    ) -> Result<String> {
        if args.is_empty() {
            return Err(Error::at(span, "budget call requires an amount argument"));
        }
        let (amount_air, _) = self.lower_value_expr(&args[0].value)?;
        let mut category = 0u8;
        for arg in &args[1..] {
            match arg.name.as_deref() {
                Some("category") => {
                    let c = self.eval_const_i16(&arg.value)?;
                    category = u8::try_from(c).map_err(|_| {
                        Error::at(arg.span, format!("budget category {c} out of u8 range"))
                    })?;
                }
                Some(other) => {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown budget option '{other}'"),
                    ));
                }
                None => {
                    return Err(Error::at(
                        arg.span,
                        "extra budget arguments must be named (e.g. category: 5)",
                    ));
                }
            }
        }
        let mut out = format!("alter_budget({amount_air}, category: {category}");
        if test_only {
            out.push_str(", test_only: true");
        }
        if income {
            out.push_str(", income: true");
        }
        out.push(')');
        Ok(out)
    }

    fn parse_find_best_object(
        &self,
        type_args: &[V2Type],
        args: &[CallArg],
        span: Span,
    ) -> Result<FindBestObjectPlan> {
        if args.is_empty() {
            return Err(Error::at(
                span,
                "find_best_object(kind [, flags: N]) requires a functional kind",
            ));
        }
        let V2Expr::Ident(kspan, kind_name) = &args[0].value else {
            return Err(Error::at(
                args[0].span,
                "find_best_object() expects a functional kind identifier (e.g. prep, cook)",
            ));
        };
        let _ = functional_kind_info(kind_name, *kspan)?;
        let typed_schema = if type_args.is_empty() {
            None
        } else if type_args.len() == 1 {
            let V2Type::Named(ref obj_name) = type_args[0] else {
                return Err(Error::at(
                    span,
                    "find_best_object::<T>() expects an imported object type name",
                ));
            };
            let schema = self
                .env
                .lookup_object_schema(&self.owner_module, obj_name, self.in_object_scope)
                .ok_or_else(|| {
                    Error::at(
                        span,
                        format!(
                            "unknown object type '{obj_name}' in 'find_best_object::<{obj_name}>({kind_name})'"
                        ),
                    )
                })?;
            if !schema.functionals.contains_key(kind_name) {
                return Err(Error::at(
                    *kspan,
                    format!(
                        "object '{}' does not declare 'functional {kind_name}'",
                        schema.display_name
                    ),
                ));
            }
            Some(schema.clone())
        } else {
            return Err(Error::at(
                span,
                "find_best_object::<T>() accepts at most 1 type argument",
            ));
        };
        let mut flags_opt: Option<u8> = None;
        for arg in &args[1..] {
            if arg.name.as_deref() == Some("flags") || arg.name.is_none() {
                let f = self.eval_const_i16(&arg.value)?;
                flags_opt = Some(
                    u8::try_from(f)
                        .map_err(|_| Error::at(arg.span, format!("flags {f} out of u8 range")))?,
                );
            } else {
                return Err(Error::at(
                    arg.span,
                    format!(
                        "unknown find_best_object option '{}'",
                        arg.name.as_deref().unwrap_or("")
                    ),
                ));
            }
        }
        let flags = flags_opt.unwrap_or(0);
        if typed_schema.is_some() && (flags & 0x0b) != 0 {
            return Err(Error::at(
                span,
                format!(
                    "typed find_best_object::<T>() does not support indoor/outdoor or custom-origin flags ({flags:#04x}); use default flags (0) or same-room (4)"
                ),
            ));
        }
        let prim_air = match flags_opt {
            Some(f) => format!("find_best_object({kind_name}, flags: {f})"),
            None => format!("find_best_object({kind_name})"),
        };
        Ok(FindBestObjectPlan {
            kind_name: kind_name.clone(),
            flags,
            prim_air,
            typed_schema,
        })
    }

    fn format_create_object(
        &self,
        type_args: &[V2Type],
        args: &[CallArg],
        span: Span,
    ) -> Result<(String, ValueType)> {
        let is_typed = !type_args.is_empty();
        let (guid, result_ty, opt_args) = if is_typed {
            if type_args.len() != 1 {
                return Err(Error::at(
                    span,
                    "create_object::<T>() expects exactly 1 object type argument",
                ));
            }
            let V2Type::Named(ref obj_name) = type_args[0] else {
                return Err(Error::at(
                    span,
                    "create_object::<T>() expects an imported object type name",
                ));
            };
            let schema = self
                .env
                .lookup_object_schema(&self.owner_module, obj_name, self.in_object_scope)
                .ok_or_else(|| {
                    Error::at(
                        span,
                        format!(
                            "unknown object type '{obj_name}' in 'create_object::<{obj_name}>(...)'"
                        ),
                    )
                })?;
            (
                schema.master_guid,
                ValueType::ObjectRef(schema.canonical_key.clone()),
                args,
            )
        } else {
            if args.is_empty() {
                return Err(Error::at(
                    span,
                    "create_object(guid [, mode: ..., flags: ...]) requires a GUID or type argument 'create_object::<T>(...)'",
                ));
            }
            if args[0].name.is_some() {
                return Err(Error::at(
                    args[0].span,
                    "create_object() requires a GUID (literal or const) or 'create_object::<ObjectType>(...)'",
                ));
            }
            let guid = self.resolve_guid_expr(&args[0].value)?;
            (guid, ValueType::ObjectId, &args[1..])
        };
        let mut mode = "here".to_owned();
        let mut flags = 0u8;
        for arg in opt_args {
            match arg.name.as_deref() {
                Some("mode") => match &arg.value {
                    V2Expr::Ident(_, m) => mode = m.clone(),
                    V2Expr::Int(_, n) => mode = n.to_string(),
                    other => {
                        return Err(Error::at(
                            other.span(),
                            "create_object mode must be an identifier (e.g. stack_ahead) or integer",
                        ));
                    }
                },
                Some("flags") => {
                    let f = self.eval_const_i16(&arg.value)?;
                    flags = u8::try_from(f)
                        .map_err(|_| Error::at(arg.span, format!("flags {f} out of u8 range")))?;
                }
                Some(other) => {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown create_object option '{other}'"),
                    ));
                }
                None => {
                    return Err(Error::at(
                        arg.span,
                        "extra create_object arguments must be named ('mode:', 'flags:')",
                    ));
                }
            }
        }
        if is_typed && (flags & 0x04) != 0 {
            return Err(Error::at(
                span,
                "create_object::<T>() cannot set FROM_RECORD flag (0x04), which overrides T's GUID from a neighbor record at runtime; use untyped create_object(0, ..., flags: 0x04)",
            ));
        }
        Ok((
            format!("create_object({guid:#010x}, mode: {mode}, flags: {flags:#04x})"),
            result_ty,
        ))
    }

    fn format_slot_prim(&self, prim: &str, args: &[CallArg], span: Span) -> Result<String> {
        if args.is_empty() {
            return Err(Error::at(
                span,
                format!("{prim}(slot [, flags: N]) requires a slot"),
            ));
        }
        let slot_idx = self.resolve_slot_expr(&args[0].value)?;
        let mut flags = 0u8;
        if let Some(farg) = args.get(1) {
            let f = self.eval_const_i16(&farg.value)?;
            flags = u8::try_from(f)
                .map_err(|_| Error::at(farg.span, format!("flags {f} out of u8 range")))?;
        }
        if flags == 0 {
            Ok(format!("{prim}({slot_idx})"))
        } else {
            Ok(format!("{prim}({slot_idx}, {flags})"))
        }
    }

    fn format_set_motive_change(&mut self, args: &[CallArg], span: Span) -> Result<String> {
        if args.len() != 3 {
            return Err(Error::at(
                span,
                "set_motive_change(motive, delta_per_hour, stop_at) expects 3 arguments",
            ));
        }
        let m = self.resolve_motive_expr(&args[0].value)?;
        let (delta, _) = self.lower_value_expr(&args[1].value)?;
        let delta_arg = if self.is_pure_single_operand(&args[2].value) {
            delta
        } else {
            let snap = self.fresh("mc_d");
            self.line(format!("let {snap} = {delta};"));
            snap
        };
        let (stop, _) = self.lower_value_expr(&args[2].value)?;
        Ok(format!("set_motive_change({m}, {delta_arg}, {stop})"))
    }

    fn resolve_dialog_str_arg(&self, expr: &V2Expr, field: &str, span: Span) -> Result<String> {
        match expr {
            V2Expr::Str(sspan, text) => {
                let interned = self.env.dialog_strings.borrow_mut().intern(text, *sspan)?;
                Ok(format!("{field}: str.{interned}"))
            }
            V2Expr::Ident(ispan, sname) => {
                if !self.env.string_names.contains_key(sname) {
                    return Err(Error::at(
                        *ispan,
                        format!(
                            "unknown string entry '{sname}' (declare it in 'strings {{ ... }}' or pass an inline string literal \"...\")"
                        ),
                    ));
                }
                Ok(format!("{field}: str.{sname}"))
            }
            V2Expr::Int(_, n) => Ok(format!("{field}: {n}")),
            _ => Err(Error::at(span, format!("invalid dialog {field}"))),
        }
    }

    fn parse_dialog_options(
        &self,
        args: &[CallArg],
        span: Span,
        for_match: bool,
    ) -> Result<ParsedDialogOptions> {
        let mut kind_num: Option<u8> = None;
        let mut effective_flags: u8 = 0;
        let mut explicit_temp_specified = false;
        let mut button_count: usize = 0;
        let mut buttons_tuple_len: Option<usize> = None;
        let mut saw_buttons_tuple = false;
        let mut saw_explicit_button = false;

        for arg in args {
            let Some(k) = arg.name.as_deref() else {
                return Err(Error::at(arg.span, "dialog() arguments must be named"));
            };
            match k {
                "kind" => match &arg.value {
                    V2Expr::Ident(_, s) => {
                        kind_num = Some(match s.as_str() {
                            "ok" | "message" => 0,
                            "yes_no" => 1,
                            "yes_no_cancel" | "three_choice" => 2,
                            "text_entry" => 3,
                            "icon" => 4,
                            other => {
                                return Err(Error::at(
                                    arg.span,
                                    format!("unknown dialog kind '{other}'"),
                                ));
                            }
                        });
                    }
                    V2Expr::Int(_, n) => {
                        let kn = u8::try_from(*n).map_err(|_| {
                            Error::at(arg.span, format!("dialog kind {n} out of u8 range"))
                        })?;
                        kind_num = Some(kn);
                    }
                    _ => return Err(Error::at(arg.span, "invalid dialog kind")),
                },
                "title" | "message" | "text" | "mode4_str" => {}
                "button1" | "button2" | "button3" => {
                    if saw_buttons_tuple {
                        return Err(Error::at(
                            arg.span,
                            "cannot combine 'buttons: (...)' with explicit 'button1'/'button2'/'button3'",
                        ));
                    }
                    saw_explicit_button = true;
                    let idx = match k {
                        "button1" => 1,
                        "button2" => 2,
                        _ => 3,
                    };
                    button_count = button_count.max(idx);
                }
                "buttons" => {
                    if saw_explicit_button || saw_buttons_tuple {
                        return Err(Error::at(
                            arg.span,
                            "duplicate 'buttons:' or conflict with 'button1'/'button2'/'button3'",
                        ));
                    }
                    saw_buttons_tuple = true;
                    let V2Expr::Tuple(tspan, elems) = &arg.value else {
                        return Err(Error::at(
                            arg.span,
                            "dialog 'buttons:' expects a tuple of 1 to 3 strings, e.g. buttons: (\"Yes\", \"No\")",
                        ));
                    };
                    if elems.is_empty() || elems.len() > 3 {
                        return Err(Error::at(
                            *tspan,
                            format!(
                                "dialog 'buttons:' tuple must have 1 to 3 entries, found {}",
                                elems.len()
                            ),
                        ));
                    }
                    buttons_tuple_len = Some(elems.len());
                    button_count = elems.len();
                }
                "flags" => {
                    let n = self.eval_const_i16(&arg.value)?;
                    let raw = u8::try_from(n).map_err(|_| {
                        Error::at(arg.span, format!("dialog flags {n} out of u8 range"))
                    })?;
                    effective_flags |= raw;
                    if (raw & 0x70) != 0 {
                        explicit_temp_specified = true;
                    }
                }
                "inform" => {
                    if self.eval_const_bool_required(&arg.value, arg.span)? {
                        effective_flags |= 1;
                    }
                }
                "non_modal" => {
                    if self.eval_const_bool_required(&arg.value, arg.span)? {
                        effective_flags |= 0x80;
                    }
                }
                "icon_mode" => {
                    let n = self.eval_const_i16(&arg.value)?;
                    let mode = u8::try_from(n)
                        .ok()
                        .filter(|&m| m <= 7)
                        .ok_or_else(|| Error::at(arg.span, "dialog icon_mode must be in 0..=7"))?;
                    effective_flags = (effective_flags & !(7 << 1)) | (mode << 1);
                }
                "temp" => {
                    let n = self.eval_const_i16(&arg.value)?;
                    let t = u8::try_from(n)
                        .ok()
                        .filter(|&m| m <= 7)
                        .ok_or_else(|| Error::at(arg.span, "dialog temp must be in 0..=7"))?;
                    effective_flags = (effective_flags & !(7 << 4)) | (t << 4);
                    explicit_temp_specified = true;
                }
                other => {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown dialog() argument '{other}'"),
                    ));
                }
            }
        }

        if !for_match && let Some(tuple_len) = buttons_tuple_len {
            let eff_kind = kind_num.unwrap_or(0) & 0x7f;
            let expected = match eff_kind {
                0 => Some((1, "'ok'")),
                1 => Some((2, "'yes_no'")),
                2 => Some((3, "'yes_no_cancel'")),
                3 => Some((1, "'text_entry'")),
                4 => Some((1, "'icon'")),
                _ => None,
            };
            match expected {
                Some((exp, label)) if tuple_len != exp => {
                    return Err(Error::at(
                        span,
                        format!(
                            "dialog 'buttons:' tuple has {tuple_len} entries, but {label} dialog requires exactly {exp}"
                        ),
                    ));
                }
                None => {
                    return Err(Error::at(
                        span,
                        format!(
                            "dialog kind {eff_kind} does not accept custom 'buttons:' tuple labels"
                        ),
                    ));
                }
                _ => {}
            }
        }

        Ok(ParsedDialogOptions {
            kind_num,
            effective_flags,
            effective_temp_slot: (effective_flags >> 4) & 7,
            explicit_temp_specified,
            button_count,
            buttons_tuple_len,
        })
    }

    fn format_dialog(&self, args: &[CallArg], span: Span) -> Result<String> {
        let _opts = self.parse_dialog_options(args, span, false)?;
        let mut parts = Vec::new();
        for arg in args {
            let Some(k) = arg.name.as_deref() else {
                return Err(Error::at(arg.span, "dialog() arguments must be named"));
            };
            match k {
                "kind" => match &arg.value {
                    V2Expr::Ident(_, s) => parts.push(format!("kind: {s}")),
                    V2Expr::Int(_, n) => parts.push(format!("kind: {n}")),
                    _ => return Err(Error::at(arg.span, "invalid dialog kind")),
                },
                "title" | "message" | "text" | "mode4_str" | "button1" | "button2" | "button3" => {
                    parts.push(self.resolve_dialog_str_arg(&arg.value, k, arg.span)?);
                }
                "buttons" => {
                    let V2Expr::Tuple(_, elems) = &arg.value else {
                        unreachable!("validated in parse_dialog_options");
                    };
                    let btn_fields = ["button1", "button2", "button3"];
                    for (idx, elem) in elems.iter().enumerate() {
                        parts.push(self.resolve_dialog_str_arg(
                            elem,
                            btn_fields[idx],
                            elem.span(),
                        )?);
                    }
                }
                "temp" | "icon_mode" | "flags" => {
                    let n = self.eval_const_i16(&arg.value)?;
                    parts.push(format!("{k}: {n}"));
                }
                "inform" | "non_modal" => {
                    let b = self.eval_const_bool_required(&arg.value, arg.span)?;
                    parts.push(format!("{k}: {b}"));
                }
                other => {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown dialog() argument '{other}'"),
                    ));
                }
            }
        }
        Ok(format!("dialog({})", parts.join(", ")))
    }
}

struct FindBestObjectPlan {
    kind_name: String,
    flags: u8,
    prim_air: String,
    typed_schema: Option<ResolvedObjectSchema>,
}

struct ParsedDialogOptions {
    kind_num: Option<u8>,
    effective_flags: u8,
    effective_temp_slot: u8,
    explicit_temp_specified: bool,
    button_count: usize,
    buttons_tuple_len: Option<usize>,
}

/// Normalizes `.air` operand spellings to their canonical SimAntics VM storage cell so alias
/// analysis treats `self.prep_score` (`stack.prep_score`) and `self.data[31]` (`stack.data[31]`)
/// as the exact same cell.
fn canonical_air_cell(air: &str) -> String {
    if let Some((prefix, rest)) = air.split_once('.')
        && matches!(prefix, "my" | "stack")
    {
        match rest {
            "id" => return format!("{prefix}.data[11]"),
            "in_use" => return format!("{prefix}.data[25]"),
            "room" => return format!("{prefix}.data[29]"),
            "prep_score" => return format!("{prefix}.data[31]"),
            "cook_score" => return format!("{prefix}.data[32]"),
            "priority" => return format!("{prefix}.person[33]"),
            _ => {}
        }
        if let Some(mname) = rest.strip_prefix("motive.")
            && let Some(id) = motive_by_name(mname)
        {
            return format!("{prefix}.motive[{id}]");
        }
        if let Some(sname) = rest.strip_prefix("skill.")
            && let Some(id) = skill_by_name(sname)
        {
            return format!("{prefix}.person[{id}]");
        }
        if let Some(pname) = rest.strip_prefix("person.")
            && let Some(id) = person_field_by_name(pname)
        {
            return format!("{prefix}.person[{id}]");
        }
    }
    air.to_owned()
}

fn is_arith_or_bit_binop(op: V2BinOp) -> bool {
    matches!(
        op,
        V2BinOp::Add
            | V2BinOp::Sub
            | V2BinOp::Mul
            | V2BinOp::Div
            | V2BinOp::Mod
            | V2BinOp::BitAnd
            | V2BinOp::BitOr
            | V2BinOp::BitXor
    )
}

fn is_compound_arith_or_neg(expr: &V2Expr) -> bool {
    match expr {
        V2Expr::Binary { op, .. } => is_arith_or_bit_binop(*op),
        V2Expr::Unary {
            op: V2UnaryOp::Neg,
            inner,
            ..
        } => !matches!(inner.as_ref(), V2Expr::Int(_, _)),
        _ => false,
    }
}

fn flatten_arith_chain(expr: &V2Expr) -> (&V2Expr, Vec<(V2BinOp, &V2Expr)>) {
    let mut ops = Vec::new();
    let mut cur = expr;
    while let V2Expr::Binary { lhs, op, rhs, .. } = cur
        && is_arith_or_bit_binop(*op)
    {
        ops.push((*op, rhs.as_ref()));
        cur = lhs.as_ref();
    }
    ops.reverse();
    (cur, ops)
}

fn single_bit_of_mask(mask: i16) -> Option<u8> {
    let bits = mask as u16;
    if bits != 0 && (bits & (bits - 1)) == 0 {
        Some((bits.trailing_zeros() as u8) + 1)
    } else {
        None
    }
}

fn cmp_op_str(op: V2BinOp) -> &'static str {
    match op {
        V2BinOp::Eq => "==",
        V2BinOp::Ne => "!=",
        V2BinOp::Lt => "<",
        V2BinOp::Le => "<=",
        V2BinOp::Gt => ">",
        V2BinOp::Ge => ">=",
        V2BinOp::FlagTest => "flag_test",
        _ => unreachable!("non-comparison op"),
    }
}

/// Parses a `raw { ... }` block's `.air` text the same way the lowering validates it.
fn parse_raw_air_stmts(code: &str) -> Option<Vec<crate::ast::Stmt>> {
    let wrapped = format!("tree 4096 \"__raw_check\" {{\n{code}\n}}\n");
    let parsed = crate::parser::parse_module(&wrapped).ok()?;
    match parsed.items.into_iter().next() {
        Some(crate::ast::Item::Tree(t)) => Some(t.body),
        _ => None,
    }
}

/// Whether a `raw { ... }` block may overwrite the frame's `stack_obj` (or names `__self_obj`
/// itself), which forces the `__self_obj` save in a Sim-on-object tree. Text that does not parse
/// is assumed to clobber.
fn raw_air_may_clobber_stack_obj(code: &str) -> bool {
    if code.contains("__self_obj") {
        return true;
    }
    match parse_raw_air_stmts(code) {
        Some(stmts) => raw_air_stmts_clobber_stack_obj(&stmts),
        None => true,
    }
}

fn is_stack_obj_operand(op: &crate::ast::OperandRef) -> bool {
    matches!(
        op,
        crate::ast::OperandRef::Direct(crate::ast::Operand {
            scope: 10,
            index: 0
        })
    )
}

/// Raw opcodes known not to write the caller frame's `stack_obj`: BHAV calls (`stack_obj` is per
/// frame and never copied back) and the operand-free / read-only primitives listed here. Every
/// other raw opcode (notably Expression `0x0002`, which can name `stack_obj` as its target) is
/// treated as a clobber.
fn raw_opcode_keeps_stack_obj(opcode: u16) -> bool {
    opcode >= 0x0100
        || matches!(
            opcode,
            0x0004 // grab
                | 0x0005 // drop
                | 0x0007 // refresh
                | 0x000F // breakpoint
                | 0x0019 // alter budget
                | 0x0023 // special effect
                | 0x002C // animate
                | 0x0031 // notify out of idle
        )
}

fn raw_condition_clobbers_stack_obj(cond: &crate::ast::Condition) -> bool {
    match cond {
        crate::ast::Condition::Compare { .. } | crate::ast::Condition::Call { .. } => false,
        crate::ast::Condition::Primitive { prim, .. } => raw_primitive_clobbers_stack_obj(prim),
        crate::ast::Condition::RawOp { opcode, .. } => !raw_opcode_keeps_stack_obj(*opcode),
        crate::ast::Condition::Not(inner) => raw_condition_clobbers_stack_obj(inner),
    }
}

fn raw_primitive_clobbers_stack_obj(prim: &crate::ast::PrimitiveCall) -> bool {
    use crate::ast::PrimitiveCall as P;
    match prim {
        // These set the running frame's stack object (or may, through an autonomy choice).
        P::FindBestInteraction
        | P::FindBestObject { .. }
        | P::CreateObject { .. }
        | P::MakeNewCharacter { .. }
        | P::GosubFoundAction => true,
        P::SetToNext { dest, .. } => dest.as_ref().is_none_or(is_stack_obj_operand),
        P::ExprRaw { lhs, .. } => is_stack_obj_operand(lhs),
        P::Random { dst, .. } | P::GetDirectionTo { dst, .. } => is_stack_obj_operand(dst),
        P::Relationship { value, .. } => is_stack_obj_operand(value),
        _ => false,
    }
}

fn raw_air_stmts_clobber_stack_obj(stmts: &[crate::ast::Stmt]) -> bool {
    use crate::ast::Stmt;
    stmts.iter().any(|s| match s {
        Stmt::Assign { lhs, .. } => is_stack_obj_operand(lhs),
        Stmt::RawOp { opcode, .. } | Stmt::InlineRawOp { opcode, .. } => {
            !raw_opcode_keeps_stack_obj(*opcode)
        }
        Stmt::Primitive { prim, .. } => raw_primitive_clobbers_stack_obj(prim),
        Stmt::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            raw_condition_clobbers_stack_obj(cond)
                || raw_air_stmts_clobber_stack_obj(then_body)
                || raw_air_stmts_clobber_stack_obj(else_body)
        }
        Stmt::While { cond, body, .. } => {
            raw_condition_clobbers_stack_obj(cond) || raw_air_stmts_clobber_stack_obj(body)
        }
        Stmt::Loop { body, .. } => raw_air_stmts_clobber_stack_obj(body),
        Stmt::Label { .. }
        | Stmt::Let { .. }
        | Stmt::Call { .. }
        | Stmt::Idle { .. }
        | Stmt::Break { .. }
        | Stmt::Continue { .. }
        | Stmt::Return { .. } => false,
    })
}

fn collect_raw_air_labels<'a>(stmts: &'a [crate::ast::Stmt], labels: &mut BTreeSet<&'a str>) {
    for s in stmts {
        match s {
            crate::ast::Stmt::Label { name, .. } => {
                labels.insert(name.as_str());
            }
            crate::ast::Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_raw_air_labels(then_body, labels);
                collect_raw_air_labels(else_body, labels);
            }
            crate::ast::Stmt::While { body, .. } => {
                collect_raw_air_labels(body, labels);
            }
            _ => {}
        }
    }
}

fn validate_raw_air_target(
    target: &crate::ast::Target,
    defined_labels: &BTreeSet<&str>,
    span: Span,
) -> Result<()> {
    match target {
        crate::ast::Target::Label(name) if defined_labels.contains(name.as_str()) => Ok(()),
        other => Err(Error::at(
            span,
            format!(
                "'raw {{ ... }}' cannot branch to '{other:?}' while cleanups, inline helpers, or cancel_on_abort loops are active"
            ),
        )),
    }
}

fn validate_raw_air_stmts(
    stmts: &[crate::ast::Stmt],
    in_raw_loop: bool,
    defined_labels: &BTreeSet<&str>,
    span: Span,
) -> Result<()> {
    for s in stmts {
        match s {
            crate::ast::Stmt::Return { .. } => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot contain 'return' while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::Break { .. } | crate::ast::Stmt::Continue { .. } if !in_raw_loop => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot break/continue out of its block while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::RawOp { branches, .. } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Primitive {
                branches: Some(branches),
                ..
            } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Call {
                branches: Some(branches),
                ..
            } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Call { branches: None, .. } => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot invoke an unbranched subroutine call (whose false branch exits the tree) while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                validate_raw_air_stmts(then_body, in_raw_loop, defined_labels, span)?;
                validate_raw_air_stmts(else_body, in_raw_loop, defined_labels, span)?;
            }
            crate::ast::Stmt::While { body, .. } => {
                validate_raw_air_stmts(body, true, defined_labels, span)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn expr_starts_with_not_self_field(expr: &V2Expr, field_name: &str) -> bool {
    match expr {
        V2Expr::Unary {
            op: V2UnaryOp::Not,
            inner,
            ..
        } => matches!(
            inner.as_ref(),
            V2Expr::Field { base, field, .. }
                if field == field_name
                    && matches!(base.as_ref(), V2Expr::Ident(_, b) if b == "self")
        ),
        V2Expr::Binary {
            lhs,
            op: V2BinOp::And,
            ..
        } => expr_starts_with_not_self_field(lhs, field_name),
        _ => false,
    }
}

fn stmts_mutate_or_borrow_ident(stmts: &[V2Stmt], target_name: &str) -> bool {
    stmts.iter().any(|s| match s {
        V2Stmt::Let { name, init, .. } => {
            name == target_name || expr_borrows_ident(init, target_name)
        }
        V2Stmt::LetSome {
            name,
            init,
            else_body,
            ..
        } => {
            name == target_name
                || expr_borrows_ident(init, target_name)
                || stmts_mutate_or_borrow_ident(else_body, target_name)
        }
        V2Stmt::Assign { target, value, .. } => {
            matches!(target, V2Expr::Ident(_, n) if n == target_name)
                || expr_borrows_ident(target, target_name)
                || expr_borrows_ident(value, target_name)
        }
        V2Stmt::WithLock {
            lock_expr, body, ..
        } => {
            matches!(lock_expr, V2Expr::Ident(_, n) if n == target_name)
                || expr_borrows_ident(lock_expr, target_name)
                || stmts_mutate_or_borrow_ident(body, target_name)
        }
        V2Stmt::Defer { body, .. } => stmts_mutate_or_borrow_ident(body, target_name),
        V2Stmt::Repeat { count, body, .. } => {
            expr_borrows_ident(count, target_name)
                || stmts_mutate_or_borrow_ident(body, target_name)
        }
        V2Stmt::While { cond, body, .. } => {
            expr_borrows_ident(cond, target_name) || stmts_mutate_or_borrow_ident(body, target_name)
        }
        V2Stmt::ForEach { binding, body, .. } => {
            binding == target_name || stmts_mutate_or_borrow_ident(body, target_name)
        }
        V2Stmt::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            expr_borrows_ident(cond, target_name)
                || stmts_mutate_or_borrow_ident(then_body, target_name)
                || stmts_mutate_or_borrow_ident(else_body, target_name)
        }
        V2Stmt::Match {
            scrutinee, arms, ..
        } => {
            expr_borrows_ident(scrutinee, target_name)
                || arms
                    .iter()
                    .any(|a| stmts_mutate_or_borrow_ident(&a.body, target_name))
        }
        V2Stmt::Return { value: Some(e), .. } | V2Stmt::Expr { expr: e, .. } => {
            expr_borrows_ident(e, target_name)
        }
        V2Stmt::LetUninit { name, .. } => name == target_name,
        V2Stmt::Animate {
            args, event, body, ..
        } => {
            event.as_deref() == Some(target_name)
                || args
                    .iter()
                    .any(|a| expr_borrows_ident(&a.value, target_name))
                || stmts_mutate_or_borrow_ident(body, target_name)
        }
        V2Stmt::Break { .. }
        | V2Stmt::Continue { .. }
        | V2Stmt::Return { value: None, .. }
        | V2Stmt::ReturnError { .. }
        | V2Stmt::RawAir { .. } => false,
    })
}

fn expr_borrows_ident(expr: &V2Expr, target_name: &str) -> bool {
    match expr {
        V2Expr::MutRef { inner, .. } => {
            matches!(inner.as_ref(), V2Expr::Ident(_, n) if n == target_name)
                || expr_borrows_ident(inner, target_name)
        }
        V2Expr::Field { base, .. }
        | V2Expr::Deref { inner: base, .. }
        | V2Expr::Unary { inner: base, .. }
        | V2Expr::Try { inner: base, .. } => expr_borrows_ident(base, target_name),
        V2Expr::Index { base, index, .. } => {
            expr_borrows_ident(base, target_name) || expr_borrows_ident(index, target_name)
        }
        V2Expr::Binary { lhs, rhs, .. } => {
            expr_borrows_ident(lhs, target_name) || expr_borrows_ident(rhs, target_name)
        }
        V2Expr::Tuple(_, elems) => elems.iter().any(|e| expr_borrows_ident(e, target_name)),
        V2Expr::Call { callee, args, .. } => {
            expr_borrows_ident(callee, target_name)
                || args
                    .iter()
                    .any(|a| expr_borrows_ident(&a.value, target_name))
        }
        V2Expr::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            (matches!(method.as_str(), "set" | "clear")
                && matches!(receiver.as_ref(), V2Expr::Ident(_, n) if n == target_name))
                || expr_borrows_ident(receiver, target_name)
                || args
                    .iter()
                    .any(|a| expr_borrows_ident(&a.value, target_name))
        }
        V2Expr::Int(..)
        | V2Expr::Bool(..)
        | V2Expr::Str(..)
        | V2Expr::Ident(..)
        | V2Expr::Path(..) => false,
    }
}

fn match_is_exhaustive(
    scrutinee: &V2Expr,
    arms: &[MatchArm],
    env: &ObjectEnv<'_>,
    owner_module: &str,
    allow_local: bool,
) -> bool {
    if arms
        .iter()
        .any(|a| matches!(a.pattern, MatchPattern::Wildcard(_)))
    {
        return true;
    }
    // `match dialog(...)` is always validated as exhaustive by `lower_match_dialog`.
    if matches!(
        scrutinee,
        V2Expr::Call { callee, .. } if matches!(callee.as_ref(), V2Expr::Ident(_, n) if n == "dialog")
    ) {
        return true;
    }
    // Type-aware enum exhaustiveness: if every arm matches `Enum::Variant` of the same enum
    // and every declared variant discriminant is covered, the match is exhaustive!
    let mut enum_key: Option<String> = None;
    let mut covered = BTreeSet::new();
    for arm in arms {
        let MatchPattern::Path(_, segs) = &arm.pattern else {
            return false;
        };
        if segs.len() != 2 {
            return false;
        }
        let Some(e) = env.lookup_enum(owner_module, &segs[0], allow_local) else {
            return false;
        };
        if let Some(ref k) = enum_key {
            if k != &e.canonical_key {
                return false;
            }
        } else {
            enum_key = Some(e.canonical_key.clone());
        }
        let Some(&val) = e.variants.get(&segs[1]) else {
            return false;
        };
        covered.insert(val);
    }
    if let Some(key) = enum_key
        && let Some(e) = env.all_enums.get(&key)
    {
        return e.variants.values().all(|v| covered.contains(v));
    }
    false
}

fn stmts_unconditionally_diverge(
    stmts: &[V2Stmt],
    env: &ObjectEnv<'_>,
    owner_module: &str,
    allow_local: bool,
) -> bool {
    match stmts.last() {
        Some(
            V2Stmt::Return { .. }
            | V2Stmt::ReturnError { .. }
            | V2Stmt::Break { .. }
            | V2Stmt::Continue { .. },
        ) => true,
        Some(V2Stmt::If {
            then_body,
            else_body,
            ..
        }) => {
            !else_body.is_empty()
                && stmts_unconditionally_diverge(then_body, env, owner_module, allow_local)
                && stmts_unconditionally_diverge(else_body, env, owner_module, allow_local)
        }
        Some(V2Stmt::Match {
            scrutinee, arms, ..
        }) => {
            !arms.is_empty()
                && match_is_exhaustive(scrutinee, arms, env, owner_module, allow_local)
                && arms
                    .iter()
                    .all(|a| stmts_unconditionally_diverge(&a.body, env, owner_module, allow_local))
        }
        Some(V2Stmt::WithLock { body, .. }) => {
            stmts_unconditionally_diverge(body, env, owner_module, allow_local)
        }
        Some(V2Stmt::While {
            cond: V2Expr::Bool(_, true),
            body,
            ..
        }) => !loop_body_breaks(body),
        _ => false,
    }
}

/// Whether a `break` in `stmts` exits the loop whose body `stmts` is (breaks inside nested loops,
/// including an `animate` event handler, exit those instead).
fn loop_body_breaks(stmts: &[V2Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        V2Stmt::Break { .. } => true,
        V2Stmt::If {
            then_body,
            else_body,
            ..
        } => loop_body_breaks(then_body) || loop_body_breaks(else_body),
        V2Stmt::Match { arms, .. } => arms.iter().any(|a| loop_body_breaks(&a.body)),
        V2Stmt::WithLock { body, .. } | V2Stmt::Defer { body, .. } => loop_body_breaks(body),
        V2Stmt::LetSome { else_body, .. } => loop_body_breaks(else_body),
        _ => false,
    })
}

fn stmts_have_return(stmts: &[V2Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        // `return error;` may resolve to an ordinary `return <local error>;` (see lowering).
        V2Stmt::Return { .. } | V2Stmt::ReturnError { .. } => true,
        V2Stmt::WithLock { body, .. }
        | V2Stmt::Defer { body, .. }
        | V2Stmt::Repeat { body, .. }
        | V2Stmt::While { body, .. }
        | V2Stmt::ForEach { body, .. }
        | V2Stmt::Animate { body, .. } => stmts_have_return(body),
        V2Stmt::LetSome { else_body, .. } => stmts_have_return(else_body),
        V2Stmt::If {
            then_body,
            else_body,
            ..
        } => stmts_have_return(then_body) || stmts_have_return(else_body),
        V2Stmt::Match { arms, .. } => arms.iter().any(|a| stmts_have_return(&a.body)),
        _ => false,
    })
}

fn stmts_always_return(
    stmts: &[V2Stmt],
    env: &ObjectEnv<'_>,
    owner_module: &str,
    allow_local: bool,
) -> bool {
    for stmt in stmts {
        match stmt {
            V2Stmt::Return { .. } | V2Stmt::ReturnError { .. } => return true,
            V2Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                if !else_body.is_empty()
                    && stmts_always_return(then_body, env, owner_module, allow_local)
                    && stmts_always_return(else_body, env, owner_module, allow_local)
                {
                    return true;
                }
            }
            V2Stmt::Match {
                scrutinee, arms, ..
            } => {
                if !arms.is_empty()
                    && match_is_exhaustive(scrutinee, arms, env, owner_module, allow_local)
                    && arms
                        .iter()
                        .all(|a| stmts_always_return(&a.body, env, owner_module, allow_local))
                {
                    return true;
                }
            }
            V2Stmt::WithLock { body, .. }
                if stmts_always_return(body, env, owner_module, allow_local) =>
            {
                return true;
            }
            // `loop { }` / `while true { }` without a `break` never falls through.
            V2Stmt::While {
                cond: V2Expr::Bool(_, true),
                body,
                ..
            } if !loop_body_breaks(body) => return true,
            _ => {}
        }
    }
    false
}

fn stmts_may_clobber_stack_obj(
    stmts: &[V2Stmt],
    env: &ObjectEnv<'_>,
    owner_mod: &str,
    allow_local_fns: bool,
    sim_name: Option<&str>,
    depth: usize,
) -> bool {
    if depth > MAX_INLINE_DEPTH {
        return true;
    }
    stmts.iter().any(|s| match s {
        // Rebinding the Sim name makes later `sim.x` reads foreign-object reads (stack_obj swaps)
        // that the field check below would otherwise treat as plain Sim reads.
        V2Stmt::Let { name, .. } | V2Stmt::LetSome { name, .. }
            if Some(name.as_str()) == sim_name =>
        {
            true
        }
        V2Stmt::LetSome {
            init, else_body, ..
        } => {
            expr_may_clobber_stack_obj(init, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    else_body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::Let { init, .. } => {
            expr_may_clobber_stack_obj(init, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        V2Stmt::Assign {
            target: V2Expr::Ident(_, name),
            ..
        } if name == "stack_obj" => true,
        V2Stmt::Assign { target, value, .. } => {
            expr_may_clobber_stack_obj(target, env, owner_mod, allow_local_fns, sim_name, depth)
                || expr_may_clobber_stack_obj(
                    value,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::WithLock {
            lock_expr, body, ..
        } => {
            expr_may_clobber_stack_obj(lock_expr, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::Defer { body, .. } => {
            stmts_may_clobber_stack_obj(body, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        V2Stmt::Repeat { count, body, .. } => {
            expr_may_clobber_stack_obj(count, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::While { cond, body, .. } => {
            expr_may_clobber_stack_obj(cond, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::ForEach {
            iter: ForEachIter::Range { start, end, .. },
            body,
            ..
        } => {
            expr_may_clobber_stack_obj(start, env, owner_mod, allow_local_fns, sim_name, depth)
                || expr_may_clobber_stack_obj(end, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::ForEach { .. } => true,
        V2Stmt::RawAir { code, .. } => raw_air_may_clobber_stack_obj(code),
        V2Stmt::Animate { args, body, .. } => {
            args.iter().any(|a| {
                expr_may_clobber_stack_obj(
                    &a.value,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
            }) || stmts_may_clobber_stack_obj(
                body,
                env,
                owner_mod,
                allow_local_fns,
                sim_name,
                depth,
            )
        }
        V2Stmt::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            expr_may_clobber_stack_obj(cond, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    then_body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
                || stmts_may_clobber_stack_obj(
                    else_body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::Match {
            scrutinee, arms, ..
        } => {
            expr_may_clobber_stack_obj(scrutinee, env, owner_mod, allow_local_fns, sim_name, depth)
                || arms.iter().any(|a| {
                    stmts_may_clobber_stack_obj(
                        &a.body,
                        env,
                        owner_mod,
                        allow_local_fns,
                        sim_name,
                        depth,
                    )
                })
        }
        V2Stmt::Return { value: Some(e), .. } | V2Stmt::Expr { expr: e, .. } => {
            expr_may_clobber_stack_obj(e, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        _ => false,
    })
}

fn expr_may_clobber_stack_obj(
    expr: &V2Expr,
    env: &ObjectEnv<'_>,
    owner_mod: &str,
    allow_local_fns: bool,
    sim_name: Option<&str>,
    depth: usize,
) -> bool {
    if depth > MAX_INLINE_DEPTH {
        return true;
    }
    match expr {
        V2Expr::Call { callee, args, .. } => {
            if let V2Expr::Ident(_, fname) = callee.as_ref() {
                if matches!(fname.as_str(), "find_best_object" | "create_object") {
                    return true;
                }
                if matches!(
                    fname.as_str(),
                    "find_best_interaction" | "gosub_found_action"
                ) && env.lookup_fn(owner_mod, fname, allow_local_fns).is_none()
                {
                    return true;
                }
                if let Some((fmod, fdecl, is_local)) =
                    env.lookup_fn(owner_mod, fname, allow_local_fns)
                {
                    let fn_sim = fdecl
                        .params
                        .iter()
                        .find(|p| p.ty == V2Type::Sim)
                        .map(|p| p.name.as_str());
                    if stmts_may_clobber_stack_obj(
                        &fdecl.body,
                        env,
                        &fmod,
                        is_local,
                        fn_sim,
                        depth + 1,
                    ) || fdecl
                        .params
                        .iter()
                        .filter_map(|p| p.default.as_ref())
                        .any(|def_expr| {
                            expr_may_clobber_stack_obj(
                                def_expr,
                                env,
                                &fmod,
                                is_local,
                                fn_sim,
                                depth + 1,
                            )
                        })
                    {
                        return true;
                    }
                }
            }
            args.iter().any(|a| {
                expr_may_clobber_stack_obj(
                    &a.value,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
            })
        }
        V2Expr::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            method == "run_functional"
                || expr_may_clobber_stack_obj(
                    receiver,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
                || args.iter().any(|a| {
                    expr_may_clobber_stack_obj(
                        &a.value,
                        env,
                        owner_mod,
                        allow_local_fns,
                        sim_name,
                        depth,
                    )
                })
        }
        V2Expr::Index { base, index, .. } => {
            if let V2Expr::Field {
                base: root, field, ..
            } = base.as_ref()
                && matches!(field.as_str(), "attr" | "data")
                && let V2Expr::Ident(_, rname) = root.as_ref()
                && rname != "self"
            {
                return true;
            }
            expr_may_clobber_stack_obj(base, env, owner_mod, allow_local_fns, sim_name, depth)
                || expr_may_clobber_stack_obj(
                    index,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Expr::Field { base, .. } => {
            // Typed foreign object field access (`fridge.stock`, `plate.nutrition`, etc.) swaps `stack_obj`!
            // (Named BCON table entries such as `LampTuning.a` are plain constant reads.)
            if let V2Expr::Ident(_, rname) = base.as_ref()
                && rname != "self"
                && Some(rname.as_str()) != sim_name
                && !env.is_bcon_table_name(owner_mod, rname, allow_local_fns)
            {
                return true;
            }
            expr_may_clobber_stack_obj(base, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        V2Expr::Deref { inner: base, .. }
        | V2Expr::MutRef { inner: base, .. }
        | V2Expr::Unary { inner: base, .. }
        | V2Expr::Try { inner: base, .. } => {
            expr_may_clobber_stack_obj(base, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        V2Expr::Binary { lhs, rhs, .. } => {
            expr_may_clobber_stack_obj(lhs, env, owner_mod, allow_local_fns, sim_name, depth)
                || expr_may_clobber_stack_obj(rhs, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        V2Expr::Tuple(_, elems) => elems.iter().any(|e| {
            expr_may_clobber_stack_obj(e, env, owner_mod, allow_local_fns, sim_name, depth)
        }),
        _ => false,
    }
}

fn make_slug(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "object".to_owned()
    } else {
        out
    }
}

fn camel_label(name: &str) -> String {
    let mut out = String::new();
    let mut cap = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if cap {
                out.push(ch.to_ascii_uppercase());
                cap = false;
            } else {
                out.push(ch);
            }
        } else {
            cap = true;
        }
    }
    if out.is_empty() {
        "Object".to_owned()
    } else {
        out
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => {
            let mut out = first.to_uppercase().to_string();
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

fn sanitize_tree_ident(label: &str, idx: usize) -> String {
    let camel = camel_label(label);
    if camel.is_empty() {
        format!("Interaction_{idx}")
    } else {
        camel
    }
}

fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => {
                for b in c.encode_utf8(&mut [0u8; 4]).as_bytes() {
                    let _ = write!(out, "\\x{b:02x}");
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[allow(dead_code)]
fn _keep_furniture_shape_import(_: FurnitureShape) {}

#[cfg(test)]
mod tests {
    use super::canonical_air_cell;

    /// Pins the `person.<field>` spellings to the same indices the `.air` parser, the lifter,
    /// and the UI use, so alias analysis treats both spellings as one VM cell.
    #[test]
    fn person_field_cells_match_air_indices() {
        for (name, index) in [
            ("cooking", 10),
            ("charisma", 11),
            ("mechanical", 12),
            ("mechanic", 12),
            ("creativity", 15),
            ("body", 17),
            ("logic", 18),
            ("priority", 33),
        ] {
            assert_eq!(
                canonical_air_cell(&format!("my.person.{name}")),
                format!("my.person[{index}]"),
                "{name}"
            );
            assert_eq!(
                canonical_air_cell(&format!("my.skill.{name}")),
                if name == "priority" {
                    format!("my.skill.{name}")
                } else {
                    format!("my.person[{index}]")
                },
                "skill.{name}"
            );
        }
        assert_eq!(canonical_air_cell("stack.priority"), "stack.person[33]");
        assert_eq!(canonical_air_cell("my.person.unknown"), "my.person.unknown");
    }
}
