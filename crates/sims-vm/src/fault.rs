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
    /// A check tree failed: an entry's check, run while a pie menu is
    /// listed, an advertisement weighed or a queued action checked before
    /// it starts, or the check Find Best Object for Function runs
    /// (`routine` names it then). The fault keeps its
    /// own trap, so a check that failed is told apart from one that met
    /// something not ported ([`Trap::unported`]).
    CheckTree {
        routine: Option<&'static str>,
        tree: u16,
        object: u16,
        fault: Box<Fault>,
    },
    /// Work an instruction started, such as another object's callback or
    /// the Init of an object it made, failed on a check tree's fault,
    /// which `fault` holds whole rather than as unsupported text.
    /// `context` says what the work was, when there is more to say than
    /// where `fault` is.
    Within { context: String, fault: Box<Fault> },
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

    /// Whether this is something the simulation has not ported (a scope,
    /// a primitive, or anything else it reports as unsupported), rather
    /// than a failure of the script or a wrong caller. A check tree's
    /// fault is what its own trap is, wherever it is held.
    pub fn unported(&self) -> bool {
        match self {
            Self::UnsupportedScope { .. } | Self::Unimplemented { .. } | Self::Unsupported(_) => {
                true
            }
            Self::CheckTree { fault, .. } | Self::Within { fault, .. } => fault.trap.unported(),
            _ => false,
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
            Self::CheckTree {
                routine,
                tree,
                object,
                fault,
            } => {
                if let Some(routine) = routine {
                    write!(f, "{routine}: ")?;
                }
                write!(f, "check tree {tree:#06x} of object {object}: {fault}")
            }
            Self::Within { context, fault } if context.is_empty() => write!(f, "{fault}"),
            Self::Within { context, fault } => write!(f, "{context}: {fault}"),
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

impl Fault {
    /// Whether the fault is a check tree's, held as it is
    /// ([`Trap::CheckTree`], [`Trap::Within`]).
    pub(crate) fn holds_check(&self) -> bool {
        matches!(self.trap, Trap::CheckTree { .. } | Trap::Within { .. })
    }

    /// The trap for work that failed on this fault, which the work reports
    /// as unsupported, "`context`: fault". A check tree's fault is kept
    /// whole instead ([`Trap::Within`], with the same text), so that
    /// whether the check met something not ported can still be told
    /// ([`Trap::unported`]).
    pub(crate) fn unsupported_in(self, context: impl Into<String>) -> Trap {
        let context = context.into();
        if self.holds_check() {
            Trap::Within {
                context,
                fault: Box::new(self),
            }
        } else {
            Trap::Unsupported(format!("{context}: {self}"))
        }
    }

    /// The trap an instruction reports for a fault in work it started,
    /// such as another object's callback: the trap itself when the fault
    /// names only `me` and no frame, and otherwise one that keeps the
    /// object, tree and node the fault happened at. An unsupported trap
    /// stays unsupported, and a check tree's fault is kept whole
    /// ([`Trap::Within`]); any other becomes invalid state with the whole
    /// fault as its message.
    pub(crate) fn nested(self, me: u16) -> Trap {
        if self.object == me && self.at.is_none() {
            return self.trap;
        }
        if self.holds_check() {
            return Trap::Within {
                context: String::new(),
                fault: Box::new(self),
            };
        }
        let mut place = format!("object {}", self.object);
        if let Some(at) = self.at {
            place.push_str(&format!(", tree {:#06x} node {}", at.tree, at.node));
        }
        match self.trap {
            Trap::Unsupported(message) => Trap::Unsupported(format!("{message} (in {place})")),
            trap => Trap::Invalid(format!("{place}: {trap}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(trap: Trap) -> Fault {
        Fault {
            object: 3,
            at: Some(Location {
                tree: 0x1002,
                node: 4,
            }),
            trap: Trap::CheckTree {
                routine: None,
                tree: 0x1012,
                object: 7,
                fault: Box::new(Fault {
                    object: 3,
                    at: Some(Location {
                        tree: 0x1012,
                        node: 1,
                    }),
                    trap,
                }),
            },
        }
    }

    #[test]
    fn work_that_meets_a_check_trees_fault_keeps_it_whole() {
        // A check that failed stays failed through the work, and one that
        // met something not ported stays unported.
        let failed = check(Trap::ErrorBranch).unsupported_in("creating an object");
        assert!(matches!(&failed, Trap::Within { .. }));
        assert!(!failed.unported());
        let unported = check(Trap::Unsupported("x".into())).unsupported_in("creating an object");
        assert!(unported.unported());
        // The text is what the unsupported trap's was.
        let fault = check(Trap::ErrorBranch);
        let text = format!("creating an object: {fault}");
        assert_eq!(fault.unsupported_in("creating an object").to_string(), text);
        // Any other fault is turned into text as before.
        let other = Fault {
            object: 3,
            at: None,
            trap: Trap::ErrorBranch,
        };
        let text = format!("creating an object: {other}");
        assert_eq!(
            other.unsupported_in("creating an object"),
            Trap::Unsupported(text)
        );
        // In another object's work it keeps where it happened, and a
        // second wrapping keeps it too.
        let nested = check(Trap::ErrorBranch).nested(5);
        assert_eq!(nested.to_string(), check(Trap::ErrorBranch).to_string());
        assert!(!nested.unported());
        let twice = Fault {
            object: 5,
            at: None,
            trap: nested,
        }
        .unsupported_in("Grab");
        assert!(matches!(&twice, Trap::Within { .. }) && !twice.unported());
    }
}
