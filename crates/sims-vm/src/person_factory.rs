//! Native dynamic person construction and ordered continuations. `registered`
//! composes registration, post-construction and Init/Main with explicit host work.
pub mod registered;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Resource/presentation services in 0x45962b. These remain synchronous: the
/// caller applies their effects before acknowledging, and saves those effects
/// with this continuation. Base installs the sentinel/off-lot base state;
/// BindAnimator includes native profile reload before graphics rebuilding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostConstructRequest {
    Base,
    BindAnimator,
    InitialScores,
    /// Actual459844 gate after scores, before neighbor attachment.
    AutonomyConstants,
    RefreshResourceProfile,
}

/// Original post-constructor for a single-part person without resource
/// containment/headline slots. Routing slots remain bound through the definition.
/// Registration precedes this operation; ConstructionReset follows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostConstruction {
    object: crate::ObjectId,
    guid: u32,
    stage: u8,
    pending: Option<PostConstructRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    constants: Option<crate::autonomy_constants::Call>,
}
impl PostConstruction {
    pub fn begin(
        vm: &crate::Vm,
        program: &crate::Program,
        object: crate::ObjectId,
    ) -> Result<Self, String> {
        let o = vm
            .world
            .objects
            .get(&object)
            .ok_or("person post-constructor object absent")?;
        if vm.object_threads.contains_key(&object) || vm.world.person.is_none() {
            return Err("person post-constructor requires fresh registered storage".into());
        }
        Self::for_object(o, program, object)
    }

    fn for_object(
        o: &crate::Object,
        program: &crate::Program,
        object: crate::ObjectId,
    ) -> Result<Self, String> {
        let definition = o
            .definition
            .as_ref()
            .ok_or("person post-constructor definition absent")?;
        let words = crate::construction::definition_words(program, definition)?;
        let resource_slots = if definition.slots == 0 {
            None
        } else {
            Some(
                program.modules[&definition.source]
                    .slots
                    .get(&definition.slots)
                    .ok_or("person post-constructor SLOT not loaded")?,
            )
        };
        if words[9] != 2
            || words[10] != 0
            || definition.slots != words[20] as u16
            || resource_slots
                .is_some_and(|slots| slots.entries.iter().any(|s| s.kind == 0 || s.kind == 1))
            || !o.slots.is_empty()
            || o.headline.is_some()
            || o.multitile.is_some()
        {
            return Err(
                "person post-constructor requires fresh single-part storage without resource containment/headline slots"
                    .into(),
            );
        }
        complete_person(
            o.person
                .as_ref()
                .ok_or("person post-constructor storage absent")?,
        )?;
        if !o.data.contains_key(&67) {
            return Err("person post-constructor registry/slot data absent".into());
        }
        Ok(Self {
            object,
            guid: definition.guid,
            stage: 0,
            pending: None,
            constants: None,
        })
    }

