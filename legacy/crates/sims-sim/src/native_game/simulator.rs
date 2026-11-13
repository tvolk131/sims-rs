use super::{Write, overlaps, publication, store, unsupported};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Deserializer, Serialize};
use sims_vm::commander::Registry;

pub trait Host {
    fn allocate_list(&mut self, bytes: u32) -> Result<Allocated, Stop>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    Entry,
    Registered,
    BuffersInitialized,
    ListAllocated,
    ListLinked,
    Returned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Construction {
    source: publication::State,
    incoming: Owner,
    owners: Vec<Owner>,
    registry: Registry<u32>,
    writes: Vec<Write>,
    cut: Cut,
    list_reply: Option<Allocated>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    source: publication::State,
    incoming: Owner,
    owners: Vec<Owner>,
    registry: Registry<u32>,
    writes: Vec<Write>,
    cut: Cut,
    list_reply: Option<Allocated>,
}

impl<'de> Deserialize<'de> for Construction {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = Retained::deserialize(deserializer)?;
        let result = Self {
            source: retained.source,
            incoming: retained.incoming,
            owners: retained.owners,
            registry: retained.registry,
            writes: retained.writes,
            cut: retained.cut,
            list_reply: retained.list_reply,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

impl Construction {
    pub fn new(source: publication::State, incoming: Owner) -> Result<Self, Stop> {
        source.validate()?;
        source
            .publication()
            .guard_span(incoming.token, incoming.data.len())?;
        if incoming.data.len() != 428 {
            return Err(unsupported(
                "Simulator constructor requires its complete 428-byte owner",
            ));
        }
        Ok(Self {
            owners: vec![incoming.clone()],
            registry: source.registry().clone(),
            source,
            incoming,
            writes: vec![],
            cut: Cut::Entry,
            list_reply: None,
        })
    }

    pub fn source(&self) -> &publication::State {
        &self.source
    }

    pub fn incoming(&self) -> &Owner {
        &self.incoming
    }

    pub fn owners(&self) -> &[Owner] {
        &self.owners
    }

    pub fn owner(&self) -> &Owner {
        &self.owners[0]
    }

    pub fn registry(&self) -> &Registry<u32> {
        &self.registry
    }

    pub fn writes(&self) -> &[Write] {
        &self.writes
    }

    pub fn cut(&self) -> Cut {
        self.cut
    }

    fn list(&self) -> Result<Owner, Stop> {
        let Some(Allocated::Bytes { token, data }) = &self.list_reply else {
            return Err(unsupported(
                "Simulator list requires its actual allocator bytes",
            ));
        };
        let owner = Owner {
            token: *token,
            data: data.clone(),
        };
        self.source
            .publication()
            .guard_span(owner.token, owner.data.len())?;
        if owner.data.len() != 28 || overlaps(&self.incoming, &owner) {
            return Err(unsupported(
                "Simulator list lacks a distinct complete 28-byte owner",
            ));
        }
        Ok(owner)
    }

    fn advance(&mut self) -> Result<(), Stop> {
        let simulator = self.incoming.token;
        let mut words = vec![];
        match self.cut {
            Cut::Entry => {
                words.extend([
                    (8, 0x407579, 0),
                    (0, 0x40757d, 0x666628),
                    (4, 0x407589, self.registry.head.unwrap_or(0)),
                    (12, 0x407592, self.registry.generation),
                ]);
                self.registry
                    .construct(simulator as u32)
                    .map_err(Stop::Unsupported)?;
                self.cut = Cut::Registered;
            }
            Cut::Registered => {
                words.extend([
                    (0xc4, 0x4732c4, 0),
                    (0xc8, 0x4732c6, 100),
                    (0xd0, 0x4732cd, 0),
                    (0xcc, 0x4732d0, 0),
                ]);
                for offset in (0xe4..0x184).step_by(4) {
                    words.push((offset, 0x46cdf9, 0));
                }
                for offset in (0x184..0x1a4).step_by(4) {
                    words.push((offset, 0x46ce08, 0));
                }
                self.cut = Cut::BuffersInitialized;
            }
            Cut::ListAllocated => {
                let list = self.list()?;
                self.owners.push(list.clone());
                for (token, offset, instruction, value) in [
                    (simulator, 0x1a4, 0x46ce17, list.token as u32),
                    (list.token, 0, 0x46ce19, list.token as u32),
                    (list.token, 4, 0x46ce1e, list.token as u32),
                ] {
                    store(
                        &mut self.owners,
                        &mut self.writes,
                        token,
                        offset,
                        instruction,
                        &value.to_le_bytes(),
                    )?;
                }
                self.cut = Cut::ListLinked;
            }
            Cut::ListLinked => {
                for (offset, instruction, value) in [
                    (0, 0x46ce29, 0x667714_u32),
                    (0xb0, 0x46ce2f, 0),
                    (0xb8, 0x46ce35, 0),
                    (0xbc, 0x46ce3b, 0),
                    (0xc0, 0x46ce41, 0),
                ] {
                    store(
                        &mut self.owners,
                        &mut self.writes,
                        simulator,
                        offset,
                        instruction,
                        &value.to_le_bytes(),
                    )?;
                }
                store(
                    &mut self.owners,
                    &mut self.writes,
                    simulator,
                    0xd4,
                    0x46ce47,
                    &[0],
                )?;
                words.extend([
                    (0x9c, 0x46ce4d, 0),
                    (0xa4, 0x46ce53, 0),
                    (0xa0, 0x46ce59, 0),
                    (0xb4, 0x46ce5f, 0),
                ]);
                for offset in (0x10..0x90).step_by(4) {
                    words.push((offset, 0x46ce65, 0));
                }
                words.extend([
                    (0xd0, 0x47332f, 30),
                    (0xd8, 0x46ce7b, 60),
                    (0xdc, 0x46ce85, 90),
                ]);
                self.registry
                    .bind(simulator as u32, 0x667714)
                    .map_err(Stop::Unsupported)?;
                self.cut = Cut::Returned;
            }
            _ => {
                return Err(unsupported(
                    "Simulator constructor cursor requires its allocator reply",
                ));
            }
        }
        for (offset, instruction, value) in words {
            store(
                &mut self.owners,
                &mut self.writes,
                simulator,
                offset,
                instruction,
                &value.to_le_bytes(),
            )?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), Stop> {
        let mut expected = Self::new(self.source.clone(), self.incoming.clone())?;
        let needs_reply = matches!(
            self.cut,
            Cut::ListAllocated | Cut::ListLinked | Cut::Returned
        );
        if self.list_reply.is_some() != needs_reply {
            return Err(unsupported(
                "Simulator list reply does not belong to its constructor cursor",
            ));
        }
        if needs_reply {
            self.list()?;
        }
        for _ in 0..5 {
            if expected.cut == self.cut {
                break;
            }
            if expected.cut == Cut::BuffersInitialized {
                expected.list_reply = self.list_reply.clone();
                expected.cut = Cut::ListAllocated;
            } else {
                expected.advance()?;
            }
        }
        if self != &expected {
            return Err(unsupported(
                "Simulator constructor differs from its complete source and ordered prefix",
            ));
        }
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        if self.cut == Cut::Returned {
            return Ok(None);
        }
        if self.cut == Cut::BuffersInitialized {
            self.list_reply = Some(host.allocate_list(28)?);
            self.cut = Cut::ListAllocated;
        } else {
            self.advance()?;
        }
        self.validate()?;
        Ok(Some(self.cut))
    }

    pub fn guard_span(&self, token: u64, bytes: usize) -> Result<(), Stop> {
        self.validate()?;
        self.source.publication().guard_span(token, bytes)?;
        let candidate = token..token + bytes as u64;
        if self.owners.iter().any(|owner| {
            candidate.start < owner.token + owner.data.len() as u64 && owner.token < candidate.end
        }) || self.list_reply.as_ref().is_some_and(|reply| {
            matches!(reply, Allocated::Bytes { token, data }
                if candidate.start < *token + data.len() as u64 && *token < candidate.end)
        }) {
            return Err(unsupported(
                "Simulator successor allocation aliases retained ownership",
            ));
        }
        Ok(())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cut != Cut::Returned {
            return Err(unsupported(
                "Simulator has not returned from its constructor",
            ));
        }
        Ok(State { construction: self })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct State {
    construction: Construction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedState {
    construction: Construction,
}

impl<'de> Deserialize<'de> for State {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = RetainedState::deserialize(deserializer)?;
        retained
            .construction
            .into_state()
            .map_err(serde::de::Error::custom)
    }
}

impl State {
    pub fn construction(&self) -> &Construction {
        &self.construction
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.construction.validate()?;
        if self.construction.cut() != Cut::Returned {
            return Err(unsupported(
                "Simulator state precedes its constructor return",
            ));
        }
        Ok(())
    }
}
