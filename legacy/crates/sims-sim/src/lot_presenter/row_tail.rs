//! Original4af1c5 pool/floor gates and local-only loop updates.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Pool,
    Neighbor(u8),
    Edge,
    Floor,
    FloorRaster,
    Extras,
    Contents,
    WaterRaster,
    OverlayRaster,
    Counter,
    X,
    Y,
    Screen,
    Branch,
    Returned,
    Next,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    PoolEdge,
    FloorRaster,
    OtherContents,
    WaterRaster,
    OverlayRaster,
    NextTile,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Field {
    Counter,
    X,
    Y,
    Screen,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    point: [i8; 3],
    rotated: [i8; 3],
    screen: [i32; 2],
    step: [i8; 3],
    count: i32,
    index: i32,
    stage: Stage,
    store: Option<(Field, u32)>,
}
pub struct Inputs<'a> {
    pub size: usize,
    pub active_size: i32,
    pub rotation: u8,
    pub pool: Option<&'a [u8]>,
    pub bounds: [i32; 4],
    pub neighbors: [[i8; 3]; 3],
    pub covering: u16,
    pub floor_enabled: bool,
    pub tile_width: i32,
}
pub fn pool_contains(
    size: usize,
    rotation: u8,
    pool: Option<&[u8]>,
    point: [i8; 3],
) -> Result<bool, String> {
    if rotation > 3 {
        return Err("pool query rotation differs".into());
    }
    if point[2] != 1 || pool.is_none() {
        return Ok(false);
    }
    let p = super::room_masks::rotate(point, 4u8.wrapping_sub(rotation) & 3);
    let Some(pool) = pool else { unreachable!() };
    if !(1..=64).contains(&size)
        || pool.len() != size * size
        || p[0] < 0
        || p[1] < 0
        || p[0] as usize >= size
        || p[1] as usize >= size
    {
        return Err("pool query outside retained grid".into());
    }
    Ok(pool[p[0] as usize * size + p[1] as usize] < 16)
}
impl Frame {
    pub fn new(
        point: [i8; 3],
        rotated: [i8; 3],
        screen: [i32; 2],
        step: [i8; 3],
        count: i32,
        index: i32,
    ) -> Self {
        Self {
            point,
            rotated,
            screen,
            step,
            count,
            index,
            stage: Stage::Pool,
            store: None,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.count <= 0
            || self.index < 0
            || self.index > self.count
            || matches!(self.stage,Stage::Neighbor(i)if i>2)
        {
            return Err("row tail loop lifetime differs".into());
        }
        Ok(())
    }
    pub fn point(&self) -> [i8; 3] {
        self.point
    }
    pub fn screen(&self) -> [i32; 2] {
        self.screen
    }
    pub fn index(&self) -> i32 {
        self.index
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Returned
    }
    ///4b0000 jumps to4af9cf; the completed floor call rejoins4af9d3.
    pub fn finish_floor(&mut self) -> Result<(), String> {
        self.validate()?;
        if self.stage != Stage::FloorRaster {
            return Err("floor return lost its row caller".into());
        }
        self.stage = Stage::Extras;
        self.store = None;
        self.validate()
    }
    pub fn last_store(&self) -> Option<(Field, u32)> {
        self.store
    }
    pub fn pending(&self) -> Option<Request> {
        match self.stage {
            Stage::Edge => Some(Request::PoolEdge),
            Stage::FloorRaster => Some(Request::FloorRaster),

            Stage::Next => Some(Request::NextTile),
            Stage::Contents => Some(Request::OtherContents),
            Stage::WaterRaster => Some(Request::WaterRaster),
            Stage::OverlayRaster => Some(Request::OverlayRaster),
            _ => None,
        }
    }
    pub fn advance(&mut self, input: Inputs<'_>) -> Result<bool, String> {
        self.validate()?;
        if self.pending().is_some() || self.complete() {
            return Ok(false);
        }
        self.store = None;
        match self.stage {
            Stage::Pool => {
                self.stage = if pool_contains(input.size, input.rotation, input.pool, self.point)? {
                    Stage::Floor
                } else {
                    Stage::Neighbor(0)
                }
            }
            Stage::Neighbor(i) => {
                let p = std::array::from_fn(|j| {
                    self.point[j].wrapping_add(input.neighbors[i as usize][j])
                });
                let [x, y] = [i32::from(p[0]), i32::from(p[1])];
                let inside = x >= input.bounds[0]
                    && x <= input.bounds[1]
                    && y >= input.bounds[2]
                    && y <= input.bounds[3];
                self.stage = if inside && pool_contains(input.size, input.rotation, input.pool, p)?
                {
                    Stage::Edge
                } else if i < 2 {
                    Stage::Neighbor(i + 1)
                } else {
                    Stage::Floor
                };
            }
            Stage::Floor => {
                self.stage = if input.covering != 0 && input.floor_enabled {
                    Stage::FloorRaster
                } else {
                    Stage::Extras
                }
            }
            Stage::Extras => {
                self.stage = if self.rotated[0] >= 1
                    && i32::from(self.rotated[0]) < input.active_size - 1
                    && self.rotated[1] >= 1
                    && i32::from(self.rotated[1]) < input.active_size - 1
                {
                    Stage::Contents
                } else {
                    Stage::Counter
                }
            }
            Stage::Counter => {
                self.index = self.index.wrapping_add(1);
                self.store = Some((Field::Counter, self.index as u32));
                self.stage = Stage::X;
            }
            Stage::X => {
                self.point[0] = self.point[0].wrapping_add(self.step[0]);
                self.store = Some((Field::X, u32::from(self.point[0] as u8)));
                self.stage = Stage::Y;
            }
            Stage::Y => {
                self.point[1] = self.point[1].wrapping_add(self.step[1]);
                self.store = Some((Field::Y, u32::from(self.point[1] as u8)));
                self.stage = Stage::Screen;
            }
            Stage::Screen => {
                self.screen[0] = self.screen[0].wrapping_add(input.tile_width);
                self.store = Some((Field::Screen, self.screen[0] as u32));
                self.stage = Stage::Branch;
            }
            Stage::Branch => {
                self.stage = if self.index < self.count {
                    Stage::Next
                } else {
                    Stage::Returned
                }
            }
            Stage::Contents
            | Stage::WaterRaster
            | Stage::OverlayRaster
            | Stage::Edge
            | Stage::FloorRaster
            | Stage::Returned
            | Stage::Next => {
                return Ok(false);
            }
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4af1c5 => self.stage == Stage::Pool,
            0x4af43c => self.stage == Stage::Floor,
            0x4af453 => self.stage == Stage::FloorRaster,
            0x4af9d3 => self.stage == Stage::Extras,
            0x4b000b => matches!(self.stage, Stage::Extras | Stage::Contents),
            0x4b0111 | 0x4b011a => self.stage == Stage::Counter,
            0x4b012c => self.stage == Stage::X,
            0x4b012f => self.stage == Stage::Y,
            0x4b0135 => self.stage == Stage::Screen,
            0x4b0138 => self.stage == Stage::Branch,
            0x4b0147 | 0x4b0155 => self.complete(),
            _ => false,
        }
    }
    ///4afa0d's water read uses the already inverse-rotated point. The
    /// non-null overlay owner suppresses water raster, then owns the overlay.
    pub fn advance_contents(
        &mut self,
        water: &[u8],
        size: usize,
        overlay: bool,
    ) -> Result<bool, String> {
        self.validate()?;
        if self.stage != Stage::Contents {
            return Ok(false);
        }
        let [x, y, z] = self.rotated;
        let wet = if z == 1 {
            if !(1..=64).contains(&size)
                || water.len() != size * size
                || x < 0
                || y < 0
                || x as usize >= size
                || y as usize >= size
            {
                return Err("row contents water outside grid".into());
            }
            water[x as usize * size + y as usize] != 255
        } else {
            false
        };
        self.stage = if wet && !overlay {
            Stage::WaterRaster
        } else if overlay {
            Stage::OverlayRaster
        } else {
            Stage::Counter
        };
        self.store = None;
        self.validate()?;
        Ok(true)
    }
}
