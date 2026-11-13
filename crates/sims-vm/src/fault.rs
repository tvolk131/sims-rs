//! What goes wrong while a script runs, and where.

use crate::memory::scope_name;
use crate::primitives::primitive_name;
use std::fmt;

/// Why an instruction could not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trap {
    /// An error the original raises itself. `code` is its native error
    /// number, as the old prototype recovered it.
    Native { code: u16, message: &'static str },
    /// A memory scope whose state the simulation does not hold yet.
    UnsupportedScope { scope: u8 },
    /// A primitive with no implementation yet.
    Unimplemented { opcode: u16 },
    /// A call or entry names a tree ID that resolves to no BHAV.
    MissingTree { id: u16 },
    /// An instruction took the explicit error branch (253).
    ErrorBranch,
    /// A branch to a node past the end of the tree.
    InvalidBranch { target: u8, count: usize },
    /// More than [`crate::MAX_STACK`] frames.
    StackOverflow,
    /// [`crate::Interpreter::run_tree`] ran out of instructions.
    BudgetExhausted { budget: usize },
    /// Something the original does that this interpreter does not model.
    Unsupported(String),
    /// State the interpreter cannot make sense of, such as a frame whose
    /// code owner is not in the lot's type table.
    Invalid(String),
}

impl Trap {
    pub(crate) fn native(code: u16, message: &'static str) -> Self {
        Self::Native { code, message }
    }

    /// The native error code, for errors the original raises itself.
    pub fn code(&self) -> Option<u16> {
        match self {
            Self::Native { code, .. } => Some(*code),
            _ => None,
        }
    }
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native { code, message } => write!(f, "{message} (native error {code:#04x})"),
            Self::UnsupportedScope { scope } => {
                write!(f, "unsupported scope {scope} ({})", scope_name(*scope))
            }
            Self::Unimplemented { opcode } => write!(
                f,
                "unimplemented primitive {opcode} ({})",
                primitive_name(*opcode)
            ),
            Self::MissingTree { id } => write!(f, "tree {id:#06x} is missing"),
            Self::ErrorBranch => write!(f, "explicit error branch (253)"),
            Self::InvalidBranch { target, count } => {
                write!(f, "branch to node {target} in a tree of {count} nodes")
            }
            Self::StackOverflow => write!(f, "call stack limit of {} reached", crate::MAX_STACK),
            Self::BudgetExhausted { budget } => {
                write!(f, "instruction budget of {budget} exhausted")
            }
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

/// A frame position: the tree a frame runs and its current node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub tree: u16,
    pub node: u16,
}

/// A [`Trap`] with the object and frame position it happened at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub object: u16,
    /// The top frame when the trap happened, or None for an empty stack.
    /// When a tree could not be entered at all, this names that tree with
    /// node 0.
    pub at: Option<Location>,
    pub trap: Trap,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "object {}", self.object)?;
        if let Some(at) = self.at {
            write!(f, ", tree {:#06x} node {}", at.tree, at.node)?;
        }
        write!(f, ": {}", self.trap)
    }
}

impl std::error::Error for Fault {}
