use super::{arrays, fixed, startup, validate_owner};
use crate::native_world_children::Owner;
use crate::saved_matrix::{Allocation, Stop};
use serde::{Deserialize, Serialize};

fn unsupported(message: &str) -> Stop {
    Stop::Unsupported(message.into())
}

fn validate_frames(mut spans: Vec<(u64, usize)>, corners: Option<&[Owner; 4]>) -> Result<(), Stop> {
    spans.push((0x6e5528, 4));
    if let Some(corners) = corners {
        spans.extend(corners.iter().map(|corner| (corner.token, 4)));
    }
    spans.sort_unstable();
    if spans
        .windows(2)
        .any(|pair| pair[0].0 + pair[0].1 as u64 > pair[1].0)
    {
        return Err(unsupported("resize caller frames alias live ownership"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Incoming {
    pub requested: u32,
    pub current_world: u32,
    pub corners: Option<[Owner; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    Entry,
    SizePublished,
    BoundaryComplete,
    GrassComplete,
    CoordinatesComplete,
    SizeRepublished,
    BoundsComplete,
    Returned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    pub offset: usize,
    pub instruction: u32,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundaryWrite {
    pub slot: u8,
    pub write: Write,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CornerCall {
    pub owner: u64,
    pub before: [u8; 4],
    pub argument: u32,
    pub return_pc: u32,
    pub after: [u8; 4],
    pub returned_eax: u64,
}

fn store(data: &mut [u8], offset: usize, after: &[u8], instruction: u32) -> Write {
    let before = data[offset..offset + after.len()].to_vec();
    data[offset..offset + after.len()].copy_from_slice(after);
    Write {
        offset,
        instruction,
        before,
        after: after.to_vec(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transition {
    source_owner: Owner,
    source_arrays: arrays::State,
    source_tables: fixed::Tables,
    incoming: Incoming,
    owner: Owner,
    arrays: arrays::State,
    tables: fixed::Tables,
    corners: Option<[Owner; 4]>,
    world_writes: Vec<Write>,
    boundary_writes: Vec<BoundaryWrite>,
    coordinate_writes: Vec<Write>,
    corner_calls: Vec<CornerCall>,
    cursor: u8,
}

impl Transition {
    fn initial(
        source_owner: Owner,
        source_arrays: arrays::State,
        source_tables: fixed::Tables,
        incoming: Incoming,
    ) -> Result<Self, Stop> {
        validate_owner(&source_owner, 136)?;
        source_arrays.validate_retained()?;
        let old = u32::from_le_bytes(source_owner.data[16..20].try_into().unwrap());
        if !(4..=64).contains(&incoming.requested)
            || !(4..=64).contains(&old)
            || ![0, source_owner.token as u32].contains(&incoming.current_world)
        {
            return Err(unsupported(
                "resize requires the bounded retained64 null/self profile",
            ));
        }
        let uses_corners = incoming.requested != old && incoming.current_world != 0;
        if uses_corners != incoming.corners.is_some()
            || (uses_corners && source_owner.data[132..136] != 0_u32.to_le_bytes())
        {
            return Err(unsupported(
                "resize lacks its actual zero-orientation corner inputs",
            ));
        }
        if let Some(corners) = &incoming.corners {
            let origin = corners[2].token;
            for (index, corner) in corners.iter().enumerate() {
                validate_owner(corner, 4)?;
                if origin.checked_add([8, 4, 0, 36][index]) != Some(corner.token) {
                    return Err(unsupported(
                        "resize requires its actual four-corner frame layout",
                    ));
                }
            }
            if corners[3].data != incoming.requested.to_le_bytes() {
                return Err(unsupported(
                    "resize fourth corner must consume the actual requested-size argument",
                ));
            }
        }
        if source_arrays.vectors.iter().any(Option::is_none) {
            return Err(unsupported(
                "resize requires both actual retained light vectors",
            ));
        }
        validate_frames(source_arrays.spans()?, incoming.corners.as_ref())?;
        for (slot, root) in &source_arrays.roots {
            let identity = source_arrays.identity(*slot)?;
            let matrix = &source_arrays
                .matrices
                .nodes
                .get(&identity)
                .ok_or_else(|| unsupported("resize matrix root has no actual registry node"))?
                .matrix;
            let side = if [0x58, 0x5c].contains(slot) { 128 } else { 64 };
            let Allocation::Known(backing) = &matrix.allocation else {
                return Err(unsupported(
                    "resize requires complete retained matrix allocations",
                ));
            };
            if matrix.width != side
                || matrix.height != side
                || backing.column_offsets
                    != (0..side)
                        .map(|column| (column * side * matrix.stride) as u32)
                        .collect::<Vec<_>>()
                || backing.data.len() != (side * side * matrix.stride) as usize
                || source_owner.data[usize::from(*slot)..usize::from(*slot) + 4]
                    != (root.token as u32).to_le_bytes()
            {
                return Err(unsupported(
                    "resize does not own its original complete backing geometry",
                ));
            }
        }
        if source_tables.coordinate_bytes().len() != 24576 {
            return Err(unsupported("resize coordinate table owner is incomplete"));
        }
        Ok(Self {
            owner: source_owner.clone(),
            arrays: source_arrays.clone(),
            tables: source_tables.clone(),
            corners: incoming.corners.clone(),
            source_owner,
            source_arrays,
            source_tables,
            incoming,
            world_writes: vec![],
            boundary_writes: vec![],
            coordinate_writes: vec![],
            corner_calls: vec![],
            cursor: 0,
        })
    }

    fn changed(&self) -> bool {
        self.source_owner.data[16..20] != self.incoming.requested.to_le_bytes()
    }

    fn cut(&self) -> Option<Cut> {
        if !self.changed() {
            return match self.cursor {
                1 => Some(Cut::Entry),
                2 => Some(Cut::Returned),
                _ => None,
            };
        }
        match self.cursor {
            1 => Some(Cut::Entry),
            2 => Some(Cut::SizePublished),
            3 => Some(Cut::BoundaryComplete),
            4 => Some(Cut::GrassComplete),
            5 => Some(Cut::CoordinatesComplete),
            6 => Some(Cut::SizeRepublished),
            7 => Some(Cut::BoundsComplete),
            8 => Some(Cut::Returned),
            _ => None,
        }
    }

    fn matrix_store(
        &mut self,
        slot: u8,
        offset: usize,
        after: &[u8],
        instruction: u32,
    ) -> Result<(), Stop> {
        self.boundary_writes.push(BoundaryWrite {
            slot,
            write: store(
                &mut self.arrays.backing_mut(slot)?.data,
                offset,
                after,
                instruction,
            ),
        });
        Ok(())
    }

    fn wall_clear(&mut self, slot: u8, offset: usize) -> Result<(), Stop> {
        let backing = self.arrays.backing_mut(slot)?;
        let address = backing.token + backing.column_offsets.len() as u64 * 4 + offset as u64;
        let leading = ((4 - address % 4) % 4) as usize;
        for index in 0..leading {
            self.matrix_store(slot, offset + index, &[0], 0x6362e3)?;
        }
        let aligned_end = leading + (14 - leading) / 4 * 4;
        for index in (leading..aligned_end).step_by(4) {
            self.matrix_store(slot, offset + index, &[0; 4], 0x636301)?;
        }
        for index in aligned_end..14 {
            self.matrix_store(slot, offset + index, &[0], 0x636307)?;
        }
        Ok(())
    }

    fn boundary(&mut self) -> Result<(), Stop> {
        let side = self.incoming.requested as usize;
        for edge in 0..2 {
            for coordinate in 0..side {
                let (column, row) = if edge == 0 {
                    (side - 1, coordinate)
                } else {
                    (coordinate, side - 1)
                };
                let cell = column * 64 + row;
                self.matrix_store(0x60, cell, &[255], [0x47797c, 0x477ad4][edge])?;
                self.matrix_store(0x64, cell, &[255], [0x47799f, 0x477afb][edge])?;
                for level in 0..2 {
                    let plane = level as u8 * 4;
                    self.matrix_store(0x24 + plane, cell * 2, &[0; 2], [0x4779cb, 0x477b2b][edge])?;
                    self.matrix_store(0x2c + plane, cell * 2, &[0; 2], [0x4779e2, 0x477b43][edge])?;
                    self.wall_clear(0x44 + plane, cell * 14)?;
                    self.matrix_store(0x3c + plane, cell * 2, &[0; 2], [0x477a22, 0x477b88][edge])?;
                    self.matrix_store(0x4c + plane, cell, &[0], [0x477a4b, 0x477bb6][edge])?;
                    let backing = self.arrays.vectors[level]
                        .as_mut()
                        .and_then(|vector| vector.backing.as_mut())
                        .ok_or_else(|| unsupported("resize light vector backing absent"))?;
                    for (index, byte) in [3, 2, 1, 0].into_iter().enumerate() {
                        let instruction = [
                            [0x477a52, 0x477a59, 0x477a5d, 0x477a61],
                            [0x477bbd, 0x477bc4, 0x477bc8, 0x477bcc],
                        ][edge][index];
                        self.boundary_writes.push(BoundaryWrite {
                            slot: 0x68 + plane,
                            write: store(&mut backing.data, cell * 4 + byte, &[127], instruction),
                        });
                    }
                    self.matrix_store(
                        0x58 + plane,
                        (column * 128 + row) * 2,
                        &[0],
                        [0x477a7e, 0x477bed][edge],
                    )?;
                }
            }
        }
        Ok(())
    }

    fn grass(&mut self) -> Result<(), Stop> {
        let grass_id = self.arrays.identity(0x1c)?;
        let target_id = self.arrays.identity(0x20)?;
        let mut grass = self.arrays.matrices.nodes[&grass_id].matrix.clone();
        let mut target = self.arrays.matrices.nodes[&target_id].matrix.clone();
        crate::native_grass::regenerate(
            &mut grass,
            &mut target,
            0,
            &mut self.arrays.context.random,
        )?;
        self.arrays
            .matrices
            .nodes
            .get_mut(&grass_id)
            .unwrap()
            .matrix = grass;
        self.arrays
            .matrices
            .nodes
            .get_mut(&target_id)
            .unwrap()
            .matrix = target;
        Ok(())
    }

    fn bounds(&mut self) {
        let Some(corners) = &mut self.corners else {
            return;
        };
        let edge = self.incoming.requested as u8 - 1;
        for (index, coordinates) in [[0, 0, 0], [edge, 0, 0], [edge, edge, 0], [0, edge, 0]]
            .into_iter()
            .enumerate()
        {
            corners[index].data[..3].copy_from_slice(&coordinates);
            let before = corners[index].data.as_slice().try_into().unwrap();
            self.corner_calls.push(CornerCall {
                owner: corners[index].token,
                before,
                argument: 0,
                return_pc: [0x477fcd, 0x477fe0, 0x477ff3, 0x478006][index],
                after: before,
                returned_eax: corners[index].token,
            });
        }
        for (position, instruction) in [
            0x478029, 0x47803b, 0x478051, 0x478066, 0x478078, 0x478094, 0x4780b1, 0x4780c3,
            0x4780d9, 0x4780ee, 0x478100, 0x478114,
        ]
        .into_iter()
        .enumerate()
        {
            let group = position / 3;
            let coordinate = group / 2;
            let offset = 112 + group * 4;
            let earlier = if position % 3 == 0 {
                i32::from(corners[0].data[coordinate] as i8)
            } else {
                i32::from_le_bytes(self.owner.data[offset..offset + 4].try_into().unwrap())
            };
            let candidate = i32::from(corners[position % 3 + 1].data[coordinate] as i8);
            let value = if group % 2 == 0 {
                earlier.min(candidate)
            } else {
                earlier.max(candidate)
            };
            self.world_writes.push(store(
                &mut self.owner.data,
                offset,
                &value.to_le_bytes(),
                instruction,
            ));
        }
    }

    fn advance(&mut self) -> Result<(), Stop> {
        if self.cut() == Some(Cut::Returned) {
            return Ok(());
        }
        if self.changed() {
            match self.cursor {
                1 | 5 => self.world_writes.push(store(
                    &mut self.owner.data,
                    16,
                    &self.incoming.requested.to_le_bytes(),
                    if self.cursor == 1 { 0x477926 } else { 0x477f7d },
                )),
                2 => {
                    let old =
                        u32::from_le_bytes(self.source_owner.data[16..20].try_into().unwrap());
                    if self.incoming.requested < old {
                        self.boundary()?;
                    }
                }
                3 => self.grass()?,
                4 => {
                    let before = self.tables.coordinate_bytes().to_vec();
                    self.tables.regenerate_coordinates();
                    for (offset, (before, after)) in before
                        .into_iter()
                        .zip(self.tables.coordinate_bytes())
                        .enumerate()
                    {
                        self.coordinate_writes.push(Write {
                            offset,
                            instruction: if offset % 2 == 0 { 0x40775e } else { 0x407764 },
                            before: vec![before],
                            after: vec![*after],
                        });
                    }
                }
                6 => self.bounds(),
                _ => {}
            }
        }
        self.cursor += 1;
        Ok(())
    }

    fn validate(&self) -> Result<(), Stop> {
        validate_owner(&self.source_owner, 136)?;
        if self.cursor > if self.changed() { 8 } else { 2 } {
            return Err(unsupported(
                "resize cursor exceeds actual returned execution",
            ));
        }
        let mut expected = Self::initial(
            self.source_owner.clone(),
            self.source_arrays.clone(),
            self.source_tables.clone(),
            self.incoming.clone(),
        )?;
        for _ in 0..self.cursor {
            expected.advance()?;
        }
        if self != &expected {
            return Err(unsupported(
                "resize lost its complete owner bytes or ordered effects",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    source: startup::World,
    transition: Transition,
}

impl Execution {
    pub fn new(source: startup::World, incoming: Incoming) -> Result<Self, Stop> {
        source.validate()?;
        let transition = Transition::initial(
            source.owner().clone(),
            source.arrays().clone(),
            source.tables().clone(),
            incoming,
        )?;
        let execution = Self { source, transition };
        execution.validate()?;
        Ok(execution)
    }

    pub fn source(&self) -> &startup::World {
        &self.source
    }
    pub fn incoming(&self) -> &Incoming {
        &self.transition.incoming
    }
    pub fn owner(&self) -> &Owner {
        &self.transition.owner
    }
    pub fn arrays(&self) -> &arrays::State {
        &self.transition.arrays
    }
    pub fn tables(&self) -> &fixed::Tables {
        &self.transition.tables
    }
    pub fn corners(&self) -> Option<&[Owner; 4]> {
        self.transition.corners.as_ref()
    }
    pub fn world_writes(&self) -> &[Write] {
        &self.transition.world_writes
    }
    pub fn boundary_writes(&self) -> &[BoundaryWrite] {
        &self.transition.boundary_writes
    }
    pub fn coordinate_writes(&self) -> &[Write] {
        &self.transition.coordinate_writes
    }
    pub fn corner_calls(&self) -> &[CornerCall] {
        &self.transition.corner_calls
    }
    pub fn cut(&self) -> Option<Cut> {
        self.transition.cut()
    }
    pub fn returned_al(&self) -> Option<u8> {
        (self.cut() == Some(Cut::Returned)).then_some(1)
    }

    pub fn validate(&self) -> Result<(), Stop> {
        self.source.validate()?;
        if self.transition.source_owner != *self.source.owner()
            || self.transition.source_arrays != *self.source.arrays()
            || self.transition.source_tables != *self.source.tables()
        {
            return Err(unsupported(
                "resize input is not its actual retained raw owner",
            ));
        }
        self.transition.validate()?;
        validate_frames(
            self.source.retained_spans()?,
            self.transition.incoming.corners.as_ref(),
        )
    }

    pub fn step(&mut self) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        self.transition.advance()?;
        self.validate()?;
        Ok(self.cut())
    }

    pub fn into_world(self) -> Result<startup::World, Stop> {
        self.validate()?;
        if self.cut() != Some(Cut::Returned) {
            return Err(unsupported("resize has not actually returned AL1"));
        }
        self.source.accept_resize(State {
            transition: self.transition,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    transition: Transition,
}

impl State {
    pub fn validate(&self) -> Result<(), Stop> {
        self.transition.validate()?;
        if self.transition.cut() != Some(Cut::Returned) {
            return Err(unsupported(
                "retained resize state has not actually returned",
            ));
        }
        Ok(())
    }
    pub fn source_owner(&self) -> &Owner {
        &self.transition.source_owner
    }
    pub(super) fn validate_frames(&self, spans: Vec<(u64, usize)>) -> Result<(), Stop> {
        validate_frames(spans, self.transition.incoming.corners.as_ref())
    }
    pub fn source_arrays(&self) -> &arrays::State {
        &self.transition.source_arrays
    }
    pub fn source_tables(&self) -> &fixed::Tables {
        &self.transition.source_tables
    }
    pub fn owner(&self) -> &Owner {
        &self.transition.owner
    }
    pub fn arrays(&self) -> &arrays::State {
        &self.transition.arrays
    }
    pub fn tables(&self) -> &fixed::Tables {
        &self.transition.tables
    }
}
