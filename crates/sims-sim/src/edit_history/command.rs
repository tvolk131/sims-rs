//!493004 execute,493022 undo,49304b redo,485a19 release. Virtual methods
//! operate on the captured command identity and may mutate the shared store.
use super::Store;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Execute,
    Undo,
    Redo,
    Release,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    Execute,
    Undo,
    Redo,
    Destroy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub command: u32,
    pub method: Method,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Executed,
    Undone,
    Redone,
    Destroyed,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    command: u32,
    operation: Operation,
    stage: Stage,
    result: u32,
    pending: Option<Request>,
}
impl Call {
    pub fn command(&self) -> u32 {
        self.command
    }
    pub fn operation(&self) -> Operation {
        self.operation
    }
    pub fn new(command: u32, operation: Operation) -> Result<Self, String> {
        if command == 0 {
            return Err("null command receiver".into());
        }
        Ok(Self {
            command,
            operation,
            stage: Stage::Entry,
            result: 0,
            pending: None,
        })
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn result(&self) -> Option<u32> {
        (self.stage == Stage::Complete && self.pending.is_none()).then_some(self.result)
    }
    pub fn validate(&self) -> Result<(), String> {
        use Stage as S;
        let valid = match (self.operation, self.stage) {
            (_, S::Entry | S::Complete) => true,
            (Operation::Execute | Operation::Undo, S::Executed) => true,
            (Operation::Undo, S::Undone)
            | (Operation::Redo, S::Redone)
            | (Operation::Release, S::Destroyed) => true,
            _ => false,
        };
        let pending = self.pending.is_none_or(|r| {
            r.command == self.command
                && matches!(
                    (r.method, self.stage),
                    (Method::Execute, S::Executed)
                        | (Method::Undo, S::Undone)
                        | (Method::Redo, S::Redone)
                        | (Method::Destroy, S::Destroyed)
                )
        });
        if self.command != 0 && valid && pending {
            Ok(())
        } else {
            Err("command call lost its virtual method".into())
        }
    }
    pub fn acknowledge(&mut self, request: Request, result: u32) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("command reply differs".into());
        }
        self.result = result;
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, store: &mut Store) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let method = loop {
            match self.stage {
                Stage::Entry => match self.operation {
                    Operation::Execute => {
                        store.command(self.command)?;
                        self.stage = Stage::Executed;
                        break Method::Execute;
                    }
                    Operation::Undo => {
                        if store.command(self.command)?.state == 0 {
                            self.stage = Stage::Executed;
                            break Method::Execute;
                        }
                        self.stage = Stage::Undone;
                        break Method::Undo;
                    }
                    Operation::Redo => {
                        store.command(self.command)?;
                        self.stage = Stage::Redone;
                        break Method::Redo;
                    }
                    Operation::Release => {
                        let c = store.command_mut(self.command)?;
                        c.references = c.references.wrapping_sub(1);
                        self.result = c.references;
                        if self.result != 0 {
                            self.stage = Stage::Complete;
                            continue;
                        }
                        self.stage = Stage::Destroyed;
                        break Method::Destroy;
                    }
                },
                Stage::Executed | Stage::Redone => {
                    store.command_mut(self.command)?.state = if self.result == 0 { 1 } else { 3 };
                    if self.operation == Operation::Undo && self.result == 0 {
                        self.stage = Stage::Undone;
                        break Method::Undo;
                    }
                    self.stage = Stage::Complete;
                }
                Stage::Undone => {
                    store.command_mut(self.command)?.state = 2 + u32::from(self.result != 0);
                    self.stage = Stage::Complete;
                }
                Stage::Destroyed => {
                    // The original does not read the command after destruction.
                    self.result = 0;
                    self.stage = Stage::Complete;
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(Request {
            command: self.command,
            method,
        });
        Ok(self.pending)
    }
}