    pub fn next(&mut self, vm: &mut crate::Vm) -> Result<Option<PostConstructRequest>, String> {
        use PostConstructRequest as R;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            let request = match self.stage {
                0 => R::Base,
                1 => {
                    let o = vm
                        .world
                        .objects
                        .get_mut(&self.object)
                        .ok_or("post-constructor object absent")?;
                    if !o.slots.is_empty() {
                        return Err("unexpected person base resource slots".into());
                    }
                    o.slots
                        .resize(crate::person::BUILTIN_SLOT_HEIGHTS.len(), None);
                    o.native_slot_geometry = Some(crate::person::builtin_slot_geometry());
                    o.data.insert(67, o.slots.len() as i16);
                    self.stage = 2;
                    continue;
                }
                2 => R::BindAnimator,
                3 => {
                    let o = vm
                        .world
                        .objects
                        .get_mut(&self.object)
                        .ok_or("post-constructor object absent")?;
                    if o.headline.is_some() {
                        return Err("unexpected person base headline slot".into());
                    }
                    // +0x148 is the headline vector, NOT routing (+0x13c).
                    // Native leaves inactive animation counters uninitialized;
                    // normalize them until a headline actually initializes them.
                    o.headline = Some(Default::default());
                    self.stage = 4;
                    continue;
                }
                4 => R::InitialScores,
                // Preserve historical stage numbers; new initial-score calls
                // enter9 before existing neighbor-attachment stage5.
                9 => {
                    if self.constants.is_none() {
                        self.constants = Some(
                            vm.world
                                .autonomy_constants
                                .as_ref()
                                .ok_or("autonomy constants process owner is unknown")?
                                .begin(),
                        );
                    }
                    match crate::autonomy_constants::advance(
                        &mut vm.world,
                        self.constants.as_mut().unwrap(),
                    )? {
                        crate::autonomy_constants::Progress::Store(_) => continue,
                        crate::autonomy_constants::Progress::Complete => {
                            self.constants = None;
                            self.stage = 5;
                            continue;
                        }
                        crate::autonomy_constants::Progress::Service(_) => R::AutonomyConstants,
                    }
                }
                5 => {
                    let person = vm
                        .world
                        .objects
                        .get_mut(&self.object)
                        .and_then(|o| o.person.as_mut())
                        .ok_or("post-constructor person absent")?;
                    vm.world
                        .person
                        .as_mut()
                        .ok_or("post-constructor neighborhood absent")?
                        .attach_person_neighbor(
                            person,
                            self.guid,
                            NeighborAttachment::Persistent,
                        )?;
                    self.stage = 6;
                    continue;
                }
                6 => R::RefreshResourceProfile,
                7 => {
                    let person = vm
                        .world
                        .objects
                        .get(&self.object)
                        .and_then(|o| o.person.as_ref())
                        .ok_or("post-constructor person absent")?;
                    complete_person(person)?;
                    if low_interests(person) {
                        vm.randomize_person_traits(self.object)?;
                    }
                    self.stage = 8;
                    continue;
                }
                8 => return Ok(None),
                _ => return Err("invalid person post-constructor stage".into()),
            };
            self.pending = Some(request);
            return Ok(self.pending);
        }
    }

    pub fn complete(&mut self, request: PostConstructRequest) -> Result<(), String> {
        if self.pending != Some(request) {
            return Err("person post-constructor acknowledgement does not match request".into());
        }
        if request == PostConstructRequest::AutonomyConstants {
            return Err("autonomy constants require a typed resource reply".into());
        }
        self.pending = None;
        self.stage = if self.stage == 4 { 9 } else { self.stage + 1 };
        Ok(())
    }
    fn constants_request(&self) -> Option<&crate::autonomy_constants::Request> {
        (self.stage == 9 && self.pending == Some(PostConstructRequest::AutonomyConstants))
            .then(|| self.constants.as_ref()?.pending())
            .flatten()
    }
    fn constants_table(&self) -> Option<&crate::autonomy_constants::Table> {
        self.constants_request()?;
        self.constants.as_ref()?.table()
    }
    fn reply_constants(&mut self, reply: crate::autonomy_constants::Reply) -> Result<(), String> {
        if self.constants_request().is_none() {
            return Err("stale constructor autonomy resource reply".into());
        }
        self.constants.as_mut().unwrap().reply(reply)?;
        self.pending = None;
        Ok(())
    }
    fn validate_constants(&self, world: &crate::World) -> Result<(), String> {
        if self.stage > 9
            || self.stage != 9 && self.constants.is_some()
            || self.stage == 9
                && self
                    .pending
                    .is_some_and(|request| request != PostConstructRequest::AutonomyConstants)
        {
            return Err("invalid constructor autonomy child stage".into());
        }
        if let Some(call) = &self.constants {
            call.validate()?;
            let owner = world
                .autonomy_constants
                .as_ref()
                .ok_or("constructor autonomy child lost process owner")?;
            if !call.belongs_to(owner) {
                return Err("constructor autonomy child configuration differs".into());
            }
            if (self.pending == Some(PostConstructRequest::AutonomyConstants))
                != call.pending().is_some()
            {
                return Err("constructor autonomy pending service differs".into());
            }
        } else if self.pending == Some(PostConstructRequest::AutonomyConstants) {
            return Err("constructor autonomy service lost child".into());
        }
        Ok(())
    }
}

