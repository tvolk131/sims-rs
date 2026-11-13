use super::file;
use crate::native_world::{validate_owner, write_word};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Serialize};

mod wide;
use wide::Kind;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocaleRequest {
    Query,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocaleReply {
    Value(Vec<u8>),
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Opened {
    File(u32),
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Seeked {
    Position(u64),
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequest {
    pub filename: Vec<u16>,
    pub access: u32,
    pub sharing: u32,
    pub security: u32,
    pub disposition: u32,
    pub flags: u32,
    pub template: u32,
}

pub trait Host {
    fn allocate_wide(&mut self, request: Request) -> Result<Allocated, Stop>;
    fn free_wide(&mut self, backing: &Owner) -> Result<(), Stop>;
    fn locale(&mut self, request: LocaleRequest) -> Result<LocaleReply, Stop>;
    fn open_readonly(&mut self, request: &OpenRequest) -> Result<Opened, Stop>;
    fn seek_current(&mut self, capability: u32) -> Result<Seeked, Stop>;
}

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn disjoint(mut spans: Vec<(u64, usize)>) -> Result<(), Stop> {
    spans.sort_unstable();
    if spans
        .windows(2)
        .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
    {
        return Err(unsupported(
            "file opening aliases retained mutable ownership",
        ));
    }
    Ok(())
}

fn word(owner: &Owner) -> u32 {
    u32::from_le_bytes(owner.data[..4].try_into().unwrap())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incoming {
    pub output: Owner,
    pub local: Owner,
    pub replacement: Owner,
    pub buffer: Owner,
    pub buffer_units: u32,
    pub encoding: Owner,
    pub wide_dispatch: Owner,
    pub arguments: [u32; 3],
}

impl Incoming {
    fn validate(&self) -> Result<(), Stop> {
        for owner in [&self.output, &self.local, &self.replacement] {
            validate_owner(owner, 12)?;
        }
        validate_owner(&self.buffer, 1024)?;
        validate_owner(&self.encoding, 4)?;
        validate_owner(&self.wide_dispatch, 4)?;
        if self.buffer_units != 512
            || self.arguments != [1, 2, 1]
            || ![0xfffffffd, 0].contains(&word(&self.encoding))
            || word(&self.wide_dispatch) == 0
        {
            return Err(unsupported(
                "file opening requires the reached bounded C-locale readonly-wide profile",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    OpenEntry,
    ConversionEntry,
    LocalInput,
    LocalReady,
    BoundedConversionEntry,
    BoundedConversionReturn,
    LocalAssigned,
    OutputCopied,
    LocalReleased,
    ReplacementInput,
    ReplacementReady,
    ShortcutChecked,
    ReadonlyEntry,
    ReadonlyReturn,
    ReplacementReleased,
    OutputReleased,
    FieldsPublished,
    Returned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Phase {
    OpenEntry,
    ConversionEntry,
    LocalInput,
    LocalAllocate,
    LocalAccept,
    LocalReady,
    ConversionReady,
    LocaleQuery,
    LocaleQueryAccept,
    LocaleDefault,
    LocaleDefaultAccept,
    Convert,
    Converted,
    GrowthAllocate,
    GrowthAccept,
    GrowthRelease,
    LocalAssigned,
    OutputAllocate,
    OutputAccept,
    OutputCopied,
    LocalReleased,
    ReplacementInput,
    ReplacementAllocate,
    ReplacementAccept,
    ReplacementReady,
    ShortcutChecked,
    ReadonlyEntry,
    OpenAccept,
    OpenReturned,
    ReplacementReleased,
    OutputReleased,
    Seek,
    SeekAccept,
    FieldsPublished,
    Returned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    closed: file::State,
    incoming: Incoming,
    owner: Owner,
    encoding: Owner,
    buffer: Owner,
    local: wide::String,
    output: wide::String,
    replacement: wide::String,
    growth: Option<wide::String>,
    pending_wide: Option<Allocated>,
    pending_locale: Option<LocaleReply>,
    pending_open: Option<Opened>,
    pending_seek: Option<Seeked>,
    capability: Option<u32>,
    position: Option<u32>,
    phase: Phase,
}

impl Execution {
    pub fn new(closed: file::State, incoming: Incoming) -> Result<Self, Stop> {
        closed.validate()?;
        incoming.validate()?;
        let execution = Self {
            owner: closed.owner.clone(),
            encoding: incoming.encoding.clone(),
            buffer: incoming.buffer.clone(),
            local: wide::String::new(incoming.local.clone())?,
            output: wide::String::new(incoming.output.clone())?,
            replacement: wide::String::new(incoming.replacement.clone())?,
            closed,
            incoming,
            growth: None,
            pending_wide: None,
            pending_locale: None,
            pending_open: None,
            pending_seek: None,
            capability: None,
            position: None,
            phase: Phase::OpenEntry,
        };
        execution.validate()?;
        Ok(execution)
    }

    pub fn owner(&self) -> &Owner {
        &self.owner
    }
    pub fn context(&self) -> &file::Context {
        &self.closed.context
    }
    pub fn encoding(&self) -> &Owner {
        &self.encoding
    }
    pub fn buffer(&self) -> &Owner {
        &self.buffer
    }
    pub fn local_descriptor(&self) -> &Owner {
        &self.local.owner
    }
    pub fn output_descriptor(&self) -> &Owner {
        &self.output.owner
    }
    pub fn replacement_descriptor(&self) -> &Owner {
        &self.replacement.owner
    }
    pub fn local_backing(&self) -> Option<&Owner> {
        self.local.backing.as_ref()
    }
    pub fn output_backing(&self) -> Option<&Owner> {
        self.output.backing.as_ref()
    }
    pub fn replacement_backing(&self) -> Option<&Owner> {
        self.replacement.backing.as_ref()
    }
    pub fn capability(&self) -> Option<u32> {
        self.capability
    }

    pub fn conversion_units(&self) -> u32 {
        if self.phase >= Phase::Converted {
            self.local_path().len() as u32 / 2
        } else {
            512
        }
    }

    fn local_path(&self) -> Vec<u8> {
        self.closed.filename_backing.data[..self.closed.filename_backing.data.len() - 1]
            .iter()
            .flat_map(|byte| u16::from(*byte).to_le_bytes())
            .collect()
    }

    fn allocation_bytes(&self) -> Option<usize> {
        match self.phase {
            Phase::LocalAccept | Phase::ReplacementAccept => Some(16),
            Phase::GrowthAccept | Phase::OutputAccept => Some(self.local_path().len() + 2),
            _ => None,
        }
    }

    fn allocation(&self) -> Result<Owner, Stop> {
        match self
            .pending_wide
            .as_ref()
            .ok_or_else(|| unsupported("wide allocation reply absent"))?
        {
            Allocated::Bytes { token, data } => Ok(Owner {
                token: *token,
                data: data.clone(),
            }),
            Allocated::Null => Err(Stop::Unavailable("wide allocation returned null".into())),
        }
    }

    pub fn cut(&self) -> Option<Cut> {
        Some(match self.phase {
            Phase::OpenEntry => Cut::OpenEntry,
            Phase::ConversionEntry => Cut::ConversionEntry,
            Phase::LocalInput => Cut::LocalInput,
            Phase::LocalReady => Cut::LocalReady,
            Phase::ConversionReady => Cut::BoundedConversionEntry,
            Phase::Converted => Cut::BoundedConversionReturn,
            Phase::LocalAssigned => Cut::LocalAssigned,
            Phase::OutputCopied => Cut::OutputCopied,
            Phase::LocalReleased => Cut::LocalReleased,
            Phase::ReplacementInput => Cut::ReplacementInput,
            Phase::ReplacementReady => Cut::ReplacementReady,
            Phase::ShortcutChecked => Cut::ShortcutChecked,
            Phase::ReadonlyEntry => Cut::ReadonlyEntry,
            Phase::OpenReturned => Cut::ReadonlyReturn,
            Phase::ReplacementReleased => Cut::ReplacementReleased,
            Phase::OutputReleased => Cut::OutputReleased,
            Phase::FieldsPublished => Cut::FieldsPublished,
            Phase::Returned => Cut::Returned,
            _ => return None,
        })
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.closed.validate()?;
        self.incoming.validate()?;
        let source = self.closed.filename()?.bytes()?.to_vec();
        if source.len() < 8
            || source.len() * 2 > 512
            || !source.is_ascii()
            || source.contains(&0)
            || self.closed.filename_backing.data.len() != source.len() + 1
        {
            return Err(unsupported(
                "file opening requires bounded ASCII conversion with actual wide growth",
            ));
        }
        let content = self.local_path();
        if word(&self.incoming.encoding) == 0
            && matches!(
                self.phase,
                Phase::LocaleQuery
                    | Phase::LocaleQueryAccept
                    | Phase::LocaleDefault
                    | Phase::LocaleDefaultAccept
            )
        {
            return Err(unsupported(
                "cached encoding cannot restore an initialization callback phase",
            ));
        }
        let assigned = self.phase >= Phase::LocalAssigned;
        for (slot, initial, kind, expected_content, capacity) in [
            (
                &self.local,
                &self.incoming.local,
                if self.phase <= Phase::LocalInput {
                    Kind::Bare
                } else if self.phase <= Phase::LocalAccept {
                    Kind::Cleared
                } else if self.phase < Phase::LocalReleased {
                    Kind::Live
                } else {
                    Kind::Released
                },
                if assigned { content.as_slice() } else { &[] },
                if assigned { content.len() + 2 } else { 16 },
            ),
            (
                &self.output,
                &self.incoming.output,
                if self.phase <= Phase::LocalAssigned {
                    Kind::Bare
                } else if self.phase <= Phase::OutputAccept {
                    Kind::Cleared
                } else if self.phase < Phase::OutputReleased {
                    Kind::Live
                } else {
                    Kind::Released
                },
                if self.phase >= Phase::OutputCopied {
                    content.as_slice()
                } else {
                    &[]
                },
                content.len() + 2,
            ),
            (
                &self.replacement,
                &self.incoming.replacement,
                if self.phase <= Phase::ReplacementInput {
                    Kind::Bare
                } else if self.phase <= Phase::ReplacementAccept {
                    Kind::Cleared
                } else if self.phase < Phase::ReplacementReleased {
                    Kind::Live
                } else {
                    Kind::Released
                },
                &[],
                16,
            ),
        ] {
            slot.validate()?;
            if slot.initial != *initial
                || slot.kind != kind
                || slot.content != expected_content
                || slot
                    .backing
                    .as_ref()
                    .is_some_and(|backing| backing.data.len() != capacity)
            {
                return Err(unsupported(
                    "wide owner differs from the actual opening phase",
                ));
            }
        }
        if self.growth.is_some() != (self.phase == Phase::GrowthRelease)
            || self.pending_wide.is_some() != self.allocation_bytes().is_some()
            || self.pending_locale.is_some()
                != matches!(
                    self.phase,
                    Phase::LocaleQueryAccept | Phase::LocaleDefaultAccept
                )
            || self.pending_open.is_some() != (self.phase == Phase::OpenAccept)
            || self.pending_seek.is_some() != (self.phase == Phase::SeekAccept)
            || self.capability.is_some() != (self.phase >= Phase::OpenReturned)
            || self.position.is_some() != (self.phase >= Phase::FieldsPublished)
        {
            return Err(unsupported(
                "file opening lost a pending service reply or phase owner",
            ));
        }
        if let Some(growth) = &self.growth {
            growth.validate()?;
            if growth.initial != self.incoming.local
                || growth.kind != Kind::Live
                || growth.content != content
                || growth.backing.as_ref().unwrap().data.len() != content.len() + 2
            {
                return Err(unsupported("wide growth lost its prepared replacement"));
            }
        }
        let mut expected_owner = self.closed.owner.clone();
        if let Some(capability) = self.capability {
            if capability == 0 || capability == u32::MAX {
                return Err(unsupported(
                    "readonly capability is not a valid owned native handle",
                ));
            }
            write_word(&mut expected_owner, 0x1c, capability);
        }
        if self.phase >= Phase::Seek {
            expected_owner.data[0x18] = 1;
            for (offset, argument) in [0x20, 0x24, 0x28].into_iter().zip(self.incoming.arguments) {
                write_word(&mut expected_owner, offset, argument);
            }
        }
        if let Some(position) = self.position {
            if position == u32::MAX {
                return Err(unsupported("seek position is the native failure sentinel"));
            }
            for offset in [0x48, 0x4c] {
                write_word(&mut expected_owner, offset, position);
            }
            for offset in [0x60, 0x64, 0x78, 0x7c] {
                write_word(&mut expected_owner, offset, 0);
            }
        }
        let mut expected_encoding = self.incoming.encoding.clone();
        if self.phase >= Phase::LocaleQuery {
            write_word(&mut expected_encoding, 0, 0);
        }
        let mut expected_buffer = self.incoming.buffer.clone();
        if self.phase >= Phase::Converted {
            expected_buffer.data[..content.len()].copy_from_slice(&content);
        }
        if self.owner != expected_owner
            || self.encoding != expected_encoding
            || self.buffer != expected_buffer
        {
            return Err(unsupported(
                "file opening changed complete owner, locale cache or bounded buffer bytes",
            ));
        }
        let mut spans = self.closed.context.spans()?;
        spans.retain(|span| *span != (self.closed.context.retired_prefix.token, 4));
        spans.extend([
            (self.owner.token, 128),
            (
                self.closed.filename_backing.token,
                self.closed.filename_backing.data.len(),
            ),
            (
                self.closed.secondary_backing.token,
                self.closed.secondary_backing.data.len(),
            ),
            (self.buffer.token, 1024),
            (self.encoding.token, 4),
            (self.incoming.wide_dispatch.token, 4),
        ]);
        for slot in [&self.local, &self.output, &self.replacement] {
            spans.extend(slot.spans()?);
        }
        if let Some(growth) = &self.growth {
            let backing = growth.backing.as_ref().unwrap();
            spans.push((backing.token, backing.data.len()));
        }
        if let Some(Allocated::Bytes { token, data }) = &self.pending_wide {
            let allocation = Owner {
                token: *token,
                data: data.clone(),
            };
            wide::validate_allocation(&allocation, self.allocation_bytes().unwrap())?;
            spans.push((*token, data.len()));
        }
        disjoint(spans)
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        match self.phase {
            Phase::OpenEntry => self.phase = Phase::ConversionEntry,
            Phase::ConversionEntry => self.phase = Phase::LocalInput,
            Phase::LocalInput => {
                self.local.clear();
                self.phase = Phase::LocalAllocate;
            }
            Phase::LocalAllocate => {
                self.pending_wide = Some(host.allocate_wide(Request::Construct(16))?);
                self.phase = Phase::LocalAccept;
            }
            Phase::LocalAccept => {
                self.local = self.local.initialized(self.allocation()?, vec![])?;
                self.pending_wide = None;
                self.phase = Phase::LocalReady;
            }
            Phase::LocalReady => self.phase = Phase::ConversionReady,
            Phase::ConversionReady => {
                self.phase = if word(&self.encoding) == 0xfffffffd {
                    write_word(&mut self.encoding, 0, 0);
                    Phase::LocaleQuery
                } else {
                    Phase::Convert
                };
            }
            Phase::LocaleQuery => {
                self.pending_locale = Some(host.locale(LocaleRequest::Query)?);
                self.phase = Phase::LocaleQueryAccept;
            }
            Phase::LocaleDefault => {
                self.pending_locale = Some(host.locale(LocaleRequest::Default)?);
                self.phase = Phase::LocaleDefaultAccept;
            }
            Phase::LocaleQueryAccept | Phase::LocaleDefaultAccept => {
                match self.pending_locale.as_ref().unwrap() {
                    LocaleReply::Value(value) if value == b"C" => {}
                    LocaleReply::Value(_) => {
                        return Err(unsupported("unqualified host locale/codepage conversion"));
                    }
                    LocaleReply::Failed => {
                        return Err(Stop::Unavailable("host locale call returned null".into()));
                    }
                }
                self.pending_locale = None;
                self.phase = if self.phase == Phase::LocaleQueryAccept {
                    Phase::LocaleDefault
                } else {
                    Phase::Convert
                };
            }
            Phase::Convert => {
                let content = self.local_path();
                self.buffer.data[..content.len()].copy_from_slice(&content);
                self.phase = Phase::Converted;
            }
            Phase::Converted => self.phase = Phase::GrowthAllocate,
            Phase::GrowthAllocate => {
                self.pending_wide =
                    Some(host.allocate_wide(Request::Grow((self.local_path().len() + 2) as u32))?);
                self.phase = Phase::GrowthAccept;
            }
            Phase::GrowthAccept => {
                self.growth = Some(
                    self.local
                        .initialized(self.allocation()?, self.local_path())?,
                );
                self.pending_wide = None;
                self.phase = Phase::GrowthRelease;
            }
            Phase::GrowthRelease => {
                host.free_wide(self.local.backing.as_ref().unwrap())?;
                self.local = self.growth.take().unwrap();
                self.phase = Phase::LocalAssigned;
            }
            Phase::LocalAssigned => {
                self.output.clear();
                self.phase = Phase::OutputAllocate;
            }
            Phase::OutputAllocate => {
                self.pending_wide = Some(
                    host.allocate_wide(Request::Construct((self.local_path().len() + 2) as u32))?,
                );
                self.phase = Phase::OutputAccept;
            }
            Phase::OutputAccept => {
                self.output = self
                    .output
                    .initialized(self.allocation()?, self.local_path())?;
                self.pending_wide = None;
                self.phase = Phase::OutputCopied;
            }
            Phase::OutputCopied => {
                host.free_wide(self.local.backing.as_ref().unwrap())?;
                self.local.kind = Kind::Released;
                self.phase = Phase::LocalReleased;
            }
            Phase::LocalReleased => self.phase = Phase::ReplacementInput,
            Phase::ReplacementInput => {
                self.replacement.clear();
                self.phase = Phase::ReplacementAllocate;
            }
            Phase::ReplacementAllocate => {
                self.pending_wide = Some(host.allocate_wide(Request::Construct(16))?);
                self.phase = Phase::ReplacementAccept;
            }
            Phase::ReplacementAccept => {
                self.replacement = self.replacement.initialized(self.allocation()?, vec![])?;
                self.pending_wide = None;
                self.phase = Phase::ReplacementReady;
            }
            Phase::ReplacementReady => {
                if self
                    .closed
                    .filename()?
                    .bytes()?
                    .to_ascii_lowercase()
                    .ends_with(b".lnk")
                {
                    return Err(unsupported(
                        "actual shortcut resolution requires its unimplemented backend",
                    ));
                }
                self.phase = Phase::ShortcutChecked;
            }
            Phase::ShortcutChecked => self.phase = Phase::ReadonlyEntry,
            Phase::ReadonlyEntry => {
                let request = OpenRequest {
                    filename: self
                        .output
                        .content
                        .chunks_exact(2)
                        .map(|unit| u16::from_le_bytes(unit.try_into().unwrap()))
                        .collect(),
                    access: 0x80000000,
                    sharing: 1,
                    security: 0,
                    disposition: 3,
                    flags: 0,
                    template: 0,
                };
                self.pending_open = Some(host.open_readonly(&request)?);
                self.phase = Phase::OpenAccept;
            }
            Phase::OpenAccept => {
                let capability = match self.pending_open.unwrap() {
                    Opened::File(capability) if capability != 0 && capability != u32::MAX => {
                        capability
                    }
                    _ => {
                        return Err(Stop::Unavailable(
                            "readonly open lacks a valid actual capability".into(),
                        ));
                    }
                };
                self.capability = Some(capability);
                write_word(&mut self.owner, 0x1c, capability);
                self.pending_open = None;
                self.phase = Phase::OpenReturned;
            }
            Phase::OpenReturned => {
                host.free_wide(self.replacement.backing.as_ref().unwrap())?;
                self.replacement.kind = Kind::Released;
                self.phase = Phase::ReplacementReleased;
            }
            Phase::ReplacementReleased => {
                host.free_wide(self.output.backing.as_ref().unwrap())?;
                self.output.kind = Kind::Released;
                self.phase = Phase::OutputReleased;
            }
            Phase::OutputReleased => {
                self.owner.data[0x18] = 1;
                for (offset, argument) in
                    [0x20, 0x24, 0x28].into_iter().zip(self.incoming.arguments)
                {
                    write_word(&mut self.owner, offset, argument);
                }
                self.phase = Phase::Seek;
            }
            Phase::Seek => {
                self.pending_seek = Some(host.seek_current(self.capability.unwrap())?);
                self.phase = Phase::SeekAccept;
            }
            Phase::SeekAccept => {
                let position = match self.pending_seek.unwrap() {
                    Seeked::Position(position) if position < u64::from(u32::MAX) => position as u32,
                    Seeked::Position(_) => {
                        return Err(unsupported("unqualified wide or sentinel current position"));
                    }
                    Seeked::Failed => {
                        return Err(Stop::Unavailable(
                            "actual current-position query failed".into(),
                        ));
                    }
                };
                self.position = Some(position);
                for offset in [0x48, 0x4c] {
                    write_word(&mut self.owner, offset, position);
                }
                for offset in [0x60, 0x64, 0x78, 0x7c] {
                    write_word(&mut self.owner, offset, 0);
                }
                self.pending_seek = None;
                self.phase = Phase::FieldsPublished;
            }
            Phase::FieldsPublished => self.phase = Phase::Returned,
            Phase::Returned => {}
        }
        self.validate()?;
        Ok(self.cut())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.phase != Phase::Returned {
            return Err(unsupported("actual readonly file open has not returned"));
        }
        Ok(State { execution: self })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    execution: Execution,
}

impl State {
    pub(super) fn retained_spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.validate()?;
        let execution = &self.execution;
        let mut spans = execution.closed.context.spans()?;
        spans.retain(|span| *span != (execution.closed.context.retired_prefix.token, 4));
        spans.extend([
            (execution.owner.token, 128),
            (
                execution.closed.filename_backing.token,
                execution.closed.filename_backing.data.len(),
            ),
            (
                execution.closed.secondary_backing.token,
                execution.closed.secondary_backing.data.len(),
            ),
            (execution.encoding.token, 4),
            (execution.incoming.wide_dispatch.token, 4),
        ]);
        disjoint(spans.clone())?;
        Ok(spans)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.execution.validate()?;
        if self.execution.phase != Phase::Returned {
            return Err(unsupported("readonly file state is not a returned opening"));
        }
        Ok(())
    }

    pub fn owner(&self) -> &Owner {
        self.execution.owner()
    }
    pub fn context(&self) -> &file::Context {
        self.execution.context()
    }
    pub fn capability(&self) -> u32 {
        self.execution.capability.unwrap()
    }
}
