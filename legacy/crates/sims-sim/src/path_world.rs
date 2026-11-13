//! Geometry boundary between the authored lot and native rectangle search.
use crate::{
    Destination, Point, Rect, Simulation,
    contention::{self, ObstacleGate},
    region_path,
    regions::Region,
};
use sims_vm::{ObjectId, Program};

impl Simulation {
    pub(crate) fn route_candidate_limit(&self, caller: ObjectId) -> usize {
        let room = self.vm.world.objects[&caller]
            .data
            .get(&29)
            .copied()
            .unwrap_or(-5);
        if self
            .vm
            .world
            .room_half_tiles
            .get(&room)
            .copied()
            .unwrap_or(0)
            / 2
            > 250
        {
            8
        } else {
            48
        }
    }
    pub(crate) fn native_path(
        &self,
        program: &Program,
        caller: ObjectId,
        goals: &[(Destination, u32)],
        ignore: Option<ObjectId>,
        ignore_people: bool,
        cross_room: bool,
    ) -> Result<Option<(Destination, Vec<Point>)>, String> {
        if goals.is_empty() {
            return Ok(None);
        }
        let actor = self.body(caller)?;
        let footprint = self.collision_bounds(caller, Point::new(0, 0));
        let native = |r: Rect, cost| Region([r.min.y, r.min.x, r.max.y, r.max.x, cost]);
        let inflate = |r: Rect| Rect {
            min: Point::new(r.min.x - footprint.max.x, r.min.y - footprint.max.y),
            max: Point::new(r.max.x - footprint.min.x, r.max.y - footprint.min.y),
        };
        let mut fixed = self
            .room
            .obstacles
            .iter()
            .filter(|_| actor.level == 1)
            .map(|&r| native(inflate(r), i32::MAX))
            .collect::<Vec<_>>();
        let rect = |x, y, z, w| Rect {
            min: Point::new(x, y),
            max: Point::new(z, w),
        };
        // Keep the authored lot boundary finite even for an exterior room.
        for r in [
            rect(-4096, -4096, 0, 4096),
            rect(self.room.width, -4096, 4096, 4096),
            rect(-4096, -4096, 4096, 0),
            rect(-4096, self.room.height, 4096, 4096),
        ] {
            fixed.push(native(inflate(r), i32::MAX));
        }
        let architecture = self
            .architecture
            .as_ref()
            .ok_or("native path needs architecture")?;
        if architecture.original.is_some() {
            return Err("original-lot route obstacle construction is not activated".into());
        }
        let allowed = self.active_portals(caller);
        for wall in architecture.walls(actor.level) {
            if wall
                .door
                .is_some_and(|parts| parts.iter().any(|id| allowed.contains(id)))
            {
                continue;
            }
            fixed.push(native(
                inflate(Rect {
                    min: wall.a.min(wall.b),
                    max: wall.a.max(wall.b),
                }),
                i32::MAX,
            ));
        }
        for y in (0..self.room.height).step_by(16) {
            for x in (0..self.room.width).step_by(16) {
                if !architecture.supported(actor.level, Point::new(x + 8, y + 8)) {
                    fixed.push(native(inflate(rect(x, y, x + 16, y + 16)), i32::MAX));
                }
            }
        }
        if !cross_room {
            let room = architecture.room_at(actor.level, actor.position);
            for y in (0..self.room.height).step_by(16) {
                for x in (0..self.room.width).step_by(16) {
                    if architecture.room_at(actor.level, Point::new(x + 8, y + 8)) != room {
                        fixed.push(native(rect(x, y, x + 16, y + 16), i32::MAX));
                    }
                }
            }
        }
        let order: Vec<_> = self.vm.world.person.as_ref().map_or_else(
            || self.bodies.keys().copied().collect(),
            |p| p.object_order.clone(),
        );
        let mut dynamic = vec![];
        for id in order {
            if id == caller || Some(id) == ignore || self.unplaced.contains(&id) {
                continue;
            }
            let Some(body) = self.bodies.get(&id) else {
                continue;
            };
            if body.level != actor.level {
                continue;
            }
            let o = &self.vm.world.objects[&id];
            let person = o.person.is_some();
            if ignore_people && person {
                continue;
            }
            let weight = o.data.get(&10).copied().unwrap_or(0);
            let gate = contention::obstacle_gate(
                o.container.is_some(),
                o.data.get(&8).copied().unwrap_or(0) as u16,
                weight,
                person,
                false,
                Point::new(
                    body.position.x - actor.position.x,
                    body.position.y - actor.position.y,
                ),
                0,
            );
            if gate == ObstacleGate::Ignore {
                continue;
            }
            if gate == ObstacleGate::ObjectFunction {
                let d = o
                    .definition
                    .as_ref()
                    .ok_or("route obstacle lacks definition")?;
                if program.modules[&d.source]
                    .object_functions
                    .get(&d.object)
                    .and_then(|f| f.get(5))
                    .is_some_and(|f| f.action != 0)
                {
                    return Err(format!(
                        "native route obstacle #{id} requires function 5 callback"
                    ));
                }
            }
            let bounds = self.collision_bounds(id, body.position);
            if bounds.min.x < bounds.max.x && bounds.min.y < bounds.max.y {
                dynamic.push(native(
                    inflate(bounds),
                    if weight == 0 { i32::MAX } else { 10 },
                ));
            }
        }
        let room = architecture.room_at(actor.level, actor.position);
        let tiles = (self
            .vm
            .world
            .room_half_tiles
            .get(&room)
            .copied()
            .unwrap_or(0)
            / 2)
        .max(0) as usize;
        let goal_rects: Vec<_> = goals
            .iter()
            .map(|(g, score)| {
                native(
                    rect(
                        g.position.x,
                        g.position.y,
                        g.position.x + 1,
                        g.position.y + 1,
                    ),
                    (*score / 1000) as i32,
                )
            })
            .collect();
        let result = region_path::search(region_path::Query {
            start: Point::new(actor.position.y, actor.position.x),
            goals: &goal_rects,
            dynamic: &dynamic,
            fixed: &fixed,
            maximum: None,
            aspect: None,
            free_cost: 0,
            smooth: true,
            steps: Some((tiles * goal_rects.len() + 50).min(1000) + 1),
        })?;
        let Some(index) = result.goal else {
            return Ok(None);
        };
        let mut path: Vec<_> = result
            .points
            .into_iter()
            .map(|p| Point::new(p.y, p.x))
            .collect();
        path.dedup();
        Ok(Some((goals[index].0, path)))
    }
}
