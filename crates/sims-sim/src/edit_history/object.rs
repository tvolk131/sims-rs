//! ObjectUndoable virtual bodies48bce9/48bd21/48bdfe/48bc14. Subcalls are
//! explicit services; driver composes the shared dependency/snapshot callers.
use super::{List, Slot, Store, command, dependencies, snapshot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod budget;
pub mod construction;
pub mod driver;
pub mod kind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Class {
    Object,
    Base,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Object {
    pub cost: Option<i32>,
    pub budget: Option<u32>,
    pub kind: i32,
    pub special: Option<u8>,
    pub snapshots: [snapshot::Snapshot; 2],
    pub class: Class,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Objects(pub BTreeMap<u32, Object>);
impl Objects {
    pub fn get(&self, id: u32) -> Result<&Object, String> {
        self.0.get(&id).ok_or("object command absent".into())
    }
    pub fn get_mut(&mut self, id: u32) -> Result<&mut Object, String> {
        self.0.get_mut(&id).ok_or("object command absent".into())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Which {
    Initial,
    Final,
}
impl Which {
    pub fn index(self) -> usize {
        usize::from(self == Self::Final)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotMethod {
    Place,
    Restore,
    Remove,
    Destroy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Capture {
        owner: u32,
        script_id: i16,
    },
    Dependencies {
        owner: u32,
        operation: dependencies::Operation,
    },
    Snapshot {
        owner: u32,
        which: Which,
        method: SnapshotMethod,
    },
    Budget {
        owner: u32,
        amount: i32,
        special: u8,
    },
    ///Direct virtual calls on contained object commands do not run the shared
    ///Execute/Undo/Redo wrappers and do not adjust their state/refcounts.
    Virtual(command::Request),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum After {
    Execute,
    Undo,
    RedoBefore,
    RedoAfter,
    Destroy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Captured,
    StartDependency,
    UndoChoose,
    RedoChoose,
    UndoSnapshot(i32),
    RedoSnapshot(i32),
    UndoBudget,
    RedoBudget,
    Walk { cursor: Slot, after: After },
    Visited { cursor: Slot, after: After },
    RedoTail,
    FinalDependency,
    DestroyFinal,
    DestroyInitial,
    DestroyBase,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    request: command::Request,
    stage: Stage,
    pending: Option<Request>,
    reply: u32,
    result: u32,
}
impl Call {
    pub fn new(request: command::Request) -> Result<Self, String> {
        if request.command == 0 {
            return Err("null object command".into());
        }
        Ok(Self {
            request,
            stage: Stage::Entry,
            pending: None,
            reply: 0,
            result: 0,
        })
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn receiver(&self) -> command::Request {
        self.request
    }
    pub fn result(&self) -> Option<u32> {
        (self.stage == Stage::Complete && self.pending.is_none()).then_some(self.result)
    }
    pub fn acknowledge(&mut self, request: Request, value: u32) -> Result<(), String> {
        if self.pending != Some(request) {
            return Err("object command reply differs".into());
        }
        self.reply = value;
        self.pending = None;
        Ok(())
    }
    fn walk(&mut self, store: &Store, after: After) -> Result<(), String> {
        self.stage = Stage::Walk {
            cursor: store.begin(List::Objects(self.request.command))?,
            after,
        };
        Ok(())
    }
    fn snapshot(&self, which: Which, method: SnapshotMethod) -> Request {
        Request::Snapshot {
            owner: self.request.command,
            which,
            method,
        }
    }
    fn budget(&self, objects: &Objects, negate: bool) -> Result<Request, String> {
        let o = objects.get(self.request.command)?;
        let cost = o.cost.ok_or("object command cost not initialized")?;
        Ok(Request::Budget {
            owner: o.budget.ok_or("object command budget not initialized")?,
            amount: if negate { cost.wrapping_neg() } else { cost },
            special: o.special.ok_or("object command category not initialized")?,
        })
    }
    fn dependencies(&self, operation: dependencies::Operation) -> Request {
        Request::Dependencies {
            owner: self.request.command,
            operation,
        }
    }
    pub fn advance(
        &mut self,
        store: &mut Store,
        objects: &mut Objects,
        refs: &snapshot::References,
    ) -> Result<Option<Request>, String> {
        use command::Method as M;
        use dependencies::{Operation as D, Side};
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let id = self.request.command;
        let r = loop {
            match self.stage {
                Stage::Entry => {
                    store.command(id)?;
                    if objects.get(id)?.class != Class::Object {
                        return Err("object virtual call has a base vtable".into());
                    }
                    match self.request.method {
                        M::Execute => {
                            let o = objects.get(id)?;
                            let script_id = if o.kind == 3 {
                                0
                            } else {
                                refs.script_id(&o.snapshots[0])?
                            };
                            self.stage = Stage::Captured;
                            break Request::Capture {
                                owner: id,
                                script_id,
                            };
                        }
                        M::Undo => {
                            self.stage = Stage::StartDependency;
                            break self.dependencies(D::Undo(Side::Before));
                        }
                        M::Redo => {
                            self.stage = Stage::StartDependency;
                            break self.dependencies(D::Redo(Side::After));
                        }
                        M::Destroy => {
                            self.walk(store, After::Destroy)?;
                        }
                    }
                }
                Stage::Captured => self.walk(store, After::Execute)?,
                Stage::StartDependency => {
                    self.result = self.reply;
                    self.stage = if self.reply != 0 {
                        Stage::Complete
                    } else if self.request.method == M::Undo {
                        Stage::UndoChoose
                    } else {
                        Stage::RedoChoose
                    };
                }
                Stage::UndoChoose => {
                    let kind = objects.get(id)?.kind;
                    let method = match kind {
                        0 => SnapshotMethod::Remove,
                        1 | 2 => SnapshotMethod::Place,
                        3 => SnapshotMethod::Restore,
                        _ => {
                            self.result = 1;
                            self.walk(store, After::Undo)?;
                            continue;
                        }
                    };
                    self.stage = Stage::UndoSnapshot(kind);
                    break self.snapshot(Which::Initial, method);
                }
                Stage::RedoChoose => {
                    let kind = objects.get(id)?.kind;
                    match kind {
                        3 => self.walk(store, After::RedoBefore)?,
                        0 | 1 | 2 => {
                            self.stage = Stage::RedoSnapshot(kind);
                            break self.snapshot(
                                Which::Final,
                                if kind == 0 {
                                    SnapshotMethod::Restore
                                } else {
                                    SnapshotMethod::Place
                                },
                            );
                        }
                        _ => {
                            self.result = 1;
                            self.stage = Stage::RedoTail;
                        }
                    }
                }
                Stage::UndoSnapshot(kind) => {
                    self.result = u32::from(self.reply as u8 == 0);
                    if self.result == 0 && (kind == 0 || kind == 3) {
                        let r = self.budget(objects, kind == 0)?;
                        self.stage = Stage::UndoBudget;
                        break r;
                    }
                    self.walk(store, After::Undo)?;
                }
                Stage::RedoSnapshot(kind) => {
                    //Original EBX starts at0 and remains0 even when a valid
                    //snapshot call returns false. Do not "fix" this asymmetry.
                    if self.reply as u8 != 0 && (kind == 0 || kind == 3) {
                        let r = self.budget(objects, kind == 3)?;
                        self.stage = Stage::RedoBudget;
                        break r;
                    }
                    self.stage = Stage::RedoTail;
                }
                Stage::UndoBudget => self.walk(store, After::Undo)?,
                Stage::RedoBudget => self.stage = Stage::RedoTail,
                Stage::Walk { cursor, after } => {
                    if cursor == store.end(List::Objects(id))? {
                        match after {
                            After::Execute => {
                                self.result = 0;
                                self.stage = Stage::Complete;
                            }
                            After::Undo => {
                                if self.result != 0 {
                                    self.stage = Stage::Complete;
                                } else {
                                    self.stage = Stage::FinalDependency;
                                    break self.dependencies(D::Undo(Side::After));
                                }
                            }
                            After::RedoBefore => {
                                self.stage = Stage::RedoSnapshot(3);
                                break self.snapshot(Which::Initial, SnapshotMethod::Remove);
                            }
                            After::RedoAfter => {
                                if self.result != 0 {
                                    self.stage = Stage::Complete;
                                } else {
                                    self.stage = Stage::FinalDependency;
                                    break self.dependencies(D::Redo(Side::Before));
                                }
                            }
                            After::Destroy => {
                                let list = List::Objects(id);
                                store.erase(list, store.begin(list)?, store.end(list)?)?;
                                self.stage = Stage::DestroyFinal;
                                break self.snapshot(Which::Final, SnapshotMethod::Destroy);
                            }
                        }
                        continue;
                    }
                    let child = store.read(cursor)?;
                    if child == 0 {
                        if after != After::Destroy {
                            return Err("null contained command virtual receiver".into());
                        }
                        self.stage = Stage::Walk {
                            cursor: cursor.next()?,
                            after,
                        };
                        continue;
                    }
                    let method = match after {
                        After::Execute => M::Execute,
                        After::Undo => M::Undo,
                        After::RedoBefore | After::RedoAfter => M::Redo,
                        After::Destroy => M::Destroy,
                    };
                    self.stage = Stage::Visited { cursor, after };
                    break Request::Virtual(command::Request {
                        command: child,
                        method,
                    });
                }
                Stage::Visited { cursor, after } => {
                    self.stage = Stage::Walk {
                        cursor: cursor.next()?,
                        after,
                    }
                }
                Stage::RedoTail => {
                    if objects.get(id)?.kind != 3 {
                        self.walk(store, After::RedoAfter)?;
                    } else if self.result != 0 {
                        self.stage = Stage::Complete;
                    } else {
                        self.stage = Stage::FinalDependency;
                        break self.dependencies(D::Redo(Side::Before));
                    }
                }
                Stage::FinalDependency => {
                    self.result = self.reply;
                    self.stage = Stage::Complete;
                }
                Stage::DestroyFinal => {
                    self.stage = Stage::DestroyInitial;
                    break self.snapshot(Which::Initial, SnapshotMethod::Destroy);
                }
                Stage::DestroyInitial => {
                    store.destroy_vector(List::Objects(id))?;
                    objects.get_mut(id)?.class = Class::Base;
                    self.stage = Stage::DestroyBase;
                    break self.dependencies(D::Clear);
                }
                Stage::DestroyBase => {
                    store.destroy_vector(List::After(id))?;
                    store.destroy_vector(List::Before(id))?;
                    store.deallocate_command(id)?;
                    objects.0.remove(&id);
                    self.result = id;
                    self.stage = Stage::Complete;
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(r);
        Ok(self.pending)
    }
}
