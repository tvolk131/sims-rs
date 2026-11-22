//! Abstract syntax tree for the two-tier `.antics` language.

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
}

/// A `BCON` integer constant table declaration in `.antics`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BconDecl {
    pub span: Span,
    pub id: u16,
    pub label: String,
    pub flags: u8,
    pub values: Vec<i16>,
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

/// Memory operand (scope + 16-bit index) usable in expressions and primitives.
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

/// High-level boolean condition usable in `if` and `while`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    Compare {
        span: Span,
        lhs: Operand,
        op: CompareOp,
        rhs: Operand,
    },
    Call {
        span: Span,
        target: CallTarget,
        args: Vec<i16>,
    },
    Primitive {
        span: Span,
        prim: PrimitiveCall,
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
        lhs: Operand,
        operator: u8,
        rhs: Operand,
    },
    Random {
        dst: Operand,
        range: Operand,
    },
    FindBestObject {
        function: u16,
        flags: u8,
    },
    IdleForInput {
        param_index: i16,
        allow_push: u16,
    },
    PlaySound {
        sound_id: u16,
        use_stack_object: bool,
    },
    StopAllSounds {
        use_stack_object: bool,
    },
    SetMotiveChange {
        motive: u8,
        delta_per_hour: Operand,
        stop_at: Operand,
    },
    ClearMotiveChanges,
    SetToNext {
        mode: u8,
        guid: u32,
    },
    Dialog {
        kind: u8,
        text_id: u16,
        flags: u8,
    },
    Animate {
        clip_id: u16,
        table: u8,
        flags: u8,
        events: i8,
        local_index: u8,
    },
    RouteTo {
        slot: u16,
        flags: u8,
    },
    Snap {
        slot: u16,
        flags: u8,
    },
}

/// Statements inside a `tree` or `interaction` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Label {
        span: Span,
        name: String,
    },
    RawOp {
        span: Span,
        opcode: u16,
        operands: [u8; 8],
        branches: Branches,
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
        lhs: Operand,
        op: AssignOp,
        rhs: Operand,
    },
    Idle {
        span: Span,
        ticks: Operand,
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
    Return {
        span: Span,
        target: Target,
    },
}
