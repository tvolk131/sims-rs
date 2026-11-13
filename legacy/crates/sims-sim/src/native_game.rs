use crate::native_world;
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, MAX_ALLOCATION_BYTES, Stop};
use serde::{Deserialize, Deserializer, Serialize};
use sims_vm::commander::Registry;

pub mod display_initialization;
pub mod object_registry;
pub mod publication;
pub mod simulator;
pub mod simulator_binding;

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn extent(owner: &Owner) -> Result<std::ops::Range<u64>, Stop> {
    let end = owner
        .token
        .checked_add(owner.data.len() as u64)
        .ok_or_else(|| unsupported("Game bootstrap allocation extent overflows"))?;
    if owner.token == 0 || owner.data.is_empty() || end > u64::from(u32::MAX) + 1 {
        return Err(unsupported(
            "Game bootstrap lacks a complete native allocation",
        ));
    }
    Ok(owner.token..end)
}

fn overlaps(left: &Owner, right: &Owner) -> bool {
    left.token < right.token + right.data.len() as u64
        && right.token < left.token + left.data.len() as u64
}

fn validate_new(owner: &Owner, owners: &[Owner]) -> Result<(), Stop> {
    let span = extent(owner)?;
    if owners.iter().any(|retained| overlaps(owner, retained))
        || [(0x6de858, 8), (0x6e5528, 4)]
            .into_iter()
            .any(|(beginning, bytes)| span.start < beginning + bytes && beginning < span.end)
    {
        return Err(unsupported(
            "Game bootstrap allocation aliases retained ownership",
        ));
    }
    Ok(())
}

