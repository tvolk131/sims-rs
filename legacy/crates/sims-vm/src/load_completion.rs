//!428041 after43fc94. Callees remain synchronous, saved boundaries; scans
//! restart from the live person vector after each removal.
use crate::ObjectId;
use serde::{Deserialize, Serialize};

pub const CONTROLLER: u32 = 0xc3249a1d;
pub trait View {
    fn people(&self) -> &[ObjectId];
    fn resource_flags(&self, id: ObjectId) -> Result<u32, String>;
    fn person(&self, id: ObjectId, field: i16) -> Result<i16, String>;
    fn manager_lot(&self) -> i32;
    fn controller_suppressed(&self) -> u32;
    fn mode(&self) -> i16;
    fn controller_gate(&self) -> Result<i16, String>;
    fn family_id(&self) -> i32;
    fn family_flags(&self) -> Result<u32, String>;
    fn write_family_flags(&mut self, value: u32) -> Result<(), String>;
    fn family_home(&self, family: i32) -> Result<Option<i16>, String>;
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub lot_argument: i32,
    /// Captured before43fc94, not reread from family flags after callbacks.
    pub reset_requested: u8,
    pub imported: u8,
    /// People whose visitor word32 was zero before the ordered visits.
    pub residents_before_visits: Vec<ObjectId>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Delete(ObjectId),
    FindController,
    LookupController,
    CreateController,
    ResetBudget,
    ResetStatistics,
    ResetGlobals,
    ResetWorld,
    ResetLot,
    RefreshVisitors,
    ImportLot,
    Environment,
    HouseOpen,
    ReleaseSnapshot,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Done,
    Found(bool),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    FirstScan,
    Controller,
    FindController,
    LookupController,
    CreateController,
    Reset,
    ResetBudget,
    ResetStatistics,
    ResetGlobals,
    ResetWorld,
    ResetLot,
    RefreshVisitors,
    SecondScan,
    Import,
    ImportLot,
    Environment,
    HouseOpen,
    ReleaseSnapshot,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    inputs: Inputs,
    stage: Stage,
    pending: Option<Request>,
}
impl Completion {
    pub fn new(inputs: Inputs) -> Result<Self, String> {
        let value = Self {
            inputs,
            stage: Stage::FirstScan,
            pending: None,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn validate(&self) -> Result<(), String> {
        if self
            .inputs
            .residents_before_visits
            .iter()
            .any(|id| *id <= 0)
        {
            return Err("invalid pre-visit person snapshot".into());
        }
        if let Some(r) = self.pending {
            let valid = match (self.stage, r) {
                (Stage::FirstScan | Stage::SecondScan, Request::Delete(id)) => id > 0,
                (Stage::FindController, Request::FindController)
                | (Stage::LookupController, Request::LookupController)
                | (Stage::CreateController, Request::CreateController)
                | (Stage::ResetBudget, Request::ResetBudget)
                | (Stage::ResetStatistics, Request::ResetStatistics)
                | (Stage::ResetGlobals, Request::ResetGlobals)
                | (Stage::ResetWorld, Request::ResetWorld)
                | (Stage::ResetLot, Request::ResetLot)
                | (Stage::RefreshVisitors, Request::RefreshVisitors)
                | (Stage::ImportLot, Request::ImportLot)
                | (Stage::Environment, Request::Environment)
                | (Stage::HouseOpen, Request::HouseOpen)
                | (Stage::ReleaseSnapshot, Request::ReleaseSnapshot) => true,
                _ => false,
            };
            if !valid {
                return Err("load-completion callback lost its caller".into());
            }
        }
        Ok(())
    }
    pub fn advance(&mut self, view: &mut impl View) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            let request = match self.stage {
                Stage::FirstScan => {
                    let mut found = None;
                    for &id in view.people() {
                        if view.resource_flags(id)? & 0xc == 4
                            && view.person(id, 61)? == 0
                            && i32::from(view.person(id, 67)?) != view.manager_lot()
                        {
                            found = Some(id);
                            break;
                        }
                    }
                    if let Some(id) = found {
                        Request::Delete(id)
                    } else {
                        self.stage = Stage::Controller;
                        continue;
                    }
                }
                Stage::Controller => {
                    self.stage = if view.controller_suppressed() == 0
                        && view.mode() < 3
                        && view.controller_gate()? != 0
                    {
                        Stage::FindController
                    } else {
                        Stage::Reset
                    };
                    continue;
                }
                Stage::FindController => Request::FindController,
                Stage::LookupController => Request::LookupController,
                Stage::CreateController => Request::CreateController,
                Stage::Reset => {
                    if self.inputs.lot_argument != 0
                        && view.family_id() != 0
                        && self.inputs.reset_requested != 0
                    {
                        view.write_family_flags(view.family_flags()? & !4)?;
                        self.stage = Stage::ResetBudget;
                    } else {
                        self.stage = Stage::RefreshVisitors;
                    }
                    continue;
                }
                Stage::ResetBudget => Request::ResetBudget,
                Stage::ResetStatistics => Request::ResetStatistics,
                Stage::ResetGlobals => Request::ResetGlobals,
                Stage::ResetWorld => Request::ResetWorld,
                Stage::ResetLot => Request::ResetLot,
                Stage::RefreshVisitors => Request::RefreshVisitors,
                Stage::SecondScan => {
                    let mut found = None;
                    for &id in view.people() {
                        if view.resource_flags(id)? & 0xc != 4 {
                            continue;
                        }
                        if self.inputs.residents_before_visits.contains(&id)
                            && view.person(id, 32)? != 0
                        {
                            found = Some(id);
                            break;
                        }
                        let family = i32::from(view.person(id, 61)?);
                        if let Some(home) = view.family_home(family)? {
                            // Preserve the actual ordered signed comparisons.
                            // <3000 followed by >=3010 is contradictory in the
                            // captured binary; do not "repair" it into a range.
                            let below = family < 3000;
                            let above = family >= 3010;
                            if family != 0
                                && below
                                && above
                                && ![2000, 4000, 5000, 5500, 6000].contains(&family)
                                && home == 0
                            {
                                found = Some(id);
                                break;
                            }
                        }
                    }
                    if let Some(id) = found {
                        Request::Delete(id)
                    } else {
                        self.stage = Stage::Import;
                        continue;
                    }
                }
                Stage::Import => {
                    self.stage = if self.inputs.imported != 0 {
                        Stage::ImportLot
                    } else {
                        Stage::Environment
                    };
                    continue;
                }
                Stage::ImportLot => Request::ImportLot,
                Stage::Environment => Request::Environment,
                Stage::HouseOpen => Request::HouseOpen,
                Stage::ReleaseSnapshot => Request::ReleaseSnapshot,
                Stage::Complete => return Ok(None),
            };
            self.pending = Some(request);
            self.validate()?;
            return Ok(self.pending);
        }
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("stale load-completion reply".into());
        }
        let stage = match (request, reply) {
            (Request::FindController, Reply::Found(found)) => {
                if found {
                    Stage::Reset
                } else {
                    Stage::LookupController
                }
            }
            (Request::LookupController, Reply::Found(found)) => {
                if found {
                    Stage::CreateController
                } else {
                    Stage::Reset
                }
            }
            (Request::Delete(_), Reply::Done) => self.stage,
            (Request::CreateController, Reply::Done) => Stage::Reset,
            (Request::ResetBudget, Reply::Done) => Stage::ResetStatistics,
            (Request::ResetStatistics, Reply::Done) => Stage::ResetGlobals,
            (Request::ResetGlobals, Reply::Done) => Stage::ResetWorld,
            (Request::ResetWorld, Reply::Done) => Stage::ResetLot,
            (Request::ResetLot, Reply::Done) => Stage::RefreshVisitors,
            (Request::RefreshVisitors, Reply::Done) => Stage::SecondScan,
            (Request::ImportLot, Reply::Done) => Stage::Environment,
            (Request::Environment, Reply::Done) => Stage::HouseOpen,
            (Request::HouseOpen, Reply::Done) => Stage::ReleaseSnapshot,
            (Request::ReleaseSnapshot, Reply::Done) => Stage::Complete,
            _ => return Err("load-completion reply kind differs".into()),
        };
        self.stage = stage;
        self.pending = None;
        Ok(())
    }
}
