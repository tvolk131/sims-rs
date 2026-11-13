use super::{opening, path, strings};
use crate::native_world::{validate_owner, write_word};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, Stop};
use path::{Kind, StringSlot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileSize {
    Bytes(u64),
    Failed,
}

pub trait Host {
    fn allocate_handle(&mut self, bytes: u32) -> Result<Allocated, Stop>;
    fn file_size(&mut self, capability: u32) -> Result<FileSize, Stop>;
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
                "handle strings require WORLD allocation ownership",
            ));
        }
        self.0.allocate_string(request)
    }

    fn free_string(&mut self, scope: path::Scope, backing: &Owner) -> Result<(), Stop> {
        if scope != path::Scope::World {
            return Err(unsupported(
                "handle strings require WORLD release ownership",
            ));
        }
        self.0.free_string(backing)
    }

    fn open_scenario(&mut self, _: &[u8], _: &[u8]) -> Result<path::Opened, Stop> {
        Err(unsupported(
            "resource handles cannot reopen a Scenario file",
        ))
    }

    fn close_scenario(&mut self, _: u64) -> Result<(), Stop> {
        Err(unsupported("resource handles cannot close a Scenario file"))
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
        return Err(unsupported(
            "resource handle aliases retained mutable ownership",
        ));
    }
    Ok(())
}

