use super::{lighting, validate_owner, write_word};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Serialize};

pub mod file;
pub mod handle;
pub mod manager;
pub mod names;
pub mod opening;
pub mod path;
pub mod population;
pub mod strings;

const OWNER: u64 = 0x6f01a0;
const OWNER_BYTES: usize = 100;
const ORDER: [usize; 7] = [0, 1, 4, 2, 5, 3, 6];

fn rounded(significand: u128, exponent: i32, precision: u32) -> (u128, i32) {
    let bits = 128 - significand.leading_zeros();
    if bits <= precision {
        return (significand, exponent);
    }
    let shift = bits - precision;
    let mut retained = significand >> shift;
    let remainder = significand & ((1_u128 << shift) - 1);
    let midpoint = 1_u128 << (shift - 1);
    if remainder > midpoint || (remainder == midpoint && retained & 1 != 0) {
        retained += 1;
    }
    if 128 - retained.leading_zeros() > precision {
        (retained >> 1, exponent + shift as i32 + 1)
    } else {
        (retained, exponent + shift as i32)
    }
}

pub fn levels(input_bits: u32) -> Result<u32, Stop> {
    if input_bits > 0x7f7fffff {
        return Err(Stop::Unsupported(
            "resource levels require a nonnegative finite original f32 input".into(),
        ));
    }
    if input_bits <= 0x3fc00000 {
        return Ok(0);
    }
    let significand = u128::from((input_bits & 0x7fffff) | 0x800000);
    let exponent = ((input_bits >> 23) & 0xff) as i32 - 127 - 23;
    let (product, exponent) = rounded(significand * 0x15555555555555, exponent - 54, 64);
    let (difference, exponent) = if exponent <= -1 {
        (product - (1_u128 << (-1 - exponent)), exponent)
    } else {
        ((product << (exponent + 1)) - 1, -1)
    };
    let (difference, exponent) = rounded(difference, exponent, 64);
    let (argument, exponent) = rounded(difference, exponent, 53);
    let integral = if exponent < 0 {
        argument.checked_shr((-exponent) as u32).unwrap_or(0)
    } else if exponent < 32 && argument <= (i32::MAX as u128 >> exponent) {
        argument << exponent
    } else {
        return Err(Stop::Unsupported(
            "resource level exceeds the supported native signed integer result".into(),
        ));
    };
    if integral > i32::MAX as u128 {
        return Err(Stop::Unsupported(
            "resource level exceeds the supported native signed integer result".into(),
        ));
    }
    Ok(integral as u32)
}

pub fn reserve_bytes(input_bits: u32) -> Result<u32, Stop> {
    levels(input_bits)?
        .checked_add(1)
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(|| {
            Stop::Unsupported("resource vector byte count overflows native width".into())
        })
}

pub trait Host {
    fn allocate_resource_vector(&mut self, vector: u8, bytes: u32) -> Result<Allocated, Stop>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub level_bits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    ResetReturn,
    VectorsReserved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Reset(u8),
    Allocate(u8),
    Accept(u8),
    Publish { ordinal: u8, words: u8 },
    Reached,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub lighting: lighting::State,
    pub owner: Owner,
    pub backings: [Option<Owner>; 7],
    pub context: Context,
}

