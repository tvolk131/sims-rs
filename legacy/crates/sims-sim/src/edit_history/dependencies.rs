//! Undoable dependency callers:492e37 clear,492e86/492ed6 cancel,
//!492f26/492f4a add,492f6e/492f95 undo,492fbc/492fe0 redo.
//! Captured allocation cursors and command receivers survive virtual calls.
use super::{List, Slot, Store, command};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Before,
    After,
}
impl Side {
    fn list(self, owner: u32) -> List {
        match self {
            Self::Before => List::Before(owner),
            Self::After => List::After(owner),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Clear,
    Cancel(Side),
    Add { side: Side, command: u32 },
    Undo(Side),
    Redo(Side),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Return {
    Void,
    /// Cancel defines AL only; upper EAX is not a result.
    Byte(u8),
    Word(u32),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Entry,
    Walk {
        side: Side,
        cursor: Slot,
        success: u8,
    },
    Adding(command::Call),
    Visiting {
        side: Side,
        cursor: Slot,
        success: u8,
        child: command::Call,
    },
    Releasing {
        side: Side,
        cursor: Slot,
        success: u8,
        undo_result: u32,
        child: command::Call,
    },
    Complete(Return),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    owner: u32,
    operation: Operation,
    phase: Phase,
}
impl Call {
    pub fn new(owner: u32, operation: Operation) -> Result<Self, String> {
        let call = Self {
            owner,
            operation,
            phase: Phase::Entry,
        };
        call.validate()?;
        Ok(call)
    }
    pub fn owner(&self) -> u32 {
        self.owner
    }
    pub fn request(&self) -> Option<command::Request> {
        match &self.phase {
            Phase::Adding(c)
            | Phase::Visiting { child: c, .. }
            | Phase::Releasing { child: c, .. } => c.request(),
            _ => None,
        }
    }
    pub fn operation(&self) -> Operation {
        self.operation
    }
    pub fn result(&self) -> Option<Return> {
        if let Phase::Complete(r) = self.phase {
            Some(r)
        } else {
            None
        }
    }
    fn method(&self) -> command::Operation {
        match self.operation {
            Operation::Clear => command::Operation::Release,
            Operation::Cancel(_) | Operation::Undo(_) => command::Operation::Undo,
            Operation::Redo(_) => command::Operation::Redo,
            Operation::Add { .. } => command::Operation::Execute,
        }
    }
    fn backwards(&self) -> bool {
        matches!(self.operation, Operation::Cancel(_) | Operation::Undo(_))
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid_side = |side| match self.operation {
            Operation::Clear => true,
            Operation::Cancel(s) | Operation::Undo(s) | Operation::Redo(s) => side == s,
            Operation::Add { .. } => false,
        };
        let valid_child = |c: &command::Call, op| -> Result<bool, String> {
            c.validate()?;
            Ok(c.operation() == op)
        };
        let valid = match &self.phase {
            Phase::Entry => true,
            Phase::Walk { side, success, .. } => valid_side(*side) && *success <= 1,
            Phase::Adding(c) => {
                matches!(self.operation, Operation::Add { command, .. } if command == c.command())
                    && valid_child(c, command::Operation::Execute)?
            }
            Phase::Visiting {
                side,
                success,
                child,
                ..
            } => valid_side(*side) && *success <= 1 && valid_child(child, self.method())?,
            Phase::Releasing {
                side,
                success,
                child,
                ..
            } => {
                self.operation == Operation::Cancel(*side)
                    && *success <= 1
                    && valid_child(child, command::Operation::Release)?
            }
            Phase::Complete(r) => matches!(
                (self.operation, r),
                (Operation::Clear | Operation::Add { .. }, Return::Void)
                    | (Operation::Cancel(_), Return::Byte(0..=1))
                    | (Operation::Undo(_) | Operation::Redo(_), Return::Word(0..=1))
            ),
        };
        if self.owner == 0 || !valid || matches!(self.operation, Operation::Add { command: 0, .. })
        {
            Err("dependency call lost its owner, traversal or child".into())
        } else {
            Ok(())
        }
    }
    pub fn acknowledge(&mut self, request: command::Request, answer: u32) -> Result<(), String> {
        self.validate()?;
        match &mut self.phase {
            Phase::Adding(c)
            | Phase::Visiting { child: c, .. }
            | Phase::Releasing { child: c, .. } => c.acknowledge(request, answer),
            _ => Err("dependency caller has no virtual reply pending".into()),
        }
    }
    fn erase(&self, s: &mut Store, side: Side) -> Result<(), String> {
        let list = side.list(self.owner);
        s.erase(list, s.begin(list)?, s.end(list)?)
    }
    pub fn advance(&mut self, s: &mut Store) -> Result<Option<command::Request>, String> {
        self.validate()?;
        loop {
            match &mut self.phase {
                Phase::Adding(c)
                | Phase::Visiting { child: c, .. }
                | Phase::Releasing { child: c, .. } => {
                    if let Some(r) = c.advance(s)? {
                        return Ok(Some(r));
                    }
                }
                _ => (),
            }
            // Only small continuation state is copied; world/history storage is borrowed.
            match self.phase.clone() {
                Phase::Entry => {
                    s.command(self.owner)?;
                    let side = match self.operation {
                        Operation::Clear => Side::Before,
                        Operation::Cancel(side) | Operation::Undo(side) | Operation::Redo(side) => {
                            side
                        }
                        Operation::Add { side, command } => {
                            s.vector(side.list(self.owner))?;
                            let child = command::Call::new(command, command::Operation::Execute)?;
                            s.acquire(command)?;
                            self.phase = Phase::Adding(child);
                            continue;
                        }
                    };
                    let list = side.list(self.owner);
                    let cursor = if self.backwards() {
                        s.end(list)?
                    } else {
                        s.begin(list)?
                    };
                    self.phase = Phase::Walk {
                        side,
                        cursor,
                        success: 0,
                    };
                }
                Phase::Walk {
                    side,
                    cursor,
                    success,
                } => {
                    let list = side.list(self.owner);
                    let boundary = if self.backwards() {
                        s.begin(list)?
                    } else {
                        s.end(list)?
                    };
                    if cursor == boundary {
                        match self.operation {
                            Operation::Clear => {
                                self.erase(s, side)?;
                                self.phase = if side == Side::Before {
                                    Phase::Walk {
                                        side: Side::After,
                                        cursor: s.begin(List::After(self.owner))?,
                                        success: 0,
                                    }
                                } else {
                                    Phase::Complete(Return::Void)
                                };
                            }
                            Operation::Cancel(_) => {
                                self.erase(s, side)?;
                                self.phase = Phase::Complete(Return::Byte(success));
                            }
                            _ => self.phase = Phase::Complete(Return::Word(0)),
                        }
                    } else {
                        let cursor = if self.backwards() {
                            cursor.previous()?
                        } else {
                            cursor
                        };
                        let command = s.read(cursor)?;
                        // Unlike UndoManager clear, dependency clear does not skip null.
                        let child = command::Call::new(command, self.method())?;
                        self.phase = Phase::Visiting {
                            side,
                            cursor,
                            success,
                            child,
                        };
                    }
                }
                Phase::Adding(c) => {
                    let Operation::Add { side, command } = self.operation else {
                        unreachable!()
                    };
                    c.result().ok_or("dependency execute child unfinished")?;
                    // Execute failure does not prevent registration.
                    s.append(side.list(self.owner), command)?;
                    self.phase = Phase::Complete(Return::Void);
                }
                Phase::Visiting {
                    side,
                    cursor,
                    success,
                    child,
                } => {
                    let answer = child.result().ok_or("dependency child unfinished")?;
                    if matches!(self.operation, Operation::Cancel(_)) {
                        self.phase = Phase::Releasing {
                            side,
                            cursor,
                            success,
                            undo_result: answer,
                            child: command::Call::new(
                                child.command(),
                                command::Operation::Release,
                            )?,
                        };
                    } else if matches!(self.operation, Operation::Clear) {
                        self.phase = Phase::Walk {
                            side,
                            cursor: cursor.next()?,
                            success,
                        };
                    } else if answer != 0 {
                        self.phase = Phase::Complete(Return::Word(1));
                    } else {
                        self.phase = Phase::Walk {
                            side,
                            cursor: if self.backwards() {
                                cursor
                            } else {
                                cursor.next()?
                            },
                            success,
                        };
                    }
                }
                Phase::Releasing {
                    side,
                    cursor,
                    success,
                    undo_result,
                    child,
                } => {
                    child
                        .result()
                        .ok_or("dependency release child unfinished")?;
                    if undo_result != 0 {
                        // Native erases the entire fresh list even if unvisited
                        // commands remain. Do not release those as an improvement.
                        self.erase(s, side)?;
                        self.phase = Phase::Complete(Return::Byte(success));
                    } else {
                        self.phase = Phase::Walk {
                            side,
                            cursor,
                            success: 1,
                        };
                    }
                }
                Phase::Complete(_) => return Ok(None),
            }
        }
    }
}
