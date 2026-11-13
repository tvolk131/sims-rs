use crate::native_world::{validate_owner, write_word};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MANAGER: u64 = 0x6fe548;
const BUCKET_COUNTS: [u32; 28] = [
    53, 97, 193, 389, 769, 1543, 3079, 6151, 12289, 24593, 49157, 98317, 196613, 393241, 786433,
    1572869, 3145739, 6291469, 12582917, 25165843, 50331653, 100663319, 201326611, 402653189,
    805306457, 1610612741, 3221225473, 4294967291,
];
const BUCKETS: usize = bucket_count(100) as usize;
const BUCKET_BYTES: usize = BUCKETS * 4;

pub const fn bucket_count(requested: u32) -> u32 {
    let mut index = 0;
    while index < BUCKET_COUNTS.len() {
        if BUCKET_COUNTS[index] >= requested {
            return BUCKET_COUNTS[index];
        }
        index += 1;
    }
    BUCKET_COUNTS[BUCKET_COUNTS.len() - 1]
}

pub trait Host {
    fn allocate_buckets(&mut self, map: u8, bytes: u32) -> Result<Allocated, Stop>;
    fn initialize_section(&mut self, address: u64, bytes: u32) -> Result<u64, Stop>;
}

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn word(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn disjoint(owners: impl IntoIterator<Item = (u64, usize)>) -> Result<(), Stop> {
    let mut spans: Vec<_> = owners.into_iter().collect();
    spans.sort_unstable();
    if spans
        .windows(2)
        .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
    {
        return Err(unsupported(
            "resource manager aliases another retained owner",
        ));
    }
    Ok(())
}

pub fn hash(input: &[u8]) -> u32 {
    let mut value = 0_u32;
    for byte in input.iter().take_while(|byte| **byte != 0) {
        value = value.wrapping_add(i32::from(*byte as i8) as u32);
        value = value.wrapping_mul(0x401);
        value ^= value >> 6;
    }
    value = value.wrapping_mul(9);
    value ^= value >> 11;
    value.wrapping_mul(0x8001)
}

pub fn split(input: &[u8]) -> Result<(Vec<u8>, Vec<u8>), Stop> {
    let length = input
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(input.len());
    if length >= 260 {
        return Err(unsupported(
            "resource manager split exceeds its native stack buffer",
        ));
    }
    let normalized: Vec<_> = input[..length]
        .iter()
        .map(|byte| match byte {
            b'A'..=b'Z' => byte + 32,
            b'/' => b'\\',
            _ => *byte,
        })
        .collect();
    let boundary = normalized
        .iter()
        .rposition(|byte| *byte == b'\\')
        .map_or(0, |index| index + 1);
    Ok((
        normalized[..boundary].to_vec(),
        normalized[boundary..].to_vec(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lookup {
    Missing,
    Resource(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub owner: Owner,
    pub buckets: [Owner; 2],
    pub retained: Vec<Owner>,
    pub synchronization: u64,
}

impl State {
    pub fn validate(&self) -> Result<(), Stop> {
        validate_owner(&self.owner, 72)?;
        if self.owner.token != MANAGER || self.synchronization == 0 {
            return Err(unsupported(
                "resource manager lacks its static owner or host lock",
            ));
        }
        for (index, buckets) in self.buckets.iter().enumerate() {
            validate_owner(buckets, BUCKET_BYTES)?;
            let offset = index * 20;
            let end = buckets.token + BUCKET_BYTES as u64;
            if end > u64::from(u32::MAX)
                || u64::from(word(&self.owner.data, offset + 4)) != buckets.token
                || u64::from(word(&self.owner.data, offset + 8)) != end
                || u64::from(word(&self.owner.data, offset + 12)) != end
            {
                return Err(unsupported(
                    "resource manager bucket descriptor differs from ownership",
                ));
            }
        }
        for owner in &self.retained {
            validate_owner(owner, owner.data.len())?;
        }
        disjoint(self.owners().map(|owner| (owner.token, owner.data.len())))
    }

    fn owners(&self) -> impl Iterator<Item = &Owner> {
        std::iter::once(&self.owner)
            .chain(self.buckets.iter())
            .chain(self.retained.iter())
    }

    fn region(&self, address: u64, bytes: usize) -> Result<&[u8], Stop> {
        if address == 0 || address.checked_add(bytes as u64).is_none() {
            return Err(unsupported(
                "resource manager lookup has invalid native bounds",
            ));
        }
        let owner = self
            .owners()
            .find(|owner| {
                owner.token <= address
                    && address + bytes as u64 <= owner.token + owner.data.len() as u64
            })
            .ok_or_else(|| unsupported("resource manager lookup lacks complete owned memory"))?;
        let offset = (address - owner.token) as usize;
        Ok(&owner.data[offset..offset + bytes])
    }

    fn find(&self, map: u64, input: &[u8]) -> Result<Option<u64>, Stop> {
        let descriptor = self.region(map, 20)?;
        let beginning = word(descriptor, 4);
        let end = word(descriptor, 8);
        let capacity = word(descriptor, 12);
        if end <= beginning || capacity < end || (end - beginning) % 4 != 0 {
            return Err(unsupported(
                "resource manager lookup has invalid bucket bounds",
            ));
        }
        let buckets = self.region(u64::from(beginning), (capacity - beginning) as usize)?;
        let index = (hash(input) % ((end - beginning) / 4)) as usize;
        let mut next = u64::from(word(buckets, index * 4));
        let mut visited = BTreeSet::new();
        while next != 0 {
            if !visited.insert(next) {
                return Err(unsupported(
                    "resource manager lookup encountered a cyclic bucket chain",
                ));
            }
            let node = self.region(next, 20)?;
            let string_beginning = word(node, 4);
            let string_end = word(node, 8);
            let string_capacity = word(node, 12);
            if string_beginning == 0
                || string_end < string_beginning
                || string_capacity <= string_end
            {
                return Err(unsupported(
                    "resource manager node has invalid string bounds",
                ));
            }
            let backing = self.region(
                u64::from(string_beginning),
                (string_capacity - string_beginning) as usize,
            )?;
            let length = (string_end - string_beginning) as usize;
            if backing[length] != 0 {
                return Err(unsupported(
                    "resource manager node key lacks its actual terminator",
                ));
            }
            if &backing[..length] == input {
                return Ok(Some(next));
            }
            next = u64::from(word(node, 0));
        }
        Ok(None)
    }

    pub fn lookup(&self, directory: &[u8], basename: &[u8]) -> Result<Lookup, Stop> {
        self.validate()?;
        let Some(directory_node) = self.find(self.owner.token, directory)? else {
            return Ok(Lookup::Missing);
        };
        match self.find(directory_node + 16, basename)? {
            Some(node) => Ok(Lookup::Resource(node + 16)),
            None => Ok(Lookup::Missing),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Initialize(u8),
    Allocate(u8),
    Accept(u8),
    Publish(u8),
    Clear(u8),
    Flag,
    Synchronize,
    AcceptSynchronization,
    Complete,
}

impl Phase {
    fn position(self) -> Result<usize, Stop> {
        let position = match self {
            Self::Initialize(index) if index < 2 => usize::from(index) * 5,
            Self::Allocate(index) if index < 2 => usize::from(index) * 5 + 1,
            Self::Accept(index) if index < 2 => usize::from(index) * 5 + 2,
            Self::Publish(index) if index < 2 => usize::from(index) * 5 + 3,
            Self::Clear(index) if index < 2 => usize::from(index) * 5 + 4,
            Self::Flag => 10,
            Self::Synchronize => 11,
            Self::AcceptSynchronization => 12,
            Self::Complete => 13,
            _ => {
                return Err(unsupported(
                    "resource manager has an invalid constructor phase",
                ));
            }
        };
        Ok(position)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    owner: Owner,
    initial_owner: Vec<u8>,
    stack_bytes: [u8; 2],
    retained: Vec<Owner>,
    backings: [Option<Owner>; 2],
    initial_backings: [Option<Vec<u8>>; 2],
    pending: Option<Allocated>,
    synchronization: Option<u64>,
    phase: Phase,
}

impl Construction {
    pub fn new(owner: Owner, stack_bytes: [u8; 2], retained: Vec<Owner>) -> Result<Self, Stop> {
        let construction = Self {
            initial_owner: owner.data.clone(),
            owner,
            stack_bytes,
            retained,
            backings: [None, None],
            initial_backings: [None, None],
            pending: None,
            synchronization: None,
            phase: Phase::Initialize(0),
        };
        construction.validate()?;
        Ok(construction)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        validate_owner(&self.owner, 72)?;
        if self.owner.token != MANAGER || self.initial_owner.len() != 72 {
            return Err(unsupported(
                "resource manager requires its complete original static input",
            ));
        }
        let position = self.phase.position()?;
        let mut expected = Owner {
            token: MANAGER,
            data: self.initial_owner.clone(),
        };
        let mut spans = vec![(self.owner.token, self.owner.data.len())];
        for (index, backing) in self.backings.iter().enumerate() {
            let offset = index * 20;
            let stage = position.saturating_sub(index * 5);
            if stage >= 1 {
                expected.data[offset..offset + 3].fill(self.stack_bytes[index]);
                for field in [4, 8, 12, 16] {
                    write_word(&mut expected, offset + field, 0);
                }
            }
            if (stage >= 3) != backing.is_some()
                || backing.is_some() != self.initial_backings[index].is_some()
            {
                return Err(unsupported(
                    "resource manager lost its actual bucket construction",
                ));
            }
            if let Some(backing) = backing {
                validate_owner(backing, BUCKET_BYTES)?;
                let end = backing.token + BUCKET_BYTES as u64;
                if end > u64::from(u32::MAX) {
                    return Err(unsupported(
                        "resource manager bucket end exceeds native width",
                    ));
                }
                let initial = self.initial_backings[index].as_ref().unwrap();
                if initial.len() != BUCKET_BYTES
                    || (stage < 5 && &backing.data != initial)
                    || (stage >= 5 && backing.data.iter().any(|byte| *byte != 0))
                {
                    return Err(unsupported(
                        "resource manager bucket initialization differs",
                    ));
                }
                if stage >= 4 {
                    write_word(&mut expected, offset + 4, backing.token as u32);
                    write_word(&mut expected, offset + 8, backing.token as u32);
                    write_word(&mut expected, offset + 12, end as u32);
                }
                if stage >= 5 {
                    write_word(&mut expected, offset + 8, end as u32);
                }
                spans.push((backing.token, backing.data.len()));
            }
        }
        if position >= 11 {
            expected.data[44] = 0;
        }
        if expected != self.owner || (position >= 12) != self.synchronization.is_some() {
            return Err(unsupported(
                "resource manager constructor stores or lock phase differ",
            ));
        }
        if position == 13 && self.synchronization == Some(0) {
            return Err(unsupported(
                "resource manager accepted a null synchronization capability",
            ));
        }
        if matches!(self.phase, Phase::Accept(_)) != self.pending.is_some() {
            return Err(unsupported(
                "resource manager lost its pending allocator reply",
            ));
        }
        for owner in &self.retained {
            validate_owner(owner, owner.data.len())?;
            spans.push((owner.token, owner.data.len()));
        }
        disjoint(spans)
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<bool, Stop> {
        self.validate()?;
        match self.phase {
            Phase::Initialize(index) => {
                let offset = usize::from(index) * 20;
                self.owner.data[offset..offset + 3].fill(self.stack_bytes[usize::from(index)]);
                for field in [4, 8, 12, 16] {
                    write_word(&mut self.owner, offset + field, 0);
                }
                self.phase = Phase::Allocate(index);
            }
            Phase::Allocate(index) => {
                self.pending = Some(host.allocate_buckets(index, BUCKET_BYTES as u32)?);
                self.phase = Phase::Accept(index);
            }
            Phase::Accept(index) => {
                let Some(Allocated::Bytes { token, data }) = &self.pending else {
                    return Err(unsupported(
                        "resource manager allocator returned no owned buckets",
                    ));
                };
                let backing = Owner {
                    token: *token,
                    data: data.clone(),
                };
                validate_owner(&backing, BUCKET_BYTES)?;
                if backing.token + BUCKET_BYTES as u64 > u64::from(u32::MAX) {
                    return Err(unsupported(
                        "resource manager bucket end exceeds native width",
                    ));
                }
                disjoint(
                    std::iter::once(&self.owner)
                        .chain(self.backings.iter().flatten())
                        .chain(self.retained.iter())
                        .chain(std::iter::once(&backing))
                        .map(|owner| (owner.token, owner.data.len())),
                )?;
                self.initial_backings[usize::from(index)] = Some(backing.data.clone());
                self.backings[usize::from(index)] = Some(backing);
                self.pending = None;
                self.phase = Phase::Publish(index);
            }
            Phase::Publish(index) => {
                let offset = usize::from(index) * 20;
                let backing = self.backings[usize::from(index)].as_ref().unwrap();
                write_word(&mut self.owner, offset + 4, backing.token as u32);
                write_word(&mut self.owner, offset + 8, backing.token as u32);
                write_word(
                    &mut self.owner,
                    offset + 12,
                    (backing.token + BUCKET_BYTES as u64) as u32,
                );
                self.phase = Phase::Clear(index);
            }
            Phase::Clear(index) => {
                let backing = self.backings[usize::from(index)].as_mut().unwrap();
                backing.data.fill(0);
                write_word(
                    &mut self.owner,
                    usize::from(index) * 20 + 8,
                    (backing.token + BUCKET_BYTES as u64) as u32,
                );
                self.phase = if index == 0 {
                    Phase::Initialize(1)
                } else {
                    Phase::Flag
                };
            }
            Phase::Flag => {
                self.owner.data[44] = 0;
                self.phase = Phase::Synchronize;
            }
            Phase::Synchronize => {
                self.synchronization = Some(host.initialize_section(self.owner.token + 48, 24)?);
                self.phase = Phase::AcceptSynchronization;
            }
            Phase::AcceptSynchronization => {
                if self.synchronization == Some(0) {
                    return Err(unsupported(
                        "resource manager host returned a null lock capability",
                    ));
                }
                self.phase = Phase::Complete;
            }
            Phase::Complete => return Ok(true),
        }
        self.validate()?;
        Ok(self.phase == Phase::Complete)
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.phase != Phase::Complete {
            return Err(unsupported(
                "resource manager construction has not returned",
            ));
        }
        let [first, second] = self.backings;
        let state = State {
            owner: self.owner,
            buckets: [first.unwrap(), second.unwrap()],
            retained: self.retained,
            synchronization: self.synchronization.unwrap(),
        };
        state.validate()?;
        Ok(state)
    }
}

#[cfg(test)]
mod tests;
