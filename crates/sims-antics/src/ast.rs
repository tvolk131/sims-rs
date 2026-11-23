//! Abstract syntax tree for the `.air` (Antics IR) low-level textual IR layer.

use crate::error::Span;

/// A parsed `.antics` source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub items: Vec<Item>,
}

/// Top-level items inside a `.antics` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Tree(TreeDecl),
    Bcon(BconDecl),
    Str(StrDecl),
}

/// A `BCON` integer constant table declaration in `.antics`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BconDecl {
    pub span: Span,
    pub id: u16,
    pub label: String,
    pub flags: u8,
    pub values: Vec<i16>,
    /// Optional symbolic names parallel to `values` (for `bcon.name` lookups).
    pub entry_names: Vec<Option<String>>,
}

/// One entry inside a `str` (`STR#`) declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrEntryDecl {
    pub span: Span,
    pub name: Option<String>,
    pub text: String,
    pub comment: String,
}

/// A `STR#` string table declaration in `.antics`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrDecl {
    pub span: Span,
    pub id: u16,
    pub label: String,
    pub entries: Vec<StrEntryDecl>,
}

/// Header options for a `tree` or `interaction` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeHeader {
    pub format: u16,
    pub kind: u8,
    pub args: Option<u8>,
    pub locals: Option<u16>,
    pub flags: u16,
    pub revision: u32,
    pub trailing: Vec<u8>,
}

impl Default for TreeHeader {
    fn default() -> Self {
        Self {
            format: 0x8002,
            kind: 0,
            args: None,
            locals: None,
            flags: 0,
            revision: 0,
            trailing: Vec::new(),
        }
    }
}

/// A `tree` or `interaction` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeDecl {
    pub span: Span,
    pub is_interaction: bool,
    pub id: u16,
    pub label: String,
    pub header: TreeHeader,
    pub body: Vec<Stmt>,
}

/// Explicit branch target in low-level exact instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Label(String),
    Node(u8),
    Raw(u8),
    ReturnTrue,
    ReturnFalse,
    Error,
}

/// Optional explicit branch routing `-> true: <t>, false: <f>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branches {
    pub on_true: Target,
    pub on_false: Target,
}

/// Resolved concrete memory operand (scope + 16-bit index) in SimAntics bytecode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operand {
    pub scope: u8,
    pub index: i16,
}

impl Operand {
    #[must_use]
    pub const fn new(scope: u8, index: i16) -> Self {
        Self { scope, index }
    }

    #[must_use]
    pub const fn literal(value: i16) -> Self {
        Self {
            scope: 7,
            index: value,
        }
    }
}

/// Symbolic or direct operand reference prior to lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperandRef {
    Direct(Operand),
    NamedLocal(String),
    NamedBcon(String),
    NamedStr(String),
}

impl From<Operand> for OperandRef {
    fn from(op: Operand) -> Self {
        Self::Direct(op)
    }
}

/// Reference to a 1-based `STR#` entry index (either integer or `str.<name>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringRef {
    Index(u8),
    Named(String),
}

/// Reference to a local variable slot (either explicit `u8` index or named local).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalRef {
    Index(u8),
    Named(String),
}

/// Assignment / arithmetic mutation operators in SimAntics Expression (opcode 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Set,       // 5: := or =
    Add,       // 3: +=
    Sub,       // 4: -=
    Mul,       // 6: *=
    Div,       // 7: /=
    Mod,       // 12: %=
    BitAnd,    // 13: &=
    BitOr,     // 18: |=
    BitXor,    // 19: ^=
    FlagSet,   // 9
    FlagClear, // 10
    Sqrt,      // 20
}

impl AssignOp {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Add => 3,
            Self::Sub => 4,
            Self::Set => 5,
            Self::Mul => 6,
            Self::Div => 7,
            Self::FlagSet => 9,
            Self::FlagClear => 10,
            Self::Mod => 12,
            Self::BitAnd => 13,
            Self::BitOr => 18,
            Self::BitXor => 19,
            Self::Sqrt => 20,
        }
    }
}

/// Comparison operators in SimAntics Expression (opcode 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Gt,       // 0
    Lt,       // 1
    Eq,       // 2
    FlagTest, // 8
    IncLt,    // 11
    Ge,       // 14
    Le,       // 15
    Ne,       // 16
    DecGt,    // 17
}

impl CompareOp {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Gt => 0,
            Self::Lt => 1,
            Self::Eq => 2,
            Self::FlagTest => 8,
            Self::IncLt => 11,
            Self::Ge => 14,
            Self::Le => 15,
            Self::Ne => 16,
            Self::DecGt => 17,
        }
    }
}

/// Target of a subroutine call: either a numeric BHAV ID or a tree name in scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallTarget {
    Id(u16),
    Name(String),
}

/// Slot selector for `drop_onto` (`kind == 0` literal slot, `kind != 0` parameter index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotSpec {
    Literal(i16),
    Param(i16),
}

/// High-level boolean condition usable in `if` and `while`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    Compare {
        span: Span,
        lhs: OperandRef,
        op: CompareOp,
        rhs: OperandRef,
    },
    Call {
        span: Span,
        target: CallTarget,
        args: Vec<i16>,
        pass_temps: bool,
    },
    Primitive {
        span: Span,
        prim: PrimitiveCall,
    },
    RawOp {
        span: Span,
        opcode: u16,
        operands: [u8; 8],
    },
    Not(Box<Condition>),
}

