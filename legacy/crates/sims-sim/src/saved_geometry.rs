//! Original 415fdf..416013 geometry-read sequence, ending at saved types.
//! Known returned statuses are ignored by that caller. Unreturned host services
//! remain pending; neither an unavailable source nor a new VM is substituted.
use crate::{
    saved_matrix::{Matrix, Scratch, Stop},
    saved_world, surface_remap,
};
use serde::{Deserialize, Serialize};

pub trait Host: saved_world::Host + surface_remap::Host {}
impl<T: saved_world::Host + surface_remap::Host> Host for T {}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub arrays: saved_world::State,
    /// Distinct native world+4c/+50 derived vertex owners, not Arry8 or rooms.
    /// An unavailable backing is allowed until a put actually consumes it.
    pub vertices: [Matrix; 2],
    pub floors: surface_remap::State,
    pub walls: surface_remap::State,
    pub scratch: Scratch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Arrays,
    Floors,
    Walls,
    Types,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    phase: Phase,
    pub arrays: saved_world::Execution,
    pub floors: surface_remap::Execution,
    pub walls: surface_remap::Execution,
    statuses: [Option<i32>; 3],
}
impl Execution {
    pub fn new(version: i32) -> Result<Self, Stop> {
        Ok(Self {
            phase: Phase::Arrays,
            arrays: saved_world::Execution::new(version)?,
            floors: surface_remap::Execution::new(surface_remap::Kind::Floor, version),
            walls: surface_remap::Execution::new(surface_remap::Kind::Wall, version),
            statuses: [None; 3],
        })
    }
    pub fn ready_for_types(&self) -> bool {
        self.phase == Phase::Types
    }
    pub fn statuses(&self) -> [Option<i32>; 3] {
        self.statuses
    }
    /// Explicit service-host variant for component/reference adapters. The
    /// default step method below binds the qualified native wall implementation.
    /// One transition through the actual retained owners. Borrowed floor views
    /// keep remap writes in the same arrays that the preceding read modified.
    /// The returned vector is diagnostic; it is not a new native status/error.
    /// Keep State, this execution and the complete source/allocator/wall host
    /// together across a research cut. No VM dispatch, RNG, TATT, primary-room
    /// rebuild, second clear or ObjM import occurs here.
    pub fn step_with_wall_services(
        &mut self,
        state: &mut State,
        host: &mut impl Host,
    ) -> Result<Option<[i32; 3]>, Stop> {
        match self.phase {
            Phase::Arrays => {
                if let Some(status) =
                    self.arrays
                        .step(&mut state.arrays, &mut state.scratch, host)?
                {
                    self.statuses[0] = Some(status);
                    self.phase = Phase::Floors;
                }
            }
            Phase::Floors | Phase::Walls => {
                let [floor0, floor1] = &mut state.arrays.floors;
                let status = if self.phase == Phase::Floors {
                    self.floors.step(
                        &mut state.floors,
                        state.arrays.active_size,
                        [&mut floor0.covering, &mut floor1.covering],
                        host,
                    )?
                } else {
                    self.walls.step(
                        &mut state.walls,
                        state.arrays.active_size,
                        [&mut floor0.covering, &mut floor1.covering],
                        host,
                    )?
                };
                if let Some(status) = status {
                    let index = if self.phase == Phase::Floors { 1 } else { 2 };
                    self.statuses[index] = Some(status);
                    self.phase = if index == 1 {
                        Phase::Walls
                    } else {
                        Phase::Types
                    };
                }
            }
            Phase::Types => {}
        }
        if self.ready_for_types() {
            Ok(Some([
                self.statuses[0].unwrap(),
                self.statuses[1].unwrap(),
                self.statuses[2].unwrap(),
            ]))
        } else {
            Ok(None)
        }
    }
    /// Bind wall get/put to the same retained raw arrays read in phase one.
    /// Only resource/codec/heap/catalog services are taken from Host here;
    /// its authored get/put alternatives are never used as a fallback.
    pub fn step(
        &mut self,
        state: &mut State,
        host: &mut impl Host,
    ) -> Result<Option<[i32; 3]>, Stop> {
        if matches!(self.phase, Phase::Arrays | Phase::Types) {
            return self.step_with_wall_services(state, host);
        }
        let active = state.arrays.active_size;
        let [floor0, floor1] = &mut state.arrays.floors;
        let [vertices0, vertices1] = &mut state.vertices;
        let mut services = NativeWalls {
            host,
            active,
            walls: [&mut floor0.wall, &mut floor1.wall],
            vertices: [vertices0, vertices1],
        };
        let status = if self.phase == Phase::Floors {
            self.floors.step(
                &mut state.floors,
                active,
                [&mut floor0.covering, &mut floor1.covering],
                &mut services,
            )?
        } else {
            self.walls.step(
                &mut state.walls,
                active,
                [&mut floor0.covering, &mut floor1.covering],
                &mut services,
            )?
        };
        if let Some(status) = status {
            let index = if self.phase == Phase::Floors { 1 } else { 2 };
            self.statuses[index] = Some(status);
            self.phase = if index == 1 {
                Phase::Walls
            } else {
                Phase::Types
            };
        }
        if self.ready_for_types() {
            Ok(Some([
                self.statuses[0].unwrap(),
                self.statuses[1].unwrap(),
                self.statuses[2].unwrap(),
            ]))
        } else {
            Ok(None)
        }
    }
}