/// A host boundary in the composed 0x4598e8 reset. `Script` runs the indicated
/// thread until the coordinator chooses the next boundary; it needs no manual
/// acknowledgement. Other requests must be completed before calling `complete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstructionResetRequest {
    Service(InitialResetRequest),
    Script(crate::ThreadId),
    /// The base reset calls virtual person Update with argument zero, even if
    /// the lot's clock is already running. This includes its motive-update tail.
    ImmediatePersonUpdate {
        object: crate::ObjectId,
        tick: i32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ScriptStage {
    FirstInit,
    Main,
    SecondInit,
}

/// Compose Main installation, both real Init calls and the initial person update
/// with the 0x4598e8 field/service wrapper. Registration and post-construction
/// precede this operation. Presentation, zoning and neighborhood services are
/// explicit acknowledgements; their completed effects must be saved with the VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstructionReset {
    object: crate::ObjectId,
    now: crate::construction::ObjectDate,
    reset: InitialReset,
    script: Option<(crate::ThreadId, ScriptStage)>,
    pending: Option<ConstructionResetRequest>,
}

impl ConstructionReset {
    pub fn begin(
        vm: &mut crate::Vm,
        object: crate::ObjectId,
        now: crate::construction::ObjectDate,
    ) -> Result<Self, String> {
        if vm.object_threads.contains_key(&object) {
            return Err("person construction reset requires a fresh object".into());
        }
        let person = vm
            .world
            .objects
            .get_mut(&object)
            .and_then(|o| o.person.as_mut())
            .ok_or("person construction reset storage missing")?;
        Ok(Self {
            object,
            now,
            reset: InitialReset::begin(person, true)?,
            script: None,
            pending: None,
        })
    }

    pub fn next(
        &mut self,
        vm: &mut crate::Vm,
        program: &crate::Program,
    ) -> Result<Option<ConstructionResetRequest>, String> {
        use ConstructionResetRequest as R;
        if let Some(request) = self.pending {
            return Ok(Some(request));
        }
        loop {
            if let Some((thread, stage)) = self.script {
                let t = vm
                    .threads
                    .get(&thread)
                    .ok_or("person constructor thread missing")?;
                if let crate::Status::Faulted(fault) = &t.status {
                    return Err(format!(
                        "person constructor script failed: {}",
                        fault.message
                    ));
                }
                match stage {
                    ScriptStage::FirstInit | ScriptStage::SecondInit if t.callback.is_some() => {
                        return Ok(Some(R::Script(thread)));
                    }
                    ScriptStage::FirstInit => {
                        self.script = Some((thread, ScriptStage::Main));
                    }
                    ScriptStage::Main => {
                        if t.callback.is_some()
                            || !matches!(
                                t.status,
                                crate::Status::Sleeping { .. } | crate::Status::Returned(_)
                            )
                        {
                            return Ok(Some(R::Script(thread)));
                        }
                        self.pending = Some(R::ImmediatePersonUpdate {
                            object: self.object,
                            tick: 0,
                        });
                        return Ok(self.pending);
                    }
                    ScriptStage::SecondInit => {
                        self.acknowledge(vm, InitialResetRequest::Init, None)?;
                        self.script = None;
                    }
                }
                continue;
            }
            match self.reset.request() {
                Some(InitialResetRequest::BaseReset { update: true }) => {
                    let thread = vm.begin_person_base_reset(program, self.object, self.now)?;
                    self.script = Some((thread, ScriptStage::FirstInit));
                }
                Some(InitialResetRequest::Init) => {
                    if let Some(thread) = vm.begin_loaded_function(program, self.object, 0)? {
                        self.script = Some((thread, ScriptStage::SecondInit));
                    } else {
                        self.acknowledge(vm, InitialResetRequest::Init, None)?;
                    }
                }
                Some(service) => {
                    self.pending = Some(R::Service(service));
                    return Ok(self.pending);
                }
                None => return Ok(None),
            }
        }
    }

