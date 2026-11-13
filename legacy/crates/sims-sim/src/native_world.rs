use crate::native_world_children::{self, Owner};
use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Serialize};
use sims_vm::commander::Registry;

pub mod arrays;
pub mod fixed;
pub mod geometry;
pub mod lighting;
pub mod resize;
pub mod resources;
pub mod returned;
pub mod startup;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    World,
    Companion,
    CompanionChild,
    CompanionVector(u8),
}

impl Request {
    pub fn bytes(self) -> Result<u32, Stop> {
        match self {
            Self::World => Ok(136),
            Self::Companion => Ok(72),
            Self::CompanionChild => Ok(96),
            Self::CompanionVector(index) if index < 4 => Ok(12),
            _ => Err(Stop::Unsupported("unknown world-prefix allocation".into())),
        }
    }
}

pub trait Host {
    fn allocate(&mut self, request: Request) -> Result<Allocated, Stop>;
}

struct ChildHost<'a, H>(&'a mut H);

impl<H: Host> native_world_children::Host for ChildHost<'_, H> {
    fn allocate(&mut self, request: native_world_children::Request) -> Result<Allocated, Stop> {
        self.0.allocate(match request {
            native_world_children::Request::Child => Request::CompanionChild,
            native_world_children::Request::Vector(index) => Request::CompanionVector(index),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    NullWorld,
    ArrayConstructionEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    AllocateWorld,
    AcceptWorld,
    Register,
    BindFixed,
    ClearSlots,
    AllocateCompanion,
    AcceptCompanion,
    ConstructCompanion,
    PublishCompanion,
    Reached(Cut),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefix {
    owner: Option<Owner>,
    registry: Registry<u32>,
    companion: Option<native_world_children::Construction>,
    phase: Phase,
    pending: Option<Allocated>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parts {
    pub owner: Owner,
    pub registry: Registry<u32>,
    pub companion: Option<native_world_children::State>,
    #[serde(default)]
    game: Option<crate::native_game::State>,
}

impl Parts {
    pub fn game(&self) -> Option<&crate::native_game::State> {
        self.game.as_ref()
    }

    pub(crate) fn with_game(mut self, game: crate::native_game::State) -> Result<Self, Stop> {
        if self.game.is_some() {
            return Err(Stop::Unsupported(
                "World already retains Game provenance".into(),
            ));
        }
        self.game = Some(game);
        self.validate_game()?;
        Ok(self)
    }

    fn validate_game(&self) -> Result<(), Stop> {
        if let Some(game) = &self.game {
            game.validate()?;
            let mut expected = game.construction().registry().clone();
            expected
                .construct(
                    u32::try_from(self.owner.token).map_err(|_| {
                        Stop::Unsupported("World identity exceeds native width".into())
                    })?,
                )
                .map_err(Stop::Unsupported)?;
            expected
                .bind(self.owner.token as u32, 0x667a88)
                .map_err(Stop::Unsupported)?;
            if self.registry != expected {
                return Err(Stop::Unsupported(
                    "World changed its actual Game/source Commander registry".into(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_owner(owner: &Owner, bytes: usize) -> Result<(), Stop> {
    if owner.token == 0
        || owner.token > u32::MAX as u64
        || owner.token + bytes as u64 > u64::from(u32::MAX) + 1
        || owner.data.len() != bytes
    {
        return Err(Stop::Unsupported(
            "world prefix lacks its complete native-width allocation".into(),
        ));
    }
    Ok(())
}

fn overlaps(owner: &Owner, token: u64, bytes: usize) -> bool {
    owner.token < token + bytes as u64 && token < owner.token + owner.data.len() as u64
}

fn write_word(owner: &mut Owner, offset: usize, value: u32) {
    owner.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn cleared_slots() -> impl Iterator<Item = usize> {
    [0x10, 0x14, 0x18, 0x1c, 0x20, 0x60, 0x64]
        .into_iter()
        .chain((0..2).flat_map(|index| {
            [0x24, 0x2c, 0x34, 0x3c, 0x44, 0x4c, 0x68, 0x58]
                .into_iter()
                .map(move |offset| offset + index * 4)
        }))
}

impl Prefix {
    pub fn new(registry: Registry<u32>) -> Result<Self, Stop> {
        registry.validate().map_err(Stop::Unsupported)?;
        Ok(Self {
            owner: None,
            registry,
            companion: None,
            phase: Phase::AllocateWorld,
            pending: None,
        })
    }

    pub fn owner(&self) -> Option<&Owner> {
        self.owner.as_ref()
    }

    pub fn registry(&self) -> &Registry<u32> {
        &self.registry
    }

    pub fn companion(&self) -> Option<&native_world_children::State> {
        self.companion
            .as_ref()
            .map(|construction| &construction.state)
    }

    pub fn cut(&self) -> Option<Cut> {
        match self.phase {
            Phase::Reached(cut) => Some(cut),
            _ => None,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), Stop> {
        self.registry.validate().map_err(Stop::Unsupported)?;
        if matches!(self.phase, Phase::AcceptWorld | Phase::AcceptCompanion)
            != self.pending.is_some()
        {
            return Err(Stop::Unsupported(
                "world-prefix allocator reply does not belong to its cursor".into(),
            ));
        }
        let needs_owner = !matches!(
            self.phase,
            Phase::AllocateWorld | Phase::AcceptWorld | Phase::Reached(Cut::NullWorld)
        );
        if needs_owner != self.owner.is_some()
            || (self.companion.is_some()
                && !matches!(
                    self.phase,
                    Phase::ConstructCompanion
                        | Phase::PublishCompanion
                        | Phase::Reached(Cut::ArrayConstructionEntry)
                ))
            || (self.phase == Phase::ConstructCompanion && self.companion.is_none())
        {
            return Err(Stop::Unsupported(
                "world-prefix owner does not belong to its cursor".into(),
            ));
        }
        if let Some(owner) = &self.owner {
            validate_owner(owner, 136)?;
            let registered = self.phase != Phase::Register;
            for identity in self.registry.nodes.keys() {
                if (!registered || u64::from(*identity) != owner.token)
                    && overlaps(owner, u64::from(*identity), 16)
                {
                    return Err(Stop::Unsupported(
                        "world allocation overlaps an existing Commander owner".into(),
                    ));
                }
            }
            if registered {
                let identity = owner.token as u32;
                let node = self.registry.nodes.get(&identity).ok_or_else(|| {
                    Stop::Unavailable("world-prefix Commander node absent".into())
                })?;
                let handler = if self.phase == Phase::BindFixed {
                    0x666628
                } else {
                    0x667a88
                };
                if self.registry.head != Some(identity)
                    || node.handler
                        != if self.phase == Phase::BindFixed {
                            0
                        } else {
                            handler
                        }
                    || [handler, node.next.unwrap_or(0), node.auxiliary, node.serial]
                        .into_iter()
                        .enumerate()
                        .any(|(index, word)| {
                            owner.data[index * 4..index * 4 + 4] != word.to_le_bytes()
                        })
                {
                    return Err(Stop::Unsupported(
                        "world-prefix Commander header and typed domain differ".into(),
                    ));
                }
            }
            if matches!(
                self.phase,
                Phase::AllocateCompanion
                    | Phase::AcceptCompanion
                    | Phase::ConstructCompanion
                    | Phase::PublishCompanion
                    | Phase::Reached(Cut::ArrayConstructionEntry)
            ) && cleared_slots().any(|offset| owner.data[offset..offset + 4] != [0; 4])
            {
                return Err(Stop::Unsupported(
                    "world array slots changed before original array construction".into(),
                ));
            }
        }
        if let Some(construction) = &self.companion {
            construction.validate()?;
            let companion = &construction.state;
            for retained in std::iter::once(&companion.owner)
                .chain(companion.child.iter())
                .chain(companion.vectors.iter().flatten())
            {
                self.validate_distinct(retained)?;
            }
            if matches!(self.phase, Phase::PublishCompanion | Phase::Reached(_))
                && construction.returned()?.is_none()
            {
                return Err(Stop::Unsupported(
                    "world companion has not reached its actual constructor return".into(),
                ));
            }
        }
        if self.cut() == Some(Cut::ArrayConstructionEntry) {
            let token = self.companion().map_or(0, |state| state.owner.token as u32);
            if self.owner.as_ref().unwrap().data[0x54..0x58] != token.to_le_bytes() {
                return Err(Stop::Unsupported(
                    "world companion publication differs from retained ownership".into(),
                ));
            }
        }
        Ok(())
    }

    fn validate_distinct(&self, owner: &Owner) -> Result<(), Stop> {
        if self
            .owner
            .as_ref()
            .is_some_and(|world| overlaps(owner, world.token, world.data.len()))
            || self
                .registry
                .nodes
                .keys()
                .any(|identity| overlaps(owner, u64::from(*identity), 16))
        {
            return Err(Stop::Unsupported(
                "world companion allocation overlaps a retained owner".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn pending_allocations(&self) -> impl Iterator<Item = &Allocated> {
        self.pending.iter().chain(
            self.companion
                .iter()
                .filter_map(|construction| construction.pending_allocation()),
        )
    }

    fn accept(&self, bytes: usize) -> Result<Option<Owner>, Stop> {
        let owner = match self.pending.as_ref() {
            Some(Allocated::Null) => return Ok(None),
            Some(Allocated::Bytes { token, data }) => Owner {
                token: *token,
                data: data.clone(),
            },
            None => {
                return Err(Stop::Unavailable(
                    "world-prefix allocator reply absent".into(),
                ));
            }
        };
        validate_owner(&owner, bytes)?;
        self.validate_distinct(&owner)?;
        Ok(Some(owner))
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        match self.phase {
            Phase::AllocateWorld => {
                self.pending = Some(host.allocate(Request::World)?);
                self.phase = Phase::AcceptWorld;
            }
            Phase::AcceptWorld => {
                self.owner = self.accept(136)?;
                self.pending = None;
                self.phase = if self.owner.is_some() {
                    Phase::Register
                } else {
                    Phase::Reached(Cut::NullWorld)
                };
            }
            Phase::Register => {
                let owner = self.owner.as_mut().unwrap();
                self.registry
                    .construct(owner.token as u32)
                    .map_err(Stop::Unsupported)?;
                let node = &self.registry.nodes[&(owner.token as u32)];
                write_word(owner, 8, 0);
                write_word(owner, 0, 0x666628);
                write_word(owner, 4, node.next.unwrap_or(0));
                write_word(owner, 12, node.serial);
                self.phase = Phase::BindFixed;
            }
            Phase::BindFixed => {
                let owner = self.owner.as_mut().unwrap();
                self.registry
                    .bind(owner.token as u32, 0x667a88)
                    .map_err(Stop::Unsupported)?;
                write_word(owner, 0, 0x667a88);
                self.phase = Phase::ClearSlots;
            }
            Phase::ClearSlots => {
                for offset in cleared_slots() {
                    write_word(self.owner.as_mut().unwrap(), offset, 0);
                }
                self.phase = Phase::AllocateCompanion;
            }
            Phase::AllocateCompanion => {
                self.pending = Some(host.allocate(Request::Companion)?);
                self.phase = Phase::AcceptCompanion;
            }
            Phase::AcceptCompanion => {
                self.companion = self
                    .accept(72)?
                    .map(native_world_children::Construction::new)
                    .transpose()?;
                self.pending = None;
                self.phase = if self.companion.is_some() {
                    Phase::ConstructCompanion
                } else {
                    Phase::PublishCompanion
                };
            }
            Phase::ConstructCompanion => {
                if self
                    .companion
                    .as_mut()
                    .unwrap()
                    .step(&mut ChildHost(host))?
                {
                    self.phase = Phase::PublishCompanion;
                }
                self.validate()?;
            }
            Phase::PublishCompanion => {
                let token = self.companion().map_or(0, |state| state.owner.token as u32);
                write_word(self.owner.as_mut().unwrap(), 0x54, token);
                self.phase = Phase::Reached(Cut::ArrayConstructionEntry);
            }
            Phase::Reached(_) => {}
        }
        Ok(self.cut())
    }

    pub fn into_parts(self) -> Result<Parts, Stop> {
        self.validate()?;
        if self.cut() != Some(Cut::ArrayConstructionEntry) {
            return Err(Stop::Unavailable(
                "world-prefix ownership has not reached original4778fc array entry".into(),
            ));
        }
        Ok(Parts {
            owner: self.owner.unwrap(),
            registry: self.registry,
            companion: self.companion.map(|construction| construction.state),
            game: None,
        })
    }
}

#[cfg(test)]
mod tests;
