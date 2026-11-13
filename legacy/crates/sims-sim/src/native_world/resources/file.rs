use super::{manager, names, path, strings};
use crate::native_world::{validate_owner, write_word};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, Stop};
use path::{Kind, StringSlot};
use serde::{Deserialize, Serialize};

pub trait Host {
    fn allocate_file(&mut self, bytes: u32) -> Result<Allocated, Stop>;
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop>;
    fn free_string(&mut self, backing: &Owner) -> Result<(), Stop>;
}

struct StringHost<'a, H>(&'a mut H);

impl<H: Host> path::Host for StringHost<'_, H> {
    fn allocate_string(
        &mut self,
        scope: path::Scope,
        request: strings::Request,
    ) -> Result<Allocated, Stop> {
        if scope != path::Scope::World {
            return Err(unsupported(
                "file strings require WORLD allocation ownership",
            ));
        }
        self.0.allocate_string(request)
    }

    fn free_string(&mut self, scope: path::Scope, backing: &Owner) -> Result<(), Stop> {
        if scope != path::Scope::World {
            return Err(unsupported("file strings require WORLD release ownership"));
        }
        self.0.free_string(backing)
    }

    fn open_scenario(&mut self, _: &[u8], _: &[u8]) -> Result<path::Opened, Stop> {
        Err(unsupported("file construction cannot open a Scenario file"))
    }

    fn close_scenario(&mut self, _: u64) -> Result<(), Stop> {
        Err(unsupported(
            "file construction cannot close a Scenario file",
        ))
    }
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
        return Err(unsupported("file construction aliases retained ownership"));
    }
    Ok(())
}

fn descriptor(owner: &Owner, offset: usize) -> Owner {
    Owner {
        token: owner.token + offset as u64,
        data: owner.data[offset..offset + 12].to_vec(),
    }
}

fn primary_fields(owner: &mut Owner) {
    write_word(owner, 4, 0x66933c);
    write_word(owner, 20, 0);
    owner.data[24] = 0;
    write_word(owner, 28, u32::MAX);
    for offset in (32..48).step_by(4) {
        write_word(owner, offset, 0);
    }
    write_word(owner, 48, 0x20000000);
}

fn secondary_fields(owner: &mut Owner) {
    write_word(owner, 52, 0x66933c);
    write_word(owner, 68, 0);
    for offset in (72..128).step_by(4) {
        write_word(owner, offset, 0);
    }
    write_word(owner, 0, 0x672b9c);
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub resources: names::Context,
    pub manager: manager::State,
    pub name: strings::State,
    pub retired_prefix: Owner,
}

