//! Native 466c3c room rebuild, 467094 dispatch and 468802 occupancy publication.
//! Rendering owns six light models. Here their shared references are stable pool
//! indices; room vectors retain the native order, including null references.
use super::{
    Aggregate, RoomScore, Scores, Totals, Tuning, aggregate, daylight, lighting, objects, score,
};
use crate::{
    lot_geometry::Lot,
    lot_links::{Cursor, Links},
    pose::transform::Terrain,
};
use serde::{Deserialize, Serialize};
use sims_vm::{Program, Vm};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub daylight_multiplier: f32,
    pub light_range: f32,
    pub rooms_enabled: bool,
    /// Results of the six renderer allocations in 4673d0, in allocation order.
    pub models: [Option<u8>; 6],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    pub active: bool,
    pub outdoors: bool,
    pub occupied: bool,
    pub pool: bool,
    pub category8: bool,
    pub tiles: Vec<[usize; 3]>,
    pub totals: Totals,
    pub scores: Scores,
    pub models: Vec<Option<u8>>,
    pub positions: Vec<[u32; 3]>,
    /// Native room+28. Empty storage is the lazy-build sentinel in 46795a.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route_rectangles: Vec<crate::regions::Region>,
}
impl Room {
    /// 46795a/482347. The builder is called with terrain enabled by its
    /// caller. An empty result is retried on the next query, as in the game.
    pub fn route_rectangles_with(
        &mut self,
        build: impl FnOnce() -> Result<Vec<crate::regions::Region>, String>,
    ) -> Result<&[crate::regions::Region], String> {
        if self.route_rectangles.is_empty() {
            self.route_rectangles = build()?;
        }
        Ok(&self.route_rectangles)
    }
    /// The below-six portion of 467e05. Null allocations still occupy a slot.
    pub fn prepare_light(&mut self, settings: Settings, point: [i32; 2], height: f32) {
        if self.models.len() < 6 {
            self.models.push(settings.models[self.models.len()]);
            self.positions
                .push(lighting::position(point, height).map(f32::to_bits));
        }
    }
    /// 4688bf, also inlined on each side of the rebuild in 466c3c.
    pub fn effective_light(&self) -> f32 {
        let light = f32::from_bits(self.scores.light);
        if self.pool
            || (self.occupied
                && self.totals.lights == 0
                && self.totals.people > 0
                && !(light >= 0.8))
        {
            0.8
        } else {
            light
        }
    }
    /// 4678b7 clears geometry alongside the lighting/environment aggregates.
    pub fn reset(&mut self) {
        self.totals = Totals::default();
        self.scores = Scores {
            light: 0,
            environment: 0,
            published_light: 0,
        };
        self.models.clear();
        self.positions.clear();
        self.route_rectangles.clear();
        self.category8 = false;
        self.pool = false;
        // 4678b7 preserves occupancy; the caller immediately reactivates it.
        self.active = true;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manager {
    pub size: usize,
    pub tuning: Tuning,
    pub settings: Settings,
    pub rooms: BTreeMap<u16, Room>,
    pub aggregate: Aggregate,
    pub light_bytes: Vec<Vec<u8>>,
    pub light_maps: Vec<lighting::Map>,
    pub ambient: u32,
}

impl Manager {
    pub fn new(lot: &Lot, tuning: Tuning, settings: Settings) -> Result<Self, String> {
        if !settings.daylight_multiplier.is_finite() || !settings.light_range.is_finite() {
            return Err("nonfinite room manager settings".into());
        }
        let mut rooms = BTreeMap::new();
        for floor in &lot.floors {
            for (&id, tiles) in &floor.rooms.room_tiles {
                if tiles.is_empty() {
                    return Err("active room has no registered tiles".into());
                }
                rooms.insert(
                    id as u16,
                    Room {
                        active: true,
                        outdoors: floor.metrics[&id].outdoors,
                        occupied: false,
                        pool: false,
                        category8: false,
                        tiles: tiles.clone(),
                        totals: Totals::default(),
                        scores: Scores {
                            light: 0,
                            environment: 0,
                            published_light: 0,
                        },
                        models: vec![],
                        positions: vec![],
                        route_rectangles: vec![],
                    },
                );
            }
        }
        Ok(Self {
            size: lot.size,
            tuning,
            settings,
            rooms,
            aggregate: Aggregate {
                indoor_area: 0,
                outdoor_sum: 0,
                outdoor_environment: 0,
                dirty_floors: 0,
            },
            light_bytes: vec![vec![0; lot.size * lot.size]; lot.floors.len()],
            light_maps: vec![lighting::Map::default(); lot.floors.len()],
            ambient: 0,
        })
    }

    /// 466c01, called by the Live-mode house refresh (416534). Rebuilds wait
    /// for the view-frame boundary; inactive rooms retain their dirty byte.
    pub fn invalidate_active_rooms(&self, vm: &mut Vm) -> Result<(), String> {
        let refresh = vm
            .world
            .refresh
            .as_mut()
            .ok_or("room refresh context absent")?;
        for (&id, room) in &self.rooms {
            if room.active {
                refresh
                    .rooms
                    .get_mut(&id)
                    .ok_or("room refresh directory differs")?
                    .dirty = true;
            }
        }
        refresh.dirty_categories = 3;
        Ok(())
    }

    /// DD/466ba3. This invalidates rooms without recomputing their scores.
    pub fn lighting_changed(&mut self, vm: &mut Vm, period: i16) -> Result<(), String> {
        self.invalidate_active_rooms(vm)?;
        self.ambient = (if period == 0 { 1.0f32 } else { 0.8f32 }).to_bits();
        Ok(())
    }

    /// Synchronous 468802: the F1 broadcast precedes light-byte publication.
    pub fn occupied(&mut self, vm: &mut Vm, id: i16, occupied: bool) -> Result<(), String> {
        let Some(room) = self.rooms.get_mut(&(id as u16)) else {
            return Ok(());
        };
        if room.outdoors || room.pool || room.occupied == occupied {
            return Ok(());
        }
        room.occupied = occupied;
        if room.totals.lights == 0 && f32::from_bits(room.scores.light) < 0.8 {
            vm.refresh_room_event(0xf1, i32::from(id as u16))?;
            let light = if occupied {
                0.8
            } else {
                f32::from_bits(room.scores.light)
            };
            room.scores.published_light = light.to_bits();
            lighting::publish(&mut self.light_bytes, self.size, &room.tiles, light)?;
        }
        Ok(())
    }

    pub fn environment(&self, id: i16) -> u32 {
        self.rooms.get(&(id as u16)).map_or(0, |room| {
            if room.outdoors {
                self.aggregate.outdoor_environment
            } else {
                room.scores.environment
            }
        })
    }

    /// 466c3c orchestration. The contribution callback is synchronous and must
    /// preserve per-tile geometry/object ordering. It owns pool classification,
    /// object location services and room light references before score() runs.
    pub fn rebuild_with(
        &mut self,
        vm: &mut Vm,
        period: i16,
        floor: i32,
        mut contributions: impl FnMut(
            u16,
            &mut Room,
            &mut [lighting::Map],
            &mut Vm,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        for (&id, room) in &mut self.rooms {
            if !room.active || (floor != 0 && floor != i32::from((id >> 10) & 3) + 1) {
                continue;
            }
            let dirty = vm
                .world
                .refresh
                .as_ref()
                .ok_or("room refresh context absent")?
                .rooms
                .get(&id)
                .ok_or("room refresh directory differs")?
                .dirty;
            if !dirty {
                continue;
            }
            let old_light = room.effective_light();
            room.reset();
            vm.world
                .refresh
                .as_mut()
                .unwrap()
                .rooms
                .get_mut(&id)
                .unwrap()
                .dirty = false;
            contributions(id, room, &mut self.light_maps, vm)?;
            room.scores = score(
                &room.totals,
                &self.tuning,
                room.outdoors,
                room.pool,
                room.occupied,
                daylight(period, self.settings.daylight_multiplier),
            );
            lighting::publish(
                &mut self.light_bytes,
                self.size,
                &room.tiles,
                f32::from_bits(room.scores.published_light),
            )?;
            let new_light = room.effective_light();
            if !old_light.is_nan() && !new_light.is_nan() && old_light != new_light {
                vm.refresh_room_event(0xf1, i32::from(id))?;
            }
        }
        let scores: Vec<_> = self
            .rooms
            .iter()
            .map(|(&id, r)| RoomScore {
                id: id as i16,
                active: r.active,
                outdoors: r.outdoors,
                half_tiles: r.totals.half_tiles,
                environment: r.scores.environment,
            })
            .collect();
        let dirty = vm
            .world
            .refresh
            .as_ref()
            .ok_or("room refresh context absent")?
            .dirty_categories;
        self.aggregate = aggregate(&scores, &self.tuning, period, dirty, floor);
        vm.world
            .globals
            .insert(27, self.aggregate.indoor_area as i16);
        let refresh = vm.world.refresh.as_mut().unwrap();
        // F1 above can leave a room dirty even though the manager clears these
        // floor bits. Do not normalize away that original ordering.
        refresh.dirty_categories = self.aggregate.dirty_floors;
        for (&id, room) in &self.rooms {
            if room.active && (floor == 0 || floor == i32::from((id >> 10) & 3) + 1) {
                // Full-viewport redraw replaces native per-view tile caches.
                refresh.redraw_rooms.insert(id);
            }
        }
        Ok(())
    }

    /// 467094 at an explicit host/view update boundary. This method does not
    /// assume that one room-manager update equals one simulation tick.
    pub fn update(
        &mut self,
        vm: &mut Vm,
        program: &Program,
        lot: &Lot,
        links: &Links,
        period: i16,
    ) -> Result<bool, String> {
        self.update_with(vm, lot, links, period, |world, id| {
            objects::Input::read(world, program, id)
        })
    }
    /// Same467094/466c3c update, with an explicit object-definition provider.
    pub fn update_with(
        &mut self,
        vm: &mut Vm,
        lot: &Lot,
        links: &Links,
        period: i16,
        mut input: impl FnMut(&sims_vm::World, sims_vm::ObjectId) -> Result<objects::Input, String>,
    ) -> Result<bool, String> {
        let floor = match vm
            .world
            .refresh
            .as_ref()
            .ok_or("room refresh context absent")?
            .dirty_categories
        {
            1 => 1,
            2 => 2,
            3 => 0,
            _ => return Ok(false),
        };
        let settings = self.settings;
        self.rebuild_with(vm, period, floor, |id, room, maps, vm| {
            let level = i32::from((id >> 10) & 3) + 1;
            let source = lot.floor(level).ok_or("room floor absent")?;
            let grid = lot.wall_grid(level)?;
            room.outdoors = source
                .metrics
                .get(&(id as i16))
                .ok_or("room metrics absent")?
                .outdoors;
            if room.tiles
                != *source
                    .rooms
                    .room_tiles
                    .get(&(id as i16))
                    .ok_or("room registration absent")?
            {
                return Err("room registration changed without rebuilding manager".into());
            }
            let first = room.tiles.first().ok_or("room registration empty")?;
            room.pool = first[2] == 1 && lot.pool[first[0] * lot.size + first[1]] < 16;
            for tile in room.tiles.clone() {
                let [x, y, _] = tile;
                let cell = x * lot.size + y;
                room.totals.add_tile(
                    room.outdoors,
                    lot.pool[cell],
                    source.source.coverings[cell],
                    &grid.expanded(x, y)?,
                )?;
                let Some(root) = links.head(&vm.world, tile.map(|p| p as i8)) else {
                    continue;
                };
                let mut cursor = Cursor::new(root);
                let mut seen = BTreeSet::new();
                while let Some(object) = cursor.current {
                    if !seen.insert(object) {
                        return Err("cyclic room object traversal".into());
                    }
                    let mut execution = objects::Execution::new(id as i16, room.outdoors);
                    loop {
                        let input = input(&vm.world, object)?;
                        let Some(request) =
                            execution.next(&mut room.totals, &mut room.category8, &input)
                        else {
                            break;
                        };
                        let response = match request {
                            objects::Request::Location => objects::Response::Located(
                                links.capture(&mut vm.world, object)?.on_lot,
                            ),
                            objects::Request::PrepareLight { position } => {
                                if room.models.len() < 6 {
                                    let height = Terrain {
                                        size: lot.size,
                                        corners: &lot.heights,
                                    }
                                    .height(position)?
                                    .ok_or("room light terrain position outside lot")?
                                        as f32;
                                    room.prepare_light(settings, position, height);
                                }
                                objects::Response::Done
                            }
                            objects::Request::ClearLight { tile, .. }
                            | objects::Request::DrawLight { tile, .. } => {
                                let draw = matches!(request, objects::Request::DrawLight { .. });
                                let floor = usize::try_from(i32::from(tile[2]) - 1)
                                    .map_err(|_| "room light floor invalid")?;
                                maps.get_mut(floor).ok_or("room light map absent")?.update(
                                    lot.size,
                                    &lot.floors
                                        .get(floor)
                                        .ok_or("room light floor absent")?
                                        .rooms,
                                    tile,
                                    settings.light_range,
                                    draw,
                                    settings.rooms_enabled,
                                )?;
                                objects::Response::Done
                            }
                        };
                        execution.acknowledge(response)?;
                    }
                    cursor.advance(links, &vm.world)?;
                }
            }
            Ok(())
        })?;
        Ok(true)
    }
}