    fn acknowledge(
        &mut self,
        vm: &mut crate::Vm,
        service: InitialResetRequest,
        residential: Option<bool>,
    ) -> Result<(), String> {
        let person = vm
            .world
            .objects
            .get_mut(&self.object)
            .and_then(|o| o.person.as_mut())
            .ok_or("person constructor storage missing")?;
        self.reset.complete(person, service, residential)
    }

    pub fn complete(
        &mut self,
        vm: &mut crate::Vm,
        program: &crate::Program,
        request: ConstructionResetRequest,
        residential: Option<bool>,
    ) -> Result<(), String> {
        if self.pending != Some(request) {
            return Err("person construction reset acknowledgement does not match request".into());
        }
        match request {
            ConstructionResetRequest::Service(service) => {
                self.acknowledge(vm, service, residential)?
            }
            ConstructionResetRequest::ImmediatePersonUpdate { .. } => {
                vm.refresh_registry_flags(program, self.object)?;
                self.acknowledge(
                    vm,
                    InitialResetRequest::BaseReset { update: true },
                    residential,
                )?;
                self.script = None;
            }
            ConstructionResetRequest::Script(_) => {
                return Err("script completion is observed from its thread".into());
            }
        }
        self.pending = None;
        Ok(())
    }
}

/// 0x4258a3 merges the versioned persistent data and family; 0x425930 merges
/// only the revision-7 trait fields. Both set field 31, including on creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NeighborAttachment {
    Persistent,
    Traits,
}

/// Services enclosing the native constructor reset. Complete each service
/// synchronously before acknowledging it. BaseReset includes its own Init and
/// (when update=true) immediate virtual update; Init is the *second* callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InitialResetRequest {
    ReloadAppearance,
    AttachNeighbor(NeighborAttachment),
    BaseReset { update: bool },
    Init,
    IsResidential,
    RandomizeTraits,
    ClearAnimator,
    LoadScores,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ResetStage {
    Profile,
    AttachBefore,
    Base,
    Init,
    AttachAfter,
    Zoning,
    Randomize,
    Traits,
    Clear,
    Scores,
    Done,
}

/// Ordered 0x4598e8 wrapper, distinct from loaded-person Reset(0) at 0x45e791.
/// This continuation does not certify that the host services have run; a native
/// factory must drive them and retain its full VM/RNG state alongside this token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialReset {
    update: bool,
    initially_low: bool,
    stage: ResetStage,
}

fn complete_person(person: &BTreeMap<i16, i16>) -> Result<(), String> {
    if person.len() != 256 || (0..256).any(|field| !person.contains_key(&field)) {
        return Err("person construction requires complete person storage".into());
    }
    Ok(())
}

fn low_interests(person: &BTreeMap<i16, i16>) -> bool {
    // Signed comparisons of only the first eight interests, even though the
    // generator also writes 54/55. This is not an average or a zero-only test.
    (46..54).all(|field| person[&field] < 11)
}

impl crate::Vm {
    /// Default traits use a separately supplied process stream. Missing seed or
    /// incomplete storage must stop before changing any fields or random state.
    pub fn randomize_person_traits(&mut self, object: crate::ObjectId) -> Result<(), String> {
        let state = self
            .crt_rng_state
            .as_mut()
            .ok_or("person factory CRT random state not supplied")?;
        let person = self
            .world
            .objects
            .get_mut(&object)
            .and_then(|o| o.person.as_mut())
            .ok_or("default traits require a person")?;
        default_traits(person, state)
    }
}

