use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Deserializer, Serialize};

pub mod vector;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Child,
    Vector(u8),
}

impl Request {
    pub fn bytes(self) -> Result<u32, Stop> {
        match self {
            Self::Child => Ok(96),
            Self::Vector(index) if index < 4 => Ok(12),
            _ => Err(Stop::Unsupported("unknown world child allocation".into())),
        }
    }
}

pub trait Host {
    fn allocate(&mut self, request: Request) -> Result<Allocated, Stop>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub token: u64,
    pub data: Vec<u8>,
}

impl Owner {
    fn validate(&self, bytes: usize) -> Result<(), Stop> {
        if self.token == 0
            || self.token > u32::MAX as u64
            || self.token + bytes as u64 > u64::from(u32::MAX) + 1
            || self.data.len() != bytes
        {
            return Err(Stop::Unsupported(
                "world child lacks its complete native-width allocation".into(),
            ));
        }
        Ok(())
    }

    fn write_word(&mut self, offset: usize, value: u32) {
        self.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.token < other.token + other.data.len() as u64
            && other.token < self.token + self.data.len() as u64
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub owner: Owner,
    pub child: Option<Owner>,
    pub vectors: [Option<Owner>; 4],
}

impl State {
    pub fn validate(&self) -> Result<(), Stop> {
        self.owner.validate(72)?;
        let mut retained = vec![&self.owner];
        if let Some(child) = &self.child {
            child.validate(96)?;
            retained.push(child);
        }
        for vector in self.vectors.iter().flatten() {
            vector.validate(12)?;
            retained.push(vector);
        }
        for (index, owner) in retained.iter().enumerate() {
            if retained[..index].iter().any(|other| owner.overlaps(other)) {
                return Err(Stop::Unsupported(
                    "world child allocations overlap live ownership".into(),
                ));
            }
        }
        Ok(())
    }

