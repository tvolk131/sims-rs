//! Native star-pool refill (446356) and indicator release (4481eb).
//! Pools retain order, duplicate entries and null members; these routines do
//! not normalize them. Factory/physical services run before acknowledgment.
use crate::{
    ObjectId,
    selection_indicators::{SECONDARY_GUID, STAR_GUID, Slot},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pools {
    pub active: Vec<Option<ObjectId>>,
    pub spare: Vec<Option<ObjectId>>,
}

pub trait View {
    fn enabled(&self) -> bool;
    fn expansions(&self) -> u16;
    fn guid(&self, object: ObjectId) -> Result<u32, String>;
    /// OBJD kind, not an RTTI cast and not the resolved occupant's kind.
    fn kind(&self, object: ObjectId) -> Result<i16, String>;
    fn slot(&self, person: ObjectId) -> Result<Option<Slot>, String>;
    fn lookup(&self, script_id: i16) -> Option<ObjectId>;
    /// Native object-data word 8 |= 0x4000, after the factory and lookup.
    fn hide(&mut self, object: ObjectId) -> Result<(), String>;
}

/// Stable resource-owner identity retained across each factory invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    LookupStar,
    CreateStar(Resource),
    /// Virtual RemoveFromLot(0), unlike the selection controller's (1).
    Detach(ObjectId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Resource(Option<Resource>),
    Created(i16),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Start,
    Lookup,
    Plan,
    Create,
    Store,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refill {
    phase: Phase,
    resource: Option<Resource>,
    remaining: u8,
    created: i16,
    pending: Option<Request>,
}
impl Default for Refill {
    fn default() -> Self {
        Self {
            phase: Phase::Start,
            resource: None,
            remaining: 0,
            created: 0,
            pending: None,
        }
    }
}
impl Refill {
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn resource(&self) -> Option<Resource> {
        self.resource
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Done
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.remaining > 4
            || self.resource == Some(Resource(0))
            || matches!(self.phase, Phase::Create | Phase::Store)
                && (self.remaining == 0 || self.resource.is_none())
            || self.pending.is_some_and(|r| match (self.phase, r) {
                (Phase::Lookup, Request::LookupStar) => false,
                (Phase::Create, Request::CreateStar(resource)) => Some(resource) != self.resource,
                _ => true,
            })
        {
            return Err("invalid native star-pool refill".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        pools: &mut Pools,
        view: &mut impl View,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            let request = match self.phase {
                Phase::Start => {
                    self.phase = if !pools.spare.is_empty() || view.expansions() & 0x40 == 0 {
                        Phase::Done
                    } else {
                        Phase::Lookup
                    };
                    continue;
                }
                Phase::Lookup => Request::LookupStar,
                Phase::Plan => {
                    // Lookup can run callbacks: both vectors are read again.
                    self.remaining = if pools.spare.is_empty() && pools.active.is_empty() {
                        4
                    } else {
                        2
                    };
                    self.phase = if self.resource.is_some() {
                        Phase::Create
                    } else {
                        Phase::Done
                    };
                    continue;
                }
                Phase::Create => Request::CreateStar(self.resource.unwrap()),
                Phase::Store => {
                    if self.created != 0 {
                        if let Some(object) = view.lookup(self.created) {
                            view.hide(object)?;
                            pools.spare.push(Some(object));
                        }
                    }
                    self.remaining -= 1;
                    self.phase = if self.remaining == 0 {
                        Phase::Done
                    } else {
                        Phase::Create
                    };
                    continue;
                }
                Phase::Done => return Ok(None),
            };
            self.pending = Some(request);
            return Ok(self.pending);
        }
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("stale star-pool refill acknowledgment".into());
        }
        match (request, reply) {
            (Request::LookupStar, Reply::Resource(resource)) if resource != Some(Resource(0)) => {
                self.resource = resource;
                self.phase = Phase::Plan;
            }
            (Request::CreateStar(_), Reply::Created(id)) => {
                self.created = id;
                self.phase = Phase::Store;
            }
            _ => return Err("mismatched star-pool refill reply".into()),
        }
        self.pending = None;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ReleasePhase {
    Start,
    Detach,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    object: Option<ObjectId>,
    resolved: Option<ObjectId>,
    phase: ReleasePhase,
}
impl Release {
    pub fn object(&self) -> Option<ObjectId> {
        self.object
    }
    pub fn new(object: Option<ObjectId>) -> Self {
        Self {
            object,
            resolved: None,
            phase: ReleasePhase::Start,
        }
    }
    pub fn complete(&self) -> bool {
        self.phase == ReleasePhase::Done
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.object == Some(0)
            || self.resolved == Some(0)
            || self.phase == ReleasePhase::Detach
                && (self.resolved.is_none() || self.object.is_none())
        {
            return Err("invalid indicator release continuation".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        pools: &mut Pools,
        view: &impl View,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.phase == ReleasePhase::Start {
            let Some(object) = self.object else {
                self.phase = ReleasePhase::Done;
                return Ok(None);
            };
            if !view.enabled() {
                self.phase = ReleasePhase::Done;
                return Ok(None);
            }
            // Read the argument's GUID before resolving a person's slot. Using
            // the occupant's GUID here changes native behavior.
            let guid = view.guid(object)?;
            let resolved = if view.kind(object)? == 2 {
                view.slot(object)?
                    .and_then(|slot| view.lookup(slot.occupant))
            } else {
                Some(object)
            };
            if let Some(resolved) = resolved {
                if guid == STAR_GUID {
                    if let Some(index) = pools.active.iter().position(|v| *v == Some(resolved)) {
                        pools.spare.push(pools.active[index]);
                        pools.active.remove(index);
                    }
                } else if guid != SECONDARY_GUID && guid != crate::load_indicators::GUIDS[0] {
                    self.phase = ReleasePhase::Done;
                    return Ok(None);
                }
                self.resolved = Some(resolved);
                self.phase = ReleasePhase::Detach;
            } else {
                self.phase = ReleasePhase::Done;
            }
        }
        Ok(if self.phase == ReleasePhase::Detach {
            Some(Request::Detach(self.resolved.unwrap()))
        } else {
            None
        })
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.phase != ReleasePhase::Detach || request != Request::Detach(self.resolved.unwrap())
        {
            return Err("stale indicator release acknowledgment".into());
        }
        self.phase = ReleasePhase::Done;
        Ok(())
    }
}
