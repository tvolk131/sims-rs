//! Registry selection (445449/4456e4) and low object flags (4320da).
//!
//! These continuations preserve synchronous callback order across host budgets.
//! Object handles must retain their native pointer identity until completion;
//! they are distinct from the script ID reread for the final E4 notification.
//! The live adapter must supply native +114 storage, invalidation and indicator
//! services. In particular, acknowledging `Indicators` without attachment work
//! does not implement native selection. No selection policy is inferred here.
use crate::ObjectId;
use serde::{Deserialize, Serialize};

pub const SIGNAL: u32 = 0xe4;

/// Borrowed view of live registry data, not a cached selected-person ID.
pub trait View {
    fn person_order(&self) -> &[ObjectId];
    fn flags(&self, object: ObjectId) -> Result<u32, String>;
    fn script_id(&self, object: ObjectId) -> Result<i16, String>;
}

/// Native 4456e4 stops at the first flag match, including an inconsistent
/// registry containing several selected people. It does not repair the flags.
pub fn selected(view: &impl View) -> Result<Option<ObjectId>, String> {
    for &object in view.person_order() {
        if view.flags(object)? & 2 != 0 {
            return Ok(Some(object));
        }
    }
    Ok(None)
}

///445501's ordinary (process6e4f64 == 0) family cycle. Missing live instances
///are skipped; a selected person outside the family returns without selecting.
pub fn cycle_family(
    selected: Option<ObjectId>,
    members: &[u32],
    mut lookup: impl FnMut(u32) -> Option<ObjectId>,
) -> Option<ObjectId> {
    if members.is_empty() {
        return None;
    }
    let start = match selected {
        None => None,
        Some(id) => Some(members.iter().position(|guid| lookup(*guid) == Some(id))?),
    };
    for offset in 1..=members.len() {
        let index = start.map_or(offset - 1, |i| (i + offset) % members.len());
        if let Some(id) = lookup(members[index]) {
            return Some(id);
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Complete 4320da, including each invalidation it causes, before ack.
    Flags { object: ObjectId, low: u32 },
    /// 434101(object, 0), even when Flags already invalidated the same object.
    Invalidate(ObjectId),
    /// Registry 44664c. This may observe or change live selection-related data.
    Indicators(ObjectId),
    /// Broadcast E4. Payload is a freshly read, sign-extended script ID or zero.
    Notify(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    OldFlags,
    OldInvalidate,
    OldIndicators,
    NewFlags,
    NewInvalidate,
    NewIndicators,
    Notify,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    target: Option<ObjectId>,
    previous: Option<ObjectId>,
    stage: Stage,
    pending: Option<Request>,
}

impl Selection {
    pub fn new(target: Option<ObjectId>) -> Self {
        Self {
            target,
            previous: None,
            stage: Stage::Start,
            pending: None,
        }
    }

    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn target(&self) -> Option<ObjectId> {
        self.target
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.target == Some(0)
            || self.previous == Some(0)
            || self.stage == Stage::Start && (self.previous.is_some() || self.pending.is_some())
            || matches!(
                self.stage,
                Stage::OldFlags | Stage::OldInvalidate | Stage::OldIndicators
            ) && self.previous.is_none()
            || matches!(self.stage, Stage::NewInvalidate | Stage::NewIndicators)
                && self.target.is_none()
            || !matches!(self.stage, Stage::Start | Stage::Complete) && self.previous == self.target
            || self
                .pending
                .is_some_and(|request| match (self.stage, request) {
                    (Stage::OldFlags, Request::Flags { object, low }) => {
                        Some(object) != self.previous || low & !0x1d != 0
                    }
                    (Stage::NewFlags, Request::Flags { object, low }) => {
                        Some(object) != self.target || low & !0x1f != 0 || low & 2 == 0
                    }
                    (Stage::OldInvalidate, Request::Invalidate(object))
                    | (Stage::OldIndicators, Request::Indicators(object)) => {
                        Some(object) != self.previous
                    }
                    (Stage::NewInvalidate, Request::Invalidate(object))
                    | (Stage::NewIndicators, Request::Indicators(object)) => {
                        Some(object) != self.target
                    }
                    (Stage::Notify, Request::Notify(payload)) => {
                        i16::try_from(payload).is_err() || self.target.is_none() && payload != 0
                    }
                    _ => true,
                })
        {
            return Err("invalid native selection continuation".into());
        }
        Ok(())
    }

    pub fn advance(&mut self, view: &impl View) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            let request = match self.stage {
                Stage::Start => {
                    self.previous = selected(view)?;
                    self.stage = if self.previous == self.target {
                        Stage::Complete
                    } else if self.previous.is_some() {
                        Stage::OldFlags
                    } else {
                        Stage::NewFlags
                    };
                    continue;
                }
                Stage::OldFlags => {
                    let object = self.previous.unwrap();
                    Request::Flags {
                        object,
                        low: view.flags(object)? & 0x1d,
                    }
                }
                Stage::OldInvalidate => Request::Invalidate(self.previous.unwrap()),
                Stage::OldIndicators => Request::Indicators(self.previous.unwrap()),
                Stage::NewFlags => {
                    let Some(object) = self.target else {
                        self.stage = Stage::Notify;
                        continue;
                    };
                    Request::Flags {
                        object,
                        low: (view.flags(object)? & 0x1f) | 2,
                    }
                }
                Stage::NewInvalidate => Request::Invalidate(self.target.unwrap()),
                Stage::NewIndicators => Request::Indicators(self.target.unwrap()),
                Stage::Notify => Request::Notify(match self.target {
                    Some(object) => i32::from(view.script_id(object)?),
                    None => 0,
                }),
                Stage::Complete => return Ok(None),
            };
            self.pending = Some(request);
            return Ok(self.pending);
        }
    }

    /// A host error leaves the request pending. Acknowledgment is only valid
    /// after all effects of that synchronous service have completed.
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("stale native selection acknowledgment".into());
        }
        self.stage = match self.stage {
            Stage::OldFlags => Stage::OldInvalidate,
            Stage::OldInvalidate => Stage::OldIndicators,
            Stage::OldIndicators => Stage::NewFlags,
            Stage::NewFlags => Stage::NewInvalidate,
            Stage::NewInvalidate => Stage::NewIndicators,
            Stage::NewIndicators => Stage::Notify,
            Stage::Notify => Stage::Complete,
            _ => return Err("native selection has no pending service".into()),
        };
        self.pending = None;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagScope {
    /// Ordinary OBJD, or multipart with native 42344e returning true.
    Single,
    /// Multipart +190. A null head starts at the requested object itself.
    Group(Option<ObjectId>),
}

pub trait Flags {
    fn scope(&self, object: ObjectId) -> Result<FlagScope, String>;
    fn flags(&self, object: ObjectId) -> Result<u32, String>;
    fn write_flags(&mut self, object: ObjectId, value: u32) -> Result<(), String>;
    fn next(&self, object: ObjectId) -> Result<Option<ObjectId>, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum FlagStage {
    Start,
    Visit,
    After,
    Complete,
}

/// Low-five-bit setter. It reads the next multipart link *after* the current
/// member's invalidation callback, and reads each member's flags only on visit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagUpdate {
    object: ObjectId,
    low: u32,
    stage: FlagStage,
    group: bool,
    cursor: Option<ObjectId>,
    visited: Vec<ObjectId>,
    pending: Option<ObjectId>,
}

impl FlagUpdate {
    pub fn new(object: ObjectId, requested: u32) -> Self {
        Self {
            object,
            low: requested & 0x1f,
            stage: FlagStage::Start,
            group: false,
            cursor: None,
            visited: vec![],
            pending: None,
        }
    }

    pub fn complete(&self) -> bool {
        self.stage == FlagStage::Complete
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.object == 0
            || self.low & !0x1f != 0
            || self.cursor == Some(0)
            || self.visited.contains(&0)
            || self
                .visited
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.visited.len()
            || self.stage == FlagStage::Start
                && (self.group || self.cursor.is_some() || !self.visited.is_empty())
            || self.stage == FlagStage::After
                && (self.cursor.is_none() || self.visited.last().copied() != self.cursor)
            || !self.group
                && self.stage != FlagStage::Start
                && (self.cursor != Some(self.object)
                    || if self.stage == FlagStage::Visit {
                        !self.visited.is_empty()
                    } else {
                        self.visited.as_slice() != [self.object]
                    })
            || self.pending.is_some()
                && (self.stage != FlagStage::After || self.pending != self.cursor)
        {
            return Err("invalid native low-flag continuation".into());
        }
        Ok(())
    }

    /// Return the object requiring 434101(object, 0). No further member writes
    /// occur until that invalidation has completed and been acknowledged.
    pub fn advance(&mut self, host: &mut impl Flags) -> Result<Option<ObjectId>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            match self.stage {
                FlagStage::Start => {
                    let (group, first) = match host.scope(self.object)? {
                        FlagScope::Single => (false, self.object),
                        FlagScope::Group(head) => (true, head.unwrap_or(self.object)),
                    };
                    if first == 0 {
                        return Err("null native low-flag group head".into());
                    }
                    self.group = group;
                    self.cursor = Some(first);
                    self.stage = FlagStage::Visit;
                }
                FlagStage::Visit => {
                    let Some(object) = self.cursor else {
                        self.stage = FlagStage::Complete;
                        continue;
                    };
                    if self.visited.contains(&object) {
                        return Err("cyclic native low-flag multipart chain".into());
                    }
                    let before = host.flags(object)?;
                    if before & 0x1f != self.low {
                        host.write_flags(object, (before & !0x1f) | self.low)?;
                        self.pending = Some(object);
                    }
                    self.visited.push(object);
                    self.stage = FlagStage::After;
                    if self.pending.is_some() {
                        return Ok(self.pending);
                    }
                }
                FlagStage::After => {
                    if self.group {
                        let next = host.next(self.cursor.unwrap())?;
                        if next == Some(0) {
                            return Err("null native low-flag next handle".into());
                        }
                        self.cursor = next;
                        self.stage = FlagStage::Visit;
                    } else {
                        self.stage = FlagStage::Complete;
                    }
                }
                FlagStage::Complete => return Ok(None),
            }
        }
    }

    pub fn acknowledge(&mut self, object: ObjectId) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(object) {
            return Err("stale native low-flag acknowledgment".into());
        }
        self.pending = None;
        Ok(())
    }
}
