use super::{handle, validate_owner, write_word};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Serialize};

pub trait Host {
    fn allocate_population(&mut self, bytes: u32) -> Result<Allocated, Stop>;
    fn free_null(&mut self) -> Result<(), Stop>;
}

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn word(owner: &Owner, offset: usize) -> u32 {
    u32::from_le_bytes(owner.data[offset..offset + 4].try_into().unwrap())
}

fn dimension(level: usize) -> u32 {
    let diameter = 2 * level as u32 + 1;
    let square = 4 * 12 * 12 - 9 * diameter * diameter;
    (1_u32..=12)
        .find(|count| (6 * count - 3).pow(2) >= square)
        .unwrap()
}

fn tail_width(index: usize) -> u32 {
    let radius_square = 12 * 12 - 5 * 5;
    let steps = (1_u32..=12)
        .find(|count| 18 * count * count >= radius_square)
        .unwrap();
    (2 * steps + 1) * (128 >> (2 - index))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    ParentAllocated,
    ParentConstructed,
    ParentPublished,
    DimensionReturned,
    FirstAllocated,
    FirstConstructed,
    SecondAllocated,
    SecondConstructed,
    FirstPublished,
    SecondPublished,
    LevelReturned,
    TailAllocated,
    TailConstructed,
    TailsReturned,
    LoadReturned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cut {
    pub ordinal: u8,
    pub kind: Kind,
}

fn cut(ordinal: u8) -> Cut {
    let kind = if ordinal < 92 {
        match ordinal % 23 {
            0 => Kind::ParentAllocated,
            1 => Kind::ParentConstructed,
            2 => Kind::ParentPublished,
            3 => Kind::DimensionReturned,
            22 => Kind::LevelReturned,
            position => match (position - 4) % 6 {
                0 => Kind::FirstAllocated,
                1 => Kind::FirstConstructed,
                2 => Kind::SecondAllocated,
                3 => Kind::SecondConstructed,
                4 => Kind::FirstPublished,
                _ => Kind::SecondPublished,
            },
        }
    } else {
        match ordinal {
            92 | 94 | 96 => Kind::TailAllocated,
            93 | 95 | 97 => Kind::TailConstructed,
            98 => Kind::TailsReturned,
            _ => Kind::LoadReturned,
        }
    };
    Cut { ordinal, kind }
}

fn allocation_bytes(ordinal: u8) -> Option<u32> {
    match cut(ordinal).kind {
        Kind::ParentAllocated => Some(24),
        Kind::FirstAllocated | Kind::SecondAllocated => Some(32),
        Kind::TailAllocated => Some(16),
        _ => None,
    }
}

fn publish(
    resources: &mut Owner,
    backings: &mut [Owner; 7],
    vector: usize,
    level: usize,
    token: u64,
) {
    write_word(&mut backings[vector], level * 4, token as u32);
    write_word(
        resources,
        vector * 12 + 4,
        backings[vector].token as u32 + (level as u32 + 1) * 4,
    );
}

fn prepare_tail(resources: &mut Owner, owners: &mut [Owner], index: usize) {
    let owner = &mut owners[28 + index];
    for offset in [0, 4, 8] {
        write_word(owner, offset, 0);
    }
    write_word(resources, 84 + index * 4, owner.token as u32);
}

fn offsets(owners: &mut [Owner]) -> u32 {
    let mut offset = 0;
    for level in 0..4 {
        let parent = &mut owners[level * 7];
        write_word(parent, 20, offset);
        offset += word(parent, 12) * word(parent, 16);
    }
    for second in 0..2 {
        for channel in 0..3 {
            for level in 0..4 {
                let child = &mut owners[level * 7 + 1 + channel * 2 + second];
                write_word(child, 24, offset);
                let scale = word(child, 12);
                offset += ((58 << (scale - 1)) + (8 << scale))
                    * (2 * word(child, 4) + 1)
                    * (16 << (scale - 1));
            }
        }
    }
    for tail in &mut owners[28..] {
        write_word(tail, 12, offset);
        let width = word(tail, 4);
        offset += (width / 2) * width;
    }
    offset
}

fn apply(ordinal: u8, resources: &mut Owner, backings: &mut [Owner; 7], owners: &mut [Owner]) {
    if ordinal < 92 {
        let level = usize::from(ordinal / 23);
        let position = ordinal % 23;
        let parent_index = level * 7;
        match position {
            1 => {
                let parent = &mut owners[parent_index];
                for offset in [0, 8, 12, 16] {
                    write_word(parent, offset, 0);
                }
                write_word(parent, 4, level as u32);
                write_word(parent, 8, dimension(level));
                write_word(parent, 12, dimension(level) << 6);
                write_word(parent, 16, 232);
            }
            2 => publish(resources, backings, 0, level, owners[parent_index].token),
            5..=21 => {
                let channel = usize::from((position - 4) / 6);
                let operation = (position - 4) % 6;
                let first = parent_index + 1 + channel * 2;
                let second = usize::from(operation == 3 || operation == 5);
                if operation == 1 || operation == 3 {
                    let parent = owners[parent_index].token;
                    let child = &mut owners[first + second];
                    for (offset, value) in [
                        (0, 0),
                        (4, dimension(level) - 1),
                        (8, level as u32),
                        (12, channel as u32 + 1),
                        (16, 1 << (channel + 1)),
                        (20, second as u32),
                        (28, parent as u32),
                    ] {
                        write_word(child, offset, value);
                    }
                } else if operation == 4 || operation == 5 {
                    publish(
                        resources,
                        backings,
                        1 + channel + second * 3,
                        level,
                        owners[first + second].token,
                    );
                }
            }
            _ => {}
        }
    } else if matches!(ordinal, 93 | 95 | 97) {
        let index = usize::from((ordinal - 93) / 2);
        prepare_tail(resources, owners, index);
        let owner = &mut owners[28 + index];
        write_word(owner, 4, tail_width(index));
        write_word(owner, 8, 128 >> (2 - index));
    } else if ordinal == 99 {
        offsets(owners);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    handle: handle::State,
    resources: Owner,
    backings: [Owner; 7],
    owners: Vec<Owner>,
    initial_owners: Vec<Owner>,
    cursor: u8,
    pending: Option<Allocated>,
    tail_prepared: bool,
}

impl Construction {
    pub fn new(handle: handle::State) -> Result<Self, Stop> {
        handle.validate()?;
        let source = &handle.opened().context().resources.resources;
        if source.context.level_bits != 12_f32.to_bits() {
            return Err(unsupported(
                "resource population only qualifies the original level-12 file-backed profile",
            ));
        }
        let backings: [Owner; 7] = source
            .backings
            .iter()
            .cloned()
            .collect::<Option<Vec<_>>>()
            .and_then(|backings| backings.try_into().ok())
            .ok_or_else(|| {
                unsupported("resource population lacks all seven genuine reservations")
            })?;
        let execution = Self {
            resources: handle.resources().clone(),
            handle,
            backings,
            owners: vec![],
            initial_owners: vec![],
            cursor: 0,
            pending: None,
            tail_prepared: false,
        };
        execution.validate()?;
        Ok(execution)
    }

    pub fn handle(&self) -> &handle::State {
        &self.handle
    }
    pub fn resources(&self) -> &Owner {
        &self.resources
    }
    pub fn backings(&self) -> &[Owner; 7] {
        &self.backings
    }
    pub fn owners(&self) -> &[Owner] {
        &self.owners
    }
    pub fn cut(&self) -> Option<Cut> {
        (self.cursor > 0 && self.pending.is_none() && !self.tail_prepared)
            .then(|| cut(self.cursor - 1))
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.handle.validate()?;
        if self.cursor > 100
            || self.owners.len() != self.initial_owners.len()
            || self.pending.is_some()
                && (self.cursor == 100 || allocation_bytes(self.cursor).is_none())
            || self.tail_prepared && !matches!(self.cursor, 93 | 95 | 97)
        {
            return Err(unsupported(
                "resource population phase or pending ownership differs",
            ));
        }
        let source = &self.handle.opened().context().resources.resources;
        if source.context.level_bits != 12_f32.to_bits() {
            return Err(unsupported("resource population profile changed"));
        }
        let mut expected_resources = self.handle.resources().clone();
        let mut expected_backings: [Owner; 7] = source
            .backings
            .iter()
            .cloned()
            .collect::<Option<Vec<_>>>()
            .and_then(|backings| backings.try_into().ok())
            .ok_or_else(|| unsupported("resource population lost its original reservations"))?;
        let mut spans = self.handle.retained_spans()?;
        spans.retain(|span| {
            *span != (expected_resources.token, 100)
                && !expected_backings
                    .iter()
                    .any(|owner| *span == (owner.token, owner.data.len()))
        });
        for owner in &expected_backings {
            validate_owner(owner, 16)?;
        }
        let mut expected_owners = vec![];
        for ordinal in 0..self.cursor {
            if let Some(bytes) = allocation_bytes(ordinal) {
                let owner = self
                    .initial_owners
                    .get(expected_owners.len())
                    .ok_or_else(|| {
                        unsupported("resource population lost an actual allocation image")
                    })?;
                validate_owner(owner, bytes as usize)?;
                expected_owners.push(owner.clone());
            }
            apply(
                ordinal,
                &mut expected_resources,
                &mut expected_backings,
                &mut expected_owners,
            );
        }
        if expected_owners.len() != self.initial_owners.len() {
            return Err(unsupported("resource population contains unreached owners"));
        }
        if self.tail_prepared {
            prepare_tail(
                &mut expected_resources,
                &mut expected_owners,
                usize::from((self.cursor - 93) / 2),
            );
        }
        if self.resources != expected_resources
            || self.backings != expected_backings
            || self.owners != expected_owners
        {
            return Err(unsupported(
                "resource population bytes differ from actual reached transitions",
            ));
        }
        if self.cursor == 100 {
            let mut owners = self.owners.clone();
            if offsets(&mut owners) != word(self.handle.owner(), 16) {
                return Err(unsupported(
                    "resource layout does not span the actual opened file size",
                ));
            }
        }
        spans.push((self.resources.token, 100));
        spans.extend(
            self.backings
                .iter()
                .chain(&self.owners)
                .map(|owner| (owner.token, owner.data.len())),
        );
        if let Some(Allocated::Bytes { token, data }) = &self.pending {
            validate_owner(
                &Owner {
                    token: *token,
                    data: data.clone(),
                },
                allocation_bytes(self.cursor).unwrap() as usize,
            )?;
            spans.push((*token, data.len()));
        }
        spans.sort_unstable();
        if spans
            .windows(2)
            .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
        {
            return Err(unsupported(
                "resource population aliases retained mutable ownership",
            ));
        }
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        if self.cursor == 100 {
            return Ok(self.cut());
        }
        if let Some(bytes) = allocation_bytes(self.cursor) {
            if self.pending.is_none() {
                self.pending = Some(host.allocate_population(bytes)?);
                return Ok(None);
            }
            let owner = match self.pending.as_ref().unwrap() {
                Allocated::Bytes { token, data } => Owner {
                    token: *token,
                    data: data.clone(),
                },
                Allocated::Null => {
                    return Err(Stop::Unavailable(
                        "resource population allocation returned null".into(),
                    ));
                }
            };
            validate_owner(&owner, bytes as usize)?;
            self.initial_owners.push(owner.clone());
            self.owners.push(owner);
            self.pending = None;
        } else if matches!(self.cursor, 93 | 95 | 97) {
            if !self.tail_prepared {
                prepare_tail(
                    &mut self.resources,
                    &mut self.owners,
                    usize::from((self.cursor - 93) / 2),
                );
                self.tail_prepared = true;
            }
            host.free_null()?;
            self.tail_prepared = false;
        }
        apply(
            self.cursor,
            &mut self.resources,
            &mut self.backings,
            &mut self.owners,
        );
        let reached = cut(self.cursor);
        self.cursor += 1;
        self.validate()?;
        Ok(Some(reached))
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cursor != 100 {
            return Err(unsupported("resource population load has not returned"));
        }
        Ok(State { execution: self })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    execution: Construction,
}

impl State {
    pub(crate) fn retained_spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.validate()?;
        let mut spans = self.handle().retained_spans()?;
        let original = &self.handle().opened().context().resources.resources;
        spans.retain(|span| {
            *span != (original.owner.token, original.owner.data.len())
                && !original
                    .backings
                    .iter()
                    .flatten()
                    .any(|backing| *span == (backing.token, backing.data.len()))
        });
        spans.push((self.resources().token, self.resources().data.len()));
        spans.extend(
            self.backings()
                .iter()
                .chain(self.owners())
                .map(|owner| (owner.token, owner.data.len())),
        );
        spans.sort_unstable();
        if spans
            .windows(2)
            .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
        {
            return Err(unsupported(
                "returned population aliases retained ownership",
            ));
        }
        Ok(spans)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.execution.validate()?;
        if self.execution.cursor != 100 {
            return Err(unsupported("resource population state is not returned"));
        }
        Ok(())
    }
    pub fn handle(&self) -> &handle::State {
        self.execution.handle()
    }
    pub fn resources(&self) -> &Owner {
        self.execution.resources()
    }
    pub fn backings(&self) -> &[Owner; 7] {
        self.execution.backings()
    }
    pub fn owners(&self) -> &[Owner] {
        self.execution.owners()
    }
}
