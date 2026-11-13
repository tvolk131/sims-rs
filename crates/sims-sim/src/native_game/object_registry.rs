use super::{Write, overlaps, simulator, store, unsupported};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Deserializer, Serialize};
use sims_vm::commander::Registry;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublishedSimulator {
    source: simulator::State,
    game: Owner,
    writes: Vec<Write>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedPublication {
    source: simulator::State,
    game: Owner,
    writes: Vec<Write>,
}

impl<'de> Deserialize<'de> for PublishedSimulator {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = RetainedPublication::deserialize(deserializer)?;
        let result = Self {
            source: retained.source,
            game: retained.game,
            writes: retained.writes,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

impl PublishedSimulator {
    pub fn new(source: simulator::State) -> Result<Self, Stop> {
        source.validate()?;
        let mut game = source.construction().source().owner().clone();
        let mut writes = vec![];
        let token = game.token;
        let simulator = source.construction().owner().token as u32;
        store(
            std::slice::from_mut(&mut game),
            &mut writes,
            token,
            0x20,
            0x415cd1,
            &simulator.to_le_bytes(),
        )?;
        Ok(Self {
            source,
            game,
            writes,
        })
    }

    pub fn source(&self) -> &simulator::State {
        &self.source
    }

    pub fn game(&self) -> &Owner {
        &self.game
    }

    pub fn writes(&self) -> &[Write] {
        &self.writes
    }

    pub fn registry(&self) -> &Registry<u32> {
        self.source.construction().registry()
    }

    pub fn validate(&self) -> Result<(), Stop> {
        if self != &Self::new(self.source.clone())? {
            return Err(unsupported(
                "Game Simulator publication differs from its actual source and store",
            ));
        }
        Ok(())
    }

    pub fn guard_span(&self, token: u64, bytes: usize) -> Result<(), Stop> {
        self.validate()?;
        self.source.construction().guard_span(token, bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Incoming {
    pub owner: Owner,
    pub comparator: Owner,
}

impl Incoming {
    fn validate(&self, source: &PublishedSimulator) -> Result<(), Stop> {
        source.guard_span(self.owner.token, self.owner.data.len())?;
        source.guard_span(self.comparator.token, self.comparator.data.len())?;
        if self.owner.data.len() != 272
            || self.comparator.data.len() != 1
            || overlaps(&self.owner, &self.comparator)
        {
            return Err(unsupported(
                "Object registry requires its complete owner and distinct actual caller byte",
            ));
        }
        Ok(())
    }
}

pub trait Host {
    fn allocate_sentinel(&mut self, bytes: u32) -> Result<Allocated, Stop>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    Entry,
    Registered,
    TreeAllocationEntry,
    TreeAllocated,
    Returned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparatorRead {
    pub source: u64,
    pub instruction: u32,
    pub byte: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Construction {
    source: PublishedSimulator,
    incoming: Incoming,
    owners: Vec<Owner>,
    registry: Registry<u32>,
    writes: Vec<Write>,
    cut: Cut,
    sentinel_reply: Option<Allocated>,
    comparator_read: Option<ComparatorRead>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    source: PublishedSimulator,
    incoming: Incoming,
    owners: Vec<Owner>,
    registry: Registry<u32>,
    writes: Vec<Write>,
    cut: Cut,
    sentinel_reply: Option<Allocated>,
    comparator_read: Option<ComparatorRead>,
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
            sentinel_reply: retained.sentinel_reply,
            comparator_read: retained.comparator_read,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

impl Construction {
    pub fn new(source: PublishedSimulator, incoming: Incoming) -> Result<Self, Stop> {
        source.validate()?;
        incoming.validate(&source)?;
        Ok(Self {
            owners: vec![incoming.owner.clone()],
            registry: source.registry().clone(),
            source,
            incoming,
            writes: vec![],
            cut: Cut::Entry,
            sentinel_reply: None,
            comparator_read: None,
        })
    }

    pub fn source(&self) -> &PublishedSimulator {
        &self.source
    }

    pub fn incoming(&self) -> &Incoming {
        &self.incoming
    }

    pub fn owner(&self) -> &Owner {
        &self.owners[0]
    }

    pub fn owners(&self) -> &[Owner] {
        &self.owners
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

    pub fn comparator_read(&self) -> Option<&ComparatorRead> {
        self.comparator_read.as_ref()
    }

    fn sentinel(&self) -> Result<Owner, Stop> {
        let Some(Allocated::Bytes { token, data }) = &self.sentinel_reply else {
            return Err(unsupported(
                "Object-registry tree requires its actual sentinel allocation",
            ));
        };
        let owner = Owner {
            token: *token,
            data: data.clone(),
        };
        self.source.guard_span(owner.token, owner.data.len())?;
        if owner.data.len() != 24
            || overlaps(&owner, &self.incoming.owner)
            || overlaps(&owner, &self.incoming.comparator)
        {
            return Err(unsupported(
                "Object-registry sentinel lacks a distinct complete 24-byte owner",
            ));
        }
        Ok(owner)
    }

    fn write(
        &mut self,
        token: u64,
        offset: usize,
        instruction: u32,
        payload: &[u8],
    ) -> Result<(), Stop> {
        store(
            &mut self.owners,
            &mut self.writes,
            token,
            offset,
            instruction,
            payload,
        )
    }

    fn root_word(&mut self, offset: usize, instruction: u32, value: u32) -> Result<(), Stop> {
        self.write(
            self.incoming.owner.token,
            offset,
            instruction,
            &value.to_le_bytes(),
        )
    }

    fn root_bytes(&mut self, offset: usize, instruction: u32, payload: &[u8]) -> Result<(), Stop> {
        self.write(self.incoming.owner.token, offset, instruction, payload)
    }

    fn advance(&mut self) -> Result<(), Stop> {
        let identity = self.incoming.owner.token as u32;
        match self.cut {
            Cut::Entry => {
                for (offset, instruction, value) in [
                    (8, 0x407579, 0),
                    (0, 0x40757d, 0x666628),
                    (4, 0x407589, self.registry.head.unwrap_or(0)),
                    (12, 0x407592, self.registry.generation),
                ] {
                    self.root_word(offset, instruction, value)?;
                }
                self.registry
                    .construct(identity)
                    .map_err(Stop::Unsupported)?;
                self.cut = Cut::Registered;
            }
            Cut::Registered => {
                self.root_bytes(0x11, 0x43f40e, &[0])?;
                for (offset, instructions) in [
                    (0x18, [0x43f421, 0x43f427, 0x43f42e]),
                    (0x24, [0x43f445, 0x43f44b, 0x43f452]),
                    (0x38, [0x43f469, 0x43f46f, 0x43f476]),
                    (0x4c, [0x43f48d, 0x43f493, 0x43f49a]),
                    (0x58, [0x43f4b1, 0x43f4b7, 0x43f4be]),
                    (0x64, [0x43f4d5, 0x43f4db, 0x43f4e2]),
                ] {
                    for (index, instruction) in instructions.into_iter().enumerate() {
                        self.root_word(offset + index * 4, instruction, 0)?;
                    }
                }
                self.root_word(0x70, 0x448a0c, 0)?;
                self.cut = Cut::TreeAllocationEntry;
            }
            Cut::TreeAllocated => {
                let sentinel = self.sentinel()?;
                let token = sentinel.token;
                self.owners.push(sentinel);
                self.root_word(0x70, 0x448a32, token as u32)?;
                self.root_word(0x74, 0x448a37, 0)?;
                self.comparator_read = Some(ComparatorRead {
                    source: self.incoming.comparator.token,
                    instruction: 0x448a3e,
                    byte: self.incoming.comparator.data[0],
                });
                self.root_bytes(0x78, 0x448a43, &[self.incoming.comparator.data[0]])?;
                self.write(token, 0, 0x448a51, &[0])?;
                self.write(token, 4, 0x448a59, &0_u32.to_le_bytes())?;
                self.write(token, 8, 0x448a67, &(token as u32).to_le_bytes())?;
                self.write(token, 12, 0x448a74, &(token as u32).to_le_bytes())?;
                for (offset, instructions) in [
                    (0x84, [0x43f51b, 0x43f521, 0x43f528]),
                    (0x90, [0x43f541, 0x43f547, 0x43f54e]),
                    (0x9c, [0x43f567, 0x43f56d, 0x43f574]),
                    (0xb0, [0x43f58d, 0x43f593, 0x43f59a]),
                    (0xbc, [0x43f5b3, 0x43f5b9, 0x43f5c0]),
                ] {
                    for (index, instruction) in instructions.into_iter().enumerate() {
                        self.root_word(offset + index * 4, instruction, 0)?;
                    }
                }
                for (offset, instruction) in [
                    (0xc8, 0x43f5ce),
                    (0xcc, 0x43f5db),
                    (0xd0, 0x43f5f3),
                    (0xd4, 0x43f5f9),
                    (0xd8, 0x43f600),
                    (0xdc, 0x448984),
                    (0xe0, 0x44898a),
                    (0xe4, 0x448991),
                ] {
                    self.root_word(offset, instruction, 0)?;
                }
                for (offset, instruction) in [
                    (0xec, 0x43f628),
                    (0xee, 0x43f636),
                    (0xf0, 0x43f644),
                    (0xf4, 0x43f652),
                ] {
                    self.root_bytes(offset, instruction, &[0, 0])?;
                }
                for (offset, instruction) in [
                    (0xf8, 0x43f660),
                    (0xfc, 0x43f66d),
                    (0x104, 0x43f67a),
                    (0x10c, 0x43f687),
                ] {
                    self.root_word(offset, instruction, 0)?;
                }
                self.root_word(0, 0x43f694, 0x667054)?;
                self.root_bytes(0x10, 0x43f6a0, &[0])?;
                self.root_word(0x30, 0x43f6aa, 0)?;
                self.root_word(0x34, 0x43f6b4, 0)?;
                self.root_bytes(0x80, 0x43f6be, &[0, 0])?;
                self.root_word(
                    0x7c,
                    0x43f6cf,
                    self.source.source().construction().owner().token as u32,
                )?;
                self.root_word(0x14, 0x43f6d8, 0)?;
                self.root_bytes(0x44, 0x43f6e2, &[1])?;
                self.root_word(0xa8, 0x43f6ec, 0)?;
                for (offset, instruction) in [(0xea, 0x43f6f9), (0xe8, 0x43f707), (0xf2, 0x43f715)]
                {
                    self.root_bytes(offset, instruction, &[0, 0])?;
                }
                self.root_bytes(0x12, 0x43f723, &[0])?;
                self.root_bytes(0x100, 0x43f72d, &[0xff, 0xff])?;
                self.registry
                    .bind(identity, 0x667054)
                    .map_err(Stop::Unsupported)?;
                self.cut = Cut::Returned;
            }
            _ => {
                return Err(unsupported(
                    "Object-registry cursor requires its actual sentinel reply",
                ));
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), Stop> {
        let mut expected = Self::new(self.source.clone(), self.incoming.clone())?;
        let needs_reply = matches!(self.cut, Cut::TreeAllocated | Cut::Returned);
        if self.sentinel_reply.is_some() != needs_reply {
            return Err(unsupported(
                "Object-registry sentinel reply does not belong to its cursor",
            ));
        }
        if needs_reply {
            self.sentinel()?;
        }
        for _ in 0..4 {
            if expected.cut == self.cut {
                break;
            }
            if expected.cut == Cut::TreeAllocationEntry {
                expected.sentinel_reply = self.sentinel_reply.clone();
                expected.cut = Cut::TreeAllocated;
            } else {
                expected.advance()?;
            }
        }
        if self != &expected {
            return Err(unsupported(
                "Object-registry constructor differs from its complete source and ordered prefix",
            ));
        }
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        if self.cut == Cut::Returned {
            return Ok(None);
        }
        if self.cut == Cut::TreeAllocationEntry {
            self.sentinel_reply = Some(host.allocate_sentinel(24)?);
            self.cut = Cut::TreeAllocated;
        } else {
            self.advance()?;
        }
        self.validate()?;
        Ok(Some(self.cut))
    }

    pub fn guard_span(&self, token: u64, bytes: usize) -> Result<(), Stop> {
        self.validate()?;
        self.source.guard_span(token, bytes)?;
        let candidate = token..token + bytes as u64;
        if self
            .owners
            .iter()
            .chain(std::iter::once(&self.incoming.comparator))
            .any(|owner| {
                candidate.start < owner.token + owner.data.len() as u64
                    && owner.token < candidate.end
            })
            || self.sentinel_reply.as_ref().is_some_and(|reply| {
                matches!(reply, Allocated::Bytes { token, data }
                if candidate.start < *token + data.len() as u64 && *token < candidate.end)
            })
        {
            return Err(unsupported(
                "Object-registry successor allocation aliases retained ownership",
            ));
        }
        Ok(())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cut != Cut::Returned {
            return Err(unsupported(
                "Object registry has not returned from its constructor",
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
                "Object-registry state precedes its constructor return",
            ));
        }
        Ok(())
    }
}
