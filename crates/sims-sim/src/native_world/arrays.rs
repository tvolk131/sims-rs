use super::{Parts, Prefix, validate_owner, write_word};
use crate::native_matrix::{self, Naming, Registry};
use crate::native_world_children::{Owner, vector};
use crate::saved_matrix::{Allocated, Allocation, Backing, Stop};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod naming;

pub trait Host: native_matrix::Host + vector::Host {
    fn allocate_array_root(&mut self, slot: u8, bytes: u32) -> Result<Allocated, Stop>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub roof_owners: u32,
    pub random: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Matrix(i32, i32),
    Pool,
    Water,
    Roof,
    Vector(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Spec {
    slot: u8,
    kind: Kind,
}

impl Spec {
    fn extent(self) -> usize {
        match self.kind {
            Kind::Pool | Kind::Water | Kind::Roof => 40,
            Kind::Vector(_) => 12,
            Kind::Matrix(_, _) => 32,
        }
    }

    fn offset(self) -> usize {
        usize::from(matches!(self.kind, Kind::Pool | Kind::Water | Kind::Roof)) * 4
    }

    fn shape(self) -> Result<(i32, i32), Stop> {
        match self.kind {
            Kind::Matrix(side, stride) => Ok((side, stride)),
            Kind::Pool | Kind::Water => Ok((64, 1)),
            Kind::Roof => Ok((128, 2)),
            Kind::Vector(_) => Err(Stop::Unsupported(
                "element vector is not a named matrix".into(),
            )),
        }
    }
}

const SPECS: [Spec; 22] = [
    Spec {
        slot: 0x14,
        kind: Kind::Matrix(64, 4),
    },
    Spec {
        slot: 0x18,
        kind: Kind::Matrix(64, 1),
    },
    Spec {
        slot: 0x1c,
        kind: Kind::Matrix(64, 1),
    },
    Spec {
        slot: 0x20,
        kind: Kind::Matrix(64, 1),
    },
    Spec {
        slot: 0x60,
        kind: Kind::Pool,
    },
    Spec {
        slot: 0x64,
        kind: Kind::Water,
    },
    Spec {
        slot: 0x24,
        kind: Kind::Matrix(64, 2),
    },
    Spec {
        slot: 0x2c,
        kind: Kind::Matrix(64, 2),
    },
    Spec {
        slot: 0x34,
        kind: Kind::Matrix(64, 1),
    },
    Spec {
        slot: 0x44,
        kind: Kind::Matrix(64, 14),
    },
    Spec {
        slot: 0x3c,
        kind: Kind::Matrix(64, 2),
    },
    Spec {
        slot: 0x4c,
        kind: Kind::Matrix(64, 1),
    },
    Spec {
        slot: 0x68,
        kind: Kind::Vector(0),
    },
    Spec {
        slot: 0x58,
        kind: Kind::Roof,
    },
    Spec {
        slot: 0x28,
        kind: Kind::Matrix(64, 2),
    },
    Spec {
        slot: 0x30,
        kind: Kind::Matrix(64, 2),
    },
    Spec {
        slot: 0x38,
        kind: Kind::Matrix(64, 1),
    },
    Spec {
        slot: 0x48,
        kind: Kind::Matrix(64, 14),
    },
    Spec {
        slot: 0x40,
        kind: Kind::Matrix(64, 2),
    },
    Spec {
        slot: 0x50,
        kind: Kind::Matrix(64, 1),
    },
    Spec {
        slot: 0x6c,
        kind: Kind::Vector(1),
    },
    Spec {
        slot: 0x5c,
        kind: Kind::Roof,
    },
];

const NAMES: [(u8, &[u8]); 18] = [
    (0x14, b"altitude"),
    (0x18, b"ground"),
    (0x1c, b"grass"),
    (0x20, b"target grass"),
    (0x60, b"Pool"),
    (0x64, b"Water"),
    (0x24, b"floors"),
    (0x2c, b"rooms"),
    (0x34, b"flags"),
    (0x44, b"walls"),
    (0x3c, b"objects"),
    (0x58, b"roof0"),
    (0x28, b"floors2"),
    (0x30, b"rooms2"),
    (0x38, b"flags2"),
    (0x48, b"walls2"),
    (0x40, b"objects2"),
    (0x5c, b"roof1"),
];

fn spec_for(slot: u8) -> Result<Spec, Stop> {
    SPECS
        .iter()
        .copied()
        .find(|spec| spec.slot == slot)
        .ok_or_else(|| Stop::Unsupported("unknown original world-array slot".into()))
}

fn empty_teardown_slots() -> impl Iterator<Item = usize> {
    [0x14, 0x18, 0x1c, 0x20]
        .into_iter()
        .chain((0..2).flat_map(|plane| {
            [0x24, 0x44, 0x3c, 0x2c, 0x34, 0x4c, 0x68, 0x58]
                .into_iter()
                .map(move |slot| slot + plane * 4)
        }))
}

fn header(node: &native_matrix::Node) -> Result<[u8; 32], Stop> {
    let directory = match &node.matrix.allocation {
        Allocation::Null => 0,
        Allocation::Known(backing) => backing.token,
        Allocation::Released { token } => *token,
        _ => {
            return Err(Stop::Unavailable(
                "fresh matrix directory is not retained".into(),
            ));
        }
    };
    let words = [
        node.next.unwrap_or(0),
        node.matrix.width as u32 as u64,
        node.matrix.height as u32 as u64,
        directory,
        node.name.token,
        node.name.token + node.name.length as u64,
        node.name.token + node.name.data.len() as u64,
        node.matrix.stride as u32 as u64,
    ];
    let mut bytes = [0; 32];
    for (index, word) in words.into_iter().enumerate() {
        let word = u32::try_from(word).map_err(|_| {
            Stop::Unsupported("matrix header exceeds original pointer width".into())
        })?;
        bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    Ok(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub prefix: Parts,
    pub matrices: Registry,
    pub roots: BTreeMap<u8, Owner>,
    pub vectors: [Option<vector::State>; 2],
    pub context: Context,
}

impl State {
    pub(super) fn spans(&self) -> Result<Vec<(u64, usize)>, Stop> {
        self.prefix.validate_game()?;
        let mut spans = Vec::new();
        let mut add = |owner: &Owner, extent| -> Result<(), Stop> {
            validate_owner(owner, extent)?;
            spans.push((owner.token, extent));
            Ok(())
        };
        add(&self.prefix.owner, 136)?;
        if let Some(companion) = &self.prefix.companion {
            companion.validate()?;
            add(&companion.owner, 72)?;
            if let Some(child) = &companion.child {
                add(child, 96)?;
            }
            for vector in companion.vectors.iter().flatten() {
                add(vector, 12)?;
            }
        }
        for (slot, owner) in &self.roots {
            add(owner, spec_for(*slot)?.extent())?;
        }
        for vector in self.vectors.iter().flatten() {
            add(&vector.owner, 12)?;
            let backing = vector.backing.as_ref().ok_or_else(|| {
                Stop::Unavailable("published element-vector backing absent".into())
            })?;
            add(backing, 16384)?;
            let beginning = u32::try_from(backing.token).unwrap();
            let end = u32::try_from(backing.token + 16384).map_err(|_| {
                Stop::Unsupported("element-vector end exceeds native pointer width".into())
            })?;
            if vector.owner.data
                != [beginning, end, end]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>()
            {
                return Err(Stop::Unsupported(
                    "retained element-vector header differs from its backing".into(),
                ));
            }
        }
        for node in self.matrices.nodes.values() {
            validate_owner(
                &Owner {
                    token: node.name.token,
                    data: node.name.data.clone(),
                },
                node.name.data.len(),
            )?;
            spans.push((node.name.token, node.name.data.len()));
            if let Allocation::Known(backing) = &node.matrix.allocation {
                let columns = backing.column_offsets.len();
                let extent = columns
                    .checked_mul(4)
                    .and_then(|bytes| bytes.checked_add(backing.data.len()))
                    .ok_or_else(|| Stop::Unsupported("matrix backing extent overflow".into()))?;
                if backing.token == 0
                    || backing.token > u32::MAX as u64
                    || backing.token + extent as u64 > u64::from(u32::MAX) + 1
                {
                    return Err(Stop::Unsupported(
                        "matrix backing exceeds native address width".into(),
                    ));
                }
                spans.push((backing.token, extent));
            }
        }
        if let Some(game) = self.prefix.game() {
            spans.extend(
                game.construction()
                    .owners()
                    .iter()
                    .map(|owner| (owner.token, owner.data.len())),
            );
        }
        for identity in self.prefix.registry.nodes.keys() {
            let beginning = u64::from(*identity);
            if self.prefix.game().is_some_and(|game| {
                game.construction().owners().iter().any(|owner| {
                    owner.token <= beginning
                        && beginning + 16 <= owner.token + owner.data.len() as u64
                })
            }) {
                continue;
            }
            if u64::from(*identity) != self.prefix.owner.token {
                spans.push((u64::from(*identity), 16));
            }
        }
        spans.sort_unstable();
        if spans
            .windows(2)
            .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
        {
            return Err(Stop::Unsupported(
                "world-array allocations overlap retained ownership".into(),
            ));
        }
        Ok(spans)
    }

    pub(super) fn identity(&self, slot: u8) -> Result<u64, Stop> {
        let owner = self
            .roots
            .get(&slot)
            .ok_or_else(|| Stop::Unavailable("world-array root absent".into()))?;
        Ok(owner.token + spec_for(slot)?.offset() as u64)
    }

    pub(super) fn backing_mut(&mut self, slot: u8) -> Result<&mut Backing, Stop> {
        let identity = self.identity(slot)?;
        let node = self
            .matrices
            .nodes
            .get_mut(&identity)
            .ok_or_else(|| Stop::Unavailable("world-array named owner absent".into()))?;
        match &mut node.matrix.allocation {
            Allocation::Known(backing) => Ok(backing),
            _ => Err(Stop::Unavailable("world-array payload unavailable".into())),
        }
    }

    pub(super) fn synchronize(&mut self) -> Result<(), Stop> {
        for (slot, owner) in &mut self.roots {
            let offset = spec_for(*slot)?.offset();
            if let Some(node) = self.matrices.nodes.get(&(owner.token + offset as u64)) {
                owner.data[offset..offset + 32].copy_from_slice(&header(node)?);
            }
        }
        Ok(())
    }

    pub(super) fn validate(&self) -> Result<(), Stop> {
        self.validate_partial()?;
        self.validate_retained()
    }

    fn validate_partial(&self) -> Result<(), Stop> {
        if self
            .matrices
            .nodes
            .values()
            .any(|node| matches!(node.matrix.allocation, Allocation::Released { .. }))
        {
            return Err(Stop::Unsupported(
                "fresh world arrays cannot retain a released matrix backing".into(),
            ));
        }
        self.validate_structure()
    }

    pub(super) fn validate_retained(&self) -> Result<(), Stop> {
        self.validate_structure()?;
        if self.roots.len() != 20
            || self.matrices.nodes.len() != 20
            || self.vectors.iter().any(Option::is_none)
        {
            return Err(Stop::Unsupported(
                "retained world arrays lack their complete published owners".into(),
            ));
        }
        for spec in SPECS {
            let token = self.root_token(spec)?;
            if self.prefix.owner.data[spec.slot as usize..spec.slot as usize + 4]
                != (token as u32).to_le_bytes()
                || (!matches!(spec.kind, Kind::Vector(_))
                    && !self.matrices.nodes.contains_key(&self.identity(spec.slot)?))
            {
                return Err(Stop::Unsupported(
                    "retained world-array publication differs from its canonical owner".into(),
                ));
            }
        }
        Ok(())
    }

    fn root_token(&self, spec: Spec) -> Result<u64, Stop> {
        match spec.kind {
            Kind::Vector(index) => self.vectors[index]
                .as_ref()
                .map(|vector| vector.owner.token),
            _ => self.roots.get(&spec.slot).map(|owner| owner.token),
        }
        .ok_or_else(|| Stop::Unavailable("published world-array owner absent".into()))
    }

    fn validate_structure(&self) -> Result<(), Stop> {
        self.prefix.registry.validate().map_err(Stop::Unsupported)?;
        self.matrices.validate()?;
        self.spans()?;
        let identity = self.prefix.owner.token as u32;
        let node = self
            .prefix
            .registry
            .nodes
            .get(&identity)
            .ok_or_else(|| Stop::Unavailable("world Commander owner absent".into()))?;
        if self.prefix.registry.head != Some(identity)
            || node.handler != 0x667a88
            || [
                node.handler,
                node.next.unwrap_or(0),
                node.auxiliary,
                node.serial,
            ]
            .into_iter()
            .enumerate()
            .any(|(index, word)| {
                self.prefix.owner.data[index * 4..index * 4 + 4] != word.to_le_bytes()
            })
        {
            return Err(Stop::Unsupported(
                "world-array fixed Commander domain differs".into(),
            ));
        }
        let companion = self
            .prefix
            .companion
            .as_ref()
            .map_or(0, |state| state.owner.token as u32);
        if self.prefix.owner.data[0x54..0x58] != companion.to_le_bytes() {
            return Err(Stop::Unsupported(
                "world-array companion publication differs".into(),
            ));
        }
        for (identity, node) in &self.matrices.nodes {
            let (slot, root) = self
                .roots
                .iter()
                .find(|(slot, root)| {
                    root.token + spec_for(**slot).map_or(0, Spec::offset) as u64 == *identity
                })
                .ok_or_else(|| {
                    Stop::Unsupported("named matrix has no retained world-array root".into())
                })?;
            let spec = spec_for(*slot)?;
            let offset = spec.offset();
            if root.data[offset..offset + 32] != header(node)?
                || node.matrix.stride != spec.shape()?.1
            {
                return Err(Stop::Unsupported(
                    "matrix root view differs from its canonical typed header".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Action {
    Construct,
    Rename(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    PublishSize,
    EmptyTeardown,
    AllocateRoot,
    AcceptRoot,
    AllocateTemporary(Action),
    AcceptTemporary(Action),
    Matrix,
    ReleaseTemporary(Action),
    Derived,
    ConstructVector,
    PublishRoot,
    IncrementReference,
    Boundary(u8),
    Rename(usize),
    Grass,
    Reached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    CoordinateTablesEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    state: State,
    initial_context: Context,
    sequence: usize,
    phase: Phase,
    pending: Option<Allocated>,
    temporary: Option<Owner>,
    matrix: Option<native_matrix::Construction>,
    vector: Option<vector::Construction>,
    naming: Option<Naming>,
}

impl Construction {
    pub fn new(prefix: Prefix, context: Context) -> Result<Self, Stop> {
        Self::from_parts(prefix.into_parts()?, context)
    }

    pub(crate) fn from_parts(prefix: Parts, context: Context) -> Result<Self, Stop> {
        let result = Self {
            state: State {
                prefix,
                matrices: Registry::default(),
                roots: BTreeMap::new(),
                vectors: [None, None],
                context,
            },
            initial_context: context,
            sequence: 0,
            phase: Phase::PublishSize,
            pending: None,
            temporary: None,
            matrix: None,
            vector: None,
            naming: None,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn temporary(&self) -> Option<&Owner> {
        self.temporary.as_ref()
    }

    pub fn vector_in_progress(&self) -> Option<&vector::State> {
        self.vector.as_ref().map(|construction| &construction.state)
    }

    pub fn cut(&self) -> Option<Cut> {
        (self.phase == Phase::Reached).then_some(Cut::CoordinateTablesEntry)
    }

    fn spec(&self) -> Result<Spec, Stop> {
        SPECS.get(self.sequence).copied().ok_or_else(|| {
            Stop::Unsupported("world-array sequence cursor outside original profile".into())
        })
    }

    fn input(&self, action: Action) -> Result<&'static [u8], Stop> {
        match action {
            Action::Construct => Ok(b""),
            Action::Rename(index) => NAMES.get(index).map(|(_, name)| *name).ok_or_else(|| {
                Stop::Unsupported("world-array rename cursor outside native profile".into())
            }),
        }
    }

    fn validate_progress(&self) -> Result<(), Stop> {
        let constructing = matches!(
            self.phase,
            Phase::AllocateRoot
                | Phase::AcceptRoot
                | Phase::AllocateTemporary(Action::Construct)
                | Phase::AcceptTemporary(Action::Construct)
                | Phase::Matrix
                | Phase::ReleaseTemporary(Action::Construct)
                | Phase::Derived
                | Phase::ConstructVector
                | Phase::PublishRoot
                | Phase::IncrementReference
        );
        if (constructing && self.sequence >= SPECS.len())
            || (matches!(self.phase, Phase::PublishSize | Phase::EmptyTeardown)
                && self.sequence != 0)
            || (!constructing
                && !matches!(self.phase, Phase::PublishSize | Phase::EmptyTeardown)
                && self.sequence != SPECS.len())
        {
            return Err(Stop::Unsupported(
                "world-array phase and sequence differ".into(),
            ));
        }
        let current_matrix = matches!(
            self.phase,
            Phase::AllocateTemporary(Action::Construct)
                | Phase::AcceptTemporary(Action::Construct)
                | Phase::Matrix
                | Phase::ReleaseTemporary(Action::Construct)
                | Phase::Derived
                | Phase::PublishRoot
                | Phase::IncrementReference
        );
        let mut roots = 0;
        let mut matrices = 0;
        for (index, spec) in SPECS.into_iter().enumerate() {
            let previous = index < self.sequence;
            let current = index == self.sequence;
            let owned = previous || (current && current_matrix);
            if let Kind::Vector(vector) = spec.kind {
                let expected = previous || (current && self.phase == Phase::PublishRoot);
                if self.state.vectors[vector].is_some() != expected
                    || (current && current_matrix && self.phase != Phase::PublishRoot)
                {
                    return Err(Stop::Unsupported(
                        "world-array vector and sequence differ".into(),
                    ));
                }
            } else {
                roots += usize::from(owned);
                if self.state.roots.contains_key(&spec.slot) != owned
                    || (current && self.phase == Phase::ConstructVector)
                {
                    return Err(Stop::Unsupported(
                        "world-array root and sequence differ".into(),
                    ));
                }
                if owned {
                    let identity = self.state.identity(spec.slot)?;
                    let header_constructed = previous
                        || (current
                            && matches!(
                                self.phase,
                                Phase::ReleaseTemporary(Action::Construct)
                                    | Phase::Derived
                                    | Phase::PublishRoot
                                    | Phase::IncrementReference
                            ));
                    let present = self.state.matrices.nodes.contains_key(&identity);
                    if (self.phase != Phase::Matrix && present != header_constructed)
                        || (previous && !present)
                    {
                        return Err(Stop::Unsupported(
                            "world-array matrix header and sequence differ".into(),
                        ));
                    }
                    matrices += usize::from(present);
                }
            }
            let published = previous || (current && self.phase == Phase::IncrementReference);
            let token = if published {
                self.state.root_token(spec)?
            } else {
                0
            };
            if self.state.prefix.owner.data[spec.slot as usize..spec.slot as usize + 4]
                != (token as u32).to_le_bytes()
            {
                return Err(Stop::Unsupported(
                    "world-array publication precedes or loses its owner".into(),
                ));
            }
        }
        if self.state.roots.len() != roots || self.state.matrices.nodes.len() != matrices {
            return Err(Stop::Unsupported(
                "world-array cursor retains foreign roots or matrices".into(),
            ));
        }
        if let Some(matrix) = &self.matrix {
            let spec = self.spec()?;
            let (side, stride) = spec.shape()?;
            matrix.validate_for(
                &self.state.matrices,
                self.state.identity(spec.slot)?,
                stride,
                side,
                side,
                b"",
            )?;
        }
        if let Phase::Rename(index) = self.phase {
            let (slot, input) = NAMES.get(index).ok_or_else(|| {
                Stop::Unsupported("world-array rename index outside profile".into())
            })?;
            self.naming
                .as_ref()
                .ok_or_else(|| Stop::Unavailable("matrix naming cursor absent".into()))?
                .validate_for(&self.state.matrices, self.state.identity(*slot)?, input)?;
        }
        if self.phase == Phase::IncrementReference
            && !matches!(self.spec()?.kind, Kind::Pool | Kind::Water)
        {
            return Err(Stop::Unsupported(
                "world-array reference cursor has no derived owner".into(),
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), Stop> {
        self.state.validate_partial()?;
        if let Some(game) = self.state.prefix.game() {
            for reply in self.pending.iter().chain(
                self.vector
                    .iter()
                    .filter_map(|vector| vector.pending_allocation()),
            ) {
                if let Allocated::Bytes { token, data } = reply {
                    game.guard_span(*token, data.len())?;
                }
            }
            if let Some(matrix) = &self.matrix {
                for (token, bytes) in matrix.retained_spans()? {
                    game.guard_span(token, bytes)?;
                }
            }
            if let Some(naming) = &self.naming {
                for (token, bytes) in naming.retained_spans() {
                    game.guard_span(token, bytes)?;
                }
            }
        }
        if self.sequence > SPECS.len()
            || matches!(self.phase, Phase::AcceptRoot | Phase::AcceptTemporary(_))
                != self.pending.is_some()
            || (self.matrix.is_some() != (self.phase == Phase::Matrix))
            || (self.vector.is_some() != (self.phase == Phase::ConstructVector))
            || (self.naming.is_some() != matches!(self.phase, Phase::Rename(_)))
            || (self.temporary.is_some()
                != matches!(
                    self.phase,
                    Phase::Matrix | Phase::Rename(_) | Phase::ReleaseTemporary(_)
                ))
        {
            return Err(Stop::Unsupported(
                "world-array cursor and retained replies/owners differ".into(),
            ));
        }
        self.validate_progress()?;
        if let Phase::Boundary(row) = self.phase {
            if row >= 64 {
                return Err(Stop::Unsupported(
                    "world-array boundary cursor outside native rows".into(),
                ));
            }
        }
        if let Some(temporary) = &self.temporary {
            validate_owner(temporary, temporary.data.len())?;
            if self.state.spans()?.iter().any(|(token, extent)| {
                temporary.token < token + *extent as u64
                    && *token < temporary.token + temporary.data.len() as u64
            }) {
                return Err(Stop::Unsupported(
                    "world-array temporary aliases retained ownership".into(),
                ));
            }
        }
        if let Some(vector) = &self.vector {
            let mut spans = self.state.spans()?;
            for owner in std::iter::once(&vector.state.owner).chain(vector.state.backing.iter()) {
                validate_owner(
                    owner,
                    if owner.token == vector.state.owner.token {
                        12
                    } else {
                        16384
                    },
                )?;
                spans.push((owner.token, owner.data.len()));
            }
            spans.sort_unstable();
            if spans
                .windows(2)
                .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
            {
                return Err(Stop::Unsupported(
                    "constructing element vector aliases retained world ownership".into(),
                ));
            }
        }
        let expected_size = if self.phase == Phase::PublishSize {
            0_u32
        } else {
            64
        };
        if self.state.prefix.owner.data[0x10..0x14] != expected_size.to_le_bytes() {
            return Err(Stop::Unsupported(
                "world-array actual active-size publication differs".into(),
            ));
        }
        if self.phase == Phase::Reached {
            self.state.validate_retained()?;
            if self.sequence != 22
                || self.state.roots.len() != 20
                || self.state.matrices.nodes.len() != 20
                || self.state.vectors.iter().any(Option::is_none)
                || self.state.context.roof_owners
                    != self.initial_context.roof_owners.wrapping_add(2)
                || self.state.context.random != self.initial_context.random
            {
                return Err(Stop::Unsupported(
                    "world-array reached cut lacks complete retained ownership/context".into(),
                ));
            }
            for spec in SPECS {
                let token = match spec.kind {
                    Kind::Vector(index) => self.state.vectors[index].as_ref().unwrap().owner.token,
                    _ => self.state.roots[&spec.slot].token,
                };
                if self.state.prefix.owner.data[spec.slot as usize..spec.slot as usize + 4]
                    != (token as u32).to_le_bytes()
                {
                    return Err(Stop::Unsupported(
                        "world-array root publication differs from owned allocation".into(),
                    ));
                }
                if let Kind::Vector(index) = spec.kind {
                    if self.state.vectors[index]
                        .as_ref()
                        .unwrap()
                        .backing
                        .as_ref()
                        .unwrap()
                        .data
                        .iter()
                        .any(|byte| *byte != 0x7f)
                    {
                        return Err(Stop::Unsupported(
                            "world-array element-vector fill differs".into(),
                        ));
                    }
                } else {
                    let (side, stride) = spec.shape()?;
                    let node = &self.state.matrices.nodes[&self.state.identity(spec.slot)?];
                    let Allocation::Known(backing) = &node.matrix.allocation else {
                        return Err(Stop::Unavailable(
                            "completed world matrix backing absent".into(),
                        ));
                    };
                    if node.matrix.width != side
                        || node.matrix.height != side
                        || backing.data.len() != (side * side * stride) as usize
                        || backing.column_offsets
                            != (0..side)
                                .map(|column| (column * side * stride) as u32)
                                .collect::<Vec<_>>()
                    {
                        return Err(Stop::Unsupported(
                            "completed world matrix shape/backing differs".into(),
                        ));
                    }
                    let dispatch = match spec.kind {
                        Kind::Pool => Some((0x667438_u32, 1_u32)),
                        Kind::Water => Some((0x667940, 1)),
                        Kind::Roof => Some((0x6714d8, 0)),
                        _ => None,
                    };
                    if let Some((dispatch, references)) = dispatch {
                        let root = &self.state.roots[&spec.slot];
                        if root.data[..4] != dispatch.to_le_bytes()
                            || root.data[0x24..0x28] != references.to_le_bytes()
                        {
                            return Err(Stop::Unsupported(
                                "completed derived matrix dispatch/reference fields differ".into(),
                            ));
                        }
                    }
                    if !matches!(spec.slot, 0x1c | 0x20) {
                        let expected = if matches!(spec.kind, Kind::Pool | Kind::Water) {
                            0xff
                        } else {
                            0
                        };
                        if backing.data.iter().any(|byte| *byte != expected) {
                            return Err(Stop::Unsupported(
                                "completed fresh matrix initialization differs".into(),
                            ));
                        }
                    }
                }
            }
            for (slot, name) in NAMES
                .into_iter()
                .chain([(0x4c, b"11".as_slice()), (0x50, b"18".as_slice())])
            {
                if self.state.matrices.nodes[&self.state.identity(slot)?]
                    .name
                    .bytes()?
                    != name
                {
                    return Err(Stop::Unsupported(
                        "completed world-array external/default names differ".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn accept(&self, extent: usize) -> Result<Owner, Stop> {
        let owner = match self.pending.as_ref() {
            Some(Allocated::Bytes { token, data }) => Owner {
                token: *token,
                data: data.clone(),
            },
            Some(Allocated::Null) => {
                return Err(Stop::Unavailable(
                    "fresh positive-size world-array operation lacks a writable allocation".into(),
                ));
            }
            None => {
                return Err(Stop::Unavailable(
                    "world-array allocator reply absent".into(),
                ));
            }
        };
        validate_owner(&owner, extent)?;
        if self.state.spans()?.iter().any(|(token, bytes)| {
            owner.token < token + *bytes as u64 && *token < owner.token + extent as u64
        }) {
            return Err(Stop::Unsupported(
                "world-array allocator reply overlaps retained ownership".into(),
            ));
        }
        Ok(owner)
    }

    fn next(&mut self) {
        self.sequence += 1;
        self.phase = if self.sequence == SPECS.len() {
            Phase::Boundary(0)
        } else {
            Phase::AllocateRoot
        };
    }

    fn boundary(&mut self, row: usize) -> Result<(), Stop> {
        for slot in [0x60, 0x64] {
            self.state.backing_mut(slot)?.data[63 * 64 + row] |= 0xff;
        }
        for plane in 0..2 {
            let walls = self.state.backing_mut(0x44 + plane as u8 * 4)?;
            let start = (63 * 64 + row) * 14;
            walls.data[start..start + 14].fill(0);
            let vector = self.state.vectors[plane]
                .as_mut()
                .ok_or_else(|| Stop::Unavailable("boundary element vector absent".into()))?;
            let backing = vector
                .backing
                .as_mut()
                .ok_or_else(|| Stop::Unavailable("boundary vector payload absent".into()))?;
            let start = (63 * 64 + row) * 4;
            for offset in [3, 2, 1, 0] {
                backing.data[start + offset] = 0x7f;
            }
            self.state.backing_mut(0x58 + plane as u8 * 4)?.data[(63 * 128 + row) * 2] = 0;
        }
        Ok(())
    }

    pub fn step(&mut self, host: &mut impl Host) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        match self.phase {
            Phase::PublishSize => {
                write_word(&mut self.state.prefix.owner, 0x10, 64);
                self.phase = Phase::EmptyTeardown;
            }
            Phase::EmptyTeardown => {
                if SPECS.iter().any(|spec| {
                    self.state.prefix.owner.data[spec.slot as usize..spec.slot as usize + 4]
                        != [0; 4]
                }) {
                    return Err(Stop::Unsupported(
                        "fresh world-array teardown encountered a non-null prior owner".into(),
                    ));
                }
                for offset in empty_teardown_slots() {
                    write_word(&mut self.state.prefix.owner, offset, 0);
                }
                self.phase = Phase::AllocateRoot;
            }
            Phase::AllocateRoot => {
                let spec = self.spec()?;
                self.pending = Some(host.allocate_array_root(spec.slot, spec.extent() as u32)?);
                self.phase = Phase::AcceptRoot;
            }
            Phase::AcceptRoot => {
                let spec = self.spec()?;
                let owner = self.accept(spec.extent())?;
                if let Kind::Vector(_) = spec.kind {
                    self.vector = Some(vector::Construction::new(owner)?);
                    self.phase = Phase::ConstructVector;
                } else {
                    self.state.roots.insert(spec.slot, owner);
                    self.phase = Phase::AllocateTemporary(Action::Construct);
                }
                self.pending = None;
            }
            Phase::AllocateTemporary(action) => {
                let bytes = self.input(action)?.len() as u32 + 1;
                self.pending = Some(host.allocate_name(bytes)?);
                self.phase = Phase::AcceptTemporary(action);
            }
            Phase::AcceptTemporary(action) => {
                let input = self.input(action)?;
                let mut owner = self.accept(input.len() + 1)?;
                if owner.token + owner.data.len() as u64 > u64::from(u32::MAX) {
                    return Err(Stop::Unsupported(
                        "temporary string end exceeds native pointer width".into(),
                    ));
                }
                owner.data[..input.len()].copy_from_slice(input);
                owner.data[input.len()] = 0;
                match action {
                    Action::Construct => {
                        let spec = self.spec()?;
                        let (side, stride) = spec.shape()?;
                        self.matrix = Some(native_matrix::Construction::new(
                            self.state.identity(spec.slot)?,
                            stride,
                            side,
                            side,
                            input.to_vec(),
                        )?);
                        self.phase = Phase::Matrix;
                    }
                    Action::Rename(index) => {
                        self.naming = Some(Naming::external(
                            self.state.identity(NAMES[index].0)?,
                            input.to_vec(),
                        )?);
                        self.phase = Phase::Rename(index);
                    }
                }
                self.temporary = Some(owner);
                self.pending = None;
            }
            Phase::Matrix => {
                if self
                    .matrix
                    .as_mut()
                    .unwrap()
                    .step(&mut self.state.matrices, host)?
                    .is_some()
                {
                    self.matrix = None;
                    self.phase = Phase::ReleaseTemporary(Action::Construct);
                }
            }
            Phase::ReleaseTemporary(action) => {
                host.release_name(self.temporary.as_ref().unwrap().token)?;
                self.temporary = None;
                self.phase = match action {
                    Action::Construct => Phase::Derived,
                    Action::Rename(index) if index + 1 == NAMES.len() => Phase::Grass,
                    Action::Rename(index) => Phase::AllocateTemporary(Action::Rename(index + 1)),
                };
            }
            Phase::Derived => {
                let spec = self.spec()?;
                let dispatch = match spec.kind {
                    Kind::Pool => Some(0x667438),
                    Kind::Water => Some(0x667940),
                    Kind::Roof => Some(0x6714d8),
                    _ => None,
                };
                if let Some(dispatch) = dispatch {
                    let root = self.state.roots.get_mut(&spec.slot).unwrap();
                    write_word(root, 0x24, 0);
                    write_word(root, 0, dispatch);
                    if spec.kind == Kind::Roof {
                        self.state.context.roof_owners =
                            self.state.context.roof_owners.wrapping_add(1);
                    } else {
                        for byte in &mut self.state.backing_mut(spec.slot)?.data {
                            *byte |= 0xff;
                        }
                    }
                }
                self.phase = Phase::PublishRoot;
            }
            Phase::ConstructVector => {
                if self.vector.as_mut().unwrap().step(host)? {
                    let Kind::Vector(index) = self.spec()?.kind else {
                        return Err(Stop::Unsupported("vector cursor has a matrix root".into()));
                    };
                    self.state.vectors[index] = Some(self.vector.take().unwrap().state);
                    self.phase = Phase::PublishRoot;
                }
            }
            Phase::PublishRoot => {
                let spec = self.spec()?;
                let token = match spec.kind {
                    Kind::Vector(index) => self.state.vectors[index].as_ref().unwrap().owner.token,
                    _ => self.state.roots[&spec.slot].token,
                };
                write_word(
                    &mut self.state.prefix.owner,
                    spec.slot as usize,
                    token as u32,
                );
                if matches!(spec.kind, Kind::Pool | Kind::Water) {
                    self.phase = Phase::IncrementReference;
                } else {
                    self.next();
                }
            }
            Phase::IncrementReference => {
                let slot = self.spec()?.slot;
                let root = self.state.roots.get_mut(&slot).unwrap();
                let previous = u32::from_le_bytes(root.data[0x24..0x28].try_into().unwrap());
                write_word(root, 0x24, previous.wrapping_add(1));
                self.next();
            }
            Phase::Boundary(row) => {
                self.boundary(usize::from(row))?;
                self.phase = if row == 63 {
                    Phase::AllocateTemporary(Action::Rename(0))
                } else {
                    Phase::Boundary(row + 1)
                };
            }
            Phase::Rename(index) => {
                if self
                    .naming
                    .as_mut()
                    .unwrap()
                    .step(&mut self.state.matrices, host)?
                {
                    self.naming = None;
                    self.phase = Phase::ReleaseTemporary(Action::Rename(index));
                }
            }
            Phase::Grass => {
                let grass = self.state.identity(0x1c)?;
                let target = self.state.identity(0x20)?;
                let mut grass_node = self.state.matrices.nodes.remove(&grass).ok_or_else(|| {
                    Stop::Unavailable("actual constructed grass owner absent".into())
                })?;
                let result = match self.state.matrices.nodes.get_mut(&target) {
                    Some(target_node) => crate::native_grass::regenerate(
                        &mut grass_node.matrix,
                        &mut target_node.matrix,
                        0,
                        &mut self.state.context.random,
                    ),
                    None => Err(Stop::Unavailable(
                        "actual constructed target-grass owner absent".into(),
                    )),
                };
                self.state.matrices.nodes.insert(grass, grass_node);
                result?;
                self.phase = Phase::Reached;
            }
            Phase::Reached => {}
        }
        self.state.synchronize()?;
        self.validate()?;
        Ok(self.cut())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.phase != Phase::Reached {
            return Err(Stop::Unavailable(
                "world arrays have not reached original407704 coordinate-table entry".into(),
            ));
        }
        Ok(self.state)
    }
}

#[cfg(test)]
mod tests;