fn header(registry: &Registry<u32>, identity: u32) -> Result<Vec<u8>, Stop> {
    let node = registry
        .nodes
        .get(&identity)
        .ok_or_else(|| unsupported("Game bootstrap Commander node is absent"))?;
    Ok([
        if node.handler == 0 {
            0x666628
        } else {
            node.handler
        },
        node.next.unwrap_or(0),
        node.auxiliary,
        node.serial,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Incoming {
    pub owners: Vec<Owner>,
    pub registry: Registry<u32>,
    pub game: u64,
    pub provider_word: u32,
}

impl Incoming {
    pub fn validate(&self) -> Result<(), Stop> {
        self.registry.validate().map_err(Stop::Unsupported)?;
        if self.owners.len() > 4096
            || self
                .owners
                .iter()
                .try_fold(0_usize, |total, owner| total.checked_add(owner.data.len()))
                .is_none_or(|total| total > MAX_ALLOCATION_BYTES)
        {
            return Err(unsupported(
                "Game bootstrap exceeds its bounded ownership profile",
            ));
        }
        for (index, owner) in self.owners.iter().enumerate() {
            validate_new(owner, &self.owners[..index])?;
        }
        let game = self
            .owners
            .iter()
            .find(|owner| owner.token == self.game)
            .ok_or_else(|| unsupported("Game112 source allocation is absent"))?;
        if game.data.len() != 112 || self.registry.nodes.contains_key(&(self.game as u32)) {
            return Err(unsupported(
                "Game112 is not a new complete constructor owner",
            ));
        }
        for identity in self.registry.nodes.keys() {
            let beginning = u64::from(*identity);
            let owner = self
                .owners
                .iter()
                .find(|owner| {
                    owner.token <= beginning
                        && beginning + 16 <= owner.token + owner.data.len() as u64
                })
                .ok_or_else(|| {
                    unsupported("Commander prefix lacks its full containing allocation")
                })?;
            if overlaps(
                game,
                &Owner {
                    token: beginning,
                    data: vec![0; 16],
                },
            ) || owner.data
                [(beginning - owner.token) as usize..(beginning - owner.token) as usize + 16]
                != header(&self.registry, *identity)?
            {
                return Err(unsupported(
                    "Commander source bytes differ from retained ownership",
                ));
            }
        }
        Ok(())
    }
}

pub trait Host {
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    Entry,
    Registered,
    StringInitialized,
    NameAllocated,
    StringConstructed,
    Returned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    pub owner: u64,
    pub offset: usize,
    pub instruction: u32,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

fn store(
    owners: &mut [Owner],
    writes: &mut Vec<Write>,
    token: u64,
    offset: usize,
    instruction: u32,
    payload: &[u8],
) -> Result<(), Stop> {
    let owner = owners
        .iter_mut()
        .find(|owner| owner.token == token)
        .ok_or_else(|| unsupported("Game constructor store lacks its complete owner"))?;
    let bytes = owner
        .data
        .get_mut(offset..offset + payload.len())
        .ok_or_else(|| unsupported("Game constructor store exceeds its owned bytes"))?;
    writes.push(Write {
        owner: token,
        offset,
        instruction,
        before: bytes.to_vec(),
        after: payload.to_vec(),
    });
    bytes.copy_from_slice(payload);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Construction {
    incoming: Incoming,
    owners: Vec<Owner>,
    registry: Registry<u32>,
    writes: Vec<Write>,
    cut: Cut,
    name_reply: Option<Allocated>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    incoming: Incoming,
    owners: Vec<Owner>,
    registry: Registry<u32>,
    writes: Vec<Write>,
    cut: Cut,
    name_reply: Option<Allocated>,
}

impl<'de> Deserialize<'de> for Construction {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = Retained::deserialize(deserializer)?;
        let result = Self {
            incoming: retained.incoming,
            owners: retained.owners,
            registry: retained.registry,
            writes: retained.writes,
            cut: retained.cut,
            name_reply: retained.name_reply,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

impl Construction {
    pub fn new(incoming: Incoming) -> Result<Self, Stop> {
        incoming.validate()?;
        Ok(Self {
            owners: incoming.owners.clone(),
            registry: incoming.registry.clone(),
            incoming,
            writes: vec![],
            cut: Cut::Entry,
            name_reply: None,
        })
    }

    pub fn incoming(&self) -> &Incoming {
        &self.incoming
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

    pub fn owner(&self) -> &Owner {
        self.owners
            .iter()
            .find(|owner| owner.token == self.incoming.game)
            .unwrap()
    }

    fn name(&self) -> Result<Owner, Stop> {
        let Some(Allocated::Bytes { token, data }) = &self.name_reply else {
            return Err(unsupported(
                "Game string construction requires its actual allocator bytes",
            ));
        };
        let owner = Owner {
            token: *token,
            data: data.clone(),
        };
        validate_new(&owner, &self.incoming.owners)?;
        if owner.data.len() != 8 || owner.token + 8 > u64::from(u32::MAX) {
            return Err(unsupported(
                "Game string requires its full eight-byte capacity",
            ));
        }
        Ok(owner)
    }

    fn advance(&mut self) -> Result<(), Stop> {
        let game = self.incoming.game;
        let mut words = vec![];
        match self.cut {
            Cut::Entry => {
                let previous_head = self.registry.head.unwrap_or(0);
                let serial = self.registry.generation;
                words.extend([
                    (8, 0x407579, 0),
                    (0, 0x40757d, 0x666628),
                    (4, 0x407589, previous_head),
                    (12, 0x407592, serial),
                ]);
                self.registry
                    .construct(game as u32)
                    .map_err(Stop::Unsupported)?;
                self.cut = Cut::Registered;
            }
            Cut::Registered => {
                words.extend([
                    (0x28, 0x5b87c3, 0),
                    (0x2c, 0x5b87c6, 0),
                    (0x30, 0x5b87ca, 0),
                ]);
                self.cut = Cut::StringInitialized;
            }
            Cut::NameAllocated => {
                let mut name = self.name()?;
                let beginning = name.token as u32;
                words.extend([
                    (0x28, 0x401a82, beginning),
                    (0x2c, 0x401a84, beginning),
                    (0x30, 0x401a89, beginning + 8),
                ]);
                for (offset, instruction, value) in words.drain(..) {
                    store(
                        &mut self.owners,
                        &mut self.writes,
                        game,
                        offset,
                        instruction,
                        &value.to_le_bytes(),
                    )?;
                }
                let before = name.data[..1].to_vec();
                name.data[0] = 0;
                self.writes.push(Write {
                    owner: name.token,
                    offset: 0,
                    instruction: 0x40163d,
                    before,
                    after: vec![0],
                });
                self.owners.push(name);
                self.cut = Cut::StringConstructed;
            }
            Cut::StringConstructed => {
                for offset in (0x38..0x60).step_by(8) {
                    words.extend([(offset, 0x415bbb, 0), (offset + 4, 0x415bbd, 0)]);
                }
                words.extend([
                    (0x60, 0x415bcc, 0),
                    (0x64, 0x415bcf, 0),
                    (0x10, 0x415bd2, 0),
                    (0x20, 0x415bd5, 0),
                    (0x18, 0x415bd8, 0),
                    (0x14, 0x415bdb, 0),
                    (0x1c, 0x415bde, 0),
                    (0x24, 0x415be1, 0),
                    (0x34, 0x415be4, self.incoming.provider_word),
                    (0x68, 0x415be7, 0),
                    (0x6c, 0x415bea, 0),
                    (0, 0x415bed, 0x666714),
                ]);
                self.registry
                    .bind(game as u32, 0x666714)
                    .map_err(Stop::Unsupported)?;
                self.cut = Cut::Returned;
            }
            _ => {
                return Err(unsupported(
                    "Game constructor cursor requires its allocator reply",
                ));
            }
        }
        for (offset, instruction, value) in words {
            store(
                &mut self.owners,
                &mut self.writes,
                game,
                offset,
                instruction,
                &value.to_le_bytes(),
            )?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), Stop> {
        let mut expected = Self::new(self.incoming.clone())?;
        let needs_reply = matches!(
            self.cut,
            Cut::NameAllocated | Cut::StringConstructed | Cut::Returned
        );
        if self.name_reply.is_some() != needs_reply {
            return Err(unsupported(
                "Game name reply does not belong to its constructor cursor",
            ));
        }
        if needs_reply {
            self.name()?;
        }
        for _ in 0..5 {
            if expected.cut == self.cut {
                break;
            }
            if expected.cut == Cut::StringInitialized {
                expected.name_reply = self.name_reply.clone();
                expected.cut = Cut::NameAllocated;
            } else {
                expected.advance()?;
            }
        }
        if self != &expected {
            return Err(unsupported(
                "Game constructor differs from its full source and ordered prefix",
            ));
        }
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        if self.cut == Cut::Returned {
            return Ok(None);
        }
        if self.cut == Cut::StringInitialized {
            self.name_reply = Some(host.allocate_name(8)?);
            self.cut = Cut::NameAllocated;
        } else {
            self.advance()?;
        }
        self.validate()?;
        Ok(Some(self.cut))
    }

    pub fn into_state(self) -> Result<State, Stop> {
        let state = State { construction: self };
        state.validate()?;
        Ok(state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct State {
    construction: Construction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Returned {
    construction: Construction,
}

impl<'de> Deserialize<'de> for State {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = Returned::deserialize(deserializer)?;
        let result = Self {
            construction: retained.construction,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

impl State {
    pub(crate) fn guard_span(&self, token: u64, bytes: usize) -> Result<(), Stop> {
        let end = token
            .checked_add(bytes as u64)
            .filter(|end| token != 0 && bytes != 0 && *end <= u64::from(u32::MAX) + 1)
            .ok_or_else(|| unsupported("World allocation lacks a complete native extent"))?;
        for retained in self.construction.owners() {
            let protected = extent(retained)?;
            if token < protected.end && protected.start < end {
                return Err(unsupported(
                    "World allocation aliases full Game/source ownership",
                ));
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.construction.validate()?;
        if self.construction.cut != Cut::Returned {
            return Err(unsupported(
                "Game state has not reached its constructor return",
            ));
        }
        Ok(())
    }

    pub fn construction(&self) -> &Construction {
        &self.construction
    }

    pub fn begin_world(self) -> Result<WorldEntry, Stop> {
        self.validate()?;
        let prefix = native_world::Prefix::new(self.construction.registry.clone())?;
        let entry = WorldEntry { game: self, prefix };
        entry.validate()?;
        Ok(entry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorldEntry {
    game: State,
    prefix: native_world::Prefix,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedWorldEntry {
    game: State,
    prefix: native_world::Prefix,
}

impl<'de> Deserialize<'de> for WorldEntry {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = RetainedWorldEntry::deserialize(deserializer)?;
        let result = Self {
            game: retained.game,
            prefix: retained.prefix,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

impl WorldEntry {
    pub fn into_arrays(
        self,
        context: native_world::arrays::Context,
    ) -> Result<native_world::arrays::Construction, Stop> {
        self.validate()?;
        let parts = self.prefix.into_parts()?.with_game(self.game)?;
        native_world::arrays::Construction::from_parts(parts, context)
    }

    pub fn game(&self) -> &State {
        &self.game
    }

    pub fn prefix(&self) -> &native_world::Prefix {
        &self.prefix
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.game.validate()?;
        self.prefix.validate()?;
        let protected = self.game.construction.owners();
        let mut expected = self.game.construction.registry.clone();
        if let Some(owner) = self.prefix.owner() {
            validate_new(owner, protected)?;
            if let Some(node) = self.prefix.registry().nodes.get(&(owner.token as u32)) {
                expected
                    .construct(owner.token as u32)
                    .map_err(Stop::Unsupported)?;
                expected
                    .bind(owner.token as u32, node.handler)
                    .map_err(Stop::Unsupported)?;
            }
        }
        if &expected != self.prefix.registry() {
            return Err(unsupported(
                "World prefix changed its actual Game/source registry",
            ));
        }
        if let Some(companion) = self.prefix.companion() {
            for owner in std::iter::once(&companion.owner)
                .chain(companion.child.iter())
                .chain(companion.vectors.iter().flatten())
            {
                validate_new(owner, protected)?;
            }
        }
        for reply in self.prefix.pending_allocations() {
            if let Allocated::Bytes { token, data } = reply {
                validate_new(
                    &Owner {
                        token: *token,
                        data: data.clone(),
                    },
                    protected,
                )?;
            }
        }
        Ok(())
    }

    pub fn step(
        &mut self,
        host: &mut impl native_world::Host,
    ) -> Result<Option<native_world::Cut>, Stop> {
        self.validate()?;
        let reached = self.prefix.step(host)?;
        self.validate()?;
        Ok(reached)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorldReturned {
    world: native_world::returned::State,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedWorldReturned {
    world: native_world::returned::State,
}

impl<'de> Deserialize<'de> for WorldReturned {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = RetainedWorldReturned::deserialize(deserializer)?;
        Self::new(retained.world).map_err(serde::de::Error::custom)
    }
}

impl WorldReturned {
    pub fn new(world: native_world::returned::State) -> Result<Self, Stop> {
        let result = Self { world };
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.world.validate()?;
        self.world
            .arrays()
            .prefix
            .game()
            .ok_or_else(|| unsupported("returned World has no actual Game constructor source"))?
            .validate()
    }

    pub fn world(&self) -> &native_world::returned::State {
        &self.world
    }

    pub fn game_constructor(&self) -> &State {
        self.world.arrays().prefix.game().unwrap()
    }
}

#[cfg(test)]
mod tests;
