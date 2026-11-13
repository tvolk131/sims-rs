//! Single-person branch of 0x440b2d after resource readiness/name acceptance.
//! Storage and enumeration are owned here. Physical registration, presentation,
//! score services, scheduler activation and broadcasts are synchronous host work.
use super::{ConstructionReset, ConstructionResetRequest, PostConstructRequest, PostConstruction};
use crate::{Definition, Object, ObjectId, Program, Vm, construction::ObjectDate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Publish sentinel physical links, previous room -5, and registry dirty
    /// state before post-construction can invoke any original callbacks.
    Registered(ObjectId),
    Post(PostConstructRequest),
    Reset(ConstructionResetRequest),
    /// Native object flag 0x80 and scheduler active bit, after the price store.
    Activate(ObjectId),
    /// Signal 0xe2. Observers must finish before the factory returns its ID.
    Created(ObjectId),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum Stage {
    Registered,
    Post(PostConstruction),
    Reset(ConstructionReset),
    Activate,
    Created,
    Done,
}

/// Save together with the VM and completed host effects. The caller must keep
/// other scheduled threads frozen until this synchronous factory completes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Factory {
    object: ObjectId,
    date: ObjectDate,
    time: [i16; 2],
    stage: Stage,
    pending: Option<Request>,
}

impl Factory {
    /// The supplied resource has already passed the native file/name guards.
    /// Returns None when the signed object-ID range is exhausted. No random
    /// seed is invented: default-trait services still require explicit CRT state.
    pub fn begin(
        vm: &mut Vm,
        p: &Program,
        definition: Definition,
        date: ObjectDate,
        time: [i16; 2],
    ) -> Result<Option<Self>, String> {
        let words = crate::construction::definition_words(p, &definition)?;
        if words.len() <= 65 || words[9] != 2 || words[10] != 0 || words[65] < 0 {
            return Err(
                "registered person factory requires a single-part person definition".into(),
            );
        }
        if definition.guid != u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16) {
            return Err("registered person definition GUID mismatch".into());
        }
        if vm.world.person.is_none() || vm.world.actions.is_none() || vm.world.refresh.is_none() {
            return Err("person factory needs registry, action and drawing storage".into());
        }
        let Some(id) = (1..=i16::MAX).find(|id| !vm.world.objects.contains_key(id)) else {
            return Ok(None);
        };
        if vm.object_threads.contains_key(&id)
            || vm.world.actions.as_ref().unwrap().actors.contains_key(&id)
            || vm.world.refresh.as_ref().unwrap().draw.contains_key(&id)
            || vm.world.positions.contains_key(&id)
        {
            return Err("person factory found stale storage for a vacant object ID".into());
        }
        let mut object = Object::construct_person(definition.clone(), p, 0)?;
        // Preflight before publishing any registry mutation.
        PostConstruction::for_object(&object, p, id)?;
        let registry = vm.world.person.as_ref().unwrap();
        if registry.object_order.contains(&id) || registry.person_order.contains(&id) {
            return Err("person factory found stale enumeration for a vacant object ID".into());
        }
        vm.world.register_scheduler_object(id, &mut object)?;
        let registry = vm.world.person.as_mut().unwrap();
        registry.object_order.insert(0, id);
        registry.person_order.push(id);
        if let Some(instances) = registry.resource_instances.get_mut(&definition.guid) {
            instances.push(id);
        }
        vm.world.objects.insert(id, object);
        vm.world.invalidate_registry_order();
        vm.world
            .actions
            .as_mut()
            .unwrap()
            .actors
            .insert(id, Default::default());
        vm.world
            .refresh
            .as_mut()
            .unwrap()
            .draw
            .insert(id, crate::refresh::DrawState::constructed(1));
        Ok(Some(Self {
            object: id,
            date,
            time,
            stage: Stage::Registered,
            pending: None,
        }))
    }

    pub fn object(&self) -> ObjectId {
        self.object
    }

    pub fn pending_request(&self) -> Option<Request> {
        self.pending
    }

    /// Only the factory's actual constructor child may own this service.
    pub fn autonomy_constants_request(&self) -> Option<&crate::autonomy_constants::Request> {
        if self.pending
            != Some(Request::Post(
                super::PostConstructRequest::AutonomyConstants,
            ))
        {
            return None;
        }
        match &self.stage {
            Stage::Post(post) => post.constants_request(),
            _ => None,
        }
    }
    pub fn autonomy_constants_table(&self) -> Option<&crate::autonomy_constants::Table> {
        self.autonomy_constants_request()?;
        match &self.stage {
            Stage::Post(post) => post.constants_table(),
            _ => None,
        }
    }
    pub fn reply_autonomy_constants(
        &mut self,
        reply: crate::autonomy_constants::Reply,
    ) -> Result<(), String> {
        if self.autonomy_constants_request().is_none() {
            return Err("stale person factory autonomy reply".into());
        }
        let Stage::Post(post) = &mut self.stage else {
            unreachable!()
        };
        post.reply_constants(reply)?;
        self.pending = None;
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.stage, Stage::Done)
    }

    pub fn activated(&self) -> bool {
        matches!(self.stage, Stage::Created | Stage::Done)
    }

    pub fn validate(&self, vm: &Vm, p: &Program) -> Result<(), String> {
        let object = vm
            .world
            .objects
            .get(&self.object)
            .ok_or("saved person factory object absent")?;
        let definition = object
            .definition
            .as_ref()
            .ok_or("saved person factory definition absent")?;
        let words = crate::construction::definition_words(p, definition)?;
        if object.person.is_none() || words[9] != 2 || words[10] != 0 {
            return Err("saved person factory subclass mismatch".into());
        }
        match &self.stage {
            Stage::Post(post) if post.object != self.object => {
                return Err("saved person post-constructor identity mismatch".into());
            }
            Stage::Reset(reset)
                if reset.object != self.object
                    || reset
                        .script
                        .is_some_and(|(t, _)| vm.object_threads.get(&self.object) != Some(&t)) =>
            {
                return Err("saved person reset identity mismatch".into());
            }
            Stage::Activate | Stage::Created | Stage::Done
                if !vm.object_threads.contains_key(&self.object) =>
            {
                return Err("saved person factory thread absent".into());
            }
            _ => {}
        }
        if let Stage::Post(post) = &self.stage {
            post.validate_constants(&vm.world)?;
        }
        if let Some(request) = self.pending {
            let valid = match (&self.stage, request) {
                (Stage::Registered, Request::Registered(id))
                | (Stage::Activate, Request::Activate(id))
                | (Stage::Created, Request::Created(id)) => id == self.object,
                (Stage::Post(post), Request::Post(r)) => post.pending == Some(r),
                (Stage::Reset(reset), Request::Reset(r)) => reset.pending == Some(r),
                _ => false,
            };
            if !valid {
                return Err("saved person factory pending stage mismatch".into());
            }
        }
        Ok(())
    }

    /// The second Init is an external synchronous callback on the suspended
    /// Main. Exclude exactly that binding from ordinary immediate actions.
    pub fn owns_callback(
        &self,
        vm: &Vm,
        p: &Program,
        thread: crate::ThreadId,
        depth: usize,
    ) -> bool {
        let Stage::Reset(reset) = &self.stage else {
            return false;
        };
        if reset.script != Some((thread, super::ScriptStage::SecondInit)) {
            return false;
        }
        let Some(t) = vm.threads.get(&thread) else {
            return false;
        };
        let Some(b) = t
            .callback
            .iter()
            .flat_map(|b| b.chain())
            .find(|b| b.base_depth == depth)
        else {
            return false;
        };
        let Some(f) = t.stack.get(depth) else {
            return false;
        };
        let Some(d) = vm
            .world
            .objects
            .get(&self.object)
            .and_then(|o| o.definition.as_ref())
        else {
            return false;
        };
        b.external
            && b.base_depth == depth
            && f.caller == self.object
            && f.callee == self.object
            && f.code_owner == d.source
            && p.modules
                .get(&d.source)
                .and_then(|m| m.object_functions.get(&d.object))
                .and_then(|v| v.first())
                .is_some_and(|entry| entry.action == f.location.bhav)
    }

    pub fn next(&mut self, vm: &mut Vm, p: &Program) -> Result<Option<Request>, String> {
        if let Some(request) = self.pending {
            return Ok(Some(request));
        }
        loop {
            let request = match &mut self.stage {
                Stage::Registered => Request::Registered(self.object),
                Stage::Post(post) => {
                    if let Some(request) = post.next(vm)? {
                        Request::Post(request)
                    } else {
                        // 0x431df3 is AFTER post-construction, BEFORE Reset(1).
                        vm.world
                            .objects
                            .get_mut(&self.object)
                            .ok_or("factory object absent")?
                            .data
                            .extend([
                                (45, self.date.year),
                                (46, self.date.month),
                                (47, self.date.day),
                                (24, self.time[0]),
                                (21, self.time[1]),
                            ]);
                        self.stage =
                            Stage::Reset(ConstructionReset::begin(vm, self.object, self.date)?);
                        continue;
                    }
                }
                Stage::Reset(reset) => {
                    if let Some(request) = reset.next(vm, p)? {
                        if matches!(request, ConstructionResetRequest::Script(_)) {
                            // Script completion is observed, not manually acknowledged.
                            return Ok(Some(Request::Reset(request)));
                        }
                        Request::Reset(request)
                    } else {
                        let def = vm.world.objects[&self.object]
                            .definition
                            .as_ref()
                            .ok_or("factory definition absent")?;
                        let price = crate::construction::definition_words(p, def)?[18];
                        vm.world
                            .objects
                            .get_mut(&self.object)
                            .unwrap()
                            .data
                            .insert(41, price);
                        // Do not call finish_object_factory here: the constructor's
                        // immediate person update has already finished exactly once.
                        self.stage = Stage::Activate;
                        continue;
                    }
                }
                Stage::Activate => Request::Activate(self.object),
                Stage::Created => Request::Created(self.object),
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request);
            return Ok(self.pending);
        }
    }

    pub fn complete(
        &mut self,
        vm: &mut Vm,
        p: &Program,
        request: Request,
        residential: Option<bool>,
    ) -> Result<(), String> {
        if self.pending != Some(request) {
            return Err("registered person factory acknowledgement mismatch".into());
        }
        if !matches!(
            request,
            Request::Reset(ConstructionResetRequest::Service(
                super::InitialResetRequest::IsResidential
            ))
        ) && residential.is_some()
        {
            return Err("unexpected registered person factory service result".into());
        }
        match (&mut self.stage, request) {
            (Stage::Registered, Request::Registered(_)) => {
                self.stage = Stage::Post(PostConstruction::begin(vm, p, self.object)?);
            }
            (Stage::Post(post), Request::Post(request)) => post.complete(request)?,
            (Stage::Reset(reset), Request::Reset(request)) => {
                reset.complete(vm, p, request, residential)?
            }
            (Stage::Activate, Request::Activate(id)) => {
                vm.world.objects.get_mut(&id).unwrap().activate_native()?;
                vm.world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .idle
                    .as_mut()
                    .ok_or("activated person registry absent")?
                    .set_flags(1, true);
                self.stage = Stage::Created;
            }
            (Stage::Created, Request::Created(_)) => self.stage = Stage::Done,
            _ => return Err("registered person factory stage mismatch".into()),
        }
        self.pending = None;
        Ok(())
    }
}

impl Vm {
    /// Storage effects of the base post-constructor for this supported person
    /// profile. The host additionally handles opaque graphics/cache services.
    pub fn prepare_person_base_postconstruction(
        &mut self,
        p: &Program,
        id: ObjectId,
    ) -> Result<(), String> {
        let object = self
            .world
            .objects
            .get_mut(&id)
            .ok_or("person base object absent")?;
        PostConstruction::for_object(object, p, id)?;
        let words = crate::construction::definition_words(p, object.definition.as_ref().unwrap())?;
        let count = usize::try_from(*words.get(65).ok_or("dynamic sprite count absent")?)
            .map_err(|_| "negative dynamic sprite count")?;
        object.data.extend([(3, -1), (14, -1), (29, -5), (67, 0)]);
        object.dynamic_sprite_flags = vec![0; count];
        self.world.positions.insert(
            id,
            crate::autonomy::Placement {
                x: -16,
                y: -16,
                level: 1,
            },
        );
        Ok(())
    }
}
