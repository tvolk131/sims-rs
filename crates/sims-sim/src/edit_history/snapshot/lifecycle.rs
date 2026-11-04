//!48c0ea removes a captured object;48c053 restores or recreates it;
//!48bbf7 destroys an embedded snapshot. These callers retain their native
//!callback boundaries. The owner supplies actual virtual/factory services.
use super::{Delete, References, Registry, Release, Snapshot, placement};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveObject(pub u32);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Remove {
    stage: u8,
    pending: Option<RemoveObject>,
    result: bool,
}
impl Remove {
    pub fn result(&self) -> Option<bool> {
        (self.stage == 2).then_some(self.result)
    }
    pub fn acknowledge(&mut self, request: RemoveObject) -> Result<(), String> {
        if self.stage != 1 || self.pending != Some(request) {
            return Err("snapshot removal reply differs or has already returned".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(
        &mut self,
        snapshot: &Snapshot,
        refs: &mut References,
    ) -> Result<Option<RemoveObject>, String> {
        if self.stage > 2 || (self.pending.is_some() && self.stage != 1) {
            return Err("snapshot removal has an invalid saved stage".into());
        }
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        match self.stage {
            0 => {
                if snapshot.active == 0 || refs.script_id(snapshot)? == 0 {
                    self.stage = 2;
                    return Ok(None);
                }
                if let Some(object) = refs.object(snapshot)? {
                    //Virtual+6c(1). This is distinct from placement's+6c(0)
                    //and final reference cleanup's44123f registry deletion.
                    self.stage = 1;
                    self.pending = Some(RemoveObject(object));
                    return Ok(self.pending);
                }
            }
            1 => {
                //The key and catalogue are reread after the virtual call.
                //A nonzero missing key is an invalid native dereference, not
                //a successful no-op. Preserve it as an explicit error.
                if snapshot.key != 0 {
                    let allocation = refs
                        .by_key(snapshot.key)?
                        .ok_or("removed snapshot key has no reference record")?;
                    refs.records.get_mut(&allocation).unwrap().removed = 1;
                }
            }
            2 => return Ok(None),
            _ => unreachable!(),
        }
        self.result = true;
        self.stage = 2;
        Ok(None)
    }
}

pub trait View: placement::View {
    ///The live application object's+14 manager, captured after the initial
    ///script-ID gate. Its identity survives the master lookup and factory.
    fn manager(&self) -> Result<u32, String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///43ae0a on the captured manager's definition catalogue.
    Master {
        manager: u32,
        definition: u32,
    },
    ///440b2d on that same manager; a null master is passed through unchanged.
    Create {
        manager: u32,
        definition: Option<u32>,
    },
    Placement(placement::Request),
    Remove(RemoveObject),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Master(Option<u32>),
    Created(i16),
    Placement(placement::Reply),
    Removed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    MasterReturned,
    CreateReturned,
    Placement(placement::Call),
    Remove(Remove),
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Restore {
    stage: Stage,
    manager: Option<u32>,
    master: Option<u32>,
    created: i16,
    result: bool,
    pending: Option<Request>,
}
impl Default for Restore {
    fn default() -> Self {
        Self {
            stage: Stage::Entry,
            manager: None,
            master: None,
            created: 0,
            result: false,
            pending: None,
        }
    }
}
impl Restore {
    pub fn result(&self) -> Option<bool> {
        matches!(self.stage, Stage::Complete).then_some(self.result)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.manager == Some(0) || self.master == Some(0) {
            return Err("null snapshot service identity must be represented as None".into());
        }
        if let Stage::Placement(c) = &self.stage {
            c.validate()?;
        }
        let valid = self.pending.is_none_or(|r| match (&self.stage, r) {
            (Stage::MasterReturned, Request::Master { manager, .. }) => {
                Some(manager) == self.manager
            }
            (
                Stage::CreateReturned,
                Request::Create {
                    manager,
                    definition,
                },
            ) => Some(manager) == self.manager && definition == self.master,
            (Stage::Placement(_), Request::Placement(_))
            | (Stage::Remove(_), Request::Remove(_)) => true,
            _ => false,
        });
        if valid {
            Ok(())
        } else {
            Err("snapshot restoration lost its saved service".into())
        }
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("snapshot restoration reply differs".into());
        }
        match (&mut self.stage, request, reply) {
            (Stage::MasterReturned, Request::Master { .. }, Reply::Master(master))
                if master != Some(0) =>
            {
                self.master = master
            }
            (Stage::CreateReturned, Request::Create { .. }, Reply::Created(id)) => {
                self.created = id;
            }
            (Stage::Placement(c), Request::Placement(r), Reply::Placement(answer)) => {
                c.acknowledge(r, answer)?;
            }
            (Stage::Remove(c), Request::Remove(r), Reply::Removed) => c.acknowledge(r)?,
            _ => return Err("snapshot restoration reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    fn begin_placement(
        &mut self,
        snapshot: &Snapshot,
        refs: &mut References,
    ) -> Result<(), String> {
        //Unlike48c0ea, a missing nonzero key is explicitly tolerated here.
        if let Some(allocation) = refs.by_key(snapshot.key)? {
            refs.records.get_mut(&allocation).unwrap().removed = 0;
        }
        self.stage = Stage::Placement(placement::Call::new());
        Ok(())
    }
    pub fn advance(
        &mut self,
        snapshot: &Snapshot,
        refs: &mut References,
        view: &impl View,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let request = loop {
            match &mut self.stage {
                Stage::Entry => {
                    if refs.script_id(snapshot)? == 0 {
                        self.stage = Stage::Complete;
                        continue;
                    }
                    let manager = view.manager()?;
                    if manager == 0 {
                        return Err("snapshot restoration manager absent".into());
                    }
                    let definition = snapshot
                        .definition
                        .ok_or("uncaptured snapshot definition")?;
                    self.manager = Some(manager);
                    self.stage = Stage::MasterReturned;
                    break Request::Master {
                        manager,
                        definition,
                    };
                }
                Stage::MasterReturned => {
                    if refs.object(snapshot)?.is_some() {
                        self.begin_placement(snapshot, refs)?;
                    } else {
                        self.stage = Stage::CreateReturned;
                        break Request::Create {
                            manager: self.manager.ok_or("snapshot manager absent")?,
                            definition: self.master,
                        };
                    }
                }
                Stage::CreateReturned => {
                    //The newly returned ID is compared with the current
                    //snapshot record. Never assign a guessed replacement
                    //object pointer or delete an unexpected created object.
                    if self.created == 0 || self.created != refs.script_id(snapshot)? {
                        self.stage = Stage::Complete;
                    } else {
                        self.begin_placement(snapshot, refs)?;
                    }
                }
                Stage::Placement(c) => {
                    if let Some(r) = c.advance(snapshot, refs, view)? {
                        break Request::Placement(r);
                    }
                    if c.result().ok_or("snapshot placement not complete")? {
                        self.result = true;
                        self.stage = Stage::Complete;
                    } else {
                        self.stage = Stage::Remove(Remove::default());
                    }
                }
                Stage::Remove(c) => {
                    if let Some(r) = c.advance(snapshot, refs)? {
                        break Request::Remove(r);
                    }
                    //Cleanup's defined AL does not replace restore's false.
                    c.result().ok_or("snapshot removal not complete")?;
                    self.stage = Stage::Complete;
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        Ok(self.pending)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Destroy {
    entered: bool,
    release: Option<Release>,
}
impl Destroy {
    pub fn complete(&self) -> bool {
        self.entered && self.release.as_ref().is_none_or(Release::complete)
    }
    pub fn acknowledge(&mut self, request: Delete) -> Result<(), String> {
        self.release
            .as_mut()
            .ok_or("snapshot destruction has no deletion service")?
            .acknowledge(request)
    }
    pub fn advance(
        &mut self,
        snapshot: &Snapshot,
        refs: &mut References,
        view: &mut impl Registry,
    ) -> Result<Option<Delete>, String> {
        if !self.entered {
            if snapshot.active != 0 && refs.script_id(snapshot)? != 0 {
                self.release = Some(Release::new(snapshot.key));
            }
            self.entered = true;
        }
        match &mut self.release {
            Some(c) => c.advance(refs, view),
            None => Ok(None),
        }
    }
}
