use super::{arrays, write_word};
use crate::lot_presenter::walls::grid;
use crate::saved_matrix::Stop;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub requested: u32,
    pub current_world: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tables {
    coordinates: Vec<u8>,
    wall_corners: Vec<u8>,
    wall_masks: Vec<u32>,
    wall_directions: Vec<u32>,
}

fn coordinates() -> Vec<u8> {
    let mut result = Vec::with_capacity(24576);
    for orientation in 1..=3 {
        for column in 0..64 {
            for row in 0..64 {
                result.extend(match orientation {
                    1 => [63 - row, column],
                    2 => [63 - column, 63 - row],
                    _ => [row, 63 - column],
                });
            }
        }
    }
    result
}

fn wall_corners() -> Result<Vec<u8>, Stop> {
    let mut result = vec![0; 1280];
    for mask in 0..=255_u8 {
        let (junction, endpoint) = grid::classification(mask);
        result[usize::from(mask)] = u8::from(endpoint);
        result[256 + usize::from(mask)] = u8::from(junction);
        for orientation in 1..=3 {
            result[(orientation as usize + 1) * 256 + usize::from(mask)] =
                grid::rotate(mask, orientation).map_err(Stop::Unsupported)?;
        }
    }
    Ok(result)
}

fn wall_masks() -> Vec<u32> {
    let mut result = Vec::with_capacity(256);
    for orientation in 0..4 {
        for mask in 0..64_u32 {
            let cardinal = mask & 15;
            let cardinal = ((cardinal << orientation) | (cardinal >> (4 - orientation))) & 15;
            let diagonal = if orientation & 1 == 0 {
                mask & 48
            } else {
                ((mask & 16) << 1) | ((mask & 32) >> 1)
            };
            result.push(cardinal | diagonal);
        }
    }
    result
}

fn wall_directions() -> Vec<u32> {
    let mut result = Vec::with_capacity(20);
    for orientation in 0..4 {
        result.push(0);
        for direction in 1..=4 {
            result.push((direction + orientation - 1) % 4 + 1);
        }
    }
    result
}

impl Tables {
    pub(super) fn coordinate_bytes(&self) -> &[u8] {
        &self.coordinates
    }

    pub(super) fn regenerate_coordinates(&mut self) {
        self.coordinates = coordinates();
    }

    pub fn regions(&self) -> BTreeMap<u32, Vec<u8>> {
        let mut result = BTreeMap::new();
        for (address, bytes) in [
            (0x6de86c, &self.coordinates),
            (0x6f4a70, &self.wall_corners),
        ] {
            if !bytes.is_empty() {
                result.insert(address, bytes.clone());
            }
        }
        for (address, words) in [
            (0x6f4670, &self.wall_masks),
            (0x6f4620, &self.wall_directions),
        ] {
            if !words.is_empty() {
                result.insert(
                    address,
                    words.iter().flat_map(|word| word.to_le_bytes()).collect(),
                );
            }
        }
        result
    }

    pub fn rotate_point(&self, mut point: [i8; 3], orientation: u32) -> Result<[i8; 3], Stop> {
        if orientation == 0 {
            return Ok(point);
        }
        if orientation > 3 || point[..2].iter().any(|value| !(0..64).contains(value)) {
            return Err(Stop::Unsupported(
                "coordinate lookup is outside the original rotation-table domain".into(),
            ));
        }
        if self.coordinates.len() != 24576 {
            return Err(Stop::Unavailable(
                "original coordinate tables have not been initialized".into(),
            ));
        }
        let offset =
            ((orientation as usize - 1) * 4096 + point[0] as usize * 64 + point[1] as usize) * 2;
        point[0] = self.coordinates[offset] as i8;
        point[1] = self.coordinates[offset + 1] as i8;
        Ok(point)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Coordinates,
    RepublishSize,
    CurrentWorldBranch,
    WallCorners,
    WallMasks,
    Reached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    LightingEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    arrays: arrays::State,
    tables: Tables,
    context: Context,
    phase: Phase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub arrays: arrays::State,
    pub tables: Tables,
}

impl State {
    pub fn validate(&self) -> Result<(), Stop> {
        self.arrays.validate()?;
        if self.arrays.prefix.owner.data[0x10..0x14] != 64_u32.to_le_bytes()
            || self.tables.coordinates != coordinates()
            || self.tables.wall_corners != wall_corners()?
            || self.tables.wall_masks != wall_masks()
            || self.tables.wall_directions != wall_directions()
        {
            return Err(Stop::Unsupported(
                "retained fixed-world size or global tables differ from actual construction".into(),
            ));
        }
        Ok(())
    }
}

impl Construction {
    pub fn new(arrays: arrays::Construction, context: Context) -> Result<Self, Stop> {
        let result = Self {
            arrays: arrays.into_state()?,
            tables: Tables::default(),
            context,
            phase: Phase::Coordinates,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn arrays(&self) -> &arrays::State {
        &self.arrays
    }

    pub fn tables(&self) -> &Tables {
        &self.tables
    }

    pub fn cut(&self) -> Option<Cut> {
        (self.phase == Phase::Reached).then_some(Cut::LightingEntry)
    }

    fn validate(&self) -> Result<(), Stop> {
        self.arrays.validate()?;
        if self.context.requested != 64
            || self.arrays.prefix.owner.data[0x10..0x14] != self.context.requested.to_le_bytes()
        {
            return Err(Stop::Unsupported(
                "fresh fixed-world continuation requires the actually constructed 64-square arrays"
                    .into(),
            ));
        }
        let has_coordinates = self.phase != Phase::Coordinates;
        let has_corners = matches!(self.phase, Phase::WallMasks | Phase::Reached);
        let has_masks = self.phase == Phase::Reached;
        if self.tables.coordinates
            != if has_coordinates {
                coordinates()
            } else {
                Vec::new()
            }
            || self.tables.wall_corners
                != if has_corners {
                    wall_corners()?
                } else {
                    Vec::new()
                }
            || self.tables.wall_masks != if has_masks { wall_masks() } else { Vec::new() }
            || self.tables.wall_directions
                != if has_masks {
                    wall_directions()
                } else {
                    Vec::new()
                }
            || matches!(
                self.phase,
                Phase::WallCorners | Phase::WallMasks | Phase::Reached
            ) && self.context.current_world != 0
        {
            return Err(Stop::Unsupported(
                "restored fixed-world tables or branch differ from the actual construction phase"
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        match self.phase {
            Phase::Coordinates => {
                self.tables.coordinates = coordinates();
                self.phase = Phase::RepublishSize;
            }
            Phase::RepublishSize => {
                write_word(&mut self.arrays.prefix.owner, 0x10, self.context.requested);
                self.phase = Phase::CurrentWorldBranch;
            }
            Phase::CurrentWorldBranch => {
                if self.context.current_world != 0 {
                    return Err(Stop::Unavailable(
                        "non-null current-world resize notification requires its actual retained owner"
                            .into(),
                    ));
                }
                self.phase = Phase::WallCorners;
            }
            Phase::WallCorners => {
                self.tables.wall_corners = wall_corners()?;
                self.phase = Phase::WallMasks;
            }
            Phase::WallMasks => {
                self.tables.wall_masks = wall_masks();
                self.tables.wall_directions = wall_directions();
                self.phase = Phase::Reached;
            }
            Phase::Reached => {}
        }
        self.validate()?;
        Ok(self.cut())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cut().is_none() {
            return Err(Stop::Unavailable(
                "fixed world has not reached the original49eea8 lighting/resource entry".into(),
            ));
        }
        Ok(State {
            arrays: self.arrays,
            tables: self.tables,
        })
    }
}
