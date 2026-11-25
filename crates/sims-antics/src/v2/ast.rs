//! Abstract syntax tree for the high-level `.antics` v2 authoring language.

use crate::art::FurnitureShape;
use crate::audio::SoundPreset;
use crate::error::Span;

/// A parsed `.antics` v2 source file.
#[derive(Debug, Clone, PartialEq)]
pub struct V2File {
    pub uses: Vec<UseDecl>,
    pub items: Vec<V2Item>,
}

/// Rust-style `use` import declaration (`use pack::mod::Item;` or `use pack::mod::{A, B};`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseDecl {
    pub span: Span,
    /// Leading module path segments (e.g. `["open_pack", "common"]`).
    pub module_path: Vec<String>,
    /// Imported item names (one for single import, multiple for `{A, B}`).
    pub items: Vec<UseItem>,
}

/// One imported symbol in a `use` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseItem {
    pub span: Span,
    pub name: String,
    pub alias: Option<String>,
}

/// Top-level item inside a `.antics` v2 file.
#[derive(Debug, Clone, PartialEq)]
pub enum V2Item {
    Const(ConstDecl),
    Fn(FnDecl),
    Template(TemplateDecl),
    Object(ObjectDecl),
    Enum(EnumDecl),
    Bitflags(BitflagsDecl),
    ExternMod(ExternModDecl),
    ExternBcon(ExternBconDecl),
}

/// Where a BHAV-call ABI parameter is passed (`a: i16` positional or `#[temp(N)] a: i16`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiParamSlot {
    /// Passed positionally as callee `param[i]` (literal call operands, or `temp[i]` via `call(temps)`).
    Positional,
    /// Written into the shared thread temporary `temp[N]` (`0..=7`) before the call.
    Temp(u8),
}

/// One parameter of an `extern mod` function or standalone `#[bhav] fn` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiParam {
    pub span: Span,
    pub name: String,
    pub ty: V2Type,
    pub slot: AbiParamSlot,
}

/// Explicit SimAntics return ABI of an `extern mod` function or standalone `#[bhav] fn`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnAbi {
    /// No declared result: the call's true/false exit is ignored by callers.
    Void,
    /// `-> bool`: the call's true/false exit is a boolean value.
    Bool,
    /// `-> Result`: true exit = success, false exit = failure (`?` propagates `return error;`).
    Result,
    /// `#[ret(temp = N)] .. -> T`: the callee leaves a value in `temp[N]` (true/false exit ignored).
    Value { ty: V2Type, temp: u8 },
    /// `#[ret(temp = N)] .. -> Result<T>`: true exit = success with the value in `temp[N]`, false = failure.
    ResultValue { ty: V2Type, temp: u8 },
}

/// One callable external BHAV inside an `extern mod` block (`#[id(N)] fn name(...) -> Abi;`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternFnDecl {
    pub span: Span,
    pub id: u16,
    pub name: String,
    pub params: Vec<AbiParam>,
    pub ret: ReturnAbi,
}

/// External global (`256..=4095`) / semi-global (`8192..`) BHAV bindings (`[pub] extern mod name { ... }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternModDecl {
    pub span: Span,
    pub is_pub: bool,
    pub name: String,
    pub fns: Vec<ExternFnDecl>,
}

/// One named external BCON constant (`name @ <address>;` or `name @ <bcon_id>[<entry>];`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternBconEntry {
    pub span: Span,
    pub name: String,
    /// Encoded SimAntics constant operand address (`(table << 7) | entry`).
    pub address: u16,
}

/// Compile-time integer constant (`[pub] const NAME = 0x...;`): an `i16` when it fits, otherwise a
/// 32-bit GUID usable where a GUID is expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstDecl {
    pub span: Span,
    pub is_pub: bool,
    pub name: String,
    pub value: i64,
}

/// External constant bindings (`[pub] extern bcon name { entry @ 16521; ... }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternBconDecl {
    pub span: Span,
    pub is_pub: bool,
    pub name: String,
    pub entries: Vec<ExternBconEntry>,
}

/// Explicit private BCON table (`bcon 4097 "LampTuning" { cheap = 10, ... }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BconTableDecl {
    pub span: Span,
    pub id: u16,
    pub label: String,
    pub entries: Vec<(Span, String, i16)>,
}