impl Context {
    pub(super) fn spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.name.validate()?;
        validate_owner(&self.retired_prefix, 4)?;
        let (directory, basename) = manager::split(self.resources.bytes()?)?;
        if self.name.bytes()? != [directory.clone(), basename.clone()].concat()
            || self.manager.lookup(&directory, &basename)? != manager::Lookup::Missing
        {
            return Err(unsupported(
                "file construction lost the reached manager-name lookup",
            ));
        }
        let mut spans = self.resources.spans()?;
        names::transfer_manager_sources(&self.resources, &self.manager, &mut spans)?;
        spans.extend(names::manager_spans(&self.manager)?);
        spans.extend([
            (self.name.owner.token, 12),
            (self.name.backing.token, self.name.backing.data.len()),
            (self.retired_prefix.token, 4),
        ]);
        Ok(spans)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        disjoint(self.spans()?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    FileAllocated,
    PrimaryReady,
    FileInitialized,
    TemporaryReady,
    FilenameAssigned,
    Returned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Phase {
    Allocate,
    Accept,
    StartPrimary,
    Primary,
    StartSecondary,
    Secondary,
    StartTemporary,
    Temporary,
    StartAssignment,
    Assignment,
    ReleaseTemporary,
    Returned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    context: Context,
    owner: Option<Owner>,
    initial_owner: Option<Owner>,
    pending: Option<Allocated>,
    primary: Option<StringSlot>,
    secondary: Option<StringSlot>,
    temporary: Owner,
    initial_temporary: Owner,
    temporary_string: StringSlot,
    phase: Phase,
}

impl Construction {
    pub fn new(names: names::State) -> Result<Self, Stop> {
        names.validate()?;
        if names.directory_descriptor.token != names.basename_descriptor.token + 12 {
            return Err(unsupported(
                "file temporary requires the actual adjacent retired descriptors",
            ));
        }
        let temporary = Owner {
            token: names.basename_descriptor.token + 4,
            data: [
                &names.basename_descriptor.data[4..],
                names.directory_descriptor.data.as_slice(),
            ]
            .concat(),
        };
        let execution = Self {
            context: Context {
                resources: names.context,
                manager: names.manager,
                name: names.name,
                retired_prefix: Owner {
                    token: names.basename_descriptor.token,
                    data: names.basename_descriptor.data[..4].to_vec(),
                },
            },
            owner: None,
            initial_owner: None,
            pending: None,
            primary: None,
            secondary: None,
            temporary_string: StringSlot::Bare(descriptor(&temporary, 4)),
            initial_temporary: temporary.clone(),
            temporary,
            phase: Phase::Allocate,
        };
        execution.validate()?;
        Ok(execution)
    }

    pub fn context(&self) -> &Context {
        &self.context
    }

    pub fn owner(&self) -> Option<&Owner> {
        self.owner.as_ref()
    }

    pub fn temporary(&self) -> &Owner {
        &self.temporary
    }

    pub fn filename(&self) -> Option<&strings::State> {
        self.primary.as_ref().and_then(|slot| slot.state().ok())
    }

    pub fn secondary(&self) -> Option<&strings::State> {
        self.secondary.as_ref().and_then(|slot| slot.state().ok())
    }

    pub fn cut(&self) -> Option<Cut> {
        match self.phase {
            Phase::StartPrimary => Some(Cut::FileAllocated),
            Phase::StartSecondary => Some(Cut::PrimaryReady),
            Phase::StartTemporary => Some(Cut::FileInitialized),
            Phase::StartAssignment => Some(Cut::TemporaryReady),
            Phase::ReleaseTemporary => Some(Cut::FilenameAssigned),
            Phase::Returned => Some(Cut::Returned),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<(), Stop> {
        let mut spans = self.context.spans()?;
        validate_owner(&self.initial_temporary, 20)?;
        validate_owner(&self.temporary, 20)?;
        for slot in self
            .primary
            .iter()
            .chain(self.secondary.iter())
            .chain(std::iter::once(&self.temporary_string))
        {
            validate_owner(slot.owner(), 12)?;
        }
        if self.temporary.token != self.context.retired_prefix.token + 4
            || self.initial_temporary.token != self.temporary.token
            || self.owner.is_some() != (self.phase >= Phase::StartPrimary)
            || self.initial_owner.is_some() != self.owner.is_some()
            || self.pending.is_some() != (self.phase == Phase::Accept)
            || self.primary.is_some() != self.owner.is_some()
            || self.secondary.is_some() != (self.phase >= Phase::StartSecondary)
        {
            return Err(unsupported(
                "file constructor phase lost its exact retained owners",
            ));
        }
        spans.push((self.temporary.token, 20));
        if let Some(Allocated::Bytes { token, data }) = &self.pending {
            validate_owner(
                &Owner {
                    token: *token,
                    data: data.clone(),
                },
                128,
            )?;
            spans.push((*token, data.len()));
        }
        if let Some(owner) = &self.owner {
            validate_owner(owner, 128)?;
            let initial = self.initial_owner.as_ref().unwrap();
            validate_owner(initial, 128)?;
            if owner.token != initial.token {
                return Err(unsupported("file allocation identity changed"));
            }
            let mut expected = initial.clone();
            if self.phase >= Phase::Primary {
                write_word(&mut expected, 0, 0x672c24);
            }
            if self.phase >= Phase::StartSecondary {
                primary_fields(&mut expected);
            }
            if self.phase >= Phase::StartTemporary {
                secondary_fields(&mut expected);
            }
            for (slot, offset) in [(self.primary.as_ref(), 8), (self.secondary.as_ref(), 56)] {
                if let Some(slot) = slot {
                    if slot.owner().token != owner.token + offset as u64 {
                        return Err(unsupported(
                            "embedded file string lost its actual descriptor",
                        ));
                    }
                    expected.data[offset..offset + 12].copy_from_slice(&slot.owner().data);
                }
            }
            if owner != &expected {
                return Err(unsupported(
                    "file constructor changed actual fields or untouched input bytes",
                ));
            }
            spans.push((owner.token, 128));
        }
        let mut expected_temporary = self.initial_temporary.clone();
        expected_temporary.data[4..16].copy_from_slice(&self.temporary_string.owner().data);
        if self.phase >= Phase::StartAssignment {
            write_word(&mut expected_temporary, 0, 0x66933c);
            write_word(&mut expected_temporary, 16, 0);
        }
        if self.temporary != expected_temporary {
            return Err(unsupported(
                "file temporary changed its reused stack input bytes",
            ));
        }
        let primary_kind = match self.phase {
            Phase::StartPrimary => Kind::Bare,
            Phase::Primary => Kind::Construct,
            Phase::Assignment => Kind::Edit,
            _ => Kind::Live,
        };
        let secondary_kind = match self.phase {
            Phase::StartSecondary => Kind::Bare,
            Phase::Secondary => Kind::Construct,
            _ => Kind::Live,
        };
        let temporary_kind = if self.phase <= Phase::StartTemporary {
            Kind::Bare
        } else if self.phase == Phase::Temporary {
            Kind::Construct
        } else if self.phase == Phase::Returned {
            Kind::Released
        } else {
            Kind::Live
        };
        let source = self.context.resources.bytes()?;
        for (index, (slot, kind)) in [
            (self.primary.as_ref(), primary_kind),
            (self.secondary.as_ref(), secondary_kind),
            (Some(&self.temporary_string), temporary_kind),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(slot) = slot else { continue };
            if slot.kind() != kind {
                return Err(unsupported(
                    "file string ownership differs from its actual phase",
                ));
            }
            if let StringSlot::Construct(execution) = slot {
                if execution.input() != if index == 2 { Some(source) } else { None } {
                    return Err(unsupported(
                        "file string construction lost its original source",
                    ));
                }
            }
            if let StringSlot::Edit(execution) = slot {
                if index != 0
                    || execution.operation() != &strings::Operation::Assign(source.to_vec())
                    || !execution.initial().bytes()?.is_empty()
                {
                    return Err(unsupported(
                        "file assignment differs from the actual constructor",
                    ));
                }
            }
            if matches!(kind, Kind::Live | Kind::Released) {
                let expected =
                    if index == 2 || (index == 0 && self.phase >= Phase::ReleaseTemporary) {
                        source
                    } else {
                        &[]
                    };
                if slot.state()?.bytes()? != expected {
                    return Err(unsupported(
                        "file string differs from its actual copied path",
                    ));
                }
            }
            let descriptor_span = (slot.owner().token, 12);
            spans.extend(
                slot.spans()?
                    .into_iter()
                    .filter(|span| *span != descriptor_span),
            );
        }
        if self.temporary_string.owner().token != self.temporary.token + 4 {
            return Err(unsupported(
                "file temporary string lost its reused descriptor identity",
            ));
        }
        disjoint(spans)
    }

    fn synchronize(&mut self) {
        if let Some(owner) = &mut self.owner {
            for (slot, offset) in [(self.primary.as_ref(), 8), (self.secondary.as_ref(), 56)] {
                if let Some(slot) = slot {
                    owner.data[offset..offset + 12].copy_from_slice(&slot.owner().data);
                }
            }
        }
        self.temporary.data[4..16].copy_from_slice(&self.temporary_string.owner().data);
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        match self.phase {
            Phase::Allocate => {
                self.pending = Some(host.allocate_file(128)?);
                self.phase = Phase::Accept;
            }
            Phase::Accept => {
                let owner = match self.pending.as_ref().unwrap() {
                    Allocated::Bytes { token, data } => Owner {
                        token: *token,
                        data: data.clone(),
                    },
                    Allocated::Null => {
                        return Err(Stop::Unavailable(
                            "file constructor has no actual allocation".into(),
                        ));
                    }
                };
                validate_owner(&owner, 128)?;
                self.primary = Some(StringSlot::Bare(descriptor(&owner, 8)));
                self.initial_owner = Some(owner.clone());
                self.owner = Some(owner);
                self.pending = None;
                self.phase = Phase::StartPrimary;
            }
            Phase::StartPrimary => {
                write_word(self.owner.as_mut().unwrap(), 0, 0x672c24);
                self.primary.as_mut().unwrap().construct(None)?;
                self.phase = Phase::Primary;
            }
            Phase::Primary => {
                if self
                    .primary
                    .as_mut()
                    .unwrap()
                    .step(&mut StringHost(host), path::Scope::World)?
                {
                    let owner = self.owner.as_mut().unwrap();
                    primary_fields(owner);
                    self.secondary = Some(StringSlot::Bare(descriptor(owner, 56)));
                    self.phase = Phase::StartSecondary;
                }
            }
            Phase::StartSecondary => {
                self.secondary.as_mut().unwrap().construct(None)?;
                self.phase = Phase::Secondary;
            }
            Phase::Secondary => {
                if self
                    .secondary
                    .as_mut()
                    .unwrap()
                    .step(&mut StringHost(host), path::Scope::World)?
                {
                    secondary_fields(self.owner.as_mut().unwrap());
                    self.phase = Phase::StartTemporary;
                }
            }
            Phase::StartTemporary => {
                self.temporary_string
                    .construct(Some(self.context.resources.bytes()?.to_vec()))?;
                self.phase = Phase::Temporary;
            }
            Phase::Temporary => {
                if self
                    .temporary_string
                    .step(&mut StringHost(host), path::Scope::World)?
                {
                    write_word(&mut self.temporary, 0, 0x66933c);
                    write_word(&mut self.temporary, 16, 0);
                    self.phase = Phase::StartAssignment;
                }
            }
            Phase::StartAssignment => {
                self.primary
                    .as_mut()
                    .unwrap()
                    .edit(strings::Operation::Assign(
                        self.temporary_string.state()?.bytes()?.to_vec(),
                    ))?;
                self.phase = Phase::Assignment;
            }
            Phase::Assignment => {
                if self
                    .primary
                    .as_mut()
                    .unwrap()
                    .step(&mut StringHost(host), path::Scope::World)?
                {
                    self.phase = Phase::ReleaseTemporary;
                }
            }
            Phase::ReleaseTemporary => {
                self.temporary_string
                    .release(&mut StringHost(host), path::Scope::World)?;
                self.phase = Phase::Returned;
            }
            Phase::Returned => {}
        }
        self.synchronize();
        self.validate()?;
        Ok(self.cut())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.phase != Phase::Returned {
            return Err(unsupported("actual file constructor has not returned"));
        }
        let StringSlot::Live(primary) = self.primary.unwrap() else {
            unreachable!()
        };
        let StringSlot::Live(secondary) = self.secondary.unwrap() else {
            unreachable!()
        };
        let state = State {
            context: self.context,
            owner: self.owner.unwrap(),
            filename_backing: primary.backing,
            secondary_backing: secondary.backing,
            temporary_descriptor: self.temporary,
        };
        state.validate()?;
        Ok(state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub context: Context,
    pub owner: Owner,
    pub filename_backing: Owner,
    pub secondary_backing: Owner,
    pub temporary_descriptor: Owner,
}

impl State {
    pub fn filename(&self) -> Result<strings::State, Stop> {
        validate_owner(&self.owner, 128)?;
        Ok(strings::State {
            owner: descriptor(&self.owner, 8),
            backing: self.filename_backing.clone(),
        })
    }

    pub fn secondary(&self) -> Result<strings::State, Stop> {
        validate_owner(&self.owner, 128)?;
        Ok(strings::State {
            owner: descriptor(&self.owner, 56),
            backing: self.secondary_backing.clone(),
        })
    }

    pub fn validate(&self) -> Result<(), Stop> {
        validate_owner(&self.owner, 128)?;
        validate_owner(&self.temporary_descriptor, 20)?;
        let mut expected = self.owner.clone();
        primary_fields(&mut expected);
        secondary_fields(&mut expected);
        let filename = self.filename()?;
        let secondary = self.secondary()?;
        let beginning = u64::from(u32::from_le_bytes(
            self.temporary_descriptor.data[4..8].try_into().unwrap(),
        ));
        let end = u64::from(u32::from_le_bytes(
            self.temporary_descriptor.data[8..12].try_into().unwrap(),
        ));
        let capacity = u64::from(u32::from_le_bytes(
            self.temporary_descriptor.data[12..16].try_into().unwrap(),
        ));
        if self.owner != expected
            || filename.bytes()? != self.context.resources.bytes()?
            || !secondary.bytes()?.is_empty()
            || secondary.backing.data.len() != 8
            || self.temporary_descriptor.token != self.context.retired_prefix.token + 4
            || self.temporary_descriptor.data[..4] != 0x66933c_u32.to_le_bytes()
            || self.temporary_descriptor.data[16..] != [0; 4]
            || beginning == 0
            || end != beginning + self.context.resources.bytes()?.len() as u64
            || capacity != end + 1
        {
            return Err(unsupported(
                "returned file constructor fields or retained strings differ",
            ));
        }
        let mut spans = self.context.spans()?;
        spans.extend([
            (self.owner.token, 128),
            (
                self.filename_backing.token,
                self.filename_backing.data.len(),
            ),
            (
                self.secondary_backing.token,
                self.secondary_backing.data.len(),
            ),
            (self.temporary_descriptor.token, 20),
        ]);
        disjoint(spans)
    }
}
