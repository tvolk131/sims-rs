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
    GuidValue, InteractionDecl, MatchArm, MatchExprArm, MatchPattern, MetaField, MetaFieldKind,
    ObjectDecl, ObjectMember, ParamType, ParamValue, ReturnAbi, SlotDecl, SoundDecl, SpriteDecl,
    StateDecl, StateInitValue, StateType, StringsDecl, TemplateDecl, TileDecl, TreeFrame,
    TreeMemberDecl, V2AssignOp, V2BinOp, V2Expr, V2Item, V2Stmt, V2Type, V2UnaryOp, VariantDecl,
    object_type_ident,
};
use crate::v2::loader::{ModuleScope, ResolvedModule};
use crate::v2::parser::snake_case_ident;
use crate::v2::specialize;
use crate::v2::{mir, raw_facts};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::rc::Rc;

mod actions;
mod air_text;
mod ast_facts;
mod builtins;
mod callable;
mod cond;
mod consteval;
mod dialog;
mod env;
mod expr;
mod inline;
mod lvalue;
mod match_lower;
mod matching;
mod object;
mod prim_table;
mod raw;
mod raw_consts;
mod schema;
mod scope;
mod stack;
mod stmt;
mod tree;
mod trees;

use self::air_text::*;
use self::ast_facts::*;
use self::env::*;
use self::matching::*;
use self::object::*;
use self::prim_table::*;
use self::raw::*;
use self::schema::*;
use self::tree::{
    definition_field, neighbor_cell, neighbor_indexed_scope, object_data_word, sim_person_word,
};
use self::trees::*;

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
    /// Extra catalog variants (`variant "Name" { ... }`), each one more OBJD/OBJf/CTSS in the
    /// same IFF sharing the object's resources. Empty for single-variant objects.
    pub variants: Vec<VariantSpec>,
    /// FWAV chunks that share another declaration's sound name, as `(id, placeholder, name)`:
    /// the manifest carries the unique placeholder, and the compiled FWAV gets `name` back.
    pub shared_sound_names: Vec<(u16, String, String)>,
}

/// One extra catalog variant of an object: the primary OBJD with these words replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantSpec {
    pub name: String,
    pub guid: u32,
    pub price: u16,
    pub description: String,
    /// BHAV ids of the variant's init / main trees (the primary's when not overridden).
    pub init_id: u16,
    pub main_id: u16,
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

/// Resolved return ABI of a callable BHAV (`extern mod` fn or standalone `#[bhav] fn`).
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallContext {
    ObjectSelf,
    SimOnObject,
    Inherit,
    Extern,
}

/// What lowering saw of calls that involve `#[context(inherit)]` trees, keyed by the callee's
/// display name (`tree 'x'`). An inherit tree may call a Sim tree by name with `me` as the Sim:
/// the callee then runs on the inherit tree's runner and stack object, which is the Sim's
/// context only when the inherit tree itself runs on a Sim. So such a tree, and every inherit
/// or Sim tree that calls it, must not be called from an object's own tree or from a
/// functional, which an object may run ([`check_inherit_calls`]).
#[derive(Default, Clone)]
struct InheritCallFacts {
    /// Inherit trees that call a Sim tree by name, with that call's span.
    calls_sim: BTreeMap<String, Span>,
    /// Calls to an inherit or Sim tree: the calling standalone inherit or Sim tree (if the
    /// caller is one), whether an object may be the caller's runner, the callee, whether the
    /// callee is an inherit tree, and the call's span.
    edges: Vec<(Option<String>, bool, String, bool, Span)>,
}