/// Standalone non-inlined BHAV (`[#[id(N)]] tree [ident] ["Label"] [(params)] [-> Abi] { ... }`).
#[derive(Debug, Clone, PartialEq)]
pub struct TreeMemberDecl {
    pub span: Span,
    pub pinned_id: Option<u16>,
    /// Call identifier (explicit, or derived in `snake_case` from the label).
    pub ident: String,
    /// BHAV chunk label (explicit string, or the identifier).
    pub label: String,
    /// `Some(name)` when the first parameter is `name: Sim` (Sim-on-object context).
    pub sim_binding: Option<String>,
    /// `#[context(inherit)]` lets callers supply either frame context unchanged.
    pub inherit_context: bool,
    pub params: Vec<AbiParam>,
    pub ret: ReturnAbi,
    pub body: Vec<V2Stmt>,
    /// `#[frame(args = N, locals = M, format = F, revision = R, kind = K)]` header pins (all optional).
    pub frame: Option<TreeFrame>,
}

/// Explicit BHAV header fields for a standalone `#[bhav] fn` (`#[frame(...)]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TreeFrame {
    pub span: Span,
    pub args: Option<u8>,
    pub locals: Option<u16>,
    pub format: Option<u16>,
    pub revision: Option<u32>,
    /// The header's kind byte (`.air` `kind:`).
    pub kind: Option<u8>,
    /// `#[frame(empty)]`: a 0-node BHAV (the body must be `{}`). Entering one faults, as the
    /// original's do; a plain `{}` body is `return true;`.
    pub empty: bool,
}

/// Zero-cost integer enum declaration (`[pub] enum Name { VariantA, VariantB = 2 }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDecl {
    pub span: Span,
    pub is_pub: bool,
    pub name: String,
    pub variants: Vec<EnumVariant>,
}

/// One variant inside an `enum` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariant {
    pub span: Span,
    pub name: String,
    pub value: i16,
}

/// Zero-cost 1-based SimAntics bitflags declaration (`[pub] bitflags Name { FlagA = 1, FlagB = 2 }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitflagsDecl {
    pub span: Span,
    pub is_pub: bool,
    pub name: String,
    pub flags: Vec<BitflagVariant>,
}

/// One flag inside a `bitflags` declaration, storing its 1-based SimAntics bit position (`1..=16`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitflagVariant {
    pub span: Span,
    pub name: String,
    pub bit: u8,
}

/// Type annotations in `.antics` v2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum V2Type {
    I16,
    Bool,
    Sim,
    Object,
    ObjectId,
    Motive,
    Skill,
    Sound,
    Slot,
    Guid,
    Named(String),
    MutRef(Box<V2Type>),
}

impl V2Type {
    #[must_use]
    pub fn display_name(&self) -> String {
        match self {
            Self::I16 => "i16".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Sim => "Sim".to_owned(),
            Self::Object => "Object".to_owned(),
            Self::ObjectId => "ObjectId".to_owned(),
            Self::Motive => "Motive".to_owned(),
            Self::Skill => "Skill".to_owned(),
            Self::Sound => "Sound".to_owned(),
            Self::Slot => "Slot".to_owned(),
            Self::Guid => "Guid".to_owned(),
            Self::Named(name) => name.clone(),
            Self::MutRef(inner) => format!("&mut {}", inner.display_name()),
        }
    }
}

/// One parameter in a `fn` / `pub fn` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct FnParam {
    pub span: Span,
    pub name: String,
    pub ty: V2Type,
    pub default: Option<V2Expr>,
}

/// Reusable helper function declaration (`[pub] fn name(...) [-> bool | -> Result | -> i16] { ... }`).
#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub span: Span,
    pub is_pub: bool,
    pub name: String,
    pub params: Vec<FnParam>,
    pub return_type: Option<V2Type>,
    pub body: Vec<V2Stmt>,
}

/// Reusable object template declaration (`[pub] template Name { ... }`).
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateDecl {
    pub span: Span,
    pub is_pub: bool,
    pub name: String,
    pub members: Vec<ObjectMember>,
}

/// Concrete object declaration (`object "Display Name" [from Template] { ... }`).
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectDecl {
    pub span: Span,
    pub name: String,
    pub from_template: Option<(String, Span)>,
    pub members: Vec<ObjectMember>,
}

