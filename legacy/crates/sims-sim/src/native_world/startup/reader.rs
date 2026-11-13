//! Owning raw saved-array cursor. The World remains quiescent while a distinct
//! naming phase owns its historical World and live child. No generic successful
//! name callback can acknowledge this reader's terminal operation.
use super::{World, naming, unsupported};
use crate::{
    native_matrix,
    saved_matrix::{Scratch, Stop},
    saved_world::{self, NamesRequest},
};
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    World(World),
    Naming {
        owner: naming::Owner,
        request: NamesRequest,
        waiting_scratch: Scratch,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    request: NamesRequest,
    scratch: Scratch,
}

#[derive(Debug)]
pub struct BeginError {
    pub stop: Stop,
    pub world: World,
    pub scratch: Scratch,
    pub version: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Owner {
    phase: Phase,
    execution: saved_world::Execution,
    scratch: Scratch,
    completed: Option<Completion>,
}

impl<'de> Deserialize<'de> for Owner {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Retained {
            phase: Phase,
            execution: saved_world::Execution,
            scratch: Scratch,
            completed: Option<Completion>,
        }
        let value = Retained::deserialize(deserializer)?;
        let owner = Self {
            phase: value.phase,
            execution: value.execution,
            scratch: value.scratch,
            completed: value.completed,
        };
        owner.validate().map_err(serde::de::Error::custom)?;
        Ok(owner)
    }
}

impl Owner {
    pub fn new(world: World, version: i32, scratch: Scratch) -> Result<Self, BeginError> {
        if let Err(stop) = world.validate() {
            return Err(BeginError {
                stop,
                world,
                scratch,
                version,
            });
        }
        let execution = match saved_world::Execution::new(version) {
            Ok(execution) => execution,
            Err(stop) => {
                return Err(BeginError {
                    stop,
                    world,
                    scratch,
                    version,
                });
            }
        };
        Ok(Self {
            phase: Phase::World(world),
            execution,
            scratch,
            completed: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn resume_parts(
        world: World,
        execution: saved_world::Execution,
        scratch: Scratch,
    ) -> Result<Self, Stop> {
        let owner = Self {
            phase: Phase::World(world),
            execution,
            scratch,
            completed: None,
        };
        owner.validate()?;
        Ok(owner)
    }

    /// During naming this is the historical source World; arrays() exposes the
    /// active child. Only returned extraction transfers an activatable World.
    pub fn world(&self) -> &World {
        match &self.phase {
            Phase::World(world) => world,
            Phase::Naming { owner, .. } => owner.source(),
        }
    }
    pub fn arrays(&self) -> &crate::native_world::arrays::State {
        match &self.phase {
            Phase::World(world) => world.arrays(),
            Phase::Naming { owner, .. } => owner.arrays(),
        }
    }
    pub fn execution(&self) -> &saved_world::Execution {
        &self.execution
    }
    pub fn scratch(&self) -> &Scratch {
        &self.scratch
    }
    pub fn status(&self) -> Option<i32> {
        self.execution.status()
    }
    pub fn naming_in_progress(&self) -> bool {
        matches!(self.phase, Phase::Naming { .. })
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.execution.validate_profile()?;
        match &self.phase {
            Phase::World(world) => {
                world.validate()?;
                if self.execution.status() == Some(0) {
                    if !self.completed.as_ref().is_some_and(|completion| {
                        completion.request.completed(&self.execution)
                            && completion.scratch == self.scratch
                    }) || world.namings().last().map(|returned| returned.arrays())
                        != Some(world.arrays())
                    {
                        return Err(unsupported(
                            "raw saved-array success lacks its exact owned naming return",
                        ));
                    }
                } else if self.completed.is_some() {
                    return Err(unsupported(
                        "raw saved-array completion ticket has no returned caller",
                    ));
                }
            }
            Phase::Naming {
                owner,
                request,
                waiting_scratch,
            } => {
                owner.validate()?;
                if self.completed.is_some()
                    || !request.matches(&self.execution)
                    || waiting_scratch != &self.scratch
                {
                    return Err(unsupported(
                        "raw saved-array naming lost its complete waiting caller",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn step<H: saved_world::Host + native_matrix::Host>(
        &mut self,
        host: &mut H,
    ) -> Result<Option<i32>, Stop> {
        self.validate()?;
        if self.status().is_some() {
            return Ok(self.status());
        }
        let result = match &mut self.phase {
            Phase::World(world) => {
                match world.advance_saved_arrays(&mut self.execution, &mut self.scratch, host)? {
                    saved_world::Advance::Pending => Ok(None),
                    saved_world::Advance::Returned(status) => Ok(Some(status)),
                    saved_world::Advance::Names(request) => {
                        let owner =
                            naming::Owner::new(world.clone()).map_err(|error| error.stop)?;
                        self.phase = Phase::Naming {
                            owner,
                            request,
                            waiting_scratch: self.scratch,
                        };
                        Ok(None)
                    }
                }
            }
            Phase::Naming {
                owner,
                request,
                waiting_scratch,
            } => {
                let result = owner.step(host);
                match result {
                    Ok(true) => {
                        let world = owner.clone().into_world().map_err(|_| {
                            unsupported("raw saved-array naming has not returned a valid World")
                        })?;
                        self.execution.acknowledge_names(request)?;
                        self.completed = Some(Completion {
                            request: request.clone(),
                            scratch: *waiting_scratch,
                        });
                        self.phase = Phase::World(world);
                        Ok(Some(0))
                    }
                    Ok(false) => Ok(None),
                    Err(stop) => Err(stop),
                }
            }
        };
        self.validate()?;
        result
    }

    pub fn into_returned(self) -> Result<Returned, Self> {
        if self.status().is_some()
            && matches!(self.phase, Phase::World(_))
            && self.validate().is_ok()
        {
            Ok(Returned { owner: self })
        } else {
            Err(self)
        }
    }
}

#[derive(Debug)]
pub struct Returned {
    owner: Owner,
}

impl Returned {
    pub fn status(&self) -> i32 {
        self.owner.status().unwrap()
    }
    /// Includes cursor-held legacy temporaries and abandoned leases. The caller
    /// must retain the associated file and services; no cleanup runs on return.
    pub fn into_parts(self) -> (World, saved_world::Execution, Scratch) {
        let Owner {
            phase,
            execution,
            scratch,
            ..
        } = self.owner;
        match phase {
            Phase::World(world) => (world, execution, scratch),
            Phase::Naming { .. } => unreachable!("returned constructor enforces a quiescent World"),
        }
    }
}
