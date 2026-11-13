use super::unsupported;
use crate::saved_matrix::Stop;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    Entry,
    WidthStored,
    HeightStored,
    DepthStored,
    Returned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    pub instruction: u32,
    pub address: u32,
    pub before: [u8; 4],
    pub after: [u8; 4],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Construction {
    source: [u8; 12],
    bytes: [u8; 12],
    writes: Vec<Write>,
    cut: Cut,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    source: [u8; 12],
    bytes: [u8; 12],
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
            bytes: retained.bytes,
            writes: retained.writes,
            cut: retained.cut,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

impl Construction {
    pub fn new(source: [u8; 12]) -> Self {
        Self {
            source,
            bytes: source,
            writes: vec![],
            cut: Cut::Entry,
        }
    }

    pub fn source(&self) -> &[u8; 12] {
        &self.source
    }

    pub fn bytes(&self) -> &[u8; 12] {
        &self.bytes
    }

    pub fn writes(&self) -> &[Write] {
        &self.writes
    }

    pub fn cut(&self) -> Cut {
        self.cut
    }

    fn advance(&mut self) {
        let (offset, instruction, value, next) = match self.cut {
            Cut::Entry => (0, 0x60791e, 640_u32, Cut::WidthStored),
            Cut::WidthStored => (4, 0x607928, 480, Cut::HeightStored),
            Cut::HeightStored => (8, 0x607932, 16, Cut::DepthStored),
            Cut::DepthStored => {
                self.cut = Cut::Returned;
                return;
            }
            Cut::Returned => return,
        };
        let before = self.bytes[offset..offset + 4].try_into().unwrap();
        let after = value.to_le_bytes();
        self.bytes[offset..offset + 4].copy_from_slice(&after);
        self.writes.push(Write {
            instruction,
            address: 0x6ff0d8 + offset as u32,
            before,
            after,
        });
        self.cut = next;
    }

    pub fn validate(&self) -> Result<(), Stop> {
        let mut expected = Self::new(self.source);
        for _ in 0..4 {
            if expected.cut == self.cut {
                break;
            }
            expected.advance();
        }
        if self != &expected {
            return Err(unsupported(
                "Display initializer differs from its owned source, ordered stores or return cut",
            ));
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        if self.cut == Cut::Returned {
            return Ok(None);
        }
        self.advance();
        self.validate()?;
        Ok(Some(self.cut))
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cut != Cut::Returned {
            return Err(unsupported(
                "Display initializer has not returned from its complete store sequence",
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
        if self.construction.cut != Cut::Returned {
            return Err(unsupported(
                "Display initializer state lacks its return cut",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "display_initialization/tests_02.rs"]
mod tests;