/// Flat top-level member inside a `template` or `object` body.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectMember {
    Const(ConstDecl),
    Meta(MetaField),
    Param(ParamDecl),
    ParamAssign(ParamAssignDecl),
    State(StateDecl),
    Sound(SoundDecl),
    Slot(SlotDecl),
    Strings(StringsDecl),
    Enum(EnumDecl),
    Bitflags(BitflagsDecl),
    Init(LifecycleBlock),
    Main(LifecycleBlock),
    Load(LifecycleBlock),
    Reset(LifecycleBlock),
    Cleanup(LifecycleBlock),
    Functional(FunctionalDecl),
    Interaction(InteractionDecl),
    Fn(FnDecl),
    ExternMod(ExternModDecl),
    ExternBcon(ExternBconDecl),
    Bcon(BconTableDecl),
    Tree(TreeMemberDecl),
    Variant(VariantDecl),
}

/// Extra catalog variant sharing the object's resource pool:
/// `variant "Name" { guid: ..., price: ..., init: tree_name, main: tree_name, description: "..." }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantDecl {
    pub span: Span,
    pub name: String,
    pub guid: Option<(Span, GuidValue)>,
    pub price: Option<(Span, i64)>,
    pub description: Option<String>,
    /// Standalone `#[bhav] fn` names replacing the primary OBJD's init / main trees.
    pub init: Option<(Span, String)>,
    pub main: Option<(Span, String)>,
}

/// Procedural sprite declaration (`Sprite::procedural("cot", primary: #RRGGBB, ...)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteDecl {
    pub span: Span,
    pub shape: FurnitureShape,
    pub primary_rgb: [u8; 3],
    pub accent_rgb: [u8; 3],
    pub trim_rgb: [u8; 3],
}

/// Multi-tile sub-object declaration inside `tiles: [ tile { ... }, ... ]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileDecl {
    pub span: Span,
    pub guid: u32,
    pub name: Option<String>,
    pub objd_id: Option<u16>,
    /// Encoded multi-tile offset `(dx << 8) | (dy & 0xff)`.
    pub sub_index: i16,
    pub level: i16,
    pub lead: bool,
    pub interaction_group: Option<i16>,
    pub dgrp_id: Option<u16>,
    pub shape: Option<FurnitureShape>,
    pub slot_id: Option<u16>,
    pub ttab_id: Option<u16>,
    pub attributes: Option<u16>,
}

/// Metadata property inside an `object` or `template`.
#[derive(Debug, Clone, PartialEq)]
pub struct MetaField {
    pub span: Span,
    pub kind: MetaFieldKind,
}

/// An object or variant `guid:`: a literal (`guid: 0x7F00_0020;`) or a `const` name
/// (`guid: CHEAP_FRIDGE;`), resolved during lowering like any other const.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuidValue {
    Lit(u32),
    Const(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum MetaFieldKind {
    Guid(GuidValue),
    Price(u16),
    Description(String),
    Category(u16),
    ObjectType(u16),
    ObjdId(u16),
    TtabId(u16),
    SlotId(u16),
    CtssId(u16),
    DgrpId(u16),
    Spr2Id(u16),
    PaltId(u16),
    MasterId(u16),
    InteractionGroup(i16),
    Data58(i16),
    Attributes(u16),
    Sprite(SpriteDecl),
    Graphics(Vec<SpriteDecl>),
    Tiles(Vec<TileDecl>),
}

/// Compile-time tuning parameter declaration (`param name: i16 [= default];` or `bool`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamDecl {
    pub span: Span,
    pub name: String,
    pub ty: ParamType,
    pub default: Option<ParamValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    I16,
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamValue {
    I16(i16),
    Bool(bool),
}

/// Template parameter binding (`energy_step: 20;` or `param energy_step = 20;`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamAssignDecl {
    pub span: Span,
    pub name: String,
    pub value: ParamValue,
}

