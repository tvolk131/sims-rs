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
    InteractionDecl, MatchArm, MatchExprArm, MatchPattern, MetaField, MetaFieldKind, ObjectDecl,
    ObjectMember, ParamType, ParamValue, ReturnAbi, SlotDecl, SoundDecl, SpriteDecl, StateDecl,
    StateInitValue, StateType, StringsDecl, TemplateDecl, TileDecl, TreeFrame, TreeMemberDecl,
    V2AssignOp, V2BinOp, V2Expr, V2Stmt, V2Type, V2UnaryOp, VariantDecl, object_type_ident,
};
use crate::v2::loader::{ModuleScope, ResolvedModule};
use crate::v2::parser::snake_case_ident;
use crate::v2::specialize;
use crate::v2::{mir, raw_facts};
use std::cell::RefCell;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallContext {
    ObjectSelf,
    SimOnObject,
    Inherit,
    Extern,
}

/// Fully resolved call signature of an `extern mod` fn or a sibling standalone `tree`.
#[derive(Debug, Clone)]
struct CallableSig {
    /// Human-readable name for diagnostics (`light_globals::init_floor`, `tree 'is_fridge'`).
    display: String,
    id: u16,
    context: CallContext,
    params: Vec<ResolvedAbiParam>,
    ret: ResolvedReturnAbi,
    /// Argument words the callee's frame reads: `@frame(args:)` for a standalone tree, else
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
    /// The frame words any string of the object may make the formatter read
    /// ([`mir::text_reads`](mod@crate::v2::mir)): every tree is finished with the object's final
    /// fact (`lower_object` lowers again when a later tree's inline dialog literal adds to it).
    text_reads: mir::TableLocals,
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
    saved_stack_obj: Option<String>,
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
    /// Set to `true` when an `ObjectSelf` lifecycle tree (`init`/`main`/`load`/`reset`/`cleanup`)
    /// touches `stack_obj` for a foreign object access, so `finish()` preserves the incoming
    /// `stack_obj` (e.g. the cancelling Sim in `cleanup` / OBJf 4) into `__saved_stack_obj`.
    used_saved_stack_obj: bool,
    /// Non-`None` when lowering a body that returns a value in `temp[N]` (typed
    /// `functional kind(...) -> T` or a standalone `tree ... -> T @ temp[N]`).
    value_return: Option<ValueReturnAbi>,
    /// The body being written.
    em: mir::Emitter,
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
    /// The tree's first `@slot(N) let`: its pinned locals are named `__slot<N>_..` through the
    /// optimizer and placed by `opt::assign_frame_slots` when the tree is finished.
    slot_span: Option<Span>,
    /// The `.air` names of the tree's `@slot(N) let` locals and their word numbers: the only
    /// names `opt::assign_frame_slots` pins (a raw block's own `__slot..` name is not one).
    pinned_locals: BTreeMap<String, usize>,
    /// Each inlined helper's local and the declaration it copies (the helper and the `let`'s
    /// position): copies of one declaration are one source local, whatever their spelling shares
    /// with another helper's.
    local_origins: BTreeMap<String, String>,
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
    /// The tree's `@frame(locals: K)`, which `animate(event_local:)` must stay outside.
    frame_locals: Option<u16>,
    /// The first `animate(event_local: N)`, as `(span, N)`.
    event_local: Option<(Span, u8)>,
    /// The first `sleep`/`idle`, which `.air` lowers through `idle()`: it cannot share a tree
    /// with a raw op such as `animate(event_local:)`.
    sleep_span: Option<Span>,
    /// Where each source statement of the tree itself (not of an inlined helper) starts in the
    /// emitted text, as `(mark, span)`: the `@context(inherit)` check points its error at the
    /// statement whose lowering differs between the caller contexts.
    stmt_marks: Vec<(usize, Span)>,
    /// The Sim pass of the `@context(inherit)` check, which emits no entry save up front: like
    /// the object pass's `__saved_stack_obj`, `finish` declares `__self_obj` (the entry stack
    /// object) first when the body reads it.
    inherited_entry_save: bool,
    /// The statement being lowered is one of the tree's leading top-level statements, none of
    /// which may move the cursor (see `lower_tree_body`): `stack_obj` still holds the entry
    /// object, so a Sim tree's `self` reads and writes it in place.
    entry_cursor: bool,
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
}

const SLOT_WITH_PIN: &str = "push_interaction(object: <handle>) cannot share a tree with '@slot(N) let': the pinned object slot is local[0], which the frame layout around '@slot' locals does not reserve; pass a constant parameter index or move one of them to its own tree";
const RAW_WITH_PIN: &str = "push_interaction(object: <handle>) cannot share a tree with 'raw { ... }' or a 'local[N]' read: they may name local slots the pinned object slot could collide with; pass a constant parameter index or move the raw block to its own tree";

/// Placeholder for the per-tree pinned local slot of `push_interaction(object: <handle>)`,
/// replaced in `TreeLowerer::finish` (not an identifier character, so no name contains it).
const PIN_SLOT: &str = "@pin@";

/// Look through inline helpers as well as the tree body. The cursor may be displaced at a
/// control-flow join or on a loop back edge, so use the same storage rules throughout the tree.
#[derive(Default)]
struct ExplicitStackOps {
    visited: BTreeSet<(String, String, bool)>,
}
