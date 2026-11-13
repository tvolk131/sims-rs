use super::{Write, object_registry, store, unsupported};
use crate::native_world_children::Owner;
use crate::saved_matrix::Stop;
use serde::{Deserialize, Deserializer, Serialize};
use sims_vm::commander::Registry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    CallerEntry,
    BindingEntry,
    ProcessLanguageInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Construction {
    source: object_registry::State,
    owners: Vec<Owner>,
    writes: Vec<Write>,
    cut: Cut,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    source: object_registry::State,
    owners: Vec<Owner>,
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
            owners: retained.owners,
            writes: retained.writes,
            cut: retained.cut,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

impl Construction {
    pub fn new(source: object_registry::State) -> Result<Self, Stop> {
        source.validate()?;
        let publication = source.construction().source();
        let owners = vec![
            publication.game().clone(),
            publication.source().construction().owner().clone(),
        ];
        Ok(Self {
            source,
            owners,
            writes: vec![],
            cut: Cut::CallerEntry,
        })
    }

    pub fn source(&self) -> &object_registry::State {
        &self.source
    }

    pub fn game(&self) -> &Owner {
        &self.owners[0]
    }

    pub fn simulator(&self) -> &Owner {
        &self.owners[1]
    }

    pub fn registry_owner(&self) -> &Owner {
        self.source.construction().owner()
    }

    pub fn registry(&self) -> &Registry<u32> {
        self.source.construction().registry()
    }

    pub fn writes(&self) -> &[Write] {
        &self.writes
    }

    pub fn cut(&self) -> Cut {
        self.cut
    }

    fn write(
        &mut self,
        token: u64,
        offset: usize,
        instruction: u32,
        bytes: &[u8],
    ) -> Result<(), Stop> {
        store(
            &mut self.owners,
            &mut self.writes,
            token,
            offset,
            instruction,
            bytes,
        )
    }

    fn advance(&mut self) -> Result<(), Stop> {
        let game = self.game().token;
        let simulator = self.simulator().token;
        let registry = self.registry_owner().token as u32;
        match self.cut {
            Cut::CallerEntry => {
                self.write(game, 0x14, 0x415cfd, &registry.to_le_bytes())?;
                self.cut = Cut::BindingEntry;
            }
            Cut::BindingEntry => {
                self.write(simulator, 0xb0, 0x46cec7, &registry.to_le_bytes())?;
                for offset in (0x10..0x90).step_by(4) {
                    self.write(simulator, offset, 0x46ced4, &0_u32.to_le_bytes())?;
                }
                self.write(simulator, 0x30, 0x46ced8, &2_u16.to_le_bytes())?;
                for (offset, instruction, value) in [
                    (0x1a8, 0x46cede, 0_u32),
                    (0x98, 0x46cee4, 0),
                    (0x90, 0x46ceea, 0),
                    (0x94, 0x46cef0, 0),
                    (0xa8, 0x46cef6, 50),
                ] {
                    self.write(simulator, offset, instruction, &value.to_le_bytes())?;
                }
                for (offset, instruction, value) in [
                    (0x10, 0x46cf00, 1_u16),
                    (0x12, 0x46cf05, 15),
                    (0x1a, 0x46cf0b, 0),
                    (0x1e, 0x46cf0f, 6),
                    (0x20, 0x46cf15, 1997),
                    (0x34, 0x46cf20, u16::MAX),
                ] {
                    self.write(simulator, offset, instruction, &value.to_le_bytes())?;
                }
                if self.simulator().data[0x10..0x12] != 1_u16.to_le_bytes() {
                    return Err(unsupported(
                        "Simulator binding prefix lacks the executed 46d525 constructor branch",
                    ));
                }
                self.write(simulator, 0x18, 0x46cf25, &2_u16.to_le_bytes())?;
                self.cut = Cut::ProcessLanguageInput;
            }
            Cut::ProcessLanguageInput => {}
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), Stop> {
        let mut expected = Self::new(self.source.clone())?;
        for _ in 0..2 {
            if expected.cut == self.cut {
                break;
            }
            expected.advance()?;
        }
        if self != &expected {
            return Err(unsupported(
                "Simulator binding prefix differs from its owned source, ordered stores or cut",
            ));
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        if self.cut == Cut::ProcessLanguageInput {
            return Ok(None);
        }
        self.advance()?;
        self.validate()?;
        Ok(Some(self.cut))
    }

    pub fn guard_span(&self, token: u64, bytes: usize) -> Result<(), Stop> {
        self.validate()?;
        self.source.construction().guard_span(token, bytes)
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cut != Cut::ProcessLanguageInput {
            return Err(unsupported(
                "Simulator binding prefix has not reached its unresolved process-language input",
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
        if self.construction.cut != Cut::ProcessLanguageInput {
            return Err(unsupported(
                "Simulator binding prefix state precedes its unresolved process-language input",
            ));
        }
        Ok(())
    }
}
