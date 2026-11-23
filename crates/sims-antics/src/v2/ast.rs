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
    Fn(FnDecl),
    Template(TemplateDecl),
    Object(ObjectDecl),
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

/// Reusable helper function declaration (`[pub] fn name(...) [-> bool | -> i16] { ... }`).
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
    Meta(MetaField),
    Param(ParamDecl),
    ParamAssign(ParamAssignDecl),
    State(StateDecl),
    Sound(SoundDecl),
    Slot(SlotDecl),
    Strings(StringsDecl),
    Init(LifecycleBlock),
    Main(LifecycleBlock),
    Functional(FunctionalDecl),
    Interaction(InteractionDecl),
    Fn(FnDecl),
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

#[derive(Debug, Clone, PartialEq)]
pub enum MetaFieldKind {
    Guid(u32),
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
    pub ty: StateType,
    pub init: Option<StateInitValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateType {
    Bool,
    I16,
    ObjectId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateInitValue {
    Int(i16),
    Bool(bool),
    Param(String),
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
    pub body: Vec<V2Stmt>,
}

/// Functional callback declaration (`functional prep { score: 50, guard(sim) => ..., run(sim) { ... } }`).
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionalDecl {
    pub span: Span,
    pub kind: String,
    pub kind_span: Span,
    pub params: Vec<FnParam>,
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
    },
    LetSome {
        span: Span,
        name: String,
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
    },
    If {
        span: Span,
        cond: V2Expr,
        then_body: Vec<V2Stmt>,
        else_body: Vec<V2Stmt>,
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
    Expr {
        span: Span,
        expr: V2Expr,
    },
    RawAir {
        span: Span,
        code: String,
    },
}

/// Built-in loop iteration sources for `for obj in ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForEachIter {
    ObjectsByGuid(u32),
    SelfParts,
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
        args: Vec<CallArg>,
    },
    MethodCall {
        span: Span,
        receiver: Box<V2Expr>,
        method: String,
        args: Vec<CallArg>,
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
            | Self::Field { span: s, .. }
            | Self::Index { span: s, .. }
            | Self::Deref { span: s, .. }
            | Self::MutRef { span: s, .. }
            | Self::Unary { span: s, .. }
            | Self::Binary { span: s, .. }
            | Self::Call { span: s, .. }
            | Self::MethodCall { span: s, .. } => *s,
        }
    }
}
