use super::{manager, path, strings};
use crate::native_world::validate_owner;
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, Stop};
use path::{Kind, StringSlot};
use serde::{Deserialize, Serialize};

pub trait Host {
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
                "manager names require the actual WORLD allocator scope",
            ));
        }
        self.0.allocate_string(request)
    }

    fn free_string(&mut self, scope: path::Scope, backing: &Owner) -> Result<(), Stop> {
        if scope != path::Scope::World {
            return Err(unsupported(
                "manager names require the actual WORLD release scope",
            ));
        }
        self.0.free_string(backing)
    }

    fn open_scenario(&mut self, _: &[u8], _: &[u8]) -> Result<path::Opened, Stop> {
        Err(unsupported(
            "manager names cannot reopen the earlier Scenario probe",
        ))
    }

    fn close_scenario(&mut self, _: u64) -> Result<(), Stop> {
        Err(unsupported(
            "manager names have no Scenario capability to close",
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
        return Err(unsupported(
            "manager names alias retained world, manager, stack or string ownership",
        ));
    }
    Ok(())
}

fn take_owner(owners: &mut Vec<Owner>, token: u64) -> Result<Owner, Stop> {
    let index = owners
        .iter()
        .position(|owner| owner.token == token)
        .ok_or_else(|| unsupported("manager-name transfer lacks its actual retained owner"))?;
    Ok(owners.remove(index))
}

pub(super) fn manager_spans(manager: &manager::State) -> Result<Vec<(u64, usize)>, Stop> {
    manager.validate()?;
    Ok(std::iter::once(&manager.owner)
        .chain(manager.buckets.iter())
        .chain(manager.retained.iter())
        .map(|owner| (owner.token, owner.data.len()))
        .collect())
}

pub(super) fn transfer_manager_sources(
    context: &Context,
    manager: &manager::State,
    spans: &mut Vec<(u64, usize)>,
) -> Result<(), Stop> {
    path::transfer_game_sources(
        &context.resources,
        std::iter::once(&manager.owner)
            .chain(&manager.buckets)
            .chain(&manager.retained),
        spans,
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub resources: super::State,
    pub application: path::Application,
    pub buffer: Owner,
    pub scenario_descriptor: Owner,
    pub retained: Vec<Owner>,
}

impl Context {
    pub fn bytes(&self) -> Result<&[u8], Stop> {
        validate_owner(&self.buffer, 272)?;
        let length = u16::from_le_bytes(self.buffer.data[8..10].try_into().unwrap()) as usize;
        if self.buffer.data[..4] != 0x6664e8_u32.to_le_bytes()
            || self.buffer.data[4..8] != ((self.buffer.token + 12) as u32).to_le_bytes()
            || self.buffer.data[10..12] != 260_u16.to_le_bytes()
            || length >= 260
            || self.buffer.data[12 + length] != 0
            || self.buffer.data[12..12 + length].contains(&0)
        {
            return Err(unsupported(
                "manager name lacks its actual composed bounded path",
            ));
        }
        Ok(&self.buffer.data[12..12 + length])
    }

    pub(super) fn spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.bytes()?;
        let mut spans = path::resource_spans(&self.resources, &self.application)?;
        path::transfer_game_sources(&self.resources, &self.retained, &mut spans)?;
        spans.extend(self.application.spans()?);
        validate_owner(&self.scenario_descriptor, 12)?;
        spans.extend([
            (self.buffer.token, self.buffer.data.len()),
            (self.scenario_descriptor.token, 12),
        ]);
        for owner in &self.retained {
            validate_owner(owner, owner.data.len())?;
            spans.push((owner.token, owner.data.len()));
        }
        Ok(spans)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        disjoint(self.spans()?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incoming {
    pub basename_descriptor: Owner,
    pub directory_descriptor: Owner,
    pub synchronization: u64,
    pub manager_retained: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    NameCopied,
    NameNormalized,
    LookupReturn,
    FileAllocationEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Phase {
    StartName,
    CopyName,
    NameCopied,
    Normalize,
    NameNormalized,
    ConstructBasename,
    StartDirectory,
    ConstructDirectory,
    StartDirectoryAssignment,
    AssignDirectory,
    StartBasenameAssignment,
    AssignBasename,
    Lookup,
    ReleaseDirectory,
    ReleaseBasename,
    LookupReturn,
    FileAllocationEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    context: Context,
    manager: manager::State,
    name: StringSlot,
    basename: StringSlot,
    directory: StringSlot,
    initial_descriptors: [Owner; 3],
    initial_buffer: Vec<u8>,
    lookup: Option<manager::Lookup>,
    phase: Phase,
}

impl Construction {
    pub fn new(mut path: path::State, incoming: Incoming) -> Result<Self, Stop> {
        path.validate()?;
        let owner = take_owner(&mut path.retained, 0x6fe548)?;
        validate_owner(&owner, 72)?;
        let beginning = |offset| {
            u64::from(u32::from_le_bytes(
                owner.data[offset..offset + 4].try_into().unwrap(),
            ))
        };
        let buckets = [
            take_owner(&mut path.retained, beginning(4))?,
            take_owner(&mut path.retained, beginning(24))?,
        ];
        let mut retained = Vec::new();
        for token in incoming.manager_retained {
            retained.push(take_owner(&mut path.retained, token)?);
        }
        let manager = manager::State {
            owner,
            buckets,
            retained,
            synchronization: incoming.synchronization,
        };
        let construction = Self {
            initial_buffer: path.buffer.data.clone(),
            initial_descriptors: [
                path.output_descriptor.clone(),
                incoming.basename_descriptor.clone(),
                incoming.directory_descriptor.clone(),
            ],
            context: Context {
                resources: path.resources,
                application: path.application,
                buffer: path.buffer,
                scenario_descriptor: path.scenario_descriptor,
                retained: path.retained,
            },
            manager,
            name: StringSlot::Bare(path.output_descriptor),
            basename: StringSlot::Bare(incoming.basename_descriptor),
            directory: StringSlot::Bare(incoming.directory_descriptor),
            lookup: None,
            phase: Phase::StartName,
        };
        construction.validate()?;
        Ok(construction)
    }

    fn split(&self) -> Result<(Vec<u8>, Vec<u8>), Stop> {
        manager::split(self.context.bytes()?)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        if self.context.buffer.data != self.initial_buffer {
            return Err(unsupported(
                "manager lookup mutated its earlier composed stack buffer",
            ));
        }
        let name_kind = match self.phase {
            Phase::StartName => Kind::Bare,
            Phase::CopyName => Kind::Construct,
            Phase::Normalize => Kind::Edit,
            _ => Kind::Live,
        };
        let basename_kind = if self.phase <= Phase::NameNormalized {
            Kind::Bare
        } else if self.phase == Phase::ConstructBasename {
            Kind::Construct
        } else if self.phase == Phase::AssignBasename {
            Kind::Edit
        } else if self.phase >= Phase::LookupReturn {
            Kind::Released
        } else {
            Kind::Live
        };
        let directory_kind = if self.phase <= Phase::StartDirectory {
            Kind::Bare
        } else if self.phase == Phase::ConstructDirectory {
            Kind::Construct
        } else if self.phase == Phase::AssignDirectory {
            Kind::Edit
        } else if self.phase >= Phase::ReleaseBasename {
            Kind::Released
        } else {
            Kind::Live
        };
        let (directory, basename) = self.split()?;
        for (index, (slot, kind)) in [
            (&self.name, name_kind),
            (&self.basename, basename_kind),
            (&self.directory, directory_kind),
        ]
        .into_iter()
        .enumerate()
        {
            validate_owner(&self.initial_descriptors[index], 12)?;
            if slot.kind() != kind
                || slot.owner().token != self.initial_descriptors[index].token
                || (kind == Kind::Bare && slot.owner() != &self.initial_descriptors[index])
            {
                return Err(unsupported(
                    "manager-name descriptor identity or ownership phase differs",
                ));
            }
            if let StringSlot::Construct(execution) = slot {
                let input = if index == 0 {
                    Some(self.context.bytes()?)
                } else {
                    None
                };
                if execution.input() != input {
                    return Err(unsupported(
                        "manager-name constructor lost its actual source",
                    ));
                }
            }
            if let StringSlot::Edit(execution) = slot {
                let expected = match index {
                    0 => strings::Operation::CanonicalizeAscii,
                    1 => strings::Operation::Assign(basename.clone()),
                    _ => strings::Operation::Assign(directory.clone()),
                };
                if execution.operation() != &expected
                    || (index == 0 && execution.initial().bytes()? != self.context.bytes()?)
                    || (index != 0 && !execution.initial().bytes()?.is_empty())
                {
                    return Err(unsupported(
                        "manager-name edit lost its actual native caller/source",
                    ));
                }
            }
        }
        if self.name.kind() == Kind::Live {
            let expected = if self.phase < Phase::NameNormalized {
                self.context.bytes()?.to_vec()
            } else {
                [directory.clone(), basename.clone()].concat()
            };
            if self.name.state()?.bytes()? != expected {
                return Err(unsupported(
                    "manager name differs from its actual copied/normalized path",
                ));
            }
        }
        for (slot, target, assigned) in [
            (
                &self.directory,
                &directory,
                self.phase >= Phase::StartBasenameAssignment,
            ),
            (&self.basename, &basename, self.phase >= Phase::Lookup),
        ] {
            if matches!(slot.kind(), Kind::Live | Kind::Released)
                && slot.state()?.bytes()? != if assigned { target.as_slice() } else { &[] }
            {
                return Err(unsupported(
                    "manager lookup temporary differs from its actual split",
                ));
            }
        }
        if (self.phase >= Phase::ReleaseDirectory) != self.lookup.is_some() {
            return Err(unsupported(
                "manager lookup lost its actual pending return value",
            ));
        }
        if let Some(result) = self.lookup {
            if result != self.manager.lookup(&directory, &basename)? {
                return Err(unsupported(
                    "manager lookup return differs from its owned graph",
                ));
            }
        }
        if self.phase == Phase::FileAllocationEntry && self.lookup != Some(manager::Lookup::Missing)
        {
            return Err(unsupported(
                "only the reached native miss enters file allocation",
            ));
        }
        let mut spans = self.context.spans()?;
        transfer_manager_sources(&self.context, &self.manager, &mut spans)?;
        spans.extend(manager_spans(&self.manager)?);
        for slot in [&self.name, &self.basename, &self.directory] {
            spans.extend(slot.spans()?);
        }
        disjoint(spans)
    }

    pub fn cut(&self) -> Option<Cut> {
        match self.phase {
            Phase::NameCopied => Some(Cut::NameCopied),
            Phase::NameNormalized => Some(Cut::NameNormalized),
            Phase::LookupReturn => Some(Cut::LookupReturn),
            Phase::FileAllocationEntry => Some(Cut::FileAllocationEntry),
            _ => None,
        }
    }

    pub fn name(&self) -> Result<&strings::State, Stop> {
        self.name.state()
    }
    pub fn basename(&self) -> Result<&strings::State, Stop> {
        self.basename.state()
    }
    pub fn directory(&self) -> Result<&strings::State, Stop> {
        self.directory.state()
    }
    pub fn lookup(&self) -> Option<manager::Lookup> {
        self.lookup
    }
    pub fn context(&self) -> &Context {
        &self.context
    }
    pub fn manager(&self) -> &manager::State {
        &self.manager
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        let mut host = StringHost(host);
        let scope = path::Scope::World;
        match self.phase {
            Phase::StartName => {
                self.name.construct(Some(self.context.bytes()?.to_vec()))?;
                self.phase = Phase::CopyName;
            }
            Phase::CopyName => {
                if self.name.step(&mut host, scope)? {
                    self.phase = Phase::NameCopied;
                }
            }
            Phase::NameCopied => {
                self.name.edit(strings::Operation::CanonicalizeAscii)?;
                self.phase = Phase::Normalize;
            }
            Phase::Normalize => {
                if self.name.step(&mut host, scope)? {
                    self.phase = Phase::NameNormalized;
                }
            }
            Phase::NameNormalized => {
                self.basename.construct(None)?;
                self.phase = Phase::ConstructBasename;
            }
            Phase::ConstructBasename => {
                if self.basename.step(&mut host, scope)? {
                    self.phase = Phase::StartDirectory;
                }
            }
            Phase::StartDirectory => {
                self.directory.construct(None)?;
                self.phase = Phase::ConstructDirectory;
            }
            Phase::ConstructDirectory => {
                if self.directory.step(&mut host, scope)? {
                    self.phase = Phase::StartDirectoryAssignment;
                }
            }
            Phase::StartDirectoryAssignment => {
                self.directory
                    .edit(strings::Operation::Assign(self.split()?.0))?;
                self.phase = Phase::AssignDirectory;
            }
            Phase::AssignDirectory => {
                if self.directory.step(&mut host, scope)? {
                    self.phase = Phase::StartBasenameAssignment;
                }
            }
            Phase::StartBasenameAssignment => {
                self.basename
                    .edit(strings::Operation::Assign(self.split()?.1))?;
                self.phase = Phase::AssignBasename;
            }
            Phase::AssignBasename => {
                if self.basename.step(&mut host, scope)? {
                    self.phase = Phase::Lookup;
                }
            }
            Phase::Lookup => {
                self.lookup = Some(self.manager.lookup(
                    self.directory.state()?.bytes()?,
                    self.basename.state()?.bytes()?,
                )?);
                self.phase = Phase::ReleaseDirectory;
            }
            Phase::ReleaseDirectory => {
                self.directory.release(&mut host, scope)?;
                self.phase = Phase::ReleaseBasename;
            }
            Phase::ReleaseBasename => {
                self.basename.release(&mut host, scope)?;
                self.phase = Phase::LookupReturn;
            }
            Phase::LookupReturn => {
                if self.lookup != Some(manager::Lookup::Missing) {
                    return Err(unsupported(
                        "cached-resource acquisition after a real lookup hit is not yet composed",
                    ));
                }
                self.phase = Phase::FileAllocationEntry;
            }
            Phase::FileAllocationEntry => {}
        }
        self.validate()?;
        Ok(self.cut())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.phase != Phase::FileAllocationEntry {
            return Err(unsupported(
                "manager lookup has not reached the actual file-allocation entry",
            ));
        }
        let StringSlot::Live(name) = self.name else {
            unreachable!()
        };
        let state = State {
            context: self.context,
            manager: self.manager,
            name,
            basename_descriptor: self.basename.owner().clone(),
            directory_descriptor: self.directory.owner().clone(),
        };
        state.validate()?;
        Ok(state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub context: Context,
    pub manager: manager::State,
    pub name: strings::State,
    pub basename_descriptor: Owner,
    pub directory_descriptor: Owner,
}

impl State {
    pub fn validate(&self) -> Result<(), Stop> {
        self.name.validate()?;
        validate_owner(&self.basename_descriptor, 12)?;
        validate_owner(&self.directory_descriptor, 12)?;
        let (directory, basename) = manager::split(self.context.bytes()?)?;
        if self.name.bytes()? != [directory.clone(), basename.clone()].concat()
            || self.manager.lookup(&directory, &basename)? != manager::Lookup::Missing
        {
            return Err(unsupported(
                "file-allocation entry lacks its actual normalized name and lookup miss",
            ));
        }
        let mut spans = self.context.spans()?;
        transfer_manager_sources(&self.context, &self.manager, &mut spans)?;
        spans.extend(manager_spans(&self.manager)?);
        spans.extend([
            (self.name.owner.token, 12),
            (self.name.backing.token, self.name.backing.data.len()),
            (self.basename_descriptor.token, 12),
            (self.directory_descriptor.token, 12),
        ]);
        disjoint(spans)
    }
}
