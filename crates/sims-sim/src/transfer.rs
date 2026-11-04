//! Ordinary-object transfer in the authored room. Native wrappers are validated;
//! complete native slot-size, footprint, callbacks and floor-placement rules are
//! still separate work. Unsupported subclasses/callbacks stop explicitly.
use crate::{Event, Point, Simulation};
use sims_vm::{Program, ThreadId, transfer::Request};
impl Simulation {
    /// 0x433e50 recursively gives contained objects their parent's physical
    /// coordinates. Bone/SLOT presentation offsets are separate from this.
    pub(crate) fn sync_contained_positions(
        &mut self,
        parent: sims_vm::ObjectId,
    ) -> Result<(), String> {
        let mut pending = vec![parent];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(parent) = pending.pop() {
            if !seen.insert(parent) {
                return Err("cyclic containment during movement".into());
            }
            let body = self.body(parent)?.clone();
            let room = self.vm.world.objects[&parent].data.get(&29).copied();
            let children: Vec<_> = self.vm.world.objects[&parent]
                .slots
                .iter()
                .flatten()
                .copied()
                .collect();
            for id in children {
                if self.vm.world.objects[&id].person.is_some() {
                    return Err(
                        "moving a container holding a person requires native placement".into(),
                    );
                }
                let child = self
                    .bodies
                    .get_mut(&id)
                    .ok_or("contained object lacks body")?;
                child.position = body.position;
                child.level = body.level;
                child.elevation = body.elevation;
                if let Some(room) = room {
                    self.vm
                        .world
                        .objects
                        .get_mut(&id)
                        .unwrap()
                        .data
                        .insert(29, room);
                }
                pending.push(id);
            }
        }
        Ok(())
    }
    pub(crate) fn service_transfer(
        &mut self,
        p: &Program,
        thread: ThreadId,
        r: &Request,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        if let Some([source, destination]) = r.onto {
            let object =
                sims_vm::transfer::onto_object(&self.vm.world, r.caller, r.target, source)?;
            let success = if let Some(object) = object {
                if destination < 0 {
                    false
                } else {
                    self.transfer_at(
                        p,
                        thread,
                        r.caller,
                        object,
                        Some((r.target, destination as u16)),
                        self.body(r.target)?.position,
                        events,
                    )?
                }
            } else {
                false
            };
            return self.vm.finish_transfer(p, thread, r.id, success);
        }
        let Some(id) = sims_vm::transfer::object(&self.vm.world, r.caller, r.target, r.grab) else {
            self.vm.finish_transfer(p, thread, r.id, false)?;
            return Ok(());
        };
        let actor = self.body(r.caller)?.clone();
        let parent = if r.grab { Some((r.caller, 0u16)) } else { None };
        let position = if r.grab {
            actor.position
        } else {
            let (point, _) = crate::routing::direct_snap(
                [actor.position.y, actor.position.x],
                actor.facing.trailing_zeros() as u8,
                0,
                true,
            )?;
            Point::new(point[1], point[0])
        };
        let success = self.transfer_at(p, thread, r.caller, id, parent, position, events)?;
        self.vm.finish_transfer(p, thread, r.id, success)
    }
    pub(crate) fn transfer_at(
        &mut self,
        p: &Program,
        thread: ThreadId,
        caller: i16,
        id: i16,
        parent: Option<(i16, u16)>,
        position: Point,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        let object = &self.vm.world.objects[&id];
        let def = object
            .definition
            .as_ref()
            .ok_or("transfer object has no definition")?;
        let words = &p.modules[&def.source].object_definition_words[&def.object];
        if object.person.is_some() || words[0x14 / 2] != 0 || id == caller {
            return Err("Grab/Drop only supports ordinary single-tile objects".into());
        }
        for object in [id, caller] {
            let def = self.vm.world.objects[&object]
                .definition
                .as_ref()
                .ok_or("placement callback definition unavailable")?;
            if p.modules[&def.source]
                .object_functions
                .get(&def.object)
                .and_then(|f| f.get(9))
                .is_some_and(|f| f.action != 0)
            {
                return Err("Grab/Drop placement callbacks are not implemented".into());
            }
        }
        let actor = self.body(parent.map_or(caller, |v| v.0))?.clone();
        let success = if let Some((target, slot)) = parent {
            self.vm.world.objects[&target]
                .slots
                .get(slot as usize)
                .is_some_and(|v| v.is_none() || *v == Some(id))
        } else {
            self.clear(id, position, None)
        };
        if success {
            if let Some((old, slot)) = self.vm.world.objects[&id].container {
                self.vm.world.objects.get_mut(&old).unwrap().slots[slot as usize] = None;
            }
            if let Some((parent, slot)) = parent {
                self.vm.world.objects.get_mut(&parent).unwrap().slots[slot as usize] = Some(id);
            }
            let room = self.vm.world.objects[&parent.map_or(caller, |v| v.0)].data[&29];
            let object = self.vm.world.objects.get_mut(&id).unwrap();
            object.container = parent;
            object.data.extend([
                (2, parent.map_or(0, |v| v.0)),
                (3, parent.map_or(-1, |v| v.1 as i16)),
                (14, parent.map_or(-1, |v| v.1 as i16 + 1)),
                (26, parent.map_or(0, |v| v.0)),
                (29, room),
            ]);
            let body = self
                .bodies
                .get_mut(&id)
                .ok_or("transfer object lacks physical body")?;
            body.position = position;
            body.level = actor.level;
            body.elevation = if parent.is_some() { actor.elevation } else { 0 };
            self.sync_contained_positions(id)?;
            self.event(
                events,
                thread,
                format!(
                    "{} object #{id} {}",
                    if parent.is_some() {
                        "contained"
                    } else {
                        "dropped"
                    },
                    if parent.is_some() {
                        "into destination slot"
                    } else {
                        "in front of caller"
                    }
                ),
            );
        }
        Ok(success)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Body, Room};
    use serde_json::Value;
    use sims_vm::{Object, Vm, World};
    use std::collections::BTreeMap;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_contained_physical_positions_across_floors() {
        let report: Value =
            serde_json::from_str(include_str!("../tests/fixtures/contained-position.json"))
                .unwrap();
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            if c["position"][0].as_i64().unwrap() < 0 {
                continue;
            }
            let slot = c["slot"].as_u64().unwrap() as usize;
            let mut parent = Object {
                slots: vec![None; 2],
                data: BTreeMap::from([(1, 0), (29, 3)]),
                ..Default::default()
            };
            parent.slots[slot] = Some(2);
            let child = Object {
                container: Some((1, slot as u16)),
                data: BTreeMap::from([(1, 0), (29, 1)]),
                ..Default::default()
            };
            let mut world = World::default();
            world.objects.extend([(1, parent), (2, child)]);
            let body = Body {
                level: 1,
                position: Point::new(8, 8),
                radius: 1,
                elevation: 0,
                facing: 1,
            };
            let mut s = Simulation::new(
                Vm::new(world, 1),
                Room {
                    width: 256,
                    height: 256,
                    obstacles: vec![],
                },
                BTreeMap::from([(1, body.clone()), (2, body)]),
            )
            .unwrap();
            s.bodies.get_mut(&1).unwrap().position = Point::new(
                c["position"][1].as_i64().unwrap() as i32,
                c["position"][0].as_i64().unwrap() as i32,
            );
            s.bodies.get_mut(&1).unwrap().level = c["floor"].as_i64().unwrap() as i32;
            s.sync_contained_positions(1).unwrap();
            for id in [1, 2] {
                let expected = &row["original"]["objects"][id as usize - 1];
                assert_eq!(
                    i64::from(s.bodies[&id].level),
                    expected["floor"].as_i64().unwrap()
                );
                assert_eq!(
                    serde_json::json!([s.bodies[&id].position.y, s.bodies[&id].position.x]),
                    expected["position"]
                );
                assert_eq!(
                    s.vm.world.objects[&id].data[&29],
                    expected["room"].as_i64().unwrap() as i16
                );
            }
        }
    }
}