/// Readable SimAntics primitive invocations (usable both low-level with explicit
/// `-> true/false` branches and high-level in structured control flow).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimitiveCall {
    Sleep {
        param_index: i16,
    },
    GenericCall {
        routine: u16,
    },
    ExprRaw {
        lhs: OperandRef,
        operator: u8,
        rhs: OperandRef,
    },
    FindBestInteraction,
    Grab,
    Drop,
    ChangeSuit {
        index: u8,
        scope: u8,
        remove: bool,
    },
    Refresh {
        target: u16,
        mode: u16,
    },
    Random {
        dst: OperandRef,
        range: OperandRef,
    },
    Burn {
        mode: u8,
        flags: u8,
    },
    Tutorial {
        mode: u8,
    },
    GetDistanceTo {
        temp: u8,
        flags: u8,
        from: Option<OperandRef>,
    },
    GetDirectionTo {
        dst: OperandRef,
        flags: u8,
        from: Option<OperandRef>,
    },
    PushInteraction {
        menu: u8,
        object_slot: u8,
        priority: u8,
        flags: u8,
        icon_local: LocalRef,
    },
    FindBestObject {
        function: u16,
        flags: u8,
        origin: Option<OperandRef>,
    },
    Breakpoint {
        cond: OperandRef,
    },
    FindLocation {
        mode: u8,
        local: LocalRef,
        flags: u8,
    },
    IdleForInput {
        param_index: i16,
        allow_push: u16,
    },
    RemoveObject {
        /// 0 = me, 1 = stack_obj, 2 = 0
        target: u16,
        flags: u8,
    },
    MakeNewCharacter {
        skin: LocalRef,
        age: LocalRef,
        sex: LocalRef,
        species: u8,
    },
    RunFunctionalTree {
        function: u16,
        change_icon: bool,
    },
    ShowString {
        table: u16,
        index: u16,
    },
    LookTowards {
        mode: i16,
    },
    PlaySound {
        sound_id: u16,
        flags: u8,
    },
    OldRelationship {
        write: bool,
        column: u8,
        param: i8,
        direction: u8,
        flags: u8,
    },
    AlterBudget {
        amount: OperandRef,
        category: u8,
        /// Bit 0 = test only, bit 1 = income (negate amount).
        flags: u8,
    },
    Relationship {
        column: u8,
        direction: u8,
        flags: u8,
        local: LocalRef,
        value: OperandRef,
    },
    GoToRelative {
        location: i8,
        facing: u8,
        options: u8,
    },
    RunTreeByName {
        table: u16,
        index: u8,
        mode: u8,
        global: bool,
    },
    StopAllSounds {
        use_stack_object: bool,
    },
    SetMotiveChange {
        motive: u8,
        delta_per_hour: OperandRef,
        stop_at: OperandRef,
    },
    ClearMotiveChanges,
    GosubFoundAction,
    SetToNext {
        mode: u8,
        guid: u32,
        dest: Option<OperandRef>,
        local: LocalRef,
    },
    TestObjectType {
        guid: u32,
        object: OperandRef,
    },
    FindWorstMotives {
        target: u16,
        mode: u16,
    },
    UiEffect {
        mode: u8,
        target: Option<OperandRef>,
        flags: u8,
    },
    SpecialEffect {
        duration: i16,
        kind: u8,
        size: u8,
        flags: u8,
        str_index: u8,
    },
    Dialog {
        kind: u8,
        message: StringRef,
        title: StringRef,
        button1: StringRef,
        button2: StringRef,
        button3: StringRef,
        mode4_str: StringRef,
        flags: u8,
    },
    TestSimInteractingWith {
        person: Option<OperandRef>,
    },
    SetHeadline {
        use_stack_object: bool,
        local: LocalRef,
        index: u8,
        kind: u8,
        duration: i16,
        options: u16,
    },
    CreateObject {
        guid: u32,
        mode: u8,
        flags: u8,
        local: LocalRef,
    },
    DropOnto {
        src: SlotSpec,
        dst: SlotSpec,
    },
    Animate {
        clip_id: u16,
        table: u8,
        flags: u8,
        events: i8,
        local_index: LocalRef,
    },
    RouteTo {
        slot: u16,
        source: u16,
        flags: u8,
    },
    Snap {
        slot: u16,
        mode: u16,
        flags: u8,
    },
    Reach {
        mode: i16,
        moves: bool,
        slot_param: i16,
    },
    NotifyIdle,
    ActionString {
        table: u16,
        index: u8,
    },
    ManageInventory {
        operation: u8,
        kind: u8,
        flags: u16,
        guid: u32,
    },
}

/// Statements inside a `tree` or `interaction` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Label {
        span: Span,
        name: String,
    },
    Let {
        span: Span,
        name: String,
        init: Option<OperandRef>,
    },
    RawOp {
        span: Span,
        opcode: u16,
        operands: [u8; 8],
        branches: Branches,
    },
    InlineRawOp {
        span: Span,
        opcode: u16,
        operands: [u8; 8],
    },
    Primitive {
        span: Span,
        prim: PrimitiveCall,
        branches: Option<Branches>,
    },
    Call {
        span: Span,
        target: CallTarget,
        args: Vec<i16>,
        pass_temps: bool,
        branches: Option<Branches>,
    },
    Assign {
        span: Span,
        lhs: OperandRef,
        op: AssignOp,
        rhs: OperandRef,
    },
    Idle {
        span: Span,
        ticks: OperandRef,
    },
    If {
        span: Span,
        cond: Condition,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    While {
        span: Span,
        cond: Condition,
        body: Vec<Stmt>,
    },
    Loop {
        span: Span,
        body: Vec<Stmt>,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    Return {
        span: Span,
        target: Target,
    },
}