impl State {
    pub(super) fn spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.validate()?;
        let mut spans = self.lighting.fixed.arrays.spans()?;
        spans.extend(
            self.lighting
                .fixed
                .tables
                .regions()
                .into_iter()
                .map(|(address, data)| (u64::from(address), data.len())),
        );
        spans.extend([
            (0x6edf48, 4320),
            (0x6c66fc, 1),
            (0x6f016c, 4),
            (self.owner.token, self.owner.data.len()),
        ]);
        spans.extend(
            self.backings
                .iter()
                .flatten()
                .map(|owner| (owner.token, owner.data.len())),
        );
        Ok(spans)
    }

    fn validate_at(&self, allocated: usize, published: usize, words: usize) -> Result<(), Stop> {
        self.lighting.validate()?;
        validate_owner(&self.owner, OWNER_BYTES)?;
        if self.owner.token != OWNER {
            return Err(Stop::Unsupported(
                "resource continuation requires the actual constructed static owner".into(),
            ));
        }
        let bytes = reserve_bytes(self.context.level_bits)?;
        let mut spans = self.lighting.fixed.arrays.spans()?;
        spans.extend(
            self.lighting
                .fixed
                .tables
                .regions()
                .into_iter()
                .map(|(address, data)| (u64::from(address), data.len())),
        );
        spans.extend([
            (0x6edf48, 4320),
            (0x6c66fc, 1),
            (0x6f016c, 4),
            (OWNER, OWNER_BYTES),
        ]);
        let mut expected = [0_u8; OWNER_BYTES];
        for (ordinal, index) in ORDER.into_iter().enumerate() {
            let backing = self.backings[index].as_ref();
            if (ordinal < allocated) != backing.is_some() {
                return Err(Stop::Unsupported(
                    "resource reserve phase lost its exact ordered backing ownership".into(),
                ));
            }
            if let Some(backing) = backing {
                validate_owner(backing, bytes as usize)?;
                let beginning = backing.token as u32;
                let capacity = beginning.checked_add(bytes).ok_or_else(|| {
                    Stop::Unsupported(
                        "resource vector capacity exceeds native pointer width".into(),
                    )
                })?;
                spans.push((backing.token, bytes as usize));
                let count = if ordinal < published {
                    3
                } else if ordinal == published {
                    words
                } else {
                    0
                };
                for (word, value) in [beginning, beginning, capacity]
                    .into_iter()
                    .enumerate()
                    .take(count)
                {
                    let offset = index * 12 + word * 4;
                    expected[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        if self.owner.data != expected {
            return Err(Stop::Unsupported(
                "resource owner differs from actual reserve publication or untouched tail fields"
                    .into(),
            ));
        }
        spans.sort_unstable();
        if spans
            .windows(2)
            .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
        {
            return Err(Stop::Unsupported(
                "resource allocations alias retained world, static tables or another resource"
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.validate_at(7, 7, 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    state: State,
    phase: Phase,
    pending: Option<Allocated>,
}

impl Construction {
    pub fn new(
        lighting: lighting::Construction,
        owner: Owner,
        context: Context,
    ) -> Result<Self, Stop> {
        let result = Self {
            state: State {
                lighting: lighting.into_state()?,
                owner,
                backings: std::array::from_fn(|_| None),
                context,
            },
            phase: Phase::Reset(0),
            pending: None,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn lighting(&self) -> &lighting::State {
        &self.state.lighting
    }

    pub fn owner(&self) -> &Owner {
        &self.state.owner
    }

    pub fn backings(&self) -> &[Option<Owner>; 7] {
        &self.state.backings
    }

    pub fn cut(&self) -> Option<Cut> {
        match self.phase {
            Phase::Allocate(0) => Some(Cut::ResetReturn),
            Phase::Reached => Some(Cut::VectorsReserved),
            _ => None,
        }
    }

    fn validate(&self) -> Result<(), Stop> {
        if matches!(self.phase, Phase::Accept(_)) != self.pending.is_some() {
            return Err(Stop::Unsupported(
                "resource reserve lost its exact pending allocator response".into(),
            ));
        }
        let (allocated, published, words) = match self.phase {
            Phase::Reset(index) if index < 7 => (0, 0, 0),
            Phase::Allocate(index) | Phase::Accept(index) if index < 7 => {
                (index as usize, index as usize, 0)
            }
            Phase::Publish { ordinal, words } if ordinal < 7 && words < 3 => {
                (ordinal as usize + 1, ordinal as usize, words as usize)
            }
            Phase::Reached => (7, 7, 0),
            _ => return Err(Stop::Unsupported("invalid resource reserve phase".into())),
        };
        self.state.validate_at(allocated, published, words)
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        match self.phase {
            Phase::Reset(ordinal) => {
                write_word(&mut self.state.owner, ORDER[ordinal as usize] * 12 + 4, 0);
                self.phase = if ordinal == 6 {
                    Phase::Allocate(0)
                } else {
                    Phase::Reset(ordinal + 1)
                };
            }
            Phase::Allocate(ordinal) => {
                self.pending = Some(host.allocate_resource_vector(
                    ORDER[ordinal as usize] as u8,
                    reserve_bytes(self.state.context.level_bits)?,
                )?);
                self.phase = Phase::Accept(ordinal);
            }
            Phase::Accept(ordinal) => {
                let backing = match self.pending.as_ref().unwrap() {
                    Allocated::Bytes { token, data } => Owner {
                        token: *token,
                        data: data.clone(),
                    },
                    Allocated::Null => {
                        return Err(Stop::Unavailable(
                            "resource reserve has no retained writable allocator response".into(),
                        ));
                    }
                };
                let mut candidate = self.state.clone();
                candidate.backings[ORDER[ordinal as usize]] = Some(backing);
                candidate.validate_at(ordinal as usize + 1, ordinal as usize, 0)?;
                self.state = candidate;
                self.pending = None;
                self.phase = Phase::Publish { ordinal, words: 0 };
            }
            Phase::Publish { ordinal, words } => {
                let index = ORDER[ordinal as usize];
                let backing = self.state.backings[index].as_ref().unwrap();
                let value = backing.token as u32
                    + if words == 2 {
                        reserve_bytes(self.state.context.level_bits)?
                    } else {
                        0
                    };
                write_word(
                    &mut self.state.owner,
                    index * 12 + words as usize * 4,
                    value,
                );
                self.phase = if words < 2 {
                    Phase::Publish {
                        ordinal,
                        words: words + 1,
                    }
                } else if ordinal < 6 {
                    Phase::Allocate(ordinal + 1)
                } else {
                    Phase::Reached
                };
            }
            Phase::Reached => {}
        }
        self.validate()?;
        Ok(self.cut())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cut() != Some(Cut::VectorsReserved) {
            return Err(Stop::Unavailable(
                "resource continuation has not reached actual4b5746 vector-reserved cut".into(),
            ));
        }
        self.state.validate()?;
        Ok(self.state)
    }
}

#[cfg(test)]
mod tests;
