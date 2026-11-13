use crate::native_world::{validate_owner, write_word};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, MAX_ALLOCATION_BYTES, Stop};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Construct(u32),
    Grow(u32),
}

impl Request {
    pub fn bytes(self) -> u32 {
        match self {
            Self::Construct(bytes) | Self::Grow(bytes) => bytes,
        }
    }
}

pub trait Host {
    fn allocate_string(&mut self, request: Request) -> Result<Allocated, Stop>;
    fn free_string(&mut self, token: u64) -> Result<(), Stop>;
}

fn extent(bytes: usize) -> Result<u32, Stop> {
    if bytes == 0 || bytes > MAX_ALLOCATION_BYTES {
        return Err(Stop::Unsupported(
            "native resource string capacity is unsupported".into(),
        ));
    }
    u32::try_from(bytes).map_err(|_| {
        Stop::Unsupported("native resource string capacity exceeds native width".into())
    })
}

fn overlap(first: &Owner, second: &Owner) -> bool {
    first.token < second.token + second.data.len() as u64
        && second.token < first.token + first.data.len() as u64
}

fn validate_backing(owner: &Owner, backing: &Owner, bytes: usize) -> Result<(), Stop> {
    extent(bytes)?;
    validate_owner(owner, 12)?;
    validate_owner(backing, bytes)?;
    if backing.token + bytes as u64 > u64::from(u32::MAX) || overlap(owner, backing) {
        return Err(Stop::Unsupported(
            "resource string backing aliases its descriptor or exceeds native end-pointer width"
                .into(),
        ));
    }
    Ok(())
}

fn word(owner: &Owner, offset: usize) -> u32 {
    u32::from_le_bytes(owner.data[offset..offset + 4].try_into().unwrap())
}