/// Flat persistent state declaration (`state occupied: bool = false;`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDecl {
    pub span: Span,
    pub name: String,
    /// Optional explicit `my.attr[slot]` binding (`state name @ 7: i16;`).
    pub slot: Option<(Span, u16)>,
    pub ty: StateType,
    pub init: Option<StateInitValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateType {
    Bool,
    I16,
    ObjectId,
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateInitValue {
    Int(i16),
    Bool(bool),
    Param(String),
    Path(Span, Vec<String>),
    BitOr(Span, Box<StateInitValue>, Box<StateInitValue>),
}

/// Typed sound declaration (`sound settle = preset("soft_thud");`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundDecl {
    pub span: Span,
    pub name: String,
    pub preset: SoundPreset,
    /// Optional explicit WAV bank name (defaults to `<slug>_<name>`).
    pub wav_name: Option<String>,
    /// Optional explicit FWAV ID override (auto-assigned `1..` if omitted).
    pub explicit_id: Option<u16>,
}

/// Typed routing/container slot declaration (`slot use_slot { ... }`).
#[derive(Debug, Clone, PartialEq)]
pub struct SlotDecl {
    pub span: Span,
    pub name: String,
    pub kind: u16,
    pub offset: [f32; 3],
    pub standing: i32,
    pub sitting: i32,
    pub ground: i32,
    pub flags: u32,
    pub snap_target: i32,
    pub proximity: [i32; 3],
    pub max_size: i32,
    pub gradient: f32,
    pub height: i32,
    pub facing: i32,
    pub resolution: i32,
}

/// Named string table (`strings [301] { title = "...", ... }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringsDecl {
    pub span: Span,
    pub id: u16,
    pub label: Option<String>,
    pub entries: Vec<(Span, String, String)>,
}

/// `init { ... }` or `main { ... }` lifecycle hook.
#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleBlock {
    pub span: Span,
    pub pinned_id: Option<u16>,
    /// `#[frame(...)]`: the lifecycle tree's BHAV header, as on a standalone `#[bhav] fn`.
    pub frame: Option<TreeFrame>,
    pub body: Vec<V2Stmt>,
}

/// Functional callback declaration (`functional prep [-> RetType] { score: 50, guard(sim) => ..., run(sim) { ... } }`).
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionalDecl {
    pub span: Span,
    pub kind: String,
    pub kind_span: Span,
    pub params: Vec<FnParam>,
    pub return_type: Option<V2Type>,
    pub score: Option<ConstIntExpr>,
    pub pinned_id: Option<u16>,
    pub pinned_guard_id: Option<u16>,
    pub guard: Option<GuardDecl>,
    pub sim_binding: String,
    pub body: Vec<V2Stmt>,
}

/// Compile-time integer expression usable in metadata (literal or `i16` param name, optional unary `-`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstIntExpr {
    Literal(i16),
    Param { name: String, negate: bool },
}

/// Motive advertisement inside an `interaction`'s `advertises: [...]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvertiseDecl {
    pub span: Span,
    pub motive_index: usize,
    pub delta: ConstIntExpr,
    pub minimum: ConstIntExpr,
    pub personality: u16,
}

/// Guard clause on an `interaction` or `functional`.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardDecl {
    pub span: Span,
    pub sim_binding: String,
    /// Optional 2nd parameter (`guard(sim, is_autonomous)`) on `interaction` guards.
    pub autonomous_binding: Option<String>,
    pub kind: GuardBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GuardBody {
    Expr(V2Expr),
    Block(Vec<V2Stmt>),
}

/// Pie-menu interaction declaration (`interaction "Take a Nap" { ... }`).
#[derive(Debug, Clone, PartialEq)]
pub struct InteractionDecl {
    pub span: Span,
    pub label: String,
    pub pinned_id: Option<u16>,
    pub pinned_guard_id: Option<u16>,
    pub autonomy: u32,
    pub attenuation_code: u32,
    pub attenuation: f32,
    pub flags: u32,
    pub flags2: u32,
    pub joining_index: i32,
    pub advertises: Vec<AdvertiseDecl>,
    /// Optional declarative occupancy lock (`lock: self.<bool_state>`).
    pub lock: Option<(Span, String)>,
    pub guard: Option<GuardDecl>,
    pub sim_binding: String,
    pub run_body: Vec<V2Stmt>,
}

/// Assignment operators in `.antics` v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V2AssignOp {
    Set,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    FlagSet,
    FlagClear,
}

/// Binary operators in `.antics` v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V2BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    FlagTest,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
}

