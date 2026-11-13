use super::strings::{self, Operation, Request};
use crate::native_world::{validate_owner, write_word};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocated, Stop};
use serde::{Deserialize, Serialize};

const ROOT: &[u8] = b"./";
const SCENARIO: &[u8] = b"GameData/Scenario.tds";
const MODE: &[u8] = b"rb";
const CAPACITY: usize = 260;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    World,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Opened {
    File(u64),
    Missing,
}

pub trait Host {
    fn allocate_string(&mut self, scope: Scope, request: Request) -> Result<Allocated, Stop>;
    fn free_string(&mut self, scope: Scope, backing: &Owner) -> Result<(), Stop>;
    fn open_scenario(&mut self, filename: &[u8], mode: &[u8]) -> Result<Opened, Stop>;
    fn close_scenario(&mut self, capability: u64) -> Result<(), Stop>;
}

struct StringHost<'a, H> {
    host: &'a mut H,
    scope: Scope,
    old: Option<Owner>,
}

impl<H: Host> strings::Host for StringHost<'_, H> {
    fn allocate_string(&mut self, request: Request) -> Result<Allocated, Stop> {
        self.host.allocate_string(self.scope, request)
    }

    fn free_string(&mut self, token: u64) -> Result<(), Stop> {
        let old = self
            .old
            .as_ref()
            .ok_or_else(|| unsupported("path string has no release owner"))?;
        if token != old.token {
            return Err(unsupported(
                "path string release differs from retained old backing",
            ));
        }
        self.host.free_string(self.scope, old)
    }
}

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn cstring(input: &[u8]) -> &[u8] {
    &input[..input
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(input.len())]
}

fn disjoint(mut spans: Vec<(u64, usize)>) -> Result<(), Stop> {
    spans.sort_unstable();
    if spans
        .windows(2)
        .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
    {
        return Err(unsupported(
            "resource path aliases retained world, application, stack or string ownership",
        ));
    }
    Ok(())
}