fn header(owner: &mut Owner, backing: &Owner, length: usize) {
    for (offset, value) in [
        (0, backing.token as u32),
        (4, (backing.token + length as u64) as u32),
        (8, (backing.token + backing.data.len() as u64) as u32),
    ] {
        write_word(owner, offset, value);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub owner: Owner,
    pub backing: Owner,
}

impl State {
    pub fn validate(&self) -> Result<(), Stop> {
        validate_backing(&self.owner, &self.backing, self.backing.data.len())?;
        let beginning = word(&self.owner, 0);
        let end = word(&self.owner, 4);
        let capacity = word(&self.owner, 8);
        if u64::from(beginning) != self.backing.token
            || end < beginning
            || end >= capacity
            || u64::from(capacity) != self.backing.token + self.backing.data.len() as u64
            || self.backing.data[(end - beginning) as usize] != 0
        {
            return Err(Stop::Unsupported(
                "resource string bounds, terminator or retained backing differ".into(),
            ));
        }
        Ok(())
    }

    pub fn bytes(&self) -> Result<&[u8], Stop> {
        self.validate()?;
        Ok(&self.backing.data[..(word(&self.owner, 4) - word(&self.owner, 0)) as usize])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ConstructorPhase {
    Clear(u8),
    Allocate,
    Accept,
    Publish(u8),
    Copy,
    End,
    Terminate,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    owner: Owner,
    initial_owner: Vec<u8>,
    input: Option<Vec<u8>>,
    phase: ConstructorPhase,
    pending: Option<Allocated>,
    backing: Option<Owner>,
    initial_backing: Option<Vec<u8>>,
}

impl Construction {
    pub(super) fn initial_owner(&self) -> &[u8] {
        &self.initial_owner
    }

    pub(super) fn owner(&self) -> &Owner {
        &self.owner
    }

    pub(super) fn input(&self) -> Option<&[u8]> {
        self.input.as_deref()
    }

    pub(super) fn spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.validate()?;
        let mut spans = vec![(self.owner.token, self.owner.data.len())];
        if let Some(backing) = &self.backing {
            spans.push((backing.token, backing.data.len()));
        }
        if let Some(Allocated::Bytes { token, data }) = &self.pending {
            validate_owner(
                &Owner {
                    token: *token,
                    data: data.clone(),
                },
                data.len(),
            )?;
            spans.push((*token, data.len()));
        }
        Ok(spans)
    }

    /// The enclosing constructor already owns the complete containing object.
    /// Return allocations only, without subtracting spans by address/value.
    pub(crate) fn allocation_spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        let mut spans = self.spans()?;
        if spans.first().copied() != Some((self.owner.token, 12)) {
            return Err(Stop::Unsupported(
                "string descriptor span is not first".into(),
            ));
        }
        spans.remove(0);
        Ok(spans)
    }

    pub(crate) fn returned_parts(&self) -> Result<Option<(&Owner, &Owner)>, Stop> {
        self.validate()?;
        Ok((self.phase == ConstructorPhase::Complete)
            .then(|| (&self.owner, self.backing.as_ref().unwrap())))
    }

    pub fn new(owner: Owner, input: Option<Vec<u8>>) -> Result<Self, Stop> {
        validate_owner(&owner, 12)?;
        let result = Self {
            initial_owner: owner.data.clone(),
            owner,
            input,
            phase: ConstructorPhase::Clear(0),
            pending: None,
            backing: None,
            initial_backing: None,
        };
        result.validate()?;
        Ok(result)
    }

    fn requested(&self) -> Result<u32, Stop> {
        extent(match &self.input {
            Some(input) => input
                .len()
                .checked_add(1)
                .ok_or_else(|| Stop::Unsupported("resource string length overflow".into()))?,
            None => 8,
        })
    }

    fn validate(&self) -> Result<(), Stop> {
        validate_owner(&self.owner, 12)?;
        if self.initial_owner.len() != 12 {
            return Err(Stop::Unsupported(
                "resource string descriptor input is incomplete".into(),
            ));
        }
        let bytes = self.requested()? as usize;
        if self.input.is_none()
            && matches!(self.phase, ConstructorPhase::Copy | ConstructorPhase::End)
        {
            return Err(Stop::Unsupported(
                "default resource string cannot restore a copy-constructor phase".into(),
            ));
        }
        let accepted = matches!(
            self.phase,
            ConstructorPhase::Publish(_)
                | ConstructorPhase::Copy
                | ConstructorPhase::End
                | ConstructorPhase::Terminate
                | ConstructorPhase::Complete
        );
        if accepted != self.backing.is_some()
            || accepted != self.initial_backing.is_some()
            || (self.phase == ConstructorPhase::Accept) != self.pending.is_some()
        {
            return Err(Stop::Unsupported(
                "resource string constructor lost phase ownership".into(),
            ));
        }
        let mut expected = self.initial_owner.clone();
        if let ConstructorPhase::Clear(count) = self.phase {
            if count >= 3 {
                return Err(Stop::Unsupported(
                    "invalid resource string clear phase".into(),
                ));
            }
            expected[..count as usize * 4].fill(0);
        } else {
            expected.fill(0);
        }
        if let Some(backing) = &self.backing {
            validate_backing(&self.owner, backing, bytes)?;
            let count = match self.phase {
                ConstructorPhase::Publish(count) if count < 3 => count as usize,
                ConstructorPhase::Publish(_) => {
                    return Err(Stop::Unsupported(
                        "invalid resource string publication".into(),
                    ));
                }
                _ => 3,
            };
            let length = if matches!(
                self.phase,
                ConstructorPhase::Terminate | ConstructorPhase::Complete
            ) {
                self.input.as_ref().map_or(0, Vec::len)
            } else {
                0
            };
            for (index, value) in [
                backing.token,
                backing.token + length as u64,
                backing.token + bytes as u64,
            ]
            .into_iter()
            .enumerate()
            .take(count)
            {
                expected[index * 4..index * 4 + 4].copy_from_slice(&(value as u32).to_le_bytes());
            }
            let mut payload = self.initial_backing.clone().unwrap();
            if payload.len() != bytes {
                return Err(Stop::Unsupported(
                    "resource string allocator input is incomplete".into(),
                ));
            }
            if matches!(
                self.phase,
                ConstructorPhase::End | ConstructorPhase::Terminate | ConstructorPhase::Complete
            ) {
                if let Some(input) = &self.input {
                    payload[..input.len()].copy_from_slice(input);
                }
            }
            if self.phase == ConstructorPhase::Complete {
                payload[length] = 0;
            }
            if backing.data != payload {
                return Err(Stop::Unsupported(
                    "resource string constructor changed copied or untouched bytes".into(),
                ));
            }
        }
        if self.owner.data != expected {
            return Err(Stop::Unsupported(
                "resource string constructor header differs from its phase".into(),
            ));
        }
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<bool, Stop> {
        self.validate()?;
        match self.phase {
            ConstructorPhase::Clear(index) => {
                write_word(&mut self.owner, index as usize * 4, 0);
                self.phase = if index == 2 {
                    ConstructorPhase::Allocate
                } else {
                    ConstructorPhase::Clear(index + 1)
                };
            }
            ConstructorPhase::Allocate => {
                self.pending = Some(host.allocate_string(Request::Construct(self.requested()?))?);
                self.phase = ConstructorPhase::Accept;
            }
            ConstructorPhase::Accept => {
                let backing = match self.pending.as_ref().unwrap() {
                    Allocated::Bytes { token, data } => Owner {
                        token: *token,
                        data: data.clone(),
                    },
                    Allocated::Null => {
                        return Err(Stop::Unavailable(
                            "resource string constructor has no writable allocation".into(),
                        ));
                    }
                };
                validate_backing(&self.owner, &backing, self.requested()? as usize)?;
                self.initial_backing = Some(backing.data.clone());
                self.backing = Some(backing);
                self.pending = None;
                self.phase = ConstructorPhase::Publish(0);
            }
            ConstructorPhase::Publish(index) => {
                let backing = self.backing.as_ref().unwrap();
                let value = backing.token
                    + if index == 2 {
                        backing.data.len() as u64
                    } else {
                        0
                    };
                write_word(&mut self.owner, index as usize * 4, value as u32);
                self.phase = if index < 2 {
                    ConstructorPhase::Publish(index + 1)
                } else if self.input.is_some() {
                    ConstructorPhase::Copy
                } else {
                    ConstructorPhase::Terminate
                };
            }
            ConstructorPhase::Copy => {
                let input = self.input.as_ref().unwrap();
                self.backing.as_mut().unwrap().data[..input.len()].copy_from_slice(input);
                self.phase = ConstructorPhase::End;
            }
            ConstructorPhase::End => {
                let backing = self.backing.as_ref().unwrap();
                write_word(
                    &mut self.owner,
                    4,
                    (backing.token + self.input.as_ref().unwrap().len() as u64) as u32,
                );
                self.phase = ConstructorPhase::Terminate;
            }
            ConstructorPhase::Terminate => {
                self.backing.as_mut().unwrap().data[self.input.as_ref().map_or(0, Vec::len)] = 0;
                self.phase = ConstructorPhase::Complete;
            }
            ConstructorPhase::Complete => {}
        }
        self.validate()?;
        Ok(self.phase == ConstructorPhase::Complete)
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.phase != ConstructorPhase::Complete {
            return Err(Stop::Unavailable(
                "resource string constructor has not returned".into(),
            ));
        }
        let result = State {
            owner: self.owner,
            backing: self.backing.unwrap(),
        };
        result.validate()?;
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Assign(Vec<u8>),
    Append(Vec<u8>),
    CanonicalizeAscii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum EditPhase {
    Prefix,
    Append,
    Allocate,
    Accept,
    Release,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    initial: State,
    state: State,
    operation: Operation,
    phase: EditPhase,
    pending: Option<Allocated>,
    replacement: Option<Owner>,
    initial_replacement: Option<Vec<u8>>,
}

impl Edit {
    pub(super) fn initial(&self) -> &State {
        &self.initial
    }

    pub(super) fn operation(&self) -> &Operation {
        &self.operation
    }

    pub(super) fn spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.validate()?;
        let mut spans = vec![
            (self.state.owner.token, self.state.owner.data.len()),
            (self.state.backing.token, self.state.backing.data.len()),
        ];
        if let Some(replacement) = &self.replacement {
            spans.push((replacement.token, replacement.data.len()));
        }
        if let Some(Allocated::Bytes { token, data }) = &self.pending {
            validate_owner(
                &Owner {
                    token: *token,
                    data: data.clone(),
                },
                data.len(),
            )?;
            spans.push((*token, data.len()));
        }
        Ok(spans)
    }

    pub fn new(state: State, operation: Operation) -> Result<Self, Stop> {
        state.validate()?;
        let result = Self {
            initial: state.clone(),
            state,
            operation,
            phase: EditPhase::Prefix,
            pending: None,
            replacement: None,
            initial_replacement: None,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn replacement(&self) -> Option<&Owner> {
        self.replacement.as_ref()
    }

    fn target(&self) -> Result<Vec<u8>, Stop> {
        let result = match &self.operation {
            Operation::Assign(input) => input.clone(),
            Operation::Append(input) => {
                let mut result = self.initial.bytes()?.to_vec();
                result.extend_from_slice(input);
                result
            }
            Operation::CanonicalizeAscii => {
                let input = self.initial.bytes()?;
                if !input.is_ascii() {
                    return Err(Stop::Unsupported(
                        "resource normalization requires the qualified ASCII C-locale input".into(),
                    ));
                }
                input
                    .iter()
                    .map(|byte| {
                        if *byte == b'/' {
                            b'\\'
                        } else {
                            byte.to_ascii_lowercase()
                        }
                    })
                    .collect()
            }
        };
        extent(
            result.len().checked_add(1).ok_or_else(|| {
                Stop::Unsupported("resource string target length overflow".into())
            })?,
        )?;
        Ok(result)
    }

    fn requested(&self, target: &[u8]) -> Result<u32, Stop> {
        let length = self.initial.bytes()?.len();
        extent(
            length
                .checked_add(length.max(target.len() - length))
                .and_then(|bytes| bytes.checked_add(1))
                .ok_or_else(|| Stop::Unsupported("resource string growth overflow".into()))?,
        )
    }

    fn validate_replacement(&self, replacement: &Owner, bytes: usize) -> Result<(), Stop> {
        validate_backing(&self.state.owner, replacement, bytes)?;
        if overlap(&self.initial.backing, replacement) {
            return Err(Stop::Unsupported(
                "resource string growth aliases its still-owned old backing".into(),
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), Stop> {
        self.initial.validate()?;
        self.state.validate()?;
        let target = self.target()?;
        let grows = target.len() >= self.initial.backing.data.len();
        if matches!(self.operation, Operation::Assign(_))
            && target.len() <= self.initial.bytes()?.len()
            && !matches!(self.phase, EditPhase::Prefix | EditPhase::Complete)
        {
            return Err(Stop::Unsupported(
                "short resource string assignment cannot restore an append phase".into(),
            ));
        }
        if (self.phase == EditPhase::Accept) != self.pending.is_some()
            || (self.phase == EditPhase::Release) != self.replacement.is_some()
            || self.initial_replacement.is_some()
                != (self.phase == EditPhase::Release
                    || (self.phase == EditPhase::Complete && grows))
            || (matches!(
                self.phase,
                EditPhase::Allocate | EditPhase::Accept | EditPhase::Release
            ) && !grows)
            || (self.phase != EditPhase::Prefix
                && self.phase != EditPhase::Complete
                && matches!(self.operation, Operation::CanonicalizeAscii))
        {
            return Err(Stop::Unsupported(
                "resource string edit lost exact stage ownership".into(),
            ));
        }
        let mut expected = self.initial.clone();
        if self.phase != EditPhase::Prefix && matches!(self.operation, Operation::Assign(_)) {
            let copied = target.len().min(self.initial.bytes()?.len());
            expected.backing.data[..copied].copy_from_slice(&target[..copied]);
        }
        if self.phase == EditPhase::Release || (self.phase == EditPhase::Complete && grows) {
            let backing = self.replacement.as_ref().unwrap_or(&self.state.backing);
            let bytes = self.requested(&target)? as usize;
            self.validate_replacement(backing, bytes)?;
            let mut data = self.initial_replacement.clone().unwrap();
            if data.len() != bytes {
                return Err(Stop::Unsupported(
                    "resource string replacement input is incomplete".into(),
                ));
            }
            data[..target.len()].copy_from_slice(&target);
            data[target.len()] = 0;
            if backing.data != data {
                return Err(Stop::Unsupported(
                    "resource string growth changed copied or untouched bytes".into(),
                ));
            }
            if self.phase == EditPhase::Complete {
                expected.backing = backing.clone();
            }
        }
        if self.phase == EditPhase::Complete {
            expected.backing.data[..target.len()].copy_from_slice(&target);
            expected.backing.data[target.len()] = 0;
            header(&mut expected.owner, &expected.backing, target.len());
        }
        if self.state != expected {
            return Err(Stop::Unsupported(
                "resource string edit bytes or descriptor differ from actual operation".into(),
            ));
        }
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<bool, Stop> {
        self.validate()?;
        let target = self.target()?;
        match self.phase {
            EditPhase::Prefix => {
                if matches!(self.operation, Operation::CanonicalizeAscii) {
                    self.state.backing.data[..target.len()].copy_from_slice(&target);
                    self.phase = EditPhase::Complete;
                } else if matches!(self.operation, Operation::Assign(_)) {
                    let copied = target.len().min(self.initial.bytes()?.len());
                    self.state.backing.data[..copied].copy_from_slice(&target[..copied]);
                    if target.len() <= self.initial.bytes()?.len() {
                        self.state.backing.data[target.len()] = 0;
                        header(&mut self.state.owner, &self.state.backing, target.len());
                        self.phase = EditPhase::Complete;
                    } else {
                        self.phase = EditPhase::Append;
                    }
                } else {
                    self.phase = EditPhase::Append;
                }
            }
            EditPhase::Append => {
                if target.len() < self.state.backing.data.len() {
                    let length = self.initial.bytes()?.len();
                    self.state.backing.data[length..target.len()]
                        .copy_from_slice(&target[length..]);
                    self.state.backing.data[target.len()] = 0;
                    header(&mut self.state.owner, &self.state.backing, target.len());
                    self.phase = EditPhase::Complete;
                } else {
                    self.phase = EditPhase::Allocate;
                }
            }
            EditPhase::Allocate => {
                self.pending = Some(host.allocate_string(Request::Grow(self.requested(&target)?))?);
                self.phase = EditPhase::Accept;
            }
            EditPhase::Accept => {
                let mut backing = match self.pending.as_ref().unwrap() {
                    Allocated::Bytes { token, data } => Owner {
                        token: *token,
                        data: data.clone(),
                    },
                    Allocated::Null => {
                        return Err(Stop::Unavailable(
                            "resource string growth has no writable allocator response".into(),
                        ));
                    }
                };
                self.validate_replacement(&backing, self.requested(&target)? as usize)?;
                self.initial_replacement = Some(backing.data.clone());
                backing.data[..target.len()].copy_from_slice(&target);
                backing.data[target.len()] = 0;
                self.replacement = Some(backing);
                self.pending = None;
                self.phase = EditPhase::Release;
            }
            EditPhase::Release => {
                host.free_string(self.state.backing.token)?;
                self.state.backing = self.replacement.take().unwrap();
                header(&mut self.state.owner, &self.state.backing, target.len());
                self.phase = EditPhase::Complete;
            }
            EditPhase::Complete => {}
        }
        self.validate()?;
        Ok(self.phase == EditPhase::Complete)
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.phase != EditPhase::Complete {
            return Err(Stop::Unavailable(
                "resource string edit has not returned".into(),
            ));
        }
        Ok(self.state)
    }
}

#[cfg(test)]
mod tests;