    fn accept(&self, reply: &Allocated, bytes: usize) -> Result<Option<Owner>, Stop> {
        let owner = match reply {
            Allocated::Null => return Ok(None),
            Allocated::Bytes { token, data } => Owner {
                token: *token,
                data: data.clone(),
            },
        };
        owner.validate(bytes)?;
        if std::iter::once(&self.owner)
            .chain(self.child.iter())
            .chain(self.vectors.iter().flatten())
            .any(|retained| owner.overlaps(retained))
        {
            return Err(Stop::Unsupported(
                "world child allocator reply aliases live ownership".into(),
            ));
        }
        Ok(Some(owner))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Header,
    Allocate(Request),
    Accept(Request),
    InitializeChild,
    PublishChild,
    InitializeVector(u8),
    PublishVector(u8),
    Finalize,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Construction {
    pub state: State,
    phase: Phase,
    pending: Option<Allocated>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetainedConstruction {
    state: State,
    phase: Phase,
    pending: Option<Allocated>,
}

impl<'de> Deserialize<'de> for Construction {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = RetainedConstruction::deserialize(deserializer)?;
        let result = Self {
            state: retained.state,
            phase: retained.phase,
            pending: retained.pending,
        };
        result.validate().map_err(serde::de::Error::custom)?;
        Ok(result)
    }
}

impl Construction {
    pub(crate) fn pending_allocation(&self) -> Option<&Allocated> {
        self.pending.as_ref()
    }

    pub(crate) fn validate(&self) -> Result<(), Stop> {
        self.state.validate()?;
        let progress = match self.phase {
            Phase::Header => 0,
            Phase::Allocate(Request::Child) => 1,
            Phase::Accept(Request::Child) => 2,
            Phase::InitializeChild => 3,
            Phase::PublishChild => 4,
            Phase::Allocate(Request::Vector(index)) if index < 4 => 5 + 4 * index,
            Phase::Accept(Request::Vector(index)) if index < 4 => 6 + 4 * index,
            Phase::InitializeVector(index) if index < 4 => 7 + 4 * index,
            Phase::PublishVector(index) if index < 4 => 8 + 4 * index,
            Phase::Finalize => 21,
            Phase::Complete => 22,
            _ => {
                return Err(Stop::Unsupported(
                    "world child constructor cursor has an invalid vector index".into(),
                ));
            }
        };
        if matches!(self.phase, Phase::Accept(_)) != self.pending.is_some()
            || (progress < 3 && self.state.child.is_some())
            || self
                .state
                .vectors
                .iter()
                .enumerate()
                .any(|(index, vector)| progress < 7 + 4 * index as u8 && vector.is_some())
        {
            return Err(Stop::Unsupported(
                "world child ownership or allocator reply does not belong to its cursor".into(),
            ));
        }
        let mut words = Vec::new();
        if progress >= 1 {
            words.extend([(0, 0x66f3d8_u32), (4, 0)]);
        }
        if progress >= 5 {
            words.push((
                0x28,
                self.state
                    .child
                    .as_ref()
                    .map_or(0, |child| child.token as u32),
            ));
        }
        for (index, vector) in self.state.vectors.iter().enumerate() {
            if progress >= 8 + 4 * index as u8
                && vector.as_ref().is_some_and(|owner| owner.data != [0; 12])
            {
                return Err(Stop::Unsupported(
                    "initialized child vector is not empty".into(),
                ));
            }
            if progress >= 9 + 4 * index as u8 {
                words.push((
                    [8, 16, 12, 20][index],
                    vector.as_ref().map_or(0, |owner| owner.token as u32),
                ));
            }
        }
        if progress == 22 {
            words.extend([(0x2c, 0), (0x30, 0), (0x34, 0)]);
            words.extend([0x3c, 0x38, 0x44, 0x40].map(|offset| (offset, 1)));
        }
        if words
            .into_iter()
            .any(|(offset, word)| self.state.owner.data[offset..offset + 4] != word.to_le_bytes())
        {
            return Err(Stop::Unsupported(
                "world child owner fields differ from its constructor cursor".into(),
            ));
        }
        if let Some(child) = &self.state.child
            && progress >= 4
        {
            if [0, 1, 2, 8, 9, 10, 11, 12, 0x2a, 0x3e, 0x24, 0x25, 0x50]
                .into_iter()
                .any(|offset| child.data[offset] != 0)
                || [4, 0x1c, 0x18, 0x14, 0x10]
                    .into_iter()
                    .any(|offset| child.data[offset..offset + 4] != [0; 4])
                || child.data[0x20..0x24] != u32::from(progress >= 5).to_le_bytes()
            {
                return Err(Stop::Unsupported(
                    "initialized sparse child fields or reference count differ".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn returned(&self) -> Result<Option<&State>, Stop> {
        self.validate()?;
        Ok((self.phase == Phase::Complete).then_some(&self.state))
    }

    pub fn new(owner: Owner) -> Result<Self, Stop> {
        owner.validate(72)?;
        Ok(Self {
            state: State {
                owner,
                child: None,
                vectors: std::array::from_fn(|_| None),
            },
            phase: Phase::Header,
            pending: None,
        })
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<bool, Stop> {
        self.validate()?;
        match self.phase {
            Phase::Header => {
                self.state.owner.write_word(0, 0x66f3d8);
                self.state.owner.write_word(4, 0);
                self.phase = Phase::Allocate(Request::Child);
            }
            Phase::Allocate(request) => {
                request.bytes()?;
                self.pending = Some(host.allocate(request)?);
                self.phase = Phase::Accept(request);
            }
            Phase::Accept(request) => {
                let reply = self.pending.as_ref().ok_or_else(|| {
                    Stop::Unavailable("world child allocator reply absent".into())
                })?;
                let owner = self.state.accept(reply, request.bytes()? as usize)?;
                match request {
                    Request::Child => {
                        self.state.child = owner;
                        self.phase = Phase::InitializeChild;
                    }
                    Request::Vector(index) => {
                        self.state.vectors[index as usize] = owner;
                        self.phase = Phase::InitializeVector(index);
                    }
                }
                self.pending = None;
            }
            Phase::InitializeChild => {
                if let Some(child) = &mut self.state.child {
                    for offset in [2, 8, 9, 10, 11, 12, 0x2a, 0x3e, 0, 1, 2] {
                        child.data[offset] = 0;
                    }
                    child.write_word(4, 0);
                    for offset in [0x1c, 0x18, 0x14, 0x10, 0x20] {
                        child.write_word(offset, 0);
                    }
                    for offset in [0x24, 0x25, 0x50] {
                        child.data[offset] = 0;
                    }
                }
                self.phase = Phase::PublishChild;
            }
            Phase::PublishChild => {
                self.state.owner.write_word(
                    0x28,
                    self.state
                        .child
                        .as_ref()
                        .map_or(0, |child| child.token as u32),
                );
                if let Some(child) = &mut self.state.child {
                    let count = u32::from_le_bytes(child.data[0x20..0x24].try_into().unwrap());
                    child.write_word(0x20, count.wrapping_add(1));
                }
                self.phase = Phase::Allocate(Request::Vector(0));
            }
            Phase::InitializeVector(index) => {
                if let Some(vector) = &mut self.state.vectors[index as usize] {
                    for offset in [0, 4, 8] {
                        vector.write_word(offset, 0);
                    }
                }
                self.phase = Phase::PublishVector(index);
            }
            Phase::PublishVector(index) => {
                let token = self.state.vectors[index as usize]
                    .as_ref()
                    .map_or(0, |vector| vector.token as u32);
                self.state
                    .owner
                    .write_word([8, 16, 12, 20][index as usize], token);
                self.phase = if index == 3 {
                    Phase::Finalize
                } else {
                    Phase::Allocate(Request::Vector(index + 1))
                };
            }
            Phase::Finalize => {
                self.state.owner.write_word(0x2c, 0);
                for offset in [0x3c, 0x38, 0x44, 0x40] {
                    self.state.owner.write_word(offset, 1);
                }
                self.state.owner.write_word(0x30, 0);
                self.state.owner.write_word(0x34, 0);
                self.phase = Phase::Complete;
            }
            Phase::Complete => {}
        }
        self.validate()?;
        Ok(self.phase == Phase::Complete)
    }
}

#[cfg(test)]
mod tests;
