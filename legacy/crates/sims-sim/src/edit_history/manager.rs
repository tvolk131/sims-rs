//!493259 append,4932fa undo,493338 redo,493376/4933a9 clear. Concrete
//! command wrappers and reference releases compose here; only command virtual
//! bodies and the real EE broadcast remain external services.
use super::{List, Slot, Store, command};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Append { manager: u32, command: u32 },
    Undo(u32),
    Redo(u32),
    ClearUndo(u32),
    ClearRedo(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    Virtual(command::Request),
    Notify,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Request {
    Command(u32, command::Operation),
    Clear(List),
    Notify,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Executed,
    Prepared,
    Failed,
    Transfer,
    Notify,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Manager {
    operation: Operation,
    stage: Stage,
    answer: u32,
    pending: Option<Request>,
}
impl Manager {
    fn new(operation: Operation) -> Self {
        Self {
            operation,
            stage: Stage::Entry,
            answer: 0,
            pending: None,
        }
    }
    fn validate(&self) -> Result<(), String> {
        let id = match self.operation {
            Operation::Append { manager, .. }
            | Operation::Undo(manager)
            | Operation::Redo(manager)
            | Operation::ClearUndo(manager)
            | Operation::ClearRedo(manager) => manager,
        };
        let valid_stage = match self.operation {
            Operation::Append { command, .. } => {
                command != 0
                    && matches!(
                        self.stage,
                        Stage::Entry
                            | Stage::Executed
                            | Stage::Prepared
                            | Stage::Failed
                            | Stage::Notify
                            | Stage::Complete
                    )
            }
            Operation::Undo(_) | Operation::Redo(_) => matches!(
                self.stage,
                Stage::Entry | Stage::Transfer | Stage::Notify | Stage::Complete
            ),
            Operation::ClearUndo(_) | Operation::ClearRedo(_) => {
                matches!(self.stage, Stage::Entry | Stage::Complete)
            }
        };
        let pending = match (self.operation, self.stage, self.pending) {
            (_, _, None) => true,
            (
                Operation::Append { command, .. },
                Stage::Executed,
                Some(Request::Command(c, command::Operation::Execute)),
            ) => command == c,
            (
                Operation::Append { manager, .. },
                Stage::Failed,
                Some(Request::Clear(List::Undo(m))),
            ) => manager == m,
            (
                Operation::Append { manager, .. },
                Stage::Notify | Stage::Complete,
                Some(Request::Clear(List::Redo(m))),
            ) => manager == m,
            (
                Operation::Undo(_),
                Stage::Transfer,
                Some(Request::Command(c, command::Operation::Undo)),
            )
            | (
                Operation::Redo(_),
                Stage::Transfer,
                Some(Request::Command(c, command::Operation::Redo)),
            ) => c != 0,
            (Operation::Undo(m), Stage::Notify, Some(Request::Clear(List::Undo(id))))
            | (Operation::Redo(m), Stage::Notify, Some(Request::Clear(List::Redo(id)))) => m == id,
            (Operation::ClearUndo(m), Stage::Complete, Some(Request::Clear(List::Undo(id))))
            | (Operation::ClearRedo(m), Stage::Complete, Some(Request::Clear(List::Redo(id)))) => {
                m == id
            }
            (_, Stage::Complete, Some(Request::Notify)) => matches!(
                self.operation,
                Operation::Append { .. } | Operation::Undo(_) | Operation::Redo(_)
            ),
            _ => false,
        };
        if id != 0 && valid_stage && pending {
            Ok(())
        } else {
            Err("history manager lost its child".into())
        }
    }
    fn advance(&mut self, s: &mut Store) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let request = loop {
            match self.stage {
                Stage::Entry => match self.operation {
                    Operation::Append { manager, command } => {
                        s.vector(List::Undo(manager))?;
                        s.vector(List::Redo(manager))?;
                        self.stage = Stage::Prepared;
                        if s.command(command)?.state != 1 {
                            self.stage = Stage::Executed;
                            break Request::Command(command, command::Operation::Execute);
                        }
                    }
                    Operation::Undo(manager) | Operation::Redo(manager) => {
                        let (list, op) = if matches!(self.operation, Operation::Undo(_)) {
                            (List::Undo(manager), command::Operation::Undo)
                        } else {
                            (List::Redo(manager), command::Operation::Redo)
                        };
                        let command = s.read(s.end(list)?.previous()?)?;
                        if command == 0 {
                            return Err("history top command is null".into());
                        }
                        self.stage = Stage::Transfer;
                        break Request::Command(command, op);
                    }
                    Operation::ClearUndo(m) => {
                        self.stage = Stage::Complete;
                        break Request::Clear(List::Undo(m));
                    }
                    Operation::ClearRedo(m) => {
                        self.stage = Stage::Complete;
                        break Request::Clear(List::Redo(m));
                    }
                },
                Stage::Executed => {
                    let Operation::Append { manager, .. } = self.operation else {
                        unreachable!()
                    };
                    self.stage = Stage::Prepared;
                    if self.answer != 0 {
                        self.stage = Stage::Failed;
                        break Request::Clear(List::Undo(manager));
                    }
                }
                Stage::Prepared => {
                    let Operation::Append { manager, command } = self.operation else {
                        unreachable!()
                    };
                    //493259's CMP3fffffff follows SAR(pointer_difference,2).
                    //No signed32-bit SAR2 result can equal3fffffff. Its eviction
                    //branch is unreachable; valid owned vectors obey this range.
                    self.stage = Stage::Notify;
                    if s.command(command)?.state == 1 {
                        s.acquire(command)?;
                        s.append(List::Undo(manager), command)?;
                        break Request::Clear(List::Redo(manager));
                    }
                }
                Stage::Failed => {
                    let Operation::Append { manager, .. } = self.operation else {
                        unreachable!()
                    };
                    self.stage = Stage::Complete;
                    break Request::Clear(List::Redo(manager));
                }
                Stage::Transfer => {
                    let (source, target) = match self.operation {
                        Operation::Undo(m) => (List::Undo(m), List::Redo(m)),
                        Operation::Redo(m) => (List::Redo(m), List::Undo(m)),
                        _ => unreachable!(),
                    };
                    self.stage = Stage::Notify;
                    if self.answer != 0 {
                        break Request::Clear(source);
                    }
                    // Read the post-callback top, not the captured command passed
                    // to undo/redo. Moving it does not change its reference count.
                    let current = s.read(s.end(source)?.previous()?)?;
                    s.append(target, current)?;
                    s.pop_end(source)?;
                }
                Stage::Notify => {
                    self.stage = Stage::Complete;
                    break Request::Notify;
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        Ok(self.pending)
    }
    fn acknowledge(&mut self, r: Request, answer: u32) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(r) {
            return Err("history manager reply differs".into());
        }
        self.answer = answer;
        self.pending = None;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ClearStage {
    Entry,
    Walk,
    Released,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Clear {
    list: List,
    stage: ClearStage,
    cursor: Slot,
    pending: Option<u32>,
}
impl Clear {
    fn new(list: List) -> Self {
        Self {
            list,
            stage: ClearStage::Entry,
            cursor: Slot::default(),
            pending: None,
        }
    }
    fn validate(&self) -> Result<(), String> {
        if !matches!(self.list, List::Undo(_) | List::Redo(_))
            || self
                .pending
                .is_some_and(|id| id == 0 || self.stage != ClearStage::Released)
        {
            Err("history clear lost its release".into())
        } else {
            Ok(())
        }
    }
    fn advance(&mut self, s: &mut Store) -> Result<Option<u32>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            match self.stage {
                ClearStage::Entry => {
                    self.cursor = s.begin(self.list)?;
                    self.stage = ClearStage::Walk;
                }
                ClearStage::Walk => {
                    if self.cursor == s.end(self.list)? {
                        s.erase(self.list, s.begin(self.list)?, s.end(self.list)?)?;
                        self.stage = ClearStage::Complete;
                        continue;
                    }
                    let id = s.read(self.cursor)?;
                    if id == 0 {
                        self.cursor = self.cursor.next()?;
                        continue;
                    }
                    self.stage = ClearStage::Released;
                    self.pending = Some(id);
                    return Ok(self.pending);
                }
                ClearStage::Released => {
                    s.write(self.cursor, 0)?;
                    self.cursor = self.cursor.next()?;
                    self.stage = ClearStage::Walk;
                }
                ClearStage::Complete => return Ok(None),
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Frame {
    Manager(Manager),
    Clear(Clear),
    Command(command::Call),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    operation: Operation,
    stack: Vec<Frame>,
}
impl Call {
    pub fn new(operation: Operation) -> Self {
        Self {
            operation,
            stack: vec![Frame::Manager(Manager::new(operation))],
        }
    }
    pub fn complete(&self) -> bool {
        self.stack.is_empty()
    }
    pub fn operation(&self) -> Operation {
        self.operation
    }
    pub fn service(&self) -> Option<Service> {
        match self.stack.last()? {
            Frame::Command(c) => c.request().map(Service::Virtual),
            Frame::Manager(c) if c.pending == Some(Request::Notify) => Some(Service::Notify),
            _ => None,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        Manager::new(self.operation).validate()?;
        if let Some(frame) = self.stack.first() {
            if !matches!(frame, Frame::Manager(m) if m.operation == self.operation) {
                return Err("history root operation differs".into());
            }
        }
        if self.stack.len() > 3 {
            return Err("history dispatch exceeds call depth".into());
        }
        for (n, f) in self.stack.iter().enumerate() {
            match f {
                Frame::Manager(c) => c.validate()?,
                Frame::Clear(c) => c.validate()?,
                Frame::Command(c) => c.validate()?,
            }
            let has_child = n + 1 < self.stack.len();
            let needs_child = match f {
                Frame::Manager(c) => {
                    matches!(c.pending, Some(Request::Command(..) | Request::Clear(_)))
                }
                Frame::Clear(c) => c.pending.is_some(),
                Frame::Command(_) => false,
            };
            if has_child != needs_child {
                return Err("history dispatch lost its concrete child".into());
            }
            if n == 0 {
                continue;
            }
            let valid = match (&self.stack[n - 1], f) {
                (Frame::Manager(c), Frame::Clear(child)) => {
                    c.pending == Some(Request::Clear(child.list))
                }
                (Frame::Manager(c), Frame::Command(child)) => {
                    c.pending == Some(Request::Command(child.command(), child.operation()))
                }
                (Frame::Clear(c), Frame::Command(child)) => {
                    c.pending == Some(child.command())
                        && child.operation() == command::Operation::Release
                }
                _ => false,
            };
            if !valid {
                return Err("history child identity differs from parent".into());
            }
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, service: Service, answer: u32) -> Result<(), String> {
        self.validate()?;
        match (self.stack.last_mut(), service) {
            (Some(Frame::Command(c)), Service::Virtual(r)) => c.acknowledge(r, answer),
            (Some(Frame::Manager(c)), Service::Notify) => c.acknowledge(Request::Notify, answer),
            _ => Err("history reply does not own current service".into()),
        }
    }
    pub fn advance(&mut self, s: &mut Store) -> Result<Option<Service>, String> {
        self.validate()?;
        s.validate()?;
        loop {
            let Some(frame) = self.stack.last_mut() else {
                return Ok(None);
            };
            let answer = match frame {
                Frame::Manager(c) => match c.advance(s)? {
                    Some(Request::Command(id, op)) => {
                        self.stack.push(Frame::Command(command::Call::new(id, op)?));
                        continue;
                    }
                    Some(Request::Clear(list)) => {
                        self.stack.push(Frame::Clear(Clear::new(list)));
                        continue;
                    }
                    Some(Request::Notify) => return Ok(Some(Service::Notify)),
                    None => 0,
                },
                Frame::Clear(c) => {
                    if let Some(id) = c.advance(s)? {
                        self.stack.push(Frame::Command(command::Call::new(
                            id,
                            command::Operation::Release,
                        )?));
                        continue;
                    }
                    0
                }
                Frame::Command(c) => {
                    if let Some(r) = c.advance(s)? {
                        return Ok(Some(Service::Virtual(r)));
                    }
                    c.result().ok_or("command child has no result")?
                }
            };
            self.stack.pop();
            match self.stack.last_mut() {
                Some(Frame::Manager(c)) => {
                    c.acknowledge(c.pending.ok_or("history parent request absent")?, answer)?
                }
                Some(Frame::Clear(c)) => c.pending = None,
                Some(Frame::Command(_)) => {
                    return Err("command unexpectedly owns a history frame".into());
                }
                None => return Ok(None),
            }
        }
    }
}