pub(super) fn resource_spans(
    resources: &super::State,
    application: &Application,
) -> Result<Vec<(u64, usize)>, Stop> {
    disjoint(application.spans()?)?;
    let mut spans = resources.spans()?;
    let identity = application.prefix.token + 0x68;
    let prefix = &resources.lighting.fixed.arrays.prefix;
    let registry = &prefix.registry;
    if let Some(node) = registry.nodes.get(&(identity as u32)) {
        let expected: Vec<_> = [
            node.handler,
            node.next.unwrap_or(0),
            node.auxiliary,
            node.serial,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
        if application.prefix.data.get(0x68..0x78) != Some(expected.as_slice()) {
            return Err(unsupported(
                "application's embedded Commander bytes differ from the retained typed registry",
            ));
        }
        if prefix.game().is_none() {
            let index = spans
                .iter()
                .position(|span| *span == (identity, 16))
                .ok_or_else(|| {
                    unsupported("application Commander has no retained registry view")
                })?;
            spans.remove(index);
        }
    }
    if let Some(game) = prefix.game() {
        let (_, transferred) = application.game_source(game)?;
        for source in transferred {
            let index = spans
                .iter()
                .position(|span| *span == source)
                .ok_or_else(|| {
                    unsupported("application transfer has no actual Game source owner")
                })?;
            spans.remove(index);
        }
    }
    Ok(spans)
}

pub(super) fn transfer_game_sources<'owner>(
    resources: &super::State,
    owners: impl IntoIterator<Item = &'owner Owner>,
    spans: &mut Vec<(u64, usize)>,
) -> Result<(), Stop> {
    let Some(game) = resources.lighting.fixed.arrays.prefix.game() else {
        return Ok(());
    };
    let incoming = game.construction().incoming();
    for owner in owners {
        let Some(source) = incoming
            .owners
            .iter()
            .find(|source| source.token == owner.token)
        else {
            continue;
        };
        if owner.token == incoming.game || owner != source {
            return Err(unsupported(
                "resource transfer differs from actual Game-source ownership",
            ));
        }
        let index = spans
            .iter()
            .position(|span| *span == (source.token, source.data.len()))
            .ok_or_else(|| unsupported("resource transfer repeats actual Game-source ownership"))?;
        spans.remove(index);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Bare,
    Construct,
    Live,
    Edit,
    Released,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum StringSlot {
    Bare(Owner),
    Construct(strings::Construction),
    Live(strings::State),
    Edit(strings::Edit),
    Released(strings::State),
}

impl StringSlot {
    pub(super) fn kind(&self) -> Kind {
        match self {
            Self::Bare(_) => Kind::Bare,
            Self::Construct(_) => Kind::Construct,
            Self::Live(_) => Kind::Live,
            Self::Edit(_) => Kind::Edit,
            Self::Released(_) => Kind::Released,
        }
    }

    pub(super) fn owner(&self) -> &Owner {
        match self {
            Self::Bare(owner) => owner,
            Self::Construct(execution) => execution.owner(),
            Self::Live(state) | Self::Released(state) => &state.owner,
            Self::Edit(execution) => &execution.state().owner,
        }
    }

    pub(super) fn state(&self) -> Result<&strings::State, Stop> {
        match self {
            Self::Live(state) | Self::Released(state) => Ok(state),
            Self::Edit(execution) => Ok(execution.state()),
            _ => Err(unsupported(
                "path string has not returned its actual construction",
            )),
        }
    }

    pub(super) fn spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        match self {
            Self::Bare(owner) => {
                validate_owner(owner, 12)?;
                Ok(vec![(owner.token, 12)])
            }
            Self::Construct(execution) => execution.spans(),
            Self::Live(state) => {
                state.validate()?;
                Ok(vec![
                    (state.owner.token, 12),
                    (state.backing.token, state.backing.data.len()),
                ])
            }
            Self::Edit(execution) => execution.spans(),
            Self::Released(state) => {
                state.validate()?;
                Ok(vec![(state.owner.token, 12)])
            }
        }
    }

    pub(super) fn construct(&mut self, input: Option<Vec<u8>>) -> Result<(), Stop> {
        if self.kind() != Kind::Bare {
            return Err(unsupported(
                "path string construction would replace owned work",
            ));
        }
        *self = Self::Construct(strings::Construction::new(self.owner().clone(), input)?);
        Ok(())
    }

    pub(super) fn edit(&mut self, operation: Operation) -> Result<(), Stop> {
        if self.kind() != Kind::Live {
            return Err(unsupported(
                "path string edit would replace pending or released work",
            ));
        }
        *self = Self::Edit(strings::Edit::new(self.state()?.clone(), operation)?);
        Ok(())
    }

    pub(super) fn step(&mut self, host: &mut impl Host, scope: Scope) -> Result<bool, Stop> {
        let old = match self {
            Self::Edit(execution) => Some(execution.state().backing.clone()),
            _ => None,
        };
        let mut host = StringHost { host, scope, old };
        let complete = match self {
            Self::Construct(execution) => execution.step(&mut host)?,
            Self::Edit(execution) => execution.step(&mut host)?,
            _ => return Err(unsupported("path string has no active constructor or edit")),
        };
        if complete {
            let state = match self.clone() {
                Self::Construct(execution) => execution.into_state()?,
                Self::Edit(execution) => execution.into_state()?,
                _ => unreachable!(),
            };
            *self = Self::Live(state);
        }
        Ok(complete)
    }

    pub(super) fn release(&mut self, host: &mut impl Host, scope: Scope) -> Result<(), Stop> {
        if self.kind() != Kind::Live {
            return Err(unsupported("path release requires its actual live backing"));
        }
        host.free_string(scope, &self.state()?.backing)?;
        *self = Self::Released(self.state()?.clone());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Application {
    prefix: Owner,
    path: StringSlot,
    suffix: Owner,
    retained: Vec<Owner>,
}

impl Application {
    fn game_source(
        &self,
        game: &crate::native_game::State,
    ) -> Result<(strings::State, Vec<(u64, usize)>), Stop> {
        game.validate()?;
        disjoint(self.spans()?)?;
        let incoming = game.construction().incoming();
        let owners = &incoming.owners;
        let source = owners
            .iter()
            .find(|owner| owner.token == self.prefix.token && owner.token != incoming.game)
            .ok_or_else(|| unsupported("application has no same-epoch Game source allocation"))?;
        validate_owner(source, 324)?;
        if self.prefix.data != source.data[..180] || self.suffix.data != source.data[192..] {
            return Err(unsupported(
                "application changed its retained Game-source prefix or suffix",
            ));
        }
        let beginning = u32::from_le_bytes(source.data[180..184].try_into().unwrap());
        let backing = owners
            .iter()
            .find(|owner| owner.token == u64::from(beginning) && owner.token != incoming.game)
            .ok_or_else(|| unsupported("application path has no same-epoch Game source backing"))?;
        let initial = strings::State {
            owner: Owner {
                token: source.token + 180,
                data: source.data[180..192].to_vec(),
            },
            backing: backing.clone(),
        };
        initial.validate()?;
        let mut transferred = vec![
            (source.token, source.data.len()),
            (backing.token, backing.data.len()),
        ];
        for retained in &self.retained {
            if retained.token == incoming.game || !owners.iter().any(|owner| owner == retained) {
                return Err(unsupported(
                    "application retained owner differs from actual Game source",
                ));
            }
            transferred.push((retained.token, retained.data.len()));
        }
        match &self.path {
            StringSlot::Edit(execution) if execution.initial() == &initial => {}
            StringSlot::Live(current) => {
                let mut assigned = initial.clone();
                if initial.bytes()?.is_empty() {
                    if assigned.backing.data.len() < 3 {
                        assigned.backing = Owner {
                            token: current.backing.token,
                            data: vec![b'.', b'/', 0],
                        };
                        write_word(&mut assigned.owner, 0, assigned.backing.token as u32);
                        write_word(&mut assigned.owner, 8, (assigned.backing.token + 3) as u32);
                    } else {
                        assigned.backing.data[..3].copy_from_slice(b"./\0");
                    }
                    write_word(&mut assigned.owner, 4, (assigned.backing.token + 2) as u32);
                }
                if current != &initial && current != &assigned {
                    return Err(unsupported(
                        "application path lost its actual Game-source continuation",
                    ));
                }
            }
            _ => {
                return Err(unsupported(
                    "application edit lost its actual Game-source path",
                ));
            }
        }
        Ok((initial, transferred))
    }

    pub fn new(owner: Owner, path_backing: Owner, retained: Vec<Owner>) -> Result<Self, Stop> {
        validate_owner(&owner, 324)?;
        let result = Self {
            prefix: Owner {
                token: owner.token,
                data: owner.data[..180].to_vec(),
            },
            path: StringSlot::Live(strings::State {
                owner: Owner {
                    token: owner.token + 180,
                    data: owner.data[180..192].to_vec(),
                },
                backing: path_backing,
            }),
            suffix: Owner {
                token: owner.token + 192,
                data: owner.data[192..].to_vec(),
            },
            retained,
        };
        disjoint(result.spans()?)?;
        Ok(result)
    }

    pub(super) fn spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        validate_owner(&self.prefix, 180)?;
        validate_owner(&self.suffix, 132)?;
        if self.path.owner().token != self.prefix.token + 180
            || self.suffix.token != self.prefix.token + 192
            || !matches!(self.path.kind(), Kind::Live | Kind::Edit)
        {
            return Err(unsupported(
                "application path is not the single owned descriptor at+b4",
            ));
        }
        let mut spans = self.path.spans()?;
        spans.extend([(self.prefix.token, 180), (self.suffix.token, 132)]);
        for owner in &self.retained {
            validate_owner(owner, owner.data.len())?;
            spans.push((owner.token, owner.data.len()));
        }
        Ok(spans)
    }

    pub fn owner(&self) -> Result<Owner, Stop> {
        disjoint(self.spans()?)?;
        let mut data = self.prefix.data.clone();
        data.extend_from_slice(&self.path.owner().data);
        data.extend_from_slice(&self.suffix.data);
        Ok(Owner {
            token: self.prefix.token,
            data,
        })
    }

    pub fn path(&self) -> Result<&strings::State, Stop> {
        self.path.state()
    }

    pub fn retained(&self) -> &[Owner] {
        &self.retained
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub directory: Option<Vec<u8>>,
    pub filename: Option<Vec<u8>>,
}

impl Inputs {
    pub fn lightmap() -> Self {
        Self {
            directory: Some(b"GameData/".to_vec()),
            filename: Some(b"lightmap.dat".to_vec()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incoming {
    pub application: Application,
    pub buffer: Owner,
    pub output_descriptor: Owner,
    pub scenario_descriptor: Owner,
    pub retained: Vec<Owner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    BufferConstructed,
    ComposedPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Phase {
    Initialize(u8),
    BindBuffer,
    StartOutput,
    Output,
    RootBranch,
    AssignApplication,
    StartScenario,
    Scenario,
    AppendScenario,
    OpenScenario,
    AcceptScenario,
    CloseScenario,
    StartOutputAssign,
    AssignOutput,
    ReleaseScenario,
    ResetBuffer,
    AppendRoot,
    AppendDirectory,
    EnsureSeparator,
    AppendFilename,
    ReleaseOutput,
    Complete,
}

fn append(buffer: &mut Owner, input: &[u8]) {
    let length = u16::from_le_bytes(buffer.data[8..10].try_into().unwrap()) as usize;
    let input = cstring(input);
    let count = input.len().min(CAPACITY - 1 - length);
    buffer.data[12 + length..12 + length + count].copy_from_slice(&input[..count]);
    buffer.data[12 + length + count] = 0;
    buffer.data[8..10].copy_from_slice(&((length + count) as u16).to_le_bytes());
}

fn initialize(buffer: &mut Owner, index: u8) {
    match index {
        0 => buffer.data[8..10].fill(0),
        1 => write_word(buffer, 4, (buffer.token + 12) as u32),
        2 => buffer.data[10..12].copy_from_slice(&(CAPACITY as u16).to_le_bytes()),
        3 => write_word(buffer, 0, 0x667858),
        4 => buffer.data[12] = 0,
        _ => unreachable!(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    resources: super::State,
    application: Application,
    buffer: Owner,
    output: StringSlot,
    scenario: StringSlot,
    retained: Vec<Owner>,
    inputs: Inputs,
    initial_buffer: Vec<u8>,
    initial_output: Owner,
    initial_scenario: Owner,
    initial_path: strings::State,
    phase: Phase,
    opened: Option<Opened>,
}

impl Construction {
    pub fn new(
        resources: super::Construction,
        incoming: Incoming,
        inputs: Inputs,
    ) -> Result<Self, Stop> {
        validate_owner(&incoming.buffer, 272)?;
        validate_owner(&incoming.output_descriptor, 12)?;
        validate_owner(&incoming.scenario_descriptor, 12)?;
        let resources = resources.into_state()?;
        if let Some(game) = resources.lighting.fixed.arrays.prefix.game() {
            let (initial, _) = incoming.application.game_source(game)?;
            if incoming.application.path()? != &initial {
                return Err(unsupported(
                    "resource path entry replayed or replaced its Game-source path",
                ));
            }
        }
        let result = Self {
            resources,
            initial_path: incoming.application.path()?.clone(),
            application: incoming.application,
            initial_buffer: incoming.buffer.data.clone(),
            buffer: incoming.buffer,
            initial_output: incoming.output_descriptor.clone(),
            output: StringSlot::Bare(incoming.output_descriptor),
            initial_scenario: incoming.scenario_descriptor.clone(),
            scenario: StringSlot::Bare(incoming.scenario_descriptor),
            retained: incoming.retained,
            inputs,
            phase: Phase::Initialize(0),
            opened: None,
        };
        result.validate()?;
        Ok(result)
    }

    fn cached(&self) -> Result<bool, Stop> {
        Ok(!self.initial_path.bytes()?.is_empty())
    }

    fn root(&self) -> Result<&[u8], Stop> {
        if self.cached()? {
            self.initial_path.bytes()
        } else {
            Ok(ROOT)
        }
    }

    fn expected_buffer(&self) -> Result<Owner, Stop> {
        let mut expected = Owner {
            token: self.buffer.token,
            data: self.initial_buffer.clone(),
        };
        validate_owner(&expected, 272)?;
        let initialized = match self.phase {
            Phase::Initialize(count) if count < 5 => count,
            Phase::Initialize(_) => {
                return Err(unsupported("invalid path buffer initialization phase"));
            }
            _ => 5,
        };
        for index in 0..initialized {
            initialize(&mut expected, index);
        }
        if self.phase >= Phase::StartOutput {
            write_word(&mut expected, 0, 0x6664e8);
        }
        if self.phase >= Phase::AppendDirectory {
            append(&mut expected, self.root()?);
        }
        if self.phase >= Phase::EnsureSeparator {
            if let Some(directory) = &self.inputs.directory {
                append(&mut expected, directory);
            }
        }
        if self.phase >= Phase::AppendFilename {
            let length = u16::from_le_bytes(expected.data[8..10].try_into().unwrap()) as usize;
            if length == 0 {
                return Err(unsupported(
                    "empty path would read before the native buffer",
                ));
            }
            if !matches!(expected.data[11 + length], b'/' | b'\\') {
                append(&mut expected, b"/");
            }
        }
        if self.phase >= Phase::ReleaseOutput {
            if let Some(filename) = &self.inputs.filename {
                append(&mut expected, filename);
            }
        }
        Ok(expected)
    }

    fn validate(&self) -> Result<(), Stop> {
        if let Some(game) = self.resources.lighting.fixed.arrays.prefix.game() {
            let (initial, _) = self.application.game_source(game)?;
            if self.initial_path != initial {
                return Err(unsupported(
                    "resource path provenance differs from actual Game source",
                ));
            }
        }
        let cached = self.cached()?;
        if cached
            && matches!(
                self.phase,
                Phase::AssignApplication
                    | Phase::StartScenario
                    | Phase::Scenario
                    | Phase::AppendScenario
                    | Phase::OpenScenario
                    | Phase::AcceptScenario
                    | Phase::CloseScenario
                    | Phase::ReleaseScenario
            )
        {
            return Err(unsupported(
                "cached application path cannot restore a Scenario-probe phase",
            ));
        }
        let output_kind = match self.phase {
            Phase::Initialize(_) | Phase::BindBuffer | Phase::StartOutput => Kind::Bare,
            Phase::Output => Kind::Construct,
            Phase::AssignOutput => Kind::Edit,
            Phase::Complete => Kind::Released,
            _ => Kind::Live,
        };
        let scenario_kind = if cached || self.phase < Phase::Scenario {
            Kind::Bare
        } else if self.phase == Phase::Scenario {
            Kind::Construct
        } else if self.phase == Phase::AppendScenario {
            Kind::Edit
        } else if self.phase >= Phase::ResetBuffer {
            Kind::Released
        } else {
            Kind::Live
        };
        if self.output.kind() != output_kind
            || self.scenario.kind() != scenario_kind
            || self.application.path.kind()
                != if self.phase == Phase::AssignApplication {
                    Kind::Edit
                } else {
                    Kind::Live
                }
            || self.output.owner().token != self.initial_output.token
            || self.scenario.owner().token != self.initial_scenario.token
            || self.application.path.owner().token != self.initial_path.owner.token
        {
            return Err(unsupported(
                "resource path lost its exact phase-specific string ownership",
            ));
        }
        if self.output.kind() == Kind::Bare && self.output.owner() != &self.initial_output
            || self.scenario.kind() == Kind::Bare && self.scenario.owner() != &self.initial_scenario
        {
            return Err(unsupported(
                "resource path changed an unconstructed stack descriptor",
            ));
        }
        for (slot, input) in [(&self.output, None), (&self.scenario, Some(self.root()?))] {
            if let StringSlot::Construct(execution) = slot {
                if execution.input() != input {
                    return Err(unsupported("path constructor lost its actual source"));
                }
            }
        }
        for (slot, operation) in [
            (&self.application.path, Operation::Assign(ROOT.to_vec())),
            (&self.output, Operation::Assign(self.root()?.to_vec())),
            (&self.scenario, Operation::Append(SCENARIO.to_vec())),
        ] {
            if let StringSlot::Edit(execution) = slot {
                if execution.operation() != &operation {
                    return Err(unsupported("path edit differs from its actual caller"));
                }
            }
        }
        if let StringSlot::Edit(execution) = &self.application.path {
            if execution.initial() != &self.initial_path {
                return Err(unsupported(
                    "application root edit lost its actual incoming backing",
                ));
            }
        }
        if self.application.path.kind() == Kind::Live {
            let mut expected = self.initial_path.clone();
            if !cached && self.phase > Phase::RootBranch {
                if expected.backing.data.len() < 3 {
                    expected.backing = Owner {
                        token: self.application.path()?.backing.token,
                        data: vec![b'.', b'/', 0],
                    };
                    write_word(&mut expected.owner, 0, expected.backing.token as u32);
                    write_word(&mut expected.owner, 8, (expected.backing.token + 3) as u32);
                } else {
                    expected.backing.data[..3].copy_from_slice(b"./\0");
                }
                write_word(&mut expected.owner, 4, (expected.backing.token + 2) as u32);
            }
            if self.application.path()? != &expected {
                return Err(unsupported(
                    "application cached root differs from actual path branch or untouched bytes",
                ));
            }
        }
        if self.phase >= Phase::ReleaseScenario && self.output.state()?.bytes()? != self.root()? {
            return Err(unsupported(
                "path output temporary differs from the retained application root",
            ));
        }
        if !cached && self.phase >= Phase::OpenScenario {
            let mut expected = self.root()?.to_vec();
            expected.extend_from_slice(SCENARIO);
            if self.scenario.state()?.bytes()? != expected {
                return Err(unsupported(
                    "Scenario filename differs from actual source concatenation",
                ));
            }
        }
        if matches!(self.phase, Phase::AcceptScenario | Phase::CloseScenario)
            != self.opened.is_some()
        {
            return Err(unsupported(
                "resource path lost its pending/open file capability",
            ));
        }
        if self.phase == Phase::CloseScenario
            && !matches!(self.opened, Some(Opened::File(token)) if token != 0 && token <= u64::from(u32::MAX))
        {
            return Err(unsupported(
                "resource path close has no native-width opened capability",
            ));
        }
        if self.buffer != self.expected_buffer()? {
            return Err(unsupported(
                "resource path changed written or untouched stack-buffer bytes",
            ));
        }
        let mut spans = resource_spans(&self.resources, &self.application)?;
        transfer_game_sources(&self.resources, &self.retained, &mut spans)?;
        spans.extend(self.application.spans()?);
        spans.extend(self.output.spans()?);
        spans.extend(self.scenario.spans()?);
        spans.push((self.buffer.token, self.buffer.data.len()));
        for owner in &self.retained {
            validate_owner(owner, owner.data.len())?;
            spans.push((owner.token, owner.data.len()));
        }
        disjoint(spans)
    }

    pub fn cut(&self) -> Option<Cut> {
        match self.phase {
            Phase::BindBuffer => Some(Cut::BufferConstructed),
            Phase::Complete => Some(Cut::ComposedPath),
            _ => None,
        }
    }

    pub fn application(&self) -> &Application {
        &self.application
    }
    pub fn buffer(&self) -> &Owner {
        &self.buffer
    }
    pub fn resources(&self) -> &super::State {
        &self.resources
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        match self.phase {
            Phase::Initialize(index) => {
                initialize(&mut self.buffer, index);
                self.phase = if index == 4 { Phase::BindBuffer } else { Phase::Initialize(index + 1) };
            }
            Phase::BindBuffer => { write_word(&mut self.buffer, 0, 0x6664e8); self.phase = Phase::StartOutput; }
            Phase::StartOutput => { self.output.construct(None)?; self.phase = Phase::Output; }
            Phase::Output => { if self.output.step(host, Scope::World)? { self.phase = Phase::RootBranch; } }
            Phase::RootBranch => {
                if self.cached()? { self.phase = Phase::StartOutputAssign; }
                else { self.application.path.edit(Operation::Assign(ROOT.to_vec()))?; self.phase = Phase::AssignApplication; }
            }
            Phase::AssignApplication => { if self.application.path.step(host, Scope::Path)? { self.phase = Phase::StartScenario; } }
            Phase::StartScenario => { self.scenario.construct(Some(self.application.path()?.bytes()?.to_vec()))?; self.phase = Phase::Scenario; }
            Phase::Scenario => {
                if self.scenario.step(host, Scope::Path)? { self.scenario.edit(Operation::Append(SCENARIO.to_vec()))?; self.phase = Phase::AppendScenario; }
            }
            Phase::AppendScenario => { if self.scenario.step(host, Scope::Path)? { self.phase = Phase::OpenScenario; } }
            Phase::OpenScenario => { self.opened = Some(host.open_scenario(cstring(self.scenario.state()?.bytes()?), MODE)?); self.phase = Phase::AcceptScenario; }
            Phase::AcceptScenario => {
                match self.opened.unwrap() {
                    Opened::File(token) if token != 0 && token <= u64::from(u32::MAX) => self.phase = Phase::CloseScenario,
                    Opened::Missing => return Err(Stop::Unavailable("original Scenario probe failed; native fatal-exit branch is not a successful path".into())),
                    _ => return Err(unsupported("Scenario open returned an unknown/null native capability")),
                }
            }
            Phase::CloseScenario => {
                let Some(Opened::File(token)) = self.opened else { unreachable!() };
                host.close_scenario(token)?;
                self.opened = None;
                self.phase = Phase::StartOutputAssign;
            }
            Phase::StartOutputAssign => { self.output.edit(Operation::Assign(self.application.path()?.bytes()?.to_vec()))?; self.phase = Phase::AssignOutput; }
            Phase::AssignOutput => {
                if self.output.step(host, Scope::World)? { self.phase = if self.cached()? { Phase::ResetBuffer } else { Phase::ReleaseScenario }; }
            }
            Phase::ReleaseScenario => { self.scenario.release(host, Scope::Path)?; self.phase = Phase::ResetBuffer; }
            Phase::ResetBuffer => { self.buffer.data[8..10].fill(0); self.buffer.data[12] = 0; self.phase = Phase::AppendRoot; }
            Phase::AppendRoot => { append(&mut self.buffer, self.output.state()?.bytes()?); self.phase = Phase::AppendDirectory; }
            Phase::AppendDirectory => {
                if let Some(directory) = &self.inputs.directory { append(&mut self.buffer, directory); }
                self.phase = Phase::EnsureSeparator;
            }
            Phase::EnsureSeparator => {
                let length = u16::from_le_bytes(self.buffer.data[8..10].try_into().unwrap()) as usize;
                if length == 0 { return Err(unsupported("empty path would read before the native buffer")); }
                if !matches!(self.buffer.data[11 + length], b'/' | b'\\') { append(&mut self.buffer, b"/"); }
                self.phase = Phase::AppendFilename;
            }
            Phase::AppendFilename => {
                if let Some(filename) = &self.inputs.filename { append(&mut self.buffer, filename); }
                self.phase = Phase::ReleaseOutput;
            }
            Phase::ReleaseOutput => { self.output.release(host, Scope::World)?; self.phase = Phase::Complete; }
            Phase::Complete => {}
        }
        self.validate()?;
        Ok(self.cut())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cut() != Some(Cut::ComposedPath) {
            return Err(Stop::Unavailable(
                "resource path has not reached actual4b5783".into(),
            ));
        }
        Ok(State {
            output_descriptor: self.output.owner().clone(),
            scenario_descriptor: self.scenario.owner().clone(),
            resources: self.resources,
            application: self.application,
            buffer: self.buffer,
            retained: self.retained,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub resources: super::State,
    pub application: Application,
    pub buffer: Owner,
    pub output_descriptor: Owner,
    pub scenario_descriptor: Owner,
    pub retained: Vec<Owner>,
}

impl State {
    pub fn bytes(&self) -> Result<&[u8], Stop> {
        validate_owner(&self.buffer, 272)?;
        let length = u16::from_le_bytes(self.buffer.data[8..10].try_into().unwrap()) as usize;
        if self.buffer.data[..4] != 0x6664e8_u32.to_le_bytes()
            || self.buffer.data[4..8] != ((self.buffer.token + 12) as u32).to_le_bytes()
            || self.buffer.data[10..12] != (CAPACITY as u16).to_le_bytes()
            || length >= CAPACITY
            || self.buffer.data[12 + length] != 0
            || self.buffer.data[12..12 + length].contains(&0)
        {
            return Err(unsupported(
                "returned resource path has an invalid native bounded buffer",
            ));
        }
        Ok(&self.buffer.data[12..12 + length])
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.bytes()?;
        if self.application.path.kind() != Kind::Live {
            return Err(unsupported(
                "returned resource path retains an unfinished application edit",
            ));
        }
        let mut spans = resource_spans(&self.resources, &self.application)?;
        transfer_game_sources(&self.resources, &self.retained, &mut spans)?;
        spans.extend(self.application.spans()?);
        for owner in [
            &self.buffer,
            &self.output_descriptor,
            &self.scenario_descriptor,
        ]
        .into_iter()
        .chain(self.retained.iter())
        {
            validate_owner(owner, owner.data.len())?;
            spans.push((owner.token, owner.data.len()));
        }
        validate_owner(&self.output_descriptor, 12)?;
        validate_owner(&self.scenario_descriptor, 12)?;
        disjoint(spans)
    }
}