struct NativeWalls<'a, H> {
    host: &'a mut H,
    active: i32,
    walls: [&'a mut Matrix; 2],
    vertices: [&'a mut Matrix; 2],
}
impl<H: surface_remap::Host> surface_remap::Host for NativeWalls<'_, H> {
    fn free_vector(&mut self, owner: u64, bytes: usize) -> Result<(), Stop> {
        self.host.free_vector(owner, bytes)
    }
    fn read_map(
        &mut self,
        kind: surface_remap::Kind,
        id: u32,
        target: &mut surface_remap::Groups,
    ) -> Result<i32, Stop> {
        self.host.read_map(kind, id, target)
    }
    fn normalize_legacy(
        &mut self,
        source: &surface_remap::Groups,
        target: &mut surface_remap::Groups,
    ) -> Result<(), Stop> {
        self.host.normalize_legacy(source, target)
    }
    fn allocate_table(&mut self, bytes: usize) -> Result<u64, Stop> {
        self.host.allocate_table(bytes)
    }
    fn free_table(&mut self, owner: u64) -> Result<(), Stop> {
        self.host.free_table(owner)
    }
    fn match_file(&mut self, name: &[u8], file: u64) -> Result<bool, Stop> {
        self.host.match_file(name, file)
    }
    fn resolve_direct(
        &mut self,
        kind: surface_remap::Kind,
        index: usize,
        local: u16,
    ) -> Result<u16, Stop> {
        self.host.resolve_direct(kind, index, local)
    }
    fn resolve_nearest(&mut self, kind: surface_remap::Kind, key: u32) -> Result<u16, Stop> {
        self.host.resolve_nearest(kind, key)
    }
    fn get_wall(
        &mut self,
        coord: surface_remap::Coord,
    ) -> Result<surface_remap::ExpandedWall, Stop> {
        let i = level(coord)?;
        crate::native_wall_io::get(self.walls[i], self.active, coord)
    }
    fn put_wall(
        &mut self,
        coord: surface_remap::Coord,
        wall: &surface_remap::ExpandedWall,
    ) -> Result<(), Stop> {
        let i = level(coord)?;
        // The qualified rotation0 routine has no yield/callback. Preflight
        // consumes only borrowed input; all exact local stores then commit.
        let mut plan = crate::native_wall_io::plan_put(
            self.walls[i],
            self.vertices[i],
            self.active,
            coord,
            wall,
        )?;
        while !plan.completed() {
            plan.step(self.walls[i], self.vertices[i])?;
        }
        Ok(())
    }
}
fn level(coord: surface_remap::Coord) -> Result<usize, Stop> {
    if !(1..=2).contains(&coord.level) {
        Err(Stop::Unsupported(
            "wall level outside retained owner".into(),
        ))
    } else {
        Ok(usize::from(coord.level - 1))
    }
}

#[cfg(test)]
mod tests;