/// Refuses a call from an object's own tree or a functional to an inherit tree that runs a Sim
/// tree on its runner (directly, or through other inherit and Sim trees), or to a Sim tree that
/// reaches one.
fn check_inherit_calls(facts: &InheritCallFacts) -> Result<()> {
    let mut sim_only: BTreeMap<&str, Span> = facts
        .calls_sim
        .iter()
        .map(|(k, s)| (k.as_str(), *s))
        .collect();
    loop {
        let mut grew = false;
        for (caller, _, callee, _, span) in &facts.edges {
            if let Some(caller) = caller
                && sim_only.contains_key(callee.as_str())
                && !sim_only.contains_key(caller.as_str())
            {
                sim_only.insert(caller, *span);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    for (_, from_object, callee, callee_inherit, span) in &facts.edges {
        if *from_object && sim_only.contains_key(callee.as_str()) {
            return Err(Error::at(
                *span,
                if *callee_inherit {
                    format!(
                        "{callee} is 'context = inherit' but runs a Sim tree on its runner, so it \
                         can only be called from interactions, Sim trees, or other inherit trees \
                         (an object can run a functional)"
                    )
                } else {
                    format!(
                        "{callee} reaches a 'context = inherit' tree that runs a Sim tree on its \
                         runner, so a functional (which an object can run) cannot call it"
                    )
                },
            ));
        }
    }
    Ok(())
}

/// Fully resolved call signature of an `extern mod` fn or a sibling standalone `#[bhav] fn`.
#[derive(Debug, Clone)]
struct CallableSig {
    /// Human-readable name for diagnostics (`light_globals::init_floor`, `tree 'is_fridge'`).
    display: String,
    id: u16,
    context: CallContext,
    params: Vec<ResolvedAbiParam>,
    ret: ResolvedReturnAbi,
    /// Argument words the callee's frame reads: `#[frame(args = )]` for a standalone tree, else
    /// the positional parameter count (an `extern` is trusted to declare its whole frame).
    /// `None` when unknown.
    frame_args: Option<usize>,
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

/// Standalone `#[bhav] fn` member ready for lowering.
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
    /// Diagnostic noun (`functional` / `#[bhav] fn`).
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
    /// Calls among the object's trees that decide which `#[context(inherit)]` trees an
    /// object-context tree may not call ([`InheritCallFacts`]).
    inherit_calls: RefCell<InheritCallFacts>,
    /// The frame words any string of the object may make the formatter read
    /// ([`mir::text_reads`](mod@crate::v2::mir)): every tree is finished with the object's final
    /// fact (`lower_object` lowers again when a later tree's inline dialog literal adds to it).
    text_reads: mir::TableLocals,
    /// `text_reads` apart from what a `$Me` name may name ([`mir::strings_read_besides_me`]).
    text_reads_besides_me: mir::TableLocals,
    /// Some tree of the object may read global 34 ([`mir::may_read_global_34`]), so every tree
    /// is finished with main's rules: a call carries the scratch pointer into the callee and
    /// back, so an Expression one tree keeps or drops can change what another tree reads.
    /// `lower_object` lowers again when a finished tree sets `saw_global_34`.
    reads_global_34: bool,
    /// A finished tree's body may read global 34.
    saw_global_34: Cell<bool>,
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
    /// Relationship's `value:`. With flag 4 the VM reads it, so any `i16` value (a constant, a
    /// local, a parameter, a cell); otherwise the VM writes the number into it, so a writable
    /// direct `i16` cell (a local, `temp[i]`, an attribute, a `&mut` parameter's cell).
    RelValue,
    /// A slot number the VM reads from the operand or from a parameter: a constant `i16`, or a
    /// tree parameter (a parameter variable or `param[N]`, spelled `param[N]`).
    SlotOrParam,
    /// A tree parameter the VM reads (a parameter variable or `param[N]`), spelled by its index.
    ParamIndex,
}

/// A v2 builtin that lowers one-to-one onto a named-argument `.air` primitive.
struct PrimBuiltin {
    v2: &'static str,
    air: &'static str,
    /// `(v2 key, .air key, kind)`, in `.air` emission order.
    keys: &'static [(&'static str, &'static str, PrimArg)],
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
    /// A read-only caller cell whose value is stable for this inline call.
    Operand { air: String, ty: ValueType },
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
    /// Compile-time 32-bit GUID passed to a helper function.
    Guid(u32),
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

/// Whether an `.air` operand reads the temps: a `temp[i]` cell, job data (`scope(33, F)`,
/// which reads the career and level in `temp[0]` and `temp[1]`), or a temp-indexed cell
/// (`scope(27, k)`, `scope(30, k)`, `scope(31, k)`, which read the index in `temp[k]`).
/// Staging that writes temps copies such an operand first.
fn reads_temps(air: &str) -> bool {
    air.contains("temp[")
        || ["scope(27,", "scope(30,", "scope(31,", "scope(33,"]
            .iter()
            .any(|s| air.contains(s))
}

/// Whether an `.air` operand reads `temp[0]`: the cell itself, job data (which reads the career
/// there), or a cell indexed by it (`scope(27, 0)`, `scope(30, 0)`, `scope(31, 0)`). Staging
/// that writes only `temp[0]` copies such an operand first.
fn reads_temp0(air: &str) -> bool {
    air.contains("temp[0]")
        || air.contains("scope(33,")
        || ["scope(27, 0)", "scope(30, 0)", "scope(31, 0)"]
            .iter()
            .any(|s| air.contains(s))
}

/// Whether a single operand reads a cell of the current stack object: `stack_obj` itself, a
/// `stack.` cell, its definition (`scope(21, ..)`, `scope(35, ..)`), its sprite flags or
/// temp-indexed person data (`scope(27, ..)`, `scope(31, ..)`), or the neighbor record it names
/// (`scope(24, ..)`, `scope(32, ..)`; `sims-vm` `memory.rs`).
/// Code that moves the cursor before reading such an operand copies it first.
fn reads_stack_obj(air: &str) -> bool {
    air == "stack_obj"
        || air.starts_with("stack.")
        || [
            "scope(21,",
            "scope(24,",
            "scope(27,",
            "scope(31,",
            "scope(32,",
            "scope(35,",
        ]
        .iter()
        .any(|s| air.starts_with(s))
}

/// Whether bitflags `key` is an engine bit-field word: a `std::flags` type or `self.flags`'s
/// any-bitflags type. Such a word also takes raw `i16` operands (`w flag_set 3`, `w = 0`,
/// `w & 4`) exactly as an `i16` cell does, so typing an engine field never breaks numeric code.
fn is_engine_flags_key(key: &str) -> bool {
    key == ANY_BITFLAGS || key.starts_with("bitflags::std::flags::")
}

/// Whether enum `key` is an engine enum (a `std::engine` type): open, so a `match` on it needs
/// a `_` arm.
fn is_engine_enum_key(key: &str) -> bool {
    key.starts_with("enum::std::engine::")
}

/// The bitflags key a `flag_test`/`flag_set`/`flag_clear` constant is checked against: the
/// target's own type, or (an `i16` cell) any bitflags type.
fn flag_op_key(ty: &ValueType) -> Option<&str> {
    match ty {
        ValueType::Bitflags(key) => Some(key),
        ValueType::I16 => Some(ANY_BITFLAGS),
        _ => None,
    }
}

/// A user bitflags type, whose flag operations take only its own constants (an engine word or
/// an `i16` cell also takes a raw bit number).
fn is_strict_flags(ty: &ValueType) -> bool {
    matches!(ty, ValueType::Bitflags(_)) && !is_engine_flags(ty)
}

/// [`is_engine_flags_key`] of a value type.
fn is_engine_flags(ty: &ValueType) -> bool {
    matches!(ty, ValueType::Bitflags(k) if is_engine_flags_key(k))
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
        // Engine bit-field words (`std::flags` types, `self.flags`) also take raw words.
        (ValueType::Bitflags(k), ValueType::I16) | (ValueType::I16, ValueType::Bitflags(k))
            if is_engine_flags_key(k) =>
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
    /// Any other `stack.` cell of a foreign object or Sim (`stack.data[i]`, `stack.person[i]`,
    /// `stack.slot[i]`, `scope(27, 0)`, ...). `setup` runs before `stack_obj` is switched (the
    /// dynamic-sprite index into `temp[0]`); `obj_air == "stack_obj"` means no switch at all.
    ForeignCell {
        obj_air: String,
        cell: String,
        setup: Option<String>,
        ty: ValueType,
    },
}

impl LValue {
    fn value_type(&self) -> &ValueType {
        match self {
            LValue::Direct { ty, .. }
            | LValue::ForeignAttr { ty, .. }
            | LValue::ForeignCell { ty, .. } => ty,
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
    /// A falling-through `for ... else` handles the exhaustion edge separately, so a
    /// source `break` restores the entry stack object on its own edge.
    restore_stack_on_break: bool,
    /// What `continue` must do before jumping back to the loop head.
    continue_step: Option<ContinueStep>,
}

/// Work a `continue` emits before it jumps back to the loop head.
#[derive(Debug, Clone)]
enum ContinueStep {
    /// `repeat`: bump the iteration counter.
    Counter(String),
    /// Edge-safe range loop: leave after the last value, otherwise step the cursor.
    Range {
        var: String,
        last: String,
        step: &'static str,
    },
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
            restore_stack_on_break: false,
            continue_step: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FnReturnKind {
    Void,
    Bool,
    Value(ValueType),
}

#[derive(Clone)]
struct ActiveInlineFn {
    ret_flag: Option<String>,
    target: InlineReturnTarget,
    ret_kind: FnReturnKind,
    /// Active `defer` token when this helper was entered (returns matching `current_defer_id`
    /// exit only this helper frame, whereas mismatched returns would escape an enclosing `defer`).
    owner_defer_id: Option<usize>,
}

struct PreparedFunctional {
    prim: String,
    sig: Option<ResolvedFunctionalSig>,
    kind_name: String,
    saved_stack_obj: StackRestore,
}

/// What `stack_obj` needs after a `run_functional` call. The callee runs in its own frame, so the
/// call itself never changes the caller's `stack_obj`; only moving it to the receiver does.
#[derive(Clone)]
enum StackRestore {
    /// The receiver was `stack_obj` itself: nothing moved, nothing to restore (an entry restore
    /// here would throw away a cursor the tree set, e.g. by Find Best Object).
    Unmoved,
    /// `stack_obj` was moved to the receiver: restore the saved value (or the entry object).
    Moved(Option<String>),
}

#[derive(Clone)]
enum InlineReturnTarget {
    Void,
    Store(String),
    Branch {
        arms: Rc<InlineBranchArms>,
        break_after: bool,
    },
}

struct InlineBranchArms {
    on_true: String,
    on_false: String,
    /// Both arms exit the tree, so the helper needs no single-pass loop.
    terminal: bool,
}

type InlineHelperCall<'a> = (String, FnDecl, bool, &'a [CallArg], Span);

enum InlineCallMode<'a> {
    Materialize,
    TailCondition,
    Into(&'a str),
    Branch(Rc<InlineBranchArms>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HelperCell {
    Temp(i16),
    Param(i16),
    OwnAttr(i16),
    /// Any other object, global or Sim cell: forwarded only into a helper whose body writes
    /// nothing but its own locals and calls nothing, so no cell (and no cursor) changes.
    Pure,
}

/// One lowering of a source declaration: the source call sites of the inlined helpers around
/// it, the defer instance, and its (line, column) (see `TreeLowerer::seen_decls`).
type DeclInstance = (Vec<(usize, usize)>, Option<usize>, (usize, usize));

/// A raw block: its module, source line, column and code (see `TreeLowerer::own_raw_consts`).
type RawBlockKey = (String, usize, usize, String);

#[derive(Clone)]
struct TreeLowerer<'a> {
    env: &'a ObjectEnv<'a>,
    tree_ctx: TreeContext,
    owner_module: String,
    /// `true` when lowering code inside the object/template (can see `local_fns` / local types),
    /// `false` inside an imported/module-level helper function.
    in_object_scope: bool,
    has_self_obj_local: bool,
    /// This tree explicitly uses the frame's stack object as a persistent cursor. Temporary
    /// moves must restore its pre-operation value, which may differ from the entry object.
    explicit_stack_obj: bool,
    /// The tree assigns `stack_obj` (directly, through a helper, or as `&mut stack_obj`), or
    /// steps it explicitly: the cursor may rest on another object than the entry object, so a
    /// builtin that moves it saves and restores it rather than putting back the entry object.
    reassigns_stack_obj: bool,
    /// A raw block lowered so far, or one in a loop being lowered, may write the cursor: like
    /// [`Self::reassigns_stack_obj`] from that point on. Set in lowering order, so a raw block
    /// after a loop does not change how the loop lowers.
    raw_moves_cursor: bool,
    /// Resolving a held place ([`Self::resolve_borrowed_lvalue`]).
    borrowing_place: bool,
    /// The source assigns `stack_obj` itself (`stack_obj = x;`), so the stack object the tree
    /// leaves at exit is the source's: no store of it is dropped as dead at the exit
    /// (`mir::FrameFacts::keeps_exit_stack_obj`).
    sets_stack_obj: bool,
    /// Set to `true` when an `ObjectSelf` lifecycle tree (`init`/`main`/`load`/`reset`/`cleanup`)
    /// touches `stack_obj` for a foreign object access, so `finish()` preserves the incoming
    /// `stack_obj` (e.g. the cancelling Sim in `cleanup` / OBJf 4) into `__saved_stack_obj`.
    used_saved_stack_obj: bool,
    /// Non-`None` when lowering a body that returns a value in `temp[N]` (typed
    /// `functional kind(...) -> T` or a standalone `#[bhav] fn .. -> T` with `#[ret(temp = N)]`).
    value_return: Option<ValueReturnAbi>,
    /// The body being written.
    em: mir::Emitter,
    /// `raw { .. }` blocks lowered so far (trial and discarded lowerings included): a
    /// `#[temp]` loop rejects a body that adds one, since raw code can write its counter.
    raw_blocks: usize,
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
    /// Nonzero while lowering code that is only checked and then discarded (a folded-away
    /// branch or loop body); `raw` blocks are syntax-checked there.
    checking_discarded: usize,
    /// Arguments in this tree's call frame (`param[0..frame_args]` are addressable).
    frame_args: usize,
    /// Where this tree first uses `push_interaction(object: <handle>)` (the pinned slot) and a
    /// `raw` block: the two cannot share a tree, since raw `.air` can name local slots in
    /// forms (`scope(25, N)`, raw `op` operands, primitive fields) the pin placement can't see.
    pin_span: Option<Span>,
    raw_span: Option<Span>,
    /// Names the tree's `raw` blocks declare as `.air` locals, and the v2 constants they name
    /// with their `.air` values (see `collect_raw_consts`).
    raw_declared: BTreeSet<String>,
    raw_consts: BTreeMap<String, String>,
    /// The raw block that declares each raw `.air` constant, and the raw blocks that name each
    /// word (see `own_raw_consts`). A block is its module, source position and code, so the
    /// copies of one block (an inlined helper, a specialized region) are one block.
    raw_const_owner: BTreeMap<String, RawBlockKey>,
    raw_names: BTreeMap<String, BTreeSet<RawBlockKey>>,
    /// A raw block of the tree formats text (see `raw_facts`), however it is spelled.
    raw_formats_text: bool,
    /// The tree's first `#[slot(N)] let`: its pinned locals are named `__slot<N>_..` through the
    /// optimizer and placed by `opt::assign_frame_slots` when the tree is finished.
    slot_span: Option<Span>,
    /// The `.air` names of the tree's `#[slot(N)] let` locals and their word numbers: the only
    /// names `opt::assign_frame_slots` pins (a raw block's own `__slot..` name is not one).
    pinned_locals: BTreeMap<String, usize>,
    /// Each inlined helper's local and the declaration it copies (the helper and the `let`'s
    /// position): copies of one declaration are one source local, whatever their spelling shares
    /// with another helper's.
    local_origins: BTreeMap<String, String>,
    /// The innermost statement being lowered (the origin of the locals it declares).
    stmt_span: Option<Span>,
    /// Declarations lowered so far, by lowering instance (the inline-helper instances and the
    /// defer instance around them) and source span. Lowering one source declaration twice in
    /// the same instance means it was copied (a const binding's region), and each copy has its
    /// own frame word: `dup_decl` records the first, which a tree that formats text refuses.
    seen_decls: BTreeSet<DeclInstance>,
    /// The source call sites of the active inlined helpers (`inline_call_stack`'s calls).
    inline_call_sites: Vec<(usize, usize)>,
    dup_decl: Option<(Span, String)>,
    /// Set when the tree's bytes after some node matter beyond `.air` semantics, so the backend
    /// pass must leave the tree alone: a `raw` block (its literal calls and ops may read the
    /// following instruction's bytes), or a literal-argument call whose callee frame may read
    /// more than the four words in the call's own operands.
    keep_layout: bool,
    /// One past the highest `param[n]` the tree names, so a functional's header can declare
    /// the words it reads.
    param_words_used: usize,
    /// The tree's `#[frame(locals = K)]`, which `animate(event_local:)` must stay outside.
    frame_locals: Option<u16>,
    /// The tree's `#[frame(format = F)]`, whose format may bound the frame (`0x8000`/`0x8001` hold
    /// four locals).
    frame_format: Option<u16>,
    /// The first `animate(event_local: N)`, as `(span, N)`.
    event_local: Option<(Span, u8)>,
    /// The first `sleep`/`idle`, which `.air` lowers through `idle()`: it cannot share a tree
    /// with a raw op such as `animate(event_local:)`.
    sleep_span: Option<Span>,
    /// Where each source statement of the tree itself (not of an inlined helper) starts in the
    /// emitted text, as `(mark, span)`: the `#[context(inherit)]` check points its error at the
    /// statement whose lowering differs between the caller contexts.
    stmt_marks: Vec<(usize, Span)>,
    /// The Sim pass of the `#[context(inherit)]` check, which emits no entry save up front: like
    /// the object pass's `__saved_stack_obj`, `finish` declares `__self_obj` (the entry stack
    /// object) first when the body reads it.
    inherited_entry_save: bool,
    /// The tree is `#[context(inherit)]`: its `me` may be a Sim in either pass, so it reads the
    /// Sim's typed fields like the Sim handle ([`TreeLowerer::is_my_ident`]).
    inherit_tree: bool,
    /// The standalone tree being lowered (`tree 'x'`), for [`InheritCallFacts`].
    tree_display: Option<String>,
    /// A functional's action or guard: lowered as a Sim tree, but `run_functional` and Find
    /// Best Object run it on whichever object asks, so an object may be its runner
    /// ([`InheritCallFacts`]).
    object_reachable: bool,
    /// The compiler's entry-object locals that a raw block of the tree declares, with the
    /// block's span: an error when the tree's lowering declares the same name (`finish`).
    raw_reserved: Vec<(&'static str, Span)>,
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

/// One `match` arm's body: statements (statement form) or a value (expression form).
#[derive(Clone, Copy)]
enum ArmBody<'a> {
    Stmts(&'a [V2Stmt]),
    Value(&'a V2Expr),
}

/// One non-`_` match arm: its alternatives as inclusive ranges, and its body.
struct MatchCase<'a> {
    alts: Vec<(i16, i16)>,
    body: ArmBody<'a>,
}

/// Where an expression `match` / `if` writes its arms' values, and their common type.
struct MatchResult {
    dest: String,
    ty: Option<ValueType>,
    /// `if` expression (for diagnostics): its arms are "branches".
    if_expr: bool,
    /// A `let` initializer's chain: an arm may be a diverging `{ ..; return ..; }` block.
    may_diverge: bool,
}

const SLOT_WITH_PIN: &str = "push_interaction(object: <handle>) cannot share a tree with '#[slot(N)] let': the pinned object slot is local[0], which the frame layout around '#[slot]' locals does not reserve; pass a constant parameter index or move one of them to its own tree";
const RAW_WITH_PIN: &str = "push_interaction(object: <handle>) cannot share a tree with 'raw { ... }' or a 'local[N]' read: they may name local slots the pinned object slot could collide with; pass a constant parameter index or move the raw block to its own tree";

/// Placeholder for the per-tree pinned local slot of `push_interaction(object: <handle>)`,
/// replaced in `TreeLowerer::finish` (not an identifier character, so no name contains it).
const PIN_SLOT: &str = "@pin@";

/// Look through inline helpers as well as the tree body. The cursor may be displaced at a
/// control-flow join or on a loop back edge, so use the same storage rules throughout the tree.
#[derive(Default)]
struct ExplicitStackOps {
    visited: BTreeSet<(String, String, bool)>,
    /// Also count a `stack_obj = x;` statement (in a helper body).
    assignments: bool,
    /// Also count a raw block that may write the cursor.
    raw_writes: bool,
}