/// Unary operators in `.antics` v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V2UnaryOp {
    Not,
    Neg,
}

/// Statements inside v2 function / interaction / lifecycle bodies.
#[derive(Debug, Clone, PartialEq)]
pub enum V2Stmt {
    Let {
        span: Span,
        name: String,
        ty: Option<V2Type>,
        init: V2Expr,
        /// `#[slot(N)] let ..`: the frame word (`local[N]`) the variable lives in.
        slot: Option<u8>,
    },
    LetSome {
        span: Span,
        name: String,
        ty: Option<V2Type>,
        init: V2Expr,
        else_body: Vec<V2Stmt>,
    },
    Assign {
        span: Span,
        target: V2Expr,
        op: V2AssignOp,
        value: V2Expr,
    },
    WithLock {
        span: Span,
        lock_expr: V2Expr,
        body: Vec<V2Stmt>,
    },
    Defer {
        span: Span,
        body: Vec<V2Stmt>,
    },
    Repeat {
        span: Span,
        count: V2Expr,
        cancel_on_abort: bool,
        body: Vec<V2Stmt>,
    },
    While {
        span: Span,
        cond: V2Expr,
        cancel_on_abort: bool,
        body: Vec<V2Stmt>,
    },
    ForEach {
        span: Span,
        binding: String,
        iter: ForEachIter,
        body: Vec<V2Stmt>,
        /// Runs on iterator exhaustion, outside the scope of the loop binding.
        else_body: Option<Vec<V2Stmt>>,
    },
    If {
        span: Span,
        cond: V2Expr,
        then_body: Vec<V2Stmt>,
        else_body: Vec<V2Stmt>,
    },
    Match {
        span: Span,
        scrutinee: V2Expr,
        arms: Vec<MatchArm>,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    Return {
        span: Span,
        value: Option<V2Expr>,
    },
    /// `return error;` — exits the whole BHAV through the SimAntics error exit.
    ReturnError {
        span: Span,
        /// Synthesized by `cond?;`: always the error exit, even when a binding named `error`
        /// is in scope (a written `return error;` then returns that binding instead).
        from_try: bool,
    },
    Expr {
        span: Span,
        expr: V2Expr,
    },
    RawAir {
        span: Span,
        code: String,
    },
    /// `let name: Type;` — declares a local without initializing it.
    LetUninit {
        span: Span,
        name: String,
        ty: V2Type,
        /// `#[slot(N)] let ..`: the frame word (`local[N]`) the variable lives in.
        slot: Option<u8>,
    },
    /// `animate(...);` (play to completion) or `animate(...) |event| { ... }` (run the body
    /// once per animation event, with `event` bound to the event number).
    Animate {
        span: Span,
        args: Vec<CallArg>,
        event: Option<String>,
        body: Vec<V2Stmt>,
    },
}

/// One arm of a `match` statement (`Pattern => { ... }`).
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub span: Span,
    pub pattern: MatchPattern,
    pub body: Vec<V2Stmt>,
}

/// One arm of a `match` expression (`Pattern => value`).
#[derive(Debug, Clone, PartialEq)]
pub struct MatchExprArm {
    pub span: Span,
    pub pattern: MatchPattern,
    pub value: V2Expr,
}

/// Pattern inside a `match` arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchPattern {
    Wildcard(Span),
    Ident(Span, String),
    Int(Span, i16),
    Path(Span, Vec<String>),
    /// `lo..hi` / `lo..=hi` on an `i16` scrutinee (stored inclusive: `lo..=hi`).
    Range(Span, i16, i16),
    /// `A | B | ..`: matches when any alternative does. Flat (no nested `Or`), at least two
    /// alternatives, none of them `_`.
    Or(Span, Vec<MatchPattern>),
}

/// Built-in loop iteration sources for `for obj in ...`.
#[derive(Debug, Clone, PartialEq)]
pub enum ForEachIter {
    /// `objects_by_guid(GUID)` (a literal or a `const`).
    ObjectsByGuid(V2Expr),
    ObjectsOf {
        span: Span,
        object_type: String,
    },
    SelfParts,
    /// `a..b` / `a..=b`, optionally `(a..b).rev()`.
    Range {
        start: V2Expr,
        end: V2Expr,
        inclusive: bool,
        rev: bool,
    },
    /// `lot.sims()`: every person on the lot (`set_to_next` person mode).
    LotSims,
    /// `lot.objects()`: every object on the lot (`set_to_next` object mode).
    LotObjects,
}

