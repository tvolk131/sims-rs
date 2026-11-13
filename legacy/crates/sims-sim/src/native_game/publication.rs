use super::{WorldReturned, Write, store, unsupported};
use crate::native_world;
use crate::native_world_children::Owner;
use crate::saved_matrix::Stop;
use serde::{Deserialize, Deserializer, Serialize};
use sims_vm::commander::Registry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    Entry,
    WorldPublished,
    ReadyForSimulator,
}

impl Cut {
    fn stores(self) -> usize {
        match self {
            Self::Entry => 0,
            Self::WorldPublished => 1,
            Self::ReadyForSimulator => 2,
        }
    }
}

fn publish(
    owner: &mut Owner,
    writes: &mut Vec<Write>,
    index: usize,
    world: u32,
) -> Result<(), Stop> {
    let (offset, instruction) = match index {
        0 => (0x18, 0x415ca4),
        1 => (0x1c, 0x415ca7),
        _ => return Err(unsupported("Game World publication has no further store")),
    };
    let token = owner.token;
    store(
        std::slice::from_mut(owner),
        writes,
        token,
        offset,
        instruction,
        &world.to_le_bytes(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Construction {
    source: WorldReturned,
    owner: Owner,
    writes: Vec<Write>,
    cut: Cut,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    source: WorldReturned,
    owner: Owner,
    writes: Vec<Write>,
    cut: Cut,
}

impl<'de> Deserialize<'de> for Construction {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = Retained::deserialize(deserializer)?;
        let result = Self {
            source: retained.source,
            owner: retained.owner,
            writes: retained.writes,
            cut: retained.cut,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

impl Construction {
    pub fn new(source: WorldReturned) -> Result<Self, Stop> {
        source.validate()?;
        let owner = source.game_constructor().construction().owner().clone();
        let result = Self {
            source,
            owner,
            writes: vec![],
            cut: Cut::Entry,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn source(&self) -> &WorldReturned {
        &self.source
    }

    pub fn owner(&self) -> &Owner {
        &self.owner
    }

    pub fn world(&self) -> &native_world::returned::State {
        self.source.world()
    }

    pub fn registry(&self) -> &Registry<u32> {
        self.world().registry()
    }

    pub fn writes(&self) -> &[Write] {
        &self.writes
    }

    pub fn cut(&self) -> Cut {
        self.cut
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.source.validate()?;
        let mut expected = self
            .source
            .game_constructor()
            .construction()
            .owner()
            .clone();
        let mut writes = vec![];
        for index in 0..self.cut.stores() {
            publish(
                &mut expected,
                &mut writes,
                index,
                self.world().owner().token as u32,
            )?;
        }
        if self.owner != expected || self.writes != writes {
            return Err(unsupported(
                "Game World publication lost its actual owner, cursor or ordered stores",
            ));
        }
        Ok(())
    }

    pub fn guard_span(&self, token: u64, bytes: usize) -> Result<(), Stop> {
        self.validate()?;
        let end = token
            .checked_add(bytes as u64)
            .filter(|end| token != 0 && bytes != 0 && *end <= u64::from(u32::MAX) + 1)
            .ok_or_else(|| unsupported("Game publication allocation lacks a native extent"))?;
        let mut spans = self.world().population().retained_spans()?;
        spans.extend(
            self.world()
                .corners()
                .iter()
                .map(|owner| (owner.token, owner.data.len())),
        );
        spans.extend([(0x6de858, 8), (0x6e5528, 4)]);
        if spans
            .into_iter()
            .any(|(beginning, length)| token < beginning + length as u64 && beginning < end)
        {
            return Err(unsupported(
                "Game publication allocation aliases its complete retained graph",
            ));
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<Cut, Stop> {
        self.validate()?;
        let next = match self.cut {
            Cut::Entry => Cut::WorldPublished,
            Cut::WorldPublished => Cut::ReadyForSimulator,
            Cut::ReadyForSimulator => return Ok(self.cut),
        };
        let world = self.world().owner().token as u32;
        publish(&mut self.owner, &mut self.writes, self.cut.stores(), world)?;
        self.cut = next;
        self.validate()?;
        Ok(self.cut)
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cut != Cut::ReadyForSimulator {
            return Err(unsupported(
                "Game has not completed both World publications",
            ));
        }
        Ok(State { publication: self })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct State {
    publication: Construction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedState {
    publication: Construction,
}

impl<'de> Deserialize<'de> for State {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = RetainedState::deserialize(deserializer)?;
        retained
            .publication
            .into_state()
            .map_err(serde::de::Error::custom)
    }
}

impl State {
    pub fn publication(&self) -> &Construction {
        &self.publication
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.publication.validate()?;
        if self.publication.cut() != Cut::ReadyForSimulator {
            return Err(unsupported(
                "Game state precedes its second World publication",
            ));
        }
        Ok(())
    }

    pub fn owner(&self) -> &Owner {
        self.publication.owner()
    }

    pub fn world(&self) -> &native_world::returned::State {
        self.publication.world()
    }

    pub fn registry(&self) -> &Registry<u32> {
        self.publication.registry()
    }

    pub fn guard_span(&self, token: u64, bytes: usize) -> Result<(), Stop> {
        self.publication.guard_span(token, bytes)
    }
}
