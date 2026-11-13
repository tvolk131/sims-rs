use super::{arrays, startup};
use crate::saved_matrix::{Allocation, Backing, Matrix, Stop};
use serde::{Deserialize, Deserializer, Serialize};

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn matrix(arrays: &arrays::State, slot: u8) -> Result<&Matrix, Stop> {
    let identity = arrays.identity(slot)?;
    arrays
        .matrices
        .nodes
        .get(&identity)
        .map(|node| &node.matrix)
        .ok_or_else(|| unsupported("geometry initialization matrix owner absent"))
}

fn backing(matrix: &Matrix, stride: i32) -> Result<&Backing, Stop> {
    if matrix.stride != stride
        || !(1..=128).contains(&matrix.width)
        || !(1..=128).contains(&matrix.height)
    {
        return Err(unsupported(
            "geometry initialization exceeds its bounded native matrix profile",
        ));
    }
    let Allocation::Known(backing) = &matrix.allocation else {
        return Err(unsupported(
            "geometry initialization lacks its complete matrix backing",
        ));
    };
    let pitch = matrix.height as usize * stride as usize;
    if backing.data.len() != matrix.width as usize * pitch
        || backing.column_offsets.len() != matrix.width as usize
        || backing.token == 0
        || backing.token > u32::MAX as u64
        || backing.token + (backing.column_offsets.len() * 4 + backing.data.len()) as u64
            > u64::from(u32::MAX) + 1
    {
        return Err(unsupported(
            "geometry initialization backing extent differs from its actual directory",
        ));
    }
    let mut columns = backing.column_offsets.clone();
    columns.sort_unstable();
    if columns
        .iter()
        .enumerate()
        .any(|(index, offset)| *offset as usize != index * pitch)
    {
        return Err(unsupported(
            "geometry initialization columns do not own a complete disjoint backing",
        ));
    }
    Ok(backing)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    Entry,
    PrimaryFilled,
    ScanEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Write {
    pub slot: u8,
    pub offset: usize,
    pub instruction: u32,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Store {
    slot: u8,
    offset: usize,
    bytes: usize,
    instruction: u32,
}

impl Store {
    fn apply(self, backing: &mut Backing) -> Result<Write, Stop> {
        let bytes = backing
            .data
            .get_mut(self.offset..self.offset + self.bytes)
            .ok_or_else(|| unsupported("geometry initialization store lacks its owned bytes"))?;
        let before = bytes.to_vec();
        if self.instruction == 0x47c945 {
            bytes[0] &= 0x28;
        } else {
            bytes.fill(0xff);
        }
        Ok(Write {
            slot: self.slot,
            offset: self.offset,
            instruction: self.instruction,
            before,
            after: bytes.to_vec(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    stores: Vec<Store>,
    primary_stores: usize,
}

impl Plan {
    fn for_matrices(primary: &Matrix, flags: &Matrix, slots: [u8; 2]) -> Result<Self, Stop> {
        let primary_backing = backing(primary, 2)?;
        let flags_backing = backing(flags, 1)?;
        if primary_backing.column_offsets.first() != Some(&0) || flags.width != flags.height {
            return Err(unsupported(
                "geometry initialization requires a primary origin and square flag map",
            ));
        }
        let mut stores = vec![];
        for offset in (0..primary_backing.data.len()).step_by(4) {
            let bytes = (primary_backing.data.len() - offset).min(4);
            stores.push(Store {
                slot: slots[0],
                offset,
                bytes,
                instruction: if bytes == 4 { 0x47c5e4 } else { 0x47c5e8 },
            });
        }
        let primary_stores = stores.len();
        for row in 0..flags.height as usize {
            for column in &flags_backing.column_offsets {
                stores.push(Store {
                    slot: slots[1],
                    offset: *column as usize + row,
                    bytes: 1,
                    instruction: 0x47c945,
                });
            }
        }
        Ok(Self {
            stores,
            primary_stores,
        })
    }

    fn new(arrays: &arrays::State, floor: u8) -> Result<Self, Stop> {
        if floor > 1 {
            return Err(unsupported(
                "geometry initialization floor is not native 1 or 2",
            ));
        }
        let slots = [0x2c + floor * 4, 0x34 + floor * 4];
        Self::for_matrices(matrix(arrays, slots[0])?, matrix(arrays, slots[1])?, slots)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Initialization {
    source: startup::World,
    arrays: arrays::State,
    floor: u8,
    stores: usize,
    #[serde(skip)]
    plan: Plan,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    source: startup::World,
    arrays: arrays::State,
    floor: u8,
    stores: usize,
}

impl<'de> Deserialize<'de> for Initialization {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        let retained = Retained::deserialize(deserializer)?;
        let plan = Plan::new(retained.source.arrays(), retained.floor)
            .map_err(serde::de::Error::custom)?;
        let initialization = Self {
            source: retained.source,
            arrays: retained.arrays,
            floor: retained.floor,
            stores: retained.stores,
            plan,
        };
        initialization
            .validate()
            .map_err(serde::de::Error::custom)?;
        Ok(initialization)
    }
}

impl Initialization {
    pub fn isolated(source: startup::World, floor: u8) -> Result<Self, Stop> {
        source.validate()?;
        let plan = Plan::new(source.arrays(), floor)?;
        Ok(Self {
            arrays: source.arrays().clone(),
            source,
            floor,
            stores: 0,
            plan,
        })
    }

    pub fn source(&self) -> &startup::World {
        &self.source
    }

    pub fn arrays(&self) -> &arrays::State {
        &self.arrays
    }

    pub fn floor(&self) -> u8 {
        self.floor
    }

    pub fn stores(&self) -> usize {
        self.stores
    }

    pub fn cut(&self) -> Option<Cut> {
        match self.stores {
            0 => Some(Cut::Entry),
            count if count == self.plan.primary_stores => Some(Cut::PrimaryFilled),
            count if count == self.plan.stores.len() => Some(Cut::ScanEntry),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.source.validate()?;
        if self.plan != Plan::new(self.source.arrays(), self.floor)?
            || self.stores > self.plan.stores.len()
        {
            return Err(unsupported(
                "geometry initialization cursor has no bounded source",
            ));
        }
        let mut expected = self.source.arrays().clone();
        for store in &self.plan.stores[..self.stores] {
            store.apply(expected.backing_mut(store.slot)?)?;
        }
        if self.arrays != expected {
            return Err(unsupported(
                "geometry initialization differs from its complete ordered store prefix",
            ));
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<Option<Write>, Stop> {
        let Some(store) = self.plan.stores.get(self.stores).copied() else {
            return Ok(None);
        };
        let write = store.apply(self.arrays.backing_mut(store.slot)?)?;
        self.stores += 1;
        Ok(Some(write))
    }
}

#[cfg(test)]
mod tests;