impl InitialReset {
    pub fn begin(person: &mut BTreeMap<i16, i16>, update: bool) -> Result<Self, String> {
        complete_person(person)?;
        let retained = [32, 34, 35].map(|field| (field, person[&field]));
        person.values_mut().for_each(|v| *v = 0);
        person.extend(retained);
        Ok(Self {
            update,
            initially_low: false,
            stage: ResetStage::Profile,
        })
    }

    pub fn request(&self) -> Option<InitialResetRequest> {
        use InitialResetRequest as R;
        Some(match self.stage {
            ResetStage::Profile => R::ReloadAppearance,
            ResetStage::AttachBefore | ResetStage::AttachAfter => {
                R::AttachNeighbor(NeighborAttachment::Persistent)
            }
            ResetStage::Base => R::BaseReset {
                update: self.update,
            },
            ResetStage::Init => R::Init,
            ResetStage::Zoning => R::IsResidential,
            ResetStage::Randomize => R::RandomizeTraits,
            ResetStage::Traits => R::AttachNeighbor(NeighborAttachment::Traits),
            ResetStage::Clear => R::ClearAnimator,
            ResetStage::Scores => R::LoadScores,
            ResetStage::Done => return None,
        })
    }

    /// The host passes a boolean only for IsResidential. The acknowledgement
    /// advances one boundary; it cannot skip or repeat a pending service.
    pub fn complete(
        &mut self,
        person: &mut BTreeMap<i16, i16>,
        completed: InitialResetRequest,
        residential: Option<bool>,
    ) -> Result<(), String> {
        complete_person(person)?;
        if self.request() != Some(completed) {
            return Err("person reset acknowledgement does not match pending request".into());
        }
        if (self.stage == ResetStage::Zoning) != residential.is_some() {
            return Err("person reset service result does not match pending request".into());
        }
        self.stage = match self.stage {
            ResetStage::Profile => {
                self.initially_low = low_interests(person);
                ResetStage::AttachBefore
            }
            ResetStage::AttachBefore => ResetStage::Base,
            ResetStage::Base => ResetStage::Init,
            ResetStage::Init => {
                if person[&25] == 0x7ae {
                    person.insert(25, 0);
                    ResetStage::Zoning
                } else {
                    ResetStage::AttachAfter
                }
            }
            ResetStage::AttachAfter => ResetStage::Zoning,
            ResetStage::Zoning => {
                if !residential.unwrap() && person[&32] != 2 {
                    person.insert(32, 1);
                }
                if !self.initially_low {
                    ResetStage::Traits
                } else if low_interests(person) {
                    ResetStage::Randomize
                } else {
                    ResetStage::Clear
                }
            }
            ResetStage::Randomize | ResetStage::Traits => ResetStage::Clear,
            ResetStage::Clear => ResetStage::Scores,
            ResetStage::Scores => ResetStage::Done,
            ResetStage::Done => return Err("person initial reset already complete".into()),
        };
        Ok(())
    }
}

impl crate::person::WorldState {
    pub fn validate_neighbor_slots(&self) -> Result<(), String> {
        let Some(slots) = &self.neighbor_slots else {
            return Ok(());
        };
        let ids: std::collections::BTreeSet<_> = slots.iter().flatten().copied().collect();
        if slots.len() > 32767
            || ids.len() != slots.iter().flatten().count()
            || ids.len() != self.neighbors.len()
            || ids
                .iter()
                .any(|id| *id <= 0 || !self.neighbors.contains_key(id))
        {
            return Err("invalid native neighborhood slot vector".into());
        }
        Ok(())
    }