fn initialize(owner: &mut Owner, opened: &opening::State, size: u32) {
    write_word(owner, 0x0c, opened.owner().token as u32);
    write_word(owner, 0x10, size);
    write_word(owner, 0x14, 0);
    write_word(owner, 0x18, 0);
    owner.data[0x1c] = 0;
    owner.data[0x1d] = 0;
    write_word(owner, 0x20, opened.context().manager.owner.token as u32);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    Allocated,
    SizeReturned,
    Constructed,
    NameAssigned,
    NameReleased,
    Published,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Phase {
    Allocate,
    Accept,
    Allocated,
    Size,
    AcceptSize,
    SizeReturned,
    Name,
    Constructed,
    Assignment,
    NameAssigned,
    NameReleased,
    Published,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    opened: opening::State,
    resources: Owner,
    owner: Option<Owner>,
    initial_owner: Option<Owner>,
    name: Option<StringSlot>,
    pending: Option<Allocated>,
    pending_size: Option<FileSize>,
    size: Option<u32>,
    phase: Phase,
}

impl Construction {
    pub fn new(opened: opening::State) -> Result<Self, Stop> {
        opened.validate()?;
        let execution = Self {
            resources: opened.context().resources.resources.owner.clone(),
            opened,
            owner: None,
            initial_owner: None,
            name: None,
            pending: None,
            pending_size: None,
            size: None,
            phase: Phase::Allocate,
        };
        execution.validate()?;
        Ok(execution)
    }

    pub fn opened(&self) -> &opening::State {
        &self.opened
    }
    pub fn resources(&self) -> &Owner {
        &self.resources
    }
    pub fn owner(&self) -> Option<&Owner> {
        self.owner.as_ref()
    }
    pub fn name(&self) -> Option<&strings::State> {
        self.name.as_ref().and_then(|slot| slot.state().ok())
    }
    pub fn size(&self) -> Option<u32> {
        self.size
    }

    pub fn cut(&self) -> Option<Cut> {
        match self.phase {
            Phase::Allocated => Some(Cut::Allocated),
            Phase::SizeReturned => Some(Cut::SizeReturned),
            Phase::Constructed => Some(Cut::Constructed),
            Phase::NameAssigned => Some(Cut::NameAssigned),
            Phase::NameReleased => Some(Cut::NameReleased),
            Phase::Published => Some(Cut::Published),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<(), Stop> {
        let mut spans = self.opened.retained_spans()?;
        validate_owner(&self.resources, 100)?;
        if self.owner.is_some() != (self.phase >= Phase::Allocated)
            || self.initial_owner.is_some() != self.owner.is_some()
            || self.name.is_some() != self.owner.is_some()
            || self.pending.is_some() != (self.phase == Phase::Accept)
            || self.pending_size.is_some() != (self.phase == Phase::AcceptSize)
            || self.size.is_some() != (self.phase >= Phase::SizeReturned)
        {
            return Err(unsupported(
                "resource handle lost its actual phase owners or pending replies",
            ));
        }
        if self.size == Some(u32::MAX) {
            return Err(unsupported(
                "resource handle size is the unqualified native sentinel",
            ));
        }
        let canonical = &self.opened.context().name;
        if self.phase >= Phase::NameReleased {
            spans.retain(|span| {
                *span != (canonical.owner.token, 12)
                    && *span != (canonical.backing.token, canonical.backing.data.len())
            });
        }
        let mut expected_resources = self.opened.context().resources.resources.owner.clone();
        if let Some(owner) = &self.owner {
            validate_owner(owner, 36)?;
            let initial = self.initial_owner.as_ref().unwrap();
            validate_owner(initial, 36)?;
            let name = self.name.as_ref().unwrap();
            validate_owner(name.owner(), 12)?;
            if owner.token != initial.token || name.owner().token != owner.token {
                return Err(unsupported(
                    "handle allocation or embedded name identity changed",
                ));
            }
            let kind = match self.phase {
                Phase::Allocated | Phase::Size | Phase::AcceptSize | Phase::SizeReturned => {
                    Kind::Bare
                }
                Phase::Name => Kind::Construct,
                Phase::Assignment => Kind::Edit,
                _ => Kind::Live,
            };
            if name.kind() != kind {
                return Err(unsupported(
                    "embedded handle name differs from the actual phase",
                ));
            }
            if kind == Kind::Bare && name.owner().data != initial.data[..12] {
                return Err(unsupported(
                    "unconstructed handle name changed allocator input",
                ));
            }
            if let StringSlot::Construct(execution) = name {
                if execution.input().is_some() || execution.initial_owner() != &initial.data[..12] {
                    return Err(unsupported(
                        "handle default name lost its actual incoming descriptor",
                    ));
                }
            }
            if let StringSlot::Edit(execution) = name {
                if execution.operation() != &strings::Operation::Assign(canonical.bytes()?.to_vec())
                    || !execution.initial().bytes()?.is_empty()
                    || execution.initial().backing.data.len() != 8
                {
                    return Err(unsupported(
                        "handle assignment lost its actual canonical-name source",
                    ));
                }
            }
            if kind == Kind::Live {
                let expected = if self.phase >= Phase::NameAssigned {
                    canonical.bytes()?
                } else {
                    &[]
                };
                let state = name.state()?;
                if state.bytes()? != expected
                    || state.backing.data.len()
                        != if expected.is_empty() {
                            8
                        } else {
                            expected.len() + 1
                        }
                {
                    return Err(unsupported(
                        "handle name differs from the actual constructor or assignment",
                    ));
                }
            }
            let mut expected = initial.clone();
            expected.data[..12].copy_from_slice(&name.owner().data);
            if self.phase >= Phase::Constructed {
                initialize(&mut expected, &self.opened, self.size.unwrap());
            }
            if *owner != expected {
                return Err(unsupported(
                    "handle fields or untouched allocator bytes changed",
                ));
            }
            spans.push((owner.token, 36));
            spans.extend(
                name.spans()?
                    .into_iter()
                    .filter(|span| *span != (owner.token, 12)),
            );
            if self.phase == Phase::Published {
                write_word(&mut expected_resources, 96, owner.token as u32);
            }
        }
        if self.resources != expected_resources {
            return Err(unsupported(
                "resource owner published early or changed other bytes",
            ));
        }
        if let Some(Allocated::Bytes { token, data }) = &self.pending {
            validate_owner(
                &Owner {
                    token: *token,
                    data: data.clone(),
                },
                36,
            )?;
            spans.push((*token, data.len()));
        }
        disjoint(spans)
    }

    fn synchronize(&mut self) {
        if let (Some(owner), Some(name)) = (&mut self.owner, &self.name) {
            owner.data[..12].copy_from_slice(&name.owner().data);
        }
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        match self.phase {
            Phase::Allocate => {
                self.pending = Some(host.allocate_handle(36)?);
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
                            "resource handle allocation returned null".into(),
                        ));
                    }
                };
                validate_owner(&owner, 36)?;
                self.name = Some(StringSlot::Bare(Owner {
                    token: owner.token,
                    data: owner.data[..12].to_vec(),
                }));
                self.initial_owner = Some(owner.clone());
                self.owner = Some(owner);
                self.pending = None;
                self.phase = Phase::Allocated;
            }
            Phase::Allocated => self.phase = Phase::Size,
            Phase::Size => {
                self.pending_size = Some(host.file_size(self.opened.capability())?);
                self.phase = Phase::AcceptSize;
            }
            Phase::AcceptSize => {
                let size = match self.pending_size.unwrap() {
                    FileSize::Bytes(size) if size < u64::from(u32::MAX) => size as u32,
                    FileSize::Bytes(_) => {
                        return Err(unsupported("unqualified wide or sentinel resource size"));
                    }
                    FileSize::Failed => {
                        return Err(Stop::Unavailable(
                            "actual resource file-size query failed".into(),
                        ));
                    }
                };
                self.size = Some(size);
                self.pending_size = None;
                self.phase = Phase::SizeReturned;
            }
            Phase::SizeReturned => {
                self.name.as_mut().unwrap().construct(None)?;
                self.phase = Phase::Name;
            }
            Phase::Name => {
                if self
                    .name
                    .as_mut()
                    .unwrap()
                    .step(&mut StringHost(host), path::Scope::World)?
                {
                    initialize(
                        self.owner.as_mut().unwrap(),
                        &self.opened,
                        self.size.unwrap(),
                    );
                    self.phase = Phase::Constructed;
                }
            }
            Phase::Constructed => {
                self.name
                    .as_mut()
                    .unwrap()
                    .edit(strings::Operation::Assign(
                        self.opened.context().name.bytes()?.to_vec(),
                    ))?;
                self.phase = Phase::Assignment;
            }
            Phase::Assignment => {
                if self
                    .name
                    .as_mut()
                    .unwrap()
                    .step(&mut StringHost(host), path::Scope::World)?
                {
                    self.phase = Phase::NameAssigned;
                }
            }
            Phase::NameAssigned => {
                host.free_string(&self.opened.context().name.backing)?;
                self.phase = Phase::NameReleased;
            }
            Phase::NameReleased => {
                write_word(
                    &mut self.resources,
                    96,
                    self.owner.as_ref().unwrap().token as u32,
                );
                self.phase = Phase::Published;
            }
            Phase::Published => {}
        }
        self.synchronize();
        self.validate()?;
        Ok(self.cut())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.phase != Phase::Published {
            return Err(unsupported("resource handle has not actually published"));
        }
        Ok(State { execution: self })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    execution: Construction,
}

impl State {
    pub(super) fn retained_spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.validate()?;
        let mut spans = self.opened().retained_spans()?;
        let canonical = &self.opened().context().name;
        spans.retain(|span| {
            *span != (canonical.owner.token, 12)
                && *span != (canonical.backing.token, canonical.backing.data.len())
        });
        spans.extend([
            (self.owner().token, 36),
            (self.name().backing.token, self.name().backing.data.len()),
        ]);
        disjoint(spans.clone())?;
        Ok(spans)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.execution.validate()?;
        if self.execution.phase != Phase::Published {
            return Err(unsupported("resource handle state is not published"));
        }
        Ok(())
    }

    pub fn opened(&self) -> &opening::State {
        self.execution.opened()
    }
    pub fn resources(&self) -> &Owner {
        self.execution.resources()
    }
    pub fn owner(&self) -> &Owner {
        self.execution.owner().unwrap()
    }
    pub fn name(&self) -> &strings::State {
        self.execution.name().unwrap()
    }
}
