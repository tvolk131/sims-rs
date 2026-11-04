//! Dynamic-object prefix of 0x4823fa and its separately verified 0x4817e9 gate.
//! The caller consumes each geometry operation before running callbacks for
//! later objects. Prefix-only laboratories can stop at a retained rectangle.
use crate::{Point, Simulation, Step, contention::ObstacleGate};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, Status, ThreadId, Vm};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Callback {
    object: ObjectId,
    thread: ThreadId,
    depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Geometry {
    pub object: ObjectId,
    /// Native rectangle constructor order: min Y, min X, max Y, max X.
    pub rectangle: crate::regions::Region,
    pub kind: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prefix {
    caller: ObjectId,
    ignored_person: Option<ObjectId>,
    ignore_people: bool,
    room: i16,
    current: Option<ObjectId>,
    visited: Vec<ObjectId>,
    callback: Option<Callback>,
    pub geometry: Option<Geometry>,
}

impl Prefix {
    pub fn begin(s: &Simulation, caller: ObjectId) -> Result<Self, String> {
        let person =
            s.vm.world
                .objects
                .get(&caller)
                .and_then(|o| o.person.as_ref())
                .ok_or("route obstacle caller is not a person")?;
        let special = [68, 84]
            .into_iter()
            .map(|i| {
                person
                    .get(&i)
                    .copied()
                    .ok_or("route obstacle person field absent")
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|v| v & 1 != 0);
        let order = &s
            .vm
            .world
            .person
            .as_ref()
            .ok_or("route obstacle registry absent")?
            .object_order;
        Ok(Self {
            caller,
            ignored_person: None,
            ignore_people: false,
            room: word(&s.vm, caller, 29)?,
            current: if special {
                None
            } else {
                order.first().copied()
            },
            visited: vec![],
            callback: None,
            geometry: None,
        })
    }
    pub(crate) fn set_person_filter(&mut self, ignored: Option<ObjectId>, all: bool) {
        self.ignored_person = ignored;
        self.ignore_people = all;
    }
    pub(crate) fn person_filter(&self) -> (Option<ObjectId>, bool) {
        (self.ignored_person, self.ignore_people)
    }
    pub(crate) fn entered(&self) -> bool {
        !self.visited.is_empty() || self.callback.is_some() || self.geometry.is_some()
    }
    pub fn owns_callback(&self, vm: &Vm, p: &Program, thread: ThreadId, depth: usize) -> bool {
        self.callback.as_ref().is_some_and(|c| {
            let Some(t) = vm.threads.get(&thread) else {
                return false;
            };
            let Some(f) = t.stack.get(depth) else {
                return false;
            };
            let Some(d) = vm
                .world
                .objects
                .get(&c.object)
                .and_then(|o| o.definition.as_ref())
            else {
                return false;
            };
            c.thread == thread
                && c.depth == depth
                && self.current == Some(c.object)
                && t.callback
                    .iter()
                    .flat_map(|b| b.chain())
                    .any(|b| b.external && b.base_depth == depth)
                && f.caller == c.object
                && f.callee == c.object
                && f.code_owner == d.source
                && p.modules
                    .get(&d.source)
                    .and_then(|m| m.object_functions.get(&d.object))
                    .and_then(|v| v.get(5))
                    .is_some_and(|b| b.action == f.location.bhav)
        })
    }
    pub fn validate(&self, s: &Simulation, p: &Program, caller: ObjectId) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        if self.caller != caller
            || !s.vm.world.objects.contains_key(&caller)
            || self.visited.iter().any(|id| !seen.insert(*id))
            || self
                .current
                .is_some_and(|id| seen.contains(&id) || !s.vm.world.objects.contains_key(&id))
            || self
                .callback
                .as_ref()
                .is_some_and(|c| !self.owns_callback(&s.vm, p, c.thread, c.depth))
            || self
                .geometry
                .as_ref()
                .is_some_and(|g| self.current != Some(g.object) || self.callback.is_some())
        {
            return Err("invalid native obstacle prefix".into());
        }
        Ok(())
    }
    fn advance(&mut self, s: &Simulation) -> Result<(), String> {
        let id = self.current.ok_or("route obstacle cursor exhausted")?;
        if self.visited.contains(&id) {
            return Err("cyclic native object registry".into());
        }
        self.visited.push(id);
        // Native reads object+0xe0 after the callback, not a pre-callback copy.
        let order = &s
            .vm
            .world
            .person
            .as_ref()
            .ok_or("route obstacle registry absent")?
            .object_order;
        let index = order
            .iter()
            .position(|&n| n == id)
            .ok_or("route obstacle deleted during callback")?;
        self.current = order.get(index + 1).copied();
        Ok(())
    }
    ///Advance after the caller has transformed/inserted the retained rectangle.
    pub(crate) fn consume_geometry(&mut self, s: &Simulation) -> Result<(), String> {
        if self.geometry.is_none() || self.callback.is_some() {
            return Err("native obstacle geometry is not ready".into());
        }
        self.geometry = None;
        self.advance(s)
    }
    fn retain(&mut self, s: &Simulation, p: &Program) -> Result<(), String> {
        let id = self.current.ok_or("route obstacle cursor exhausted")?;
        let [max_x, max_y, min_x, min_y] = *s
            .native_lot
            .as_ref()
            .ok_or("native host absent")?
            .placement
            .bounds
            .get(&id)
            .ok_or("route obstacle retained bounds absent")?;
        let cost = if word(&s.vm, id, 10)? == 0 {
            i32::MAX
        } else {
            10
        };
        self.geometry = Some(Geometry {
            object: id,
            rectangle: crate::regions::Region([min_y, min_x, max_y, max_x, cost]),
            kind: kind(&s.vm, p, id)?,
        });
        Ok(())
    }
}
fn word(vm: &Vm, id: ObjectId, n: i16) -> Result<i16, String> {
    vm.world
        .objects
        .get(&id)
        .and_then(|o| o.data.get(&n))
        .copied()
        .ok_or("route obstacle object field absent".into())
}
fn kind(vm: &Vm, p: &Program, id: ObjectId) -> Result<i16, String> {
    let d = vm
        .world
        .objects
        .get(&id)
        .and_then(|o| o.definition.as_ref())
        .ok_or("route obstacle definition absent")?;
    p.modules
        .get(&d.source)
        .and_then(|m| m.object_definition_words.get(&d.object))
        .and_then(|w| w.get(9))
        .copied()
        .ok_or("route obstacle kind absent".into())
}

impl Simulation {
    pub(crate) fn drive_native_obstacle_prefix(
        &mut self,
        p: &Program,
        e: &mut Prefix,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<bool, String> {
        loop {
            if e.geometry.is_some() || e.current.is_none() {
                return Ok(true);
            }
            if let Some(c) = e.callback.clone() {
                if self.vm.threads[&c.thread].callback.is_some() {
                    if *remaining == 0 {
                        return Ok(false);
                    }
                    if self.vm.threads[&c.thread].status != Status::Ready {
                        return Err(format!(
                            "native obstacle callback host required: {:?}",
                            self.vm.threads[&c.thread].status
                        ));
                    }
                    let before = self.vm.threads[&c.thread].stack.clone();
                    let slice = self.vm.run_slice(p, c.thread, 1)?;
                    if slice.trace.is_empty()
                        && self.vm.threads[&c.thread].callback.is_some()
                        && before == self.vm.threads[&c.thread].stack
                        && self.vm.threads[&c.thread].status == Status::Ready
                    {
                        return Err("native obstacle callback made no progress".into());
                    }
                    *remaining -= slice.trace.len();
                    step.trace.extend(slice.trace);
                    self.sync_script_directions()?;
                    if self.vm.threads[&c.thread].callback.is_some() {
                        continue;
                    }
                }
                let ignored = self.vm.threads[&c.thread]
                    .callback_result
                    .ok_or("native obstacle callback result absent")?;
                e.callback = None;
                if ignored {
                    e.advance(self)?;
                } else {
                    e.retain(self, p)?;
                }
                continue;
            }
            let id = e.current.unwrap();
            if e.visited.contains(&id) {
                return Err("cyclic native object registry".into());
            }
            if id == e.caller || word(&self.vm, id, 29)? != e.room {
                e.advance(self)?;
                continue;
            }
            let contained = self
                .native_lot
                .as_ref()
                .ok_or("native host absent")?
                .placement
                .links
                .capture(&mut self.vm.world, id)?
                .container
                .is_some();
            let flags = word(&self.vm, id, 8)? as u16;
            let weight = word(&self.vm, id, 10)?;
            let mut gate = crate::contention::obstacle_gate(
                contained,
                flags,
                weight,
                false,
                false,
                Point::new(0, 0),
                0,
            );
            if gate != ObstacleGate::Ignore && kind(&self.vm, p, id)? == 2 {
                gate = if self.vm.world.objects[&id].person.is_none() {
                    // A kind-2 definition whose native RTTI cast fails blocks.
                    ObstacleGate::Block
                } else {
                    let actor = self
                        .vm
                        .world
                        .positions
                        .get(&e.caller)
                        .ok_or("route actor position absent")?;
                    let other = self
                        .vm
                        .world
                        .positions
                        .get(&id)
                        .ok_or("route obstacle position absent")?;
                    crate::contention::obstacle_gate(
                        false,
                        flags,
                        weight,
                        true,
                        e.ignored_person == Some(id),
                        Point::new(other.x - actor.x, other.y - actor.y),
                        other.level - actor.level,
                    )
                };
            }
            match gate {
                ObstacleGate::Ignore => e.advance(self)?,
                ObstacleGate::Block => e.retain(self, p)?,
                ObstacleGate::ObjectFunction => {
                    let depth = self
                        .vm
                        .object_threads
                        .get(&id)
                        .and_then(|id| self.vm.threads.get(id))
                        .map_or(0, |t| t.stack.len());
                    if let Some(thread) = self.vm.begin_object_function(p, id, 5, e.caller)? {
                        e.callback = Some(Callback {
                            object: id,
                            thread,
                            depth,
                        });
                    } else {
                        e.retain(self, p)?;
                    }
                }
            }
        }
    }
}