    /// 0x4257a5. A null resource returns null without touching the registry.
    /// Allocation failure and IDs outside the signed native ID profile are not
    /// modeled. Existing resource identity is represented by its bound GUID.
    pub fn create_neighbor(
        &mut self,
        guid: Option<u32>,
        requested: i16,
    ) -> Result<Option<i16>, String> {
        let Some(guid) = guid else { return Ok(None) };
        self.validate_neighbor_slots()?;
        let mut slots = self
            .neighbor_slots
            .as_ref()
            .ok_or("native neighborhood slots not supplied")?
            .clone();
        let requested = usize::try_from(requested).ok().filter(|&id| id > 0);
        if let Some(id) = requested {
            slots.resize(slots.len().max(id), None);
        }
        let slot = requested
            .map(|id| id - 1)
            .filter(|&i| slots[i].is_none())
            .or_else(|| slots.iter().rposition(Option::is_none))
            .unwrap_or(slots.len());
        let id: i16 = (slot + 1)
            .try_into()
            .map_err(|_| "neighbor ID outside signed native profile")?;
        if self.neighbors.contains_key(&id) {
            return Err("neighbor slot/index identity collision".into());
        }
        if slot == slots.len() {
            slots.push(None);
        }
        slots[slot] = Some(id);
        self.neighbors.insert(
            id,
            crate::person::Neighbor {
                guid,
                person: (0..256).map(|i| (i, 0)).collect(),
                ..Default::default()
            },
        );
        self.neighbor_slots = Some(slots);
        Ok(Some(id))
    }

    pub fn attach_person_neighbor(
        &mut self,
        person: &mut BTreeMap<i16, i16>,
        guid: u32,
        mode: NeighborAttachment,
    ) -> Result<i16, String> {
        if person.len() != 256 || (0..256).any(|field| !person.contains_key(&field)) {
            return Err("neighbor attachment requires complete person storage".into());
        }
        self.validate_neighbor_slots()?;
        let slots = self
            .neighbor_slots
            .as_ref()
            .ok_or("native neighborhood slots not supplied")?;
        let existing = slots
            .iter()
            .flatten()
            .copied()
            .find(|id| self.neighbors[id].guid == guid);
        if let Some(id) = existing {
            let neighbor = &self.neighbors[&id];
            let mut updated = person.clone();
            match mode {
                NeighborAttachment::Persistent => crate::loading::attach_neighbor(
                    &mut updated,
                    &neighbor.person,
                    id,
                    neighbor.person_revision,
                )?,
                NeighborAttachment::Traits => {
                    updated.insert(31, id);
                    if neighbor.person_revision >= 7 {
                        for field in [46, 47, 48, 49, 50, 51, 52, 53, 13, 14, 16, 20, 26, 54, 55] {
                            updated.insert(
                                field,
                                *neighbor
                                    .person
                                    .get(&field)
                                    .ok_or("missing neighbor trait field")?,
                            );
                        }
                    }
                }
            }
            *person = updated;
            Ok(id)
        } else {
            let id = self.create_neighbor(Some(guid), -1)?.unwrap();
            // A fresh record does not merge its zero fields or family.
            person.insert(31, id);
            Ok(id)
        }
    }
}

/// 0x459a9c: distribute ten interest values and five personality values across
/// low/middle/high categories. Exhausted categories consume retries from the
/// separate CRT stream; no draw is taken from SimAntics' lot RNG.
pub fn default_traits(person: &mut BTreeMap<i16, i16>, crt_state: &mut u32) -> Result<(), String> {
    complete_person(person)?;
    for (fields, mut counts) in [
        (vec![46, 47, 48, 49, 50, 51, 52, 53, 54, 55], [4, 3, 3]),
        (vec![13, 14, 16, 20, 26], [1, 2, 2]),
    ] {
        for field in fields {
            let category = loop {
                let candidate = (crate::rng::next_crt(crt_state) % 3) as usize;
                if counts[candidate] > 0 {
                    counts[candidate] -= 1;
                    break candidate;
                }
            };
            let draw = crate::rng::next_crt(crt_state);
            let value = match category {
                0 => draw % 4,
                1 => draw % 3 + 4,
                2 => draw % 4 + 7,
                _ => unreachable!(),
            };
            person.insert(field, (value * 100) as i16);
        }
    }
    Ok(())
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[path = "person_factory/constants_tests.rs"]
mod constants_tests;
