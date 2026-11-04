//! Synchronous off-lot Create Object during frozen loading. Shared storage and
//! Init/Main stages use the same VM factory as the live simulation. Floor and
//! containment placement remain the virtual placement host's responsibility.
use crate::lot_links::Links;
use serde::{Deserialize, Serialize};
use sims_vm::{Definition, ObjectId, Program, Status, ThreadId, Vm, construction::ObjectDate};
pub mod group;
pub mod live;
pub use live::Execution;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Ordinary graphic owner4376f6, after registration but before date/Init.
    /// Complete its actual asset initialization before acknowledgment.
    Graphic(ObjectId),
    /// Factory 0xe2 broadcast, after immediate Init/Main and before returning
    /// the created ID to the parent. Its observer must run before advancing.
    Created(ObjectId),
    Host {
        thread: ThreadId,
        status: Status,
    },
    Budget,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OffLot {
    /// Absent for direct engine factories (e.g. the load-time indicators).
    parent: Option<(ThreadId, u64)>,
    child: Option<ObjectId>,
    phase: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graphic: Option<Graphic>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Graphic {
    now: ObjectDate,
    time: [i16; 2],
    complete: bool,
}
impl OffLot {
    pub fn begin(
        vm: &mut Vm,
        p: &Program,
        links: &mut Links,
        parent: ThreadId,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<Self, String> {
        let Some(Status::AwaitingCreation(r)) = vm.threads.get(&parent).map(|t| &t.status) else {
            return Err("loaded creation needs a waiting parent".into());
        };
        if r.operands[4] != 6 || r.operands[5] != 0 || r.created.is_some() || r.placement.is_some()
        {
            return Err(
                "loaded factory requires a new off-lot mode-6 request with zero flags".into(),
            );
        }
        let request = r.id;
        let guid = u32::from_le_bytes(r.operands[..4].try_into().unwrap());
        let mut factory = Self::begin_resource(vm, p, links, guid, now, time)?;
        factory.parent = Some((parent, request));
        if factory.child.is_none() {
            vm.finish_creation(p, parent, request, None)?;
        } else {
            let Status::AwaitingCreation(r) = &mut vm.threads.get_mut(&parent).unwrap().status
            else {
                unreachable!()
            };
            r.created = factory.child;
        }
        Ok(factory)
    }

    /// Native direct 0x440b2d factory without a synthetic SimAntics parent.
    /// Returns the same Created boundary after Init/Main, before the caller
    /// continues. Missing linked resources remain explicit load errors.
    pub fn begin_resource(
        vm: &mut Vm,
        p: &Program,
        links: &mut Links,
        guid: u32,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<Self, String> {
        Self::begin_resource_inner(vm, p, links, guid, now, time, false)
    }

    /// Same native factory with the ordinary graphic service exposed at its
    /// original post-construction call site. The legacy entry is retained for
    /// explicit storage-only diagnostics; live asset owners use this entry.
    pub fn begin_resource_with_graphics(
        vm: &mut Vm,
        p: &Program,
        links: &mut Links,
        guid: u32,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<Self, String> {
        Self::begin_resource_inner(vm, p, links, guid, now, time, true)
    }
    fn begin_resource_inner(
        vm: &mut Vm,
        p: &Program,
        links: &mut Links,
        guid: u32,
        now: ObjectDate,
        time: [i16; 2],
        graphics: bool,
    ) -> Result<Self, String> {
        let mut matches = p.modules.values().flat_map(|m| {
            m.object_definition_words
                .iter()
                .filter_map(move |(&id, w)| {
                    (w.len() > 15
                        && (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) == guid)
                        .then_some((m, id))
                })
        });
        let (module, number) = matches
            .next()
            .ok_or_else(|| format!("Create Object GUID {guid:08x} is not loaded"))?;
        if matches.next().is_some() {
            return Err("ambiguous loaded creation GUID".into());
        }
        let words = &module.object_definition_words[&number];
        let definition = Definition {
            source: module.source.clone(),
            object: number,
            guid,
            slots: words[20] as u16,
        };
        Self::begin_definition_inner(vm, p, links, definition, now, time, graphics)
    }
    /// Direct factory with the resource identity already resolved by its
    /// caller. Repeated pool factories must not perform another GUID lookup.
    pub fn begin_definition_with_graphics(
        vm: &mut Vm,
        p: &Program,
        links: &mut Links,
        definition: Definition,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<Self, String> {
        Self::begin_definition_inner(vm, p, links, definition, now, time, true)
    }
    fn begin_definition_inner(
        vm: &mut Vm,
        p: &Program,
        links: &mut Links,
        definition: Definition,
        now: ObjectDate,
        time: [i16; 2],
        graphics: bool,
    ) -> Result<Self, String> {
        let words = sims_vm::construction::definition_words(p, &definition)?;
        if (u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16)) != definition.guid
            || words[20] as u16 != definition.slots
        {
            return Err("factory resource identity differs from definition".into());
        }
        let module = &p.modules[&definition.source];
        let mut number = definition.object;
        let group = module.object_definition_words[&number][10] != 0;
        if group {
            let plan = sims_vm::multitile::construction_plan(module, number)?;
            if plan.len() != 1 {
                return Err("loaded creation of multipart groups needs a group factory".into());
            }
            number = plan[0];
        }
        let Some(id) = (1..=i16::MAX).find(|id| !vm.world.objects.contains_key(id)) else {
            return Ok(Self {
                parent: None,
                child: None,
                phase: 3,
                graphic: None,
            });
        };
        let words = &module.object_definition_words[&number];
        let definition = Definition {
            source: module.source.clone(),
            object: number,
            guid: u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16),
            slots: words[20] as u16,
        };
        let graphic = graphics && words.get(102).is_some_and(|w| w & 1 != 0);
        if graphic {
            vm.begin_object_factory_graphics(p, id, definition)?;
        } else {
            vm.begin_object_factory(p, id, definition, group, now, time)?;
        }
        // Physical slot zero belongs to the new registration, even for a
        // recycled ID. Publish it before any original Init instructions run.
        links.next.insert(id, 0);
        Ok(Self {
            parent: None,
            child: Some(id),
            phase: 0,
            graphic: graphic.then_some(Graphic {
                now,
                time,
                complete: false,
            }),
        })
    }
    pub fn object(&self) -> Option<ObjectId> {
        self.child
    }
    pub fn pending_graphic(&self) -> Option<ObjectId> {
        self.graphic
            .as_ref()
            .is_some_and(|g| !g.complete)
            .then_some(self.child)
            .flatten()
    }
    pub fn pending_created(&self) -> Option<ObjectId> {
        (self.phase == 2).then_some(self.child).flatten()
    }
    pub fn validate_direct(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        if self.parent.is_some() || self.phase > 3 {
            return Err("invalid direct object factory".into());
        }
        let Some(id) = self.child else {
            return if self.phase == 3 && self.graphic.is_none() {
                Ok(())
            } else {
                Err("direct factory lost its instance".into())
            };
        };
        let object = vm.world.objects.get(&id).ok_or("factory object absent")?;
        sims_vm::construction::definition_words(
            p,
            object
                .definition
                .as_ref()
                .ok_or("factory definition absent")?,
        )?;
        if self.pending_graphic().is_some() {
            if self.phase != 0
                || vm.object_threads.contains_key(&id)
                || vm.lifecycles.contains_key(&id)
            {
                return Err("factory ran scripts before graphic initialization".into());
            }
            if object.container.is_some()
                || vm
                    .world
                    .positions
                    .get(&id)
                    .is_none_or(|p| (p.x, p.y, p.level) != (-16, -16, 1))
                || [45, 46, 47, 24, 21]
                    .iter()
                    .any(|k| object.data.get(k) != Some(&0))
                || object.data.get(&11) != Some(&id)
                || object.data.get(&29) != Some(&-5)
            {
                return Err("factory graphic no longer owns fresh off-lot storage".into());
            }
        } else if !vm.object_threads.contains_key(&id) || !vm.lifecycles.contains_key(&id) {
            return Err("factory script lifecycle absent".into());
        }
        Ok(())
    }
    pub fn complete_graphic(
        &mut self,
        vm: &mut Vm,
        p: &Program,
        id: ObjectId,
    ) -> Result<(), String> {
        let graphic = self
            .graphic
            .as_mut()
            .ok_or("ordinary factory has no graphic service")?;
        if self.child != Some(id) || self.phase != 0 || graphic.complete {
            return Err("stale ordinary factory graphic acknowledgment".into());
        }
        vm.finish_object_factory_graphics(p, id, graphic.now, graphic.time)?;
        graphic.complete = true;
        Ok(())
    }
    pub fn advance(
        &mut self,
        vm: &mut Vm,
        p: &Program,
        budget: usize,
    ) -> Result<Option<Request>, String> {
        let Some(child) = self.child else {
            return Ok(None);
        };
        if self.graphic.as_ref().is_some_and(|g| !g.complete) {
            return Ok(Some(Request::Graphic(child)));
        }
        if self.phase == 0 {
            let thread = vm.object_threads[&child];
            let mut remaining = budget;
            loop {
                if remaining == 0 {
                    return Ok(Some(Request::Budget));
                }
                vm.prepare_factory_object_visit(child)?;
                let budget = if vm.scheduled_object_visit.is_some()
                    && vm.threads[&thread].callback.is_some()
                {
                    remaining.min(1)
                } else {
                    remaining
                };
                let slice = vm.run_slice(p, thread, budget)?;
                remaining -= slice.trace.len();
                let t = &vm.threads[&thread];
                match &t.status {
                    Status::Ready if !slice.trace.is_empty() => {}
                    Status::Sleeping { .. } | Status::Returned(_) if t.callback.is_none() => break,
                    Status::Faulted(f) => return Err(format!("loaded factory Init/Main: {f:?}")),
                    status => {
                        return Ok(Some(Request::Host {
                            thread,
                            status: status.clone(),
                        }));
                    }
                }
            }
            vm.finish_object_factory(p, child)?;
            self.phase = 1;
        }
        if self.phase == 1 {
            self.phase = 2;
            return Ok(Some(Request::Created(child)));
        }
        if self.phase == 2 {
            if let Some((parent, request)) = self.parent {
                vm.finish_creation(p, parent, request, Some(child))?;
            }
            self.phase = 3;
        }
        Ok(None)
    }
}
