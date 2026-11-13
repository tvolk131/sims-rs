//! Completed-registry reentry, distinct from C0's protected loader seam.
//!
//! The token describes an observed ordinary Main/direct-call/Sleep chain at
//! 46d51d. It supplies neither a past yield nor a future wake deadline. Only an
//! eligible native registry Tick may consume this owner. Provider selection and
//! the Program's binding to the *resident* native BHAV payloads are separate
//! admission proofs; the token's hashes alone do not establish either one.
use crate::{
    Frame, ObjectId, Program, Status, Thread, Vm, World, lifecycle::Phase,
    retained_epoch::FreshUnpublishedRequestEpoch, retained_frames::ContextBinding,
};
use serde::{Deserialize, Serialize};

const PROFILE: &str = "sims191/ordinary-main-sleep-registry-reentry-v1";
const IMAGE: &str = "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321";
// Canonical sorted/compact JSON of the helper's eleven verified CODE blobs.
const WITNESS: &str = "de6e1070f01253349cec2f33e85ec0edee265f561088c6d4e7455e4280c7b29f";
const ADVANCED: &str = "4bf5122f344554c53bde2ebb8cd2b7e3d1600ad631c385a5d7cce23c7785459a";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub address: u32,
    pub bytes: usize,
    pub raw: String,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Boundary {
    pub kind: String,
    pub tick: u64,
    pub eip: u32,
    pub simulator: u32,
    pub registry: u32,
    pub advanced: u8,
    pub advanced_observation: Observation,
    pub scheduler_cursor: u32,
    pub main_thread_tib_sha256: String,
    pub main_thread_stack_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceBinding {
    pub resource: u32,
    pub context: u32,
    pub definition: u32,
    pub guid: u32,
    pub objd: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrameShape {
    pub offset_words: usize,
    pub tree: u16,
    pub node: usize,
    pub context: u32,
    pub argc: usize,
    pub locals: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Flags {
    pub enabled: bool,
    pub run_while_paused: bool,
    pub error_service_bit: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Reentry {
    pub kind: String,
    pub last_frame: usize,
    pub argument_index: usize,
    pub argument_value: i16,
    pub registry_word: u32,
    pub countdown: i16,
    pub flags: u16,
    pub known_flags: Flags,
    pub last_yield_tick: String,
    pub wake_tick: String,
}
/// Exact registry_token schema emitted by qemu_completed_control_projection.
/// It is source evidence, not a complete native VM/import qualification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Token {
    pub schema: u32,
    pub profile: String,
    pub boundary: Boundary,
    pub source_binding: SourceBinding,
    pub object: ObjectId,
    pub native_pointer: u32,
    pub cached_main: u16,
    pub cached_init: u16,
    pub frame_shapes: Vec<FrameShape>,
    pub native_reentry: Reentry,
    pub reference_image_sha256: String,
    pub source_witness_sha256: String,
    pub raw_capture_sha256: String,
    pub adapter_epoch_source: String,
    pub construction_ownership_source: String,
    pub native_consumer_ownership_source: String,
    pub group_ownership_source: String,
}
/// Explicit replacement-adapter policy supplied alongside the native token.
/// These declarations do not infer native Init history or an absent consumer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterOwnership {
    pub fresh: FreshUnpublishedRequestEpoch,
    pub retained_construction_owners: Vec<ObjectId>,
    pub retained_native_consumers: Vec<ObjectId>,
    pub non_group_objects: Vec<ObjectId>,
    pub construction_ownership_source: String,
    pub native_consumer_ownership_source: String,
    pub group_ownership_source: String,
}
fn key(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 4096
}
fn hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
impl Token {
    fn validate(&self, adapter: &AdapterOwnership) -> Result<(), String> {
        let b = &self.boundary;
        let a = &b.advanced_observation;
        let r = &self.native_reentry;
        if self.schema != 1
            || self.profile != PROFILE
            || self.reference_image_sha256 != IMAGE
            || self.source_witness_sha256 != WITNESS
            || !hash(&self.raw_capture_sha256)
            || !hash(&b.main_thread_tib_sha256)
            || !hash(&b.main_thread_stack_sha256)
            || !matches!(b.kind.as_str(), "baseline" | "lot_update_return")
            || b.eip != 0x46d51d
            || b.tick > u64::from(u32::MAX)
            || b.simulator == 0
            || b.registry == 0
            || b.advanced != 1
            || b.scheduler_cursor != 0
            || a.address == 0
            || a.bytes != 1
            || a.raw != "01"
            || a.sha256 != ADVANCED
        {
            return Err("completed registry token source/boundary evidence differs".into());
        }
        if self.object <= 0
            || self.native_pointer == 0
            || self.source_binding.resource == 0
            || self.source_binding.context == 0
            || self.source_binding.definition == 0
            || self.cached_main == 0
            || self.frame_shapes.is_empty()
            || self.frame_shapes.len() > crate::MAX_STACK
            || r.kind != "ordinary-main-at-sleep-next-eligible-registry-tick"
            || r.last_frame != self.frame_shapes.len() - 1
            || r.argument_index >= self.frame_shapes[r.last_frame].argc
            || r.registry_word as i16 != r.countdown
            || (r.registry_word >> 16) as u16 != r.flags
            || r.known_flags.enabled != (r.flags & 1 != 0)
            || r.known_flags.run_while_paused != (r.flags & 2 != 0)
            || r.known_flags.error_service_bit != (r.flags & 4 != 0)
            || r.last_yield_tick != "not inferred"
            || r.wake_tick != "not synthesized"
        {
            return Err("completed registry token ordinary reentry evidence differs".into());
        }
        let fresh = &adapter.fresh;
        if !key(&fresh.source_key)
            || fresh.accepted_commands != 0
            || fresh.published_thread_handles != 0
            || fresh.published_request_handles != 0
            || fresh.source_key != self.adapter_epoch_source
            || !key(&adapter.construction_ownership_source)
            || !key(&adapter.native_consumer_ownership_source)
            || !key(&adapter.group_ownership_source)
            || adapter.construction_ownership_source != self.construction_ownership_source
            || adapter.native_consumer_ownership_source != self.native_consumer_ownership_source
            || adapter.group_ownership_source != self.group_ownership_source
            || adapter.retained_construction_owners.contains(&self.object)
            || adapter.retained_native_consumers.contains(&self.object)
            || !adapter.non_group_objects.contains(&self.object)
            || [
                &adapter.retained_construction_owners,
                &adapter.retained_native_consumers,
                &adapter.non_group_objects,
            ]
            .iter()
            .any(|v| v.len() > 32767 || v.iter().any(|i| *i <= 0))
        {
            return Err(
                "completed registry token needs explicit fresh non-group adapter ownership".into(),
            );
        }
        Ok(())
    }
}

/// Constructed from a validated retained arena plus explicit source/adapter
/// inputs; its fields cannot be manufactured by a generic Ready conversion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    token: Token,
    adapter: AdapterOwnership,
    binding: ContextBinding,
}
impl Owner {
    pub(crate) fn new(
        p: &Program,
        world: &World,
        tick: u64,
        token: Token,
        adapter: AdapterOwnership,
        binding: ContextBinding,
        frames: &[Frame],
    ) -> Result<Self, String> {
        let owner = Self {
            token,
            adapter,
            binding,
        };
        owner.frames(p, frames)?;
        owner.current_world(world)?;
        if tick != owner.token.boundary.tick
            || world.objects[&owner.object()].native_tree_error != Some(0)
            || world.objects[&owner.object()]
                .idle
                .as_ref()
                .unwrap()
                .registry_word()
                != owner.token.native_reentry.registry_word
            || frames.last().unwrap().args[owner.token.native_reentry.argument_index]
                != owner.token.native_reentry.argument_value
        {
            return Err("completed registry initial tick/countdown/Sleep argument differs".into());
        }
        Ok(owner)
    }
    pub fn object(&self) -> ObjectId {
        self.binding.object
    }
    pub fn token(&self) -> &Token {
        &self.token
    }
    fn frames(&self, p: &Program, frames: &[Frame]) -> Result<(), String> {
        self.token.validate(&self.adapter)?;
        let s = &self.token.source_binding;
        if self.token.object != self.binding.object
            || self.binding.registry_lifetime.is_none()
            || s.context != self.binding.native_context
            || s.guid != self.binding.definition.guid
            || s.objd != self.binding.definition.object
            || frames.len() != self.token.frame_shapes.len()
        {
            return Err("completed registry retained source/context binding differs".into());
        }
        let module = p
            .modules
            .get(&self.binding.definition.source)
            .ok_or("completed registry private module absent")?;
        let words = module
            .object_definition_words
            .get(&self.binding.definition.object)
            .ok_or("completed registry OBJD absent")?;
        let callbacks = module
            .object_functions
            .get(&self.binding.definition.object)
            .ok_or("completed registry cached Init/Main binding absent")?;
        if words.get(9) != Some(&4)
            || words.get(10) != Some(&0)
            || callbacks.first().map(|f| f.action) != Some(self.token.cached_init)
            || callbacks.get(1).map(|f| f.action) != Some(self.token.cached_main)
            || frames
                .first()
                .is_none_or(|f| f.location.bhav != self.token.cached_main)
        {
            return Err("completed registry ordinary OBJD/cached Init/Main differs".into());
        }
        let mut offset = 0;
        for (i, (f, shape)) in frames.iter().zip(&self.token.frame_shapes).enumerate() {
            if f.storage_offset != offset
                || shape.offset_words != offset
                || f.location.bhav != shape.tree
                || f.location.instruction != shape.node
                || f.args.len() != shape.argc
                || f.locals.len() != shape.locals
                || shape.context != self.binding.native_context
                || f.caller != self.object()
                || f.callee != self.object()
                || f.code_owner != self.binding.definition.source
                || f.continuation != 0
                || f.action_target.is_some()
                || f.action_icon
                || !(256..=9000).contains(&shape.tree)
                || shape.tree & 0x8000 != 0
                || shape.node > 250
            {
                return Err("completed registry ordinary frame/source shape differs".into());
            }
            let (source, routine) = p.resolve(&f.code_owner, f.location.bhav)?;
            if source != f.location.source {
                return Err("completed registry selected routine source differs".into());
            }
            let ins = routine
                .body
                .instructions
                .get(f.location.instruction)
                .ok_or("completed registry current instruction absent")?;
            if let Some(child) = frames.get(i + 1) {
                if ins.opcode < 256 || ins.opcode & 0x8000 != 0 || ins.opcode != child.location.bhav
                {
                    return Err("completed registry parent is not a pure direct BHAV call".into());
                }
            } else if ins.opcode != 0
                || usize::try_from(i16::from_le_bytes([ins.operands[0], ins.operands[1]])).ok()
                    != Some(self.token.native_reentry.argument_index)
            {
                return Err("completed registry leaf is not the observed Sleep argument".into());
            }
            offset += crate::frame_words(f.args.len(), f.locals.len());
        }
        Ok(())
    }
    fn current_world(&self, world: &World) -> Result<(), String> {
        crate::retained_frames::validate_current_binding(world, &self.binding)?;
        let object = &world.objects[&self.object()];
        if object.person.is_some()
            || object.multitile.is_some()
            || object.cursor.is_some()
            || object.native_tree_error.is_none()
            || object.native_flags.is_none_or(|v| v & 0x80 == 0)
            || object.idle.is_none()
        {
            return Err(
                "completed registry World ordinary/non-group/activation owner differs".into(),
            );
        }
        Ok(())
    }
    fn validate(&self, p: &Program, t: &Thread, depth: usize) -> Result<(), String> {
        let frames = t
            .stack
            .get(..depth)
            .ok_or("completed registry parent depth exceeds stack")?;
        self.frames(p, frames)?;
        if crate::retained_frames::registry_binding(t, depth)? != self.binding {
            return Err("completed registry retained arena binding changed".into());
        }
        Ok(())
    }
    fn shape_current(&self, world: &World, t: &Thread, depth: usize) -> Result<(), String> {
        // Runtime guards have no Program argument. Full source checks happen
        // at construction/restore; preserve every retained structural owner.
        self.token.validate(&self.adapter)?;
        let frames = t
            .stack
            .get(..depth)
            .ok_or("completed registry parent depth exceeds stack")?;
        if frames.len() != self.token.frame_shapes.len()
            || crate::retained_frames::registry_binding(t, depth)? != self.binding
            || frames.iter().zip(&self.token.frame_shapes).any(|(f, s)| {
                f.storage_offset != s.offset_words
                    || f.location.bhav != s.tree
                    || f.location.instruction != s.node
                    || f.args.len() != s.argc
                    || f.locals.len() != s.locals
            })
        {
            return Err("completed registry retained frame owner changed".into());
        }
        self.current_world(world)
    }
}

pub(crate) fn contains(t: &Thread) -> bool {
    matches!(t.status, Status::RegistrySuspended(_))
        || t.callback
            .iter()
            .flat_map(|b| b.chain())
            .any(|b| matches!(b.resume_status, Status::RegistrySuspended(_)))
        || t.service
            .iter()
            .flat_map(|s| s.chain())
            .any(|s| matches!(s.resume_status, Status::RegistrySuspended(_)))
}
/// Source/arena validation also applies to suspended callback parents. A target
/// service parent has no qualified mapping in this profile and is rejected.
pub(crate) fn validate(p: &Program, t: &Thread) -> Result<(), String> {
    if let Status::RegistrySuspended(owner) = &t.status {
        owner.validate(p, t, t.stack.len())?;
        if t.check
            || t.callback.is_some()
            || t.service.is_some()
            || t.dispatch.is_some()
            || t.autonomy.is_some()
            || !t.find_best.is_empty()
            || !t.completed_actions.is_empty()
        {
            return Err(
                "completed registry suspension overlaps an active continuation/service".into(),
            );
        }
    }
    for b in t.callback.iter().flat_map(|b| b.chain()) {
        if let Status::RegistrySuspended(owner) = &b.resume_status {
            if b.resume_check {
                return Err("completed registry callback must restore check=false".into());
            }
            owner.validate(p, t, b.base_depth)?;
        }
    }
    if t.service
        .iter()
        .flat_map(|s| s.chain())
        .any(|s| matches!(s.resume_status, Status::RegistrySuspended(_)))
    {
        return Err("completed registry target-service parent is unsupported".into());
    }
    Ok(())
}

pub(crate) fn validate_vm(p: &Program, vm: &Vm) -> Result<(), String> {
    for (&id, t) in &vm.threads {
        if !contains(t) {
            continue;
        }
        validate(p, t)?;
        for (status, depth) in std::iter::once((&t.status, t.stack.len())).chain(
            t.callback
                .iter()
                .flat_map(|b| b.chain())
                .map(|b| (&b.resume_status, b.base_depth)),
        ) {
            let Status::RegistrySuspended(owner) = status else {
                continue;
            };
            owner.shape_current(&vm.world, t, depth)?;
            let lifecycle = vm
                .lifecycles
                .get(&owner.object())
                .ok_or("completed registry Main lifecycle absent")?;
            if vm.object_threads.get(&owner.object()) != Some(&id)
                || lifecycle.thread != id
                || lifecycle.phase != Phase::Main
                || lifecycle.run_init
                || lifecycle.owner != owner.binding.definition.source
                || lifecycle.definition != owner.binding.definition.object
                || lifecycle.init != owner.token.cached_init
                || lifecycle.main != owner.token.cached_main
                || vm.tick < owner.token.boundary.tick
            {
                return Err(
                    "completed registry registered Main/lifetime/tick ownership differs".into(),
                );
            }
        }
    }
    Ok(())
}
pub(crate) fn validate_epoch(t: &Thread, source: &str, tick: u64) -> Result<(), String> {
    for status in std::iter::once(&t.status).chain(
        t.callback
            .iter()
            .flat_map(|b| b.chain())
            .map(|b| &b.resume_status),
    ) {
        if let Status::RegistrySuspended(owner) = status
            && (owner.adapter.fresh.source_key != source || owner.token.boundary.tick != tick)
        {
            return Err("completed registry adapter epoch/capture tick differs".into());
        }
    }
    Ok(())
}

impl Vm {
    pub(crate) fn validate_registry_suspension_reset(
        &self,
        object: ObjectId,
    ) -> Result<(), String> {
        let Some(t) = self
            .object_threads
            .get(&object)
            .and_then(|id| self.threads.get(id))
        else {
            return Ok(());
        };
        if !contains(t) {
            return Ok(());
        }
        let Status::RegistrySuspended(owner) = &t.status else {
            return Err("completed registry Reset retains an active callback/continuation".into());
        };
        owner.shape_current(&self.world, t, t.stack.len())?;
        if owner.object() != object
            || t.check
            || t.callback.is_some()
            || t.service.is_some()
            || t.dispatch.is_some()
            || t.autonomy.is_some()
            || !t.find_best.is_empty()
            || !t.completed_actions.is_empty()
        {
            return Err(
                "completed registry Reset retains an active callback/continuation/service".into(),
            );
        }
        Ok(())
    }
    pub(crate) fn prepare_registry_suspension_tick(&self, object: ObjectId) -> Result<(), String> {
        let Some(t) = self
            .object_threads
            .get(&object)
            .and_then(|id| self.threads.get(id))
        else {
            return Ok(());
        };
        let Status::RegistrySuspended(owner) = &t.status else {
            return Ok(());
        };
        self.validate_registry_suspension_reset(object)?;
        owner.shape_current(&self.world, t, t.stack.len())?;
        let value = &self.world.objects[&object];
        let idle = value.idle.as_ref().unwrap();
        if idle.flags & 1 == 0 || idle.countdown > 0 || value.native_tree_error != Some(0) {
            return Err(
                "completed registry suspension has not passed registry eligibility/error gate"
                    .into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
