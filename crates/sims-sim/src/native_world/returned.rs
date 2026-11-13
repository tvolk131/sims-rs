use super::{arrays, resources::population, validate_owner};
use crate::native_world_children::Owner;
use crate::saved_matrix::Stop;
use serde::{Deserialize, Serialize};
use sims_vm::commander::Registry;

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn original_world(source: &population::State) -> &Owner {
    &source
        .handle()
        .opened()
        .context()
        .resources
        .resources
        .lighting
        .fixed
        .arrays
        .prefix
        .owner
}

fn original_registry(source: &population::State) -> &Registry<u32> {
    &source
        .handle()
        .opened()
        .context()
        .resources
        .resources
        .lighting
        .fixed
        .arrays
        .prefix
        .registry
}

fn disjoint(mut spans: Vec<(u64, usize)>) -> Result<(), Stop> {
    spans.sort_unstable();
    if spans
        .windows(2)
        .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
    {
        return Err(unsupported(
            "world return aliases actual retained ownership",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incoming {
    pub corners: [Owner; 4],
}

impl Incoming {
    fn validate(&self) -> Result<(), Stop> {
        for corner in &self.corners {
            validate_owner(corner, 4)?;
        }
        let origin = self.corners[2].token;
        if self.corners[0].token != origin + 4
            || self.corners[1].token != origin + 8
            || self.corners[3].token != origin + 20
        {
            return Err(unsupported(
                "rotatable return requires its actual four-corner frame layout",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    ResourceLoadReturned,
    LightingReturned,
    FixedReturned,
    OrientationInitialized,
    VtableBound,
    CornerReturned(u8),
    ReturnInstruction,
    Returned,
}

fn cut(step: u8) -> Option<Cut> {
    match step {
        0 => Some(Cut::ResourceLoadReturned),
        1 => Some(Cut::LightingReturned),
        3 => Some(Cut::FixedReturned),
        4 => Some(Cut::OrientationInitialized),
        5 => Some(Cut::VtableBound),
        6..=9 => Some(Cut::CornerReturned(step - 6)),
        21 => Some(Cut::ReturnInstruction),
        22 => Some(Cut::Returned),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    pub offset: usize,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

fn store(owner: &mut Owner, writes: &mut Vec<Write>, offset: usize, bytes: &[u8]) {
    writes.push(Write {
        offset,
        before: owner.data[offset..offset + bytes.len()].to_vec(),
        after: bytes.to_vec(),
    });
    owner.data[offset..offset + bytes.len()].copy_from_slice(bytes);
}

fn bound(owner: &Owner, corners: &[Owner; 4], position: u8) -> i32 {
    let group = usize::from(position / 3);
    let coordinate = group / 2;
    let maximum = group % 2 == 1;
    let (earlier, candidate) = if position % 3 == 0 {
        (
            i32::from(corners[0].data[coordinate] as i8),
            i32::from(corners[1].data[coordinate] as i8),
        )
    } else {
        let offset = 112 + group * 4;
        (
            i32::from_le_bytes(owner.data[offset..offset + 4].try_into().unwrap()),
            i32::from(corners[usize::from(position % 3) + 1].data[coordinate] as i8),
        )
    };
    if maximum {
        earlier.max(candidate)
    } else {
        earlier.min(candidate)
    }
}

fn transform(corner: &Owner, orientation: u32) -> Result<u64, Stop> {
    validate_owner(corner, 4)?;
    if orientation != 0 {
        return Err(unsupported(
            "world construction only reaches the actual zero-orientation corner branch",
        ));
    }
    Ok(corner.token)
}

fn apply(
    step: u8,
    owner: &mut Owner,
    corners: &mut [Owner; 4],
    writes: &mut Vec<Write>,
    returns: &mut Vec<u64>,
) -> Result<(), Stop> {
    match step {
        2 => store(owner, writes, 128, &[0]),
        3 => store(owner, writes, 129, &[0]),
        4 => store(owner, writes, 132, &0_u32.to_le_bytes()),
        5 => {
            corners[0].data[..3].fill(0);
            corners[1].data[1..3].fill(0);
            corners[2].data[2] = 0;
            corners[3].data[0] = 0;
            corners[3].data[2] = 0;
            store(owner, writes, 0, &0x667a94_u32.to_le_bytes());
        }
        6 => {
            let edge = owner.data[16].wrapping_sub(1);
            corners[1].data[0] = edge;
            corners[2].data[0] = edge;
            corners[2].data[1] = edge;
            corners[3].data[1] = edge;
        }
        10..=21 => {
            let position = step - 10;
            let value = bound(owner, corners, position);
            store(
                owner,
                writes,
                112 + usize::from(position / 3) * 4,
                &value.to_le_bytes(),
            );
        }
        _ => {}
    }
    if (6..=9).contains(&step) {
        let orientation = u32::from_le_bytes(owner.data[132..136].try_into().unwrap());
        returns.push(transform(&corners[usize::from(step - 6)], orientation)?);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    source: population::State,
    incoming: Incoming,
    owner: Owner,
    corners: [Owner; 4],
    writes: Vec<Write>,
    corner_returns: Vec<u64>,
    registry: Registry<u32>,
    cursor: u8,
}

impl Construction {
    pub fn new(source: population::State, incoming: Incoming) -> Result<Self, Stop> {
        source.validate()?;
        incoming.validate()?;
        let execution = Self {
            owner: original_world(&source).clone(),
            corners: incoming.corners.clone(),
            registry: original_registry(&source).clone(),
            source,
            incoming,
            writes: vec![],
            corner_returns: vec![],
            cursor: 0,
        };
        execution.validate()?;
        Ok(execution)
    }

    pub fn population(&self) -> &population::State {
        &self.source
    }
    pub fn registry(&self) -> &Registry<u32> {
        &self.registry
    }
    pub fn incoming(&self) -> &Incoming {
        &self.incoming
    }
    pub fn owner(&self) -> &Owner {
        &self.owner
    }
    pub fn corners(&self) -> &[Owner; 4] {
        &self.corners
    }
    pub fn writes(&self) -> &[Write] {
        &self.writes
    }
    pub fn corner_returns(&self) -> &[u64] {
        &self.corner_returns
    }
    pub fn cut(&self) -> Option<Cut> {
        self.cursor.checked_sub(1).and_then(cut)
    }
    pub fn returned_owner(&self) -> Option<u64> {
        matches!(
            self.cut(),
            Some(Cut::FixedReturned | Cut::ReturnInstruction | Cut::Returned)
        )
        .then_some(self.owner.token)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.source.validate()?;
        self.incoming.validate()?;
        validate_owner(&self.owner, 136)?;
        if self.cursor > 23 {
            return Err(unsupported("world return has an unreached phase"));
        }
        let original = original_world(&self.source);
        let mut expected_owner = original.clone();
        let mut expected_corners = self.incoming.corners.clone();
        let mut expected_writes = vec![];
        let mut expected_returns = vec![];
        for step in 0..self.cursor {
            apply(
                step,
                &mut expected_owner,
                &mut expected_corners,
                &mut expected_writes,
                &mut expected_returns,
            )?;
        }
        if self.owner != expected_owner
            || self.corners != expected_corners
            || self.writes != expected_writes
            || self.corner_returns != expected_returns
        {
            return Err(unsupported(
                "world return lost its actual sparse writes or corner input bytes",
            ));
        }
        let mut expected_registry = original_registry(&self.source).clone();
        expected_registry
            .bind(
                self.owner.token as u32,
                u32::from_le_bytes(expected_owner.data[..4].try_into().unwrap()),
            )
            .map_err(Stop::Unsupported)?;
        if self.registry != expected_registry {
            return Err(unsupported(
                "World return changed its actual Commander ownership",
            ));
        }
        let mut spans = self.source.retained_spans()?;
        spans.retain(|span| *span != (original.token, 136));
        spans.push((self.owner.token, 136));
        spans.extend(self.incoming.corners.iter().map(|owner| (owner.token, 4)));
        disjoint(spans)
    }

    pub fn step(&mut self) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        if self.cursor == 23 {
            return Ok(Some(Cut::Returned));
        }
        apply(
            self.cursor,
            &mut self.owner,
            &mut self.corners,
            &mut self.writes,
            &mut self.corner_returns,
        )?;
        self.registry
            .bind(
                self.owner.token as u32,
                u32::from_le_bytes(self.owner.data[..4].try_into().unwrap()),
            )
            .map_err(Stop::Unsupported)?;
        let reached = cut(self.cursor);
        self.cursor += 1;
        self.validate()?;
        Ok(reached)
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cursor != 23 {
            return Err(unsupported("world constructor has not genuinely returned"));
        }
        Ok(State { execution: self })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    execution: Construction,
}

impl State {
    pub fn registry(&self) -> &Registry<u32> {
        self.execution.registry()
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.execution.validate()?;
        if self.execution.cursor != 23 {
            return Err(unsupported("world state is not a returned constructor"));
        }
        Ok(())
    }
    pub fn owner(&self) -> &Owner {
        self.execution.owner()
    }
    pub fn population(&self) -> &population::State {
        self.execution.population()
    }
    pub fn arrays(&self) -> &arrays::State {
        &self
            .population()
            .handle()
            .opened()
            .context()
            .resources
            .resources
            .lighting
            .fixed
            .arrays
    }
    pub fn corners(&self) -> &[Owner; 4] {
        self.execution.corners()
    }
    pub fn writes(&self) -> &[Write] {
        self.execution.writes()
    }
    pub fn active_size(&self) -> u32 {
        u32::from_le_bytes(self.owner().data[16..20].try_into().unwrap())
    }
    pub fn bounds(&self) -> [i32; 4] {
        std::array::from_fn(|index| {
            i32::from_le_bytes(
                self.owner().data[112 + index * 4..116 + index * 4]
                    .try_into()
                    .unwrap(),
            )
        })
    }
    pub fn orientation(&self) -> u32 {
        u32::from_le_bytes(self.owner().data[132..136].try_into().unwrap())
    }
}
