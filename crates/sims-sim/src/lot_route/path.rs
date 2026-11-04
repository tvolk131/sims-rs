//! Original-array path assembly. Captured-world wrapper/caller comparisons
//! cover the producer and publication stages; whole gameplay parity remains
//! a separate requirement. Strict mode requires the retained native map owner.
use crate::{Point, Simulation, Step, lot_route, regions::Region};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program};

#[cfg(test)]
mod captured_tests;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Policy {
    /// Native map ownership and animator required; no backing-size/tuned fallback.
    #[default]
    Strict,
    /// Historical experiments can use the backing extent and tuned movement.
    InferredRectangles,
}
impl Policy {
    pub(crate) fn builds_path(self, world: &sims_vm::World) -> bool {
        self == Self::InferredRectangles || world.map_size.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Inputs {
    pub start: Point,
    pub level: i32,
    pub room: i16,
    actor_bounds: Region,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_actor: Option<super::search::Actor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_result: Option<u8>,
    pub dynamic: Vec<Region>,
    pub fixed: Vec<Region>,
    pub points: Option<Vec<Point>>,
}
impl Inputs {
    pub fn begin(s: &Simulation, actor: ObjectId) -> Result<Self, String> {
        let p =
            s.vm.world
                .positions
                .get(&actor)
                .ok_or("route actor position absent")?;
        let [max_x, max_y, min_x, min_y] = *s
            .native_lot
            .as_ref()
            .ok_or("native host absent")?
            .placement
            .bounds
            .get(&actor)
            .ok_or("route actor bounds absent")?;
        let object = &s.vm.world.objects[&actor];
        let room = object.data[&29];
        let environment = s
            .room_environment
            .as_ref()
            .ok_or("native route requires retained room environment")?;
        let retained_room = environment
            .manager
            .rooms
            .get(&(room as u16))
            .ok_or("native route retained room absent")?;
        let person = object
            .person
            .as_ref()
            .ok_or("route actor is not a person")?;
        let search_actor = super::search::Actor {
            special: [68, 84].into_iter().any(|n| person[&n] & 1 != 0),
            ground: object.data[&17] == 0,
            level: i32::from(((room as u16) >> 10) & 3) + 1,
            outdoors: retained_room.outdoors,
        };
        Ok(Self {
            start: Point::new(p.y, p.x),
            level: p.level,
            room,
            actor_bounds: Region([min_y, min_x, max_y, max_x, 0]),
            search_actor: Some(search_actor),
            failure_result: None,
            dynamic: vec![],
            fixed: vec![],
            points: None,
        })
    }
    ///482608 transform/insertion, compared through the full captured wrapper.
    fn retain_geometry(
        &mut self,
        s: &Simulation,
        g: &super::obstacle::Geometry,
    ) -> Result<(), String> {
        let (mask, direction) = if g.kind == 8 {
            let o =
                s.vm.world
                    .objects
                    .get(&g.object)
                    .ok_or("route portal absent")?;
            (o.data[&53], o.data[&1])
        } else {
            (0, 0)
        };
        // The wrapper still uses the observed process setting 0x6c297c = 6.
        let result =
            super::geometry::dynamic(g.rectangle, self.actor_bounds, g.kind, mask, direction, 3);
        if !result.excluded {
            self.dynamic.push(result.rectangle);
        }
        Ok(())
    }
}

impl Simulation {
    /// Borrow the physical grid and resolve original portal-part inputs. Public
    /// for captured-world differential checks of this exact runtime producer.
    pub fn original_route_occupants(
        world: &sims_vm::World,
        a: &crate::architecture::Architecture,
        links: &crate::lot_links::Links,
        level: i32,
    ) -> Result<Vec<Vec<lot_route::TileOccupant>>, String> {
        //Unknown historical inferred sessions retain their existing extent.
        //An explicitly owned native map must not borrow the backing stride.
        let active_size = super::active_size(world, links.size)?;
        let heads = links
            .heads
            .get((level - 1) as usize)
            .ok_or("route floor absent")?;
        let parts: Vec<_> = world
            .objects
            .iter()
            .filter_map(|(&id, o)| {
                let d = o.definition.as_ref()?;
                let f = a.footprints.get(&d.guid)?;
                let part = o.multitile.as_ref()?;
                Some(crate::portals::PairPart {
                    id,
                    head: part.head,
                    next: part.next,
                    style: f.portal_style,
                    base_graphic: f.base_graphic,
                    portal: f.portal,
                })
            })
            .collect();
        heads
            .iter()
            .enumerate()
            .map(|(cell, head)| {
                let mut out = vec![];
                if cell / links.size >= active_size || cell % links.size >= active_size {
                    return Ok(out);
                }
                if *head <= 0 || !world.objects.contains_key(head) {
                    return Ok(out);
                }
                for id in links.walk(world, *head)? {
                    let o = &world.objects[&id];
                    let d = o
                        .definition
                        .as_ref()
                        .ok_or("route tile definition absent")?;
                    let f = a
                        .footprints
                        .get(&d.guid)
                        .ok_or("route tile footprint absent")?;
                    let partner_room = crate::portals::partner(&parts, id)
                        .map(|other| {
                            world
                                .objects
                                .get(&other)
                                .and_then(|o| o.data.get(&29))
                                .copied()
                                .ok_or("route partner room absent")
                        })
                        .transpose()?;
                    out.push(lot_route::TileOccupant {
                        kind: f.kind,
                        flags: o.data[&8] as u16,
                        function15: f.portal,
                        portal_class: f.kind == 8,
                        partner_room,
                    });
                }
                Ok(out)
            })
            .collect()
    }
    pub(crate) fn drive_original_path(
        &mut self,
        p: &Program,
        e: &mut super::live::Execution,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        if e.path.is_none() {
            if e.obstacles.as_ref().is_some_and(|o| o.entered()) {
                return Err("legacy partial route lacks pre-callback path inputs; resume that research checkpoint with its original build".into());
            }
            let input = Inputs::begin(self, e.caller)?;
            if e.potential.is_none() {
                // 45c3ea clears person+ae0 before entering4823fa. Object
                // callbacks must observe the cleared path, including across
                // diagnostic yields. Potential queries use a temporary output.
                self.native_lot
                    .as_mut()
                    .unwrap()
                    .route_paths
                    .entry(e.caller)
                    .or_default()
                    .clear();
            }
            e.path = Some(input);
        }
        if e.path.as_ref().unwrap().points.is_some() {
            return Ok(true);
        }
        let actor = e.path.as_ref().unwrap().search_actor
            .ok_or("legacy partial experimental route lacks pre-callback search inputs; resume that research checkpoint with its original build")?;
        loop {
            if !self.drive_native_obstacle_prefix(
                p,
                e.obstacles.as_mut().unwrap(),
                remaining,
                step,
            )? {
                return Ok(false);
            }
            let Some(g) = e.obstacles.as_ref().unwrap().geometry.clone() else {
                break;
            };
            //The complete4823fa probe retains nearby-person rectangles even
            //when context+8e is set. Only the ignored pointer at+7c changes
            //4817e9's person filter, already handled by the obstacle prefix.
            e.path.as_mut().unwrap().retain_geometry(self, &g)?;
            e.obstacles.as_mut().unwrap().consume_geometry(self)?;
        }
        let input = e.path.as_mut().unwrap();
        if !actor.special {
            let architecture = self
                .architecture
                .as_ref()
                .ok_or("route architecture absent")?;
            let native = self.native_lot.as_ref().ok_or("native host absent")?;
            let room = self
                .room_environment
                .as_mut()
                .ok_or("native route requires retained room environment")?
                .manager
                .rooms
                .get_mut(&(input.room as u16))
                .ok_or("native route retained room absent")?;
            let build = |terrain| {
                let occupants = Self::original_route_occupants(
                    &self.vm.world,
                    architecture,
                    &native.placement.links,
                    input.level,
                )?;
                let prepared = architecture
                    .original
                    .as_ref()
                    .ok_or("original route lot absent")?
                    .route_obstacles_in(
                        input.room,
                        &occupants,
                        terrain,
                        false,
                        super::active_size(&self.vm.world, native.placement.links.size)?,
                    )?;
                super::geometry::finalize(prepared)
            };
            input.fixed = if !actor.ground && actor.level == 1 && actor.outdoors {
                // 482802 requests a temporary terrain-disabled vector.
                build(false)?
            } else {
                room.route_rectangles_with(|| build(true))?.to_vec()
            };
        }
        let set = e.candidates.as_ref().ok_or("route candidates absent")?;
        let (goals, indices) = super::search::goals(set);
        // Original reads the retained totals here, after object callbacks.
        // Do not refresh/rebuild rooms or substitute geometric tile metrics.
        let room = self
            .room_environment
            .as_ref()
            .ok_or("native route requires retained room environment")?
            .manager
            .rooms
            .get(&(input.room as u16))
            .ok_or("native route retained room absent")?;
        let config = actor.configure(
            room.totals.half_tiles,
            room.totals.objects,
            goals
                .len()
                .try_into()
                .map_err(|_| "native route goal count exceeds signed vector length")?,
        );
        let outcome = config.run(input.start, &goals, &input.dynamic, &input.fixed)?;
        if let Some(failure) = outcome.failure_result {
            input.failure_result = Some(failure);
            e.failed = true;
            return Ok(true);
        }
        let result = outcome.search.ok_or("native route final search absent")?;
        let Some(chosen) = result.goal else {
            e.failed = true;
            return Ok(true);
        };
        let candidate = set
            .candidates
            .get(
                *indices
                    .get(chosen)
                    .ok_or("native route selected an absent goal mapping")?,
            )
            .ok_or("native route selected an absent candidate")?
            .clone();
        if e.potential.is_some() {
            // 0x4823fa fills the temporary output vector; 0x45c38e's
            // animator submission, entry flags and retained person path do not run.
            e.path.as_mut().unwrap().points = Some(result.points);
            return Ok(true);
        }
        // 0x45c38e publishes selected entry flags before animator submission,
        // including its later short-path/invalid-look failure branches.
        self.person(e.caller, 37, candidate.flags as i16)?;
        let points = result.points;
        self.native_lot
            .as_mut()
            .unwrap()
            .route_paths
            .insert(e.caller, points.iter().map(|p| [p.y, p.x]).collect());
        if points.first() != Some(&input.start)
            || points.last() != Some(&Point::new(candidate.position[1], candidate.position[0]))
        {
            return Err("original-lot path endpoints do not match route".into());
        }
        if candidate.seat != 0 {
            e.path.as_mut().unwrap().points = Some(points);
            e.selected_seat = Some(candidate);
            return Ok(true);
        }
        let goal = e.destination(&candidate, &points)?;
        // 0x45f659 appends the look point to the person's retained path. It
        // is an orientation marker, not another walking destination.
        let retained = self
            .native_lot
            .as_mut()
            .unwrap()
            .route_paths
            .get_mut(&e.caller)
            .unwrap();
        if let Some(look) = e.endpoint_look(&points)? {
            retained.push(look);
        }

        e.path.as_mut().unwrap().points = Some(points);
        e.failed = goal.is_none();
        e.goal = goal;
        Ok(true)
    }
}