/// Named or positional call argument.
#[derive(Debug, Clone, PartialEq)]
pub struct CallArg {
    pub span: Span,
    pub name: Option<String>,
    pub value: V2Expr,
}

/// Expressions in `.antics` v2.
#[derive(Debug, Clone, PartialEq)]
pub enum V2Expr {
    Int(Span, i64),
    Bool(Span, bool),
    Str(Span, String),
    Ident(Span, String),
    /// Qualified path like `module::helper` or `Motive::Energy`.
    Path(Span, Vec<String>),
    /// Parenthesized tuple like `("Order Groceries", "Chat", "Info")`.
    Tuple(Span, Vec<V2Expr>),
    Field {
        span: Span,
        base: Box<V2Expr>,
        field: String,
    },
    Index {
        span: Span,
        base: Box<V2Expr>,
        index: Box<V2Expr>,
    },
    Deref {
        span: Span,
        inner: Box<V2Expr>,
    },
    MutRef {
        span: Span,
        inner: Box<V2Expr>,
    },
    Unary {
        span: Span,
        op: V2UnaryOp,
        inner: Box<V2Expr>,
    },
    Binary {
        span: Span,
        lhs: Box<V2Expr>,
        op: V2BinOp,
        rhs: Box<V2Expr>,
    },
    Call {
        span: Span,
        callee: Box<V2Expr>,
        type_args: Vec<V2Type>,
        args: Vec<CallArg>,
    },
    MethodCall {
        span: Span,
        receiver: Box<V2Expr>,
        method: String,
        type_args: Vec<V2Type>,
        args: Vec<CallArg>,
    },
    /// Postfix `expr?`: exits through `return error;` when `expr` is false / fails.
    Try {
        span: Span,
        inner: Box<V2Expr>,
    },
    /// Expression `match`: every arm yields a value, and the arms are exhaustive.
    Match {
        span: Span,
        scrutinee: Box<V2Expr>,
        arms: Vec<MatchExprArm>,
    },
    /// Expression `if c { a } else { b }` (`else if` nests another `If` as `else_value`).
    If {
        span: Span,
        cond: Box<V2Expr>,
        then_value: Box<V2Expr>,
        else_value: Box<V2Expr>,
    },
    /// A diverging branch of an `if` / `match` expression: `{ ..; return false; }`, or a
    /// bare `return` / `break` / `continue` arm. It yields no value: every path through it
    /// leaves (checked when it is lowered). The parser makes one only as such a branch.
    Block {
        span: Span,
        body: Vec<V2Stmt>,
    },
}

impl V2Expr {
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Int(s, _)
            | Self::Bool(s, _)
            | Self::Str(s, _)
            | Self::Ident(s, _)
            | Self::Path(s, _)
            | Self::Tuple(s, _)
            | Self::Field { span: s, .. }
            | Self::Index { span: s, .. }
            | Self::Deref { span: s, .. }
            | Self::MutRef { span: s, .. }
            | Self::Unary { span: s, .. }
            | Self::Binary { span: s, .. }
            | Self::Call { span: s, .. }
            | Self::MethodCall { span: s, .. }
            | Self::Try { span: s, .. }
            | Self::Match { span: s, .. }
            | Self::If { span: s, .. }
            | Self::Block { span: s, .. } => *s,
        }
    }
}

/// Converts a display name (e.g. `"Open Frost Fridge"`) into its PascalCase type identifier
/// (`"OpenFrostFridge"`).
#[must_use]
pub(crate) fn object_type_ident(name: &str) -> String {
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

/// Returns `true` if `name` is a built-in/reserved type name that user `enum`, `bitflags`,
/// `object` type identifiers, and `use ... as <Alias>` bindings may not shadow.
#[must_use]
pub(crate) fn is_reserved_type_name(name: &str) -> bool {
    matches!(
        name,
        "i16"
            | "bool"
            | "Object"
            | "ObjectId"
            | "Sim"
            | "Motive"
            | "Skill"
            | "Sound"
            | "Slot"
            | "Guid"
            | "Height"
            | "Placement"
            | "Self"
    )
}
