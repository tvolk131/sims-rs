//! Typed completed-registry reentry. No constructor, Init, prior host request,
//! last-yield timestamp or future wake deadline is inferred by this importer.
//! Source/absence keys declare external evidence; validating their capture
//! provenance and the complete Character/Session owners remains the host's job.
use crate::{
    Definition, Frame, ObjectId, Program, Status, Thread, Vm, World,
    lifecycle::Phase,
    registry_suspension::{Boundary, FrameShape, Observation, SourceBinding},
    retained_epoch::FreshUnpublishedRequestEpoch,
    retained_frames::ContextBinding,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const IMAGE: &str = "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321";
const WITNESS: &str = "de6e1070f01253349cec2f33e85ec0edee265f561088c6d4e7455e4280c7b29f";
const BASE_TICK: &str = "03ef81ca58417e4d98f6eeda9d4a6df8c8950f4c12369eb749586bf09a70e49f";
const PERSON_PREFIX: &str = "eb6cfd6140eaeb687346da267917f2b7012320752e19c3a003b4aa6286466285";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TickDispatch {
    /// Actual object first DWORD, binding the vtable to native_pointer.
    pub object_vtable: Observation,
    /// Actual vtable+0x14 DWORD, not an inferred definition-class dispatch.
    pub slot: Observation,
    /// Full81-byte base Tick, or23-byte person delegation prefix.
    pub code: Observation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub enum Leaf {
    BaseTickSleep {
        argument_index: usize,
        argument_value: i16,
    },
    PersonAnimatePolling {
        operands: [u8; 8],
        /// Actual person+0xadc DWORD and its current0x2a0-byte animator.
        animator_pointer: Observation,
        animator_header: Observation,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub schema: u32,
    pub boundary: Boundary,
    pub object: ObjectId,
    pub native_pointer: u32,
    pub source_binding: SourceBinding,
    pub cached_init: u16,
    pub cached_main: u16,
    pub frame_shapes: Vec<FrameShape>,
    pub registry_word: u32,
    pub tick_dispatch: TickDispatch,
    pub leaf: Leaf,
    pub reference_image_sha256: String,
    pub source_witness_sha256: String,
    pub raw_capture_sha256: String,
    pub adapter_epoch_source: String,
}

/// Explicit absence of retained SOURCE construction/reset continuations or
/// retained SOURCE result consumers, supplied from a separately qualified
/// native closure. Empty new-adapter queues do not establish either absence.
/// No Default: a passive Main stack alone cannot manufacture these declarations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Absence {
    pub objects: Vec<ObjectId>,
    pub source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GroupMember {
    pub object: ObjectId,
    pub registry_lifetime: u32,
    pub definition: Definition,
    pub part: crate::multitile::Part,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub enum Topology {
    NonGroup {
        source: String,
    },
    /// Ordered actual head/next chain, not a reconstructed construction plan.
    LiveGroup {
        source: String,
        members: Vec<GroupMember>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterOwnership {
    pub fresh: FreshUnpublishedRequestEpoch,
    pub construction_absent: Absence,
    pub native_consumer_absent: Absence,
    pub topology: Topology,
}

fn key(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 4096
}
fn hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn bytes(o: &Observation, expected: usize) -> Result<Vec<u8>, String> {
    if o.address < 0x10000
        || o.address.checked_add(expected as u32).is_none()
        || o.bytes != expected
        || expected > 4096
        || o.raw.len() != expected * 2
        || !hash(&o.sha256)
    {
        return Err("native registry observation extent/address/hash differs".into());
    }
    let raw: Vec<u8> = o
        .raw
        .as_bytes()
        .chunks_exact(2)
        .map(|p| {
            std::str::from_utf8(p)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .ok_or_else(|| "native registry observation hex differs".to_string())
        })
        .collect::<Result<_, _>>()?;
    if format!("{:x}", Sha256::digest(&raw)) != o.sha256 {
        return Err("native registry observation payload hash differs".into());
    }
    Ok(raw)
}
fn dword(raw: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap())
}
fn closure(a: &Absence, objects: &[ObjectId]) -> bool {
    key(&a.source)
        && a.objects.len() <= 32767
        && a.objects.iter().all(|i| *i > 0)
        && a.objects.iter().copied().collect::<BTreeSet<_>>().len() == a.objects.len()
        && objects.iter().all(|id| a.objects.contains(id))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    evidence: Evidence,
    adapter: AdapterOwnership,
    binding: ContextBinding,
}
impl Owner {
    pub fn object(&self) -> ObjectId {
        self.binding.object
    }
    pub fn evidence(&self) -> &Evidence {
        &self.evidence
    }
    /// Adapter permission, not a claim that native Init ever ran. Already-live
    /// groups must not acquire begin_multitile_init's fresh-Reset permission.
    pub fn closed_group_init_permission(&self) -> bool {
        matches!(self.adapter.topology, Topology::LiveGroup { .. })
    }
    pub(crate) fn new(
        p: &Program,
        world: &World,
        tick: u64,
        evidence: Evidence,
        adapter: AdapterOwnership,
        binding: ContextBinding,
        frames: &[Frame],
    ) -> Result<Self, String> {
        let owner = Self {
            evidence,
            adapter,
            binding,
        };
        owner.frames(p, frames)?;
        owner.world(world, true)?;
        let e = &owner.evidence;
        if tick != e.boundary.tick
            || world.objects[&owner.object()].native_tree_error != Some(0)
            || world.objects[&owner.object()]
                .idle
                .as_ref()
                .unwrap()
                .registry_word()
                != e.registry_word
            || matches!(e.leaf, Leaf::BaseTickSleep { argument_index, argument_value }
                if frames.last().unwrap().args[argument_index] != argument_value)
        {
            return Err("native registry initial tick/word/leaf argument differs".into());
        }
        Ok(owner)
    }
    fn validate_evidence(&self) -> Result<(), String> {
        let e = &self.evidence;
        let b = &e.boundary;
        let a = &self.adapter.fresh;
        if e.schema != 1
            || e.reference_image_sha256 != IMAGE
            || e.source_witness_sha256 != WITNESS
            || !hash(&e.raw_capture_sha256)
            || !hash(&b.main_thread_tib_sha256)
            || !hash(&b.main_thread_stack_sha256)
            || !matches!(b.kind.as_str(), "baseline" | "lot_update_return")
            || b.eip != 0x46d51d
            || b.tick > u64::from(u32::MAX)
            || b.simulator == 0
            || b.registry == 0
            || b.advanced != 1
            || b.scheduler_cursor != 0
            || bytes(&b.advanced_observation, 1)? != [1]
            || e.object <= 0
            || e.native_pointer < 0x10000
            || e.source_binding.resource == 0
            || e.source_binding.context == 0
            || e.source_binding.definition == 0
            || e.cached_main == 0
            || e.frame_shapes.is_empty()
            || e.frame_shapes.len() > crate::MAX_STACK
            || !key(&a.source_key)
            || a.source_key != e.adapter_epoch_source
            || a.accepted_commands != 0
            || a.published_thread_handles != 0
            || a.published_request_handles != 0
        {
            return Err("native registry source/boundary/adapter evidence differs".into());
        }
        let dispatch = &e.tick_dispatch;
        let vt = dword(&bytes(&dispatch.object_vtable, 4)?, 0);
        let target = dword(&bytes(&dispatch.slot, 4)?, 0);
        if dispatch.object_vtable.address != e.native_pointer
            || vt.checked_add(0x14) != Some(dispatch.slot.address)
        {
            return Err("native registry actual Tick vtable/slot binding differs".into());
        }
        match &e.leaf {
            Leaf::BaseTickSleep { .. } => {
                if target != 0x4521f7
                    || dispatch.code.address != target
                    || bytes(&dispatch.code, 81)?.len() != 81
                    || dispatch.code.sha256 != BASE_TICK
                {
                    return Err("native registry base Tick witness differs".into());
                }
            }
            Leaf::PersonAnimatePolling {
                operands,
                animator_pointer,
                animator_header,
            } => {
                let animator = dword(&bytes(animator_pointer, 4)?, 0);
                let header = bytes(animator_header, 0x2a0)?;
                if target != 0x45cc8c
                    || dispatch.code.address != target
                    || bytes(&dispatch.code, 23)?.len() != 23
                    || dispatch.code.sha256 != PERSON_PREFIX
                    || e.native_pointer.checked_add(0xadc) != Some(animator_pointer.address)
                    || animator < 0x10000
                    || animator_header.address != animator
                    || dword(&header, 0) != 0x671ee4
                    || dword(&header, 0x138) != e.native_pointer
                    || dword(&header, 0xa0) != 1
                    || dword(&header, 0xc) == 0
                    || i16::from_le_bytes([operands[0], operands[1]]) == 0
                    || operands[5] & 4 != 0
                {
                    return Err("native registry engaged person polling witness differs".into());
                }
            }
        }
        let objects = match &self.adapter.topology {
            Topology::NonGroup { source } => {
                if !key(source) {
                    return Err("native registry topology source absent".into());
                }
                vec![self.object()]
            }
            Topology::LiveGroup { source, members } => {
                if !key(source)
                    || members.is_empty()
                    || members.len() > 32767
                    || members
                        .iter()
                        .any(|m| m.object <= 0 || m.registry_lifetime == 0)
                    || members
                        .iter()
                        .map(|m| m.object)
                        .collect::<BTreeSet<_>>()
                        .len()
                        != members.len()
                    || !members.iter().any(|m| m.object == self.object())
                {
                    return Err("native registry live-group topology declaration differs".into());
                }
                let captured = members.iter().map(|m| (m.object, m.part.clone())).collect();
                if crate::multitile::retained_chain(&captured, self.object())?
                    != members.iter().map(|m| m.object).collect::<Vec<_>>()
                {
                    return Err("native registry captured group chain differs".into());
                }
                members.iter().map(|m| m.object).collect()
            }
        };
        if !closure(&self.adapter.construction_absent, &objects)
            || !closure(&self.adapter.native_consumer_absent, &objects)
        {
            return Err(
                "native registry needs explicit construction/native-consumer absence closure"
                    .into(),
            );
        }
        Ok(())
    }
    fn frames(&self, p: &Program, frames: &[Frame]) -> Result<(), String> {
        self.validate_evidence()?;
        let e = &self.evidence;
        let s = &e.source_binding;
        if e.object != self.object()
            || self.binding.registry_lifetime.is_none()
            || s.context != self.binding.native_context
            || s.guid != self.binding.definition.guid
            || s.objd != self.binding.definition.object
            || frames.len() != e.frame_shapes.len()
        {
            return Err("native registry retained context/definition binding differs".into());
        }
        let module = p
            .modules
            .get(&self.binding.definition.source)
            .ok_or("native registry module absent")?;
        let words = module
            .object_definition_words
            .get(&self.binding.definition.object)
            .ok_or("native registry definition absent")?;
        let person = matches!(e.leaf, Leaf::PersonAnimatePolling { .. });
        if words.get(9).is_none_or(|kind| (*kind == 2) != person)
            || words
                .get(10)
                .is_none_or(|group| (*group != 0) != self.closed_group_init_permission())
        {
            return Err("native registry definition/profile/topology differs".into());
        }
        let callbacks = module
            .object_functions
            .get(&self.binding.definition.object)
            .ok_or("native registry cached Init/Main absent")?;
        if callbacks.first().map(|f| f.action) != Some(e.cached_init)
            || callbacks.get(1).map(|f| f.action) != Some(e.cached_main)
            || frames
                .first()
                .is_none_or(|f| f.location.bhav != e.cached_main)
        {
            return Err("native registry cached Init/Main differs".into());
        }
        let mut offset = 0;
        for (i, (frame, shape)) in frames.iter().zip(&e.frame_shapes).enumerate() {
            if frame.storage_offset != offset
                || shape.offset_words != offset
                || frame.location.bhav != shape.tree
                || frame.location.instruction != shape.node
                || frame.args.len() != shape.argc
                || frame.locals.len() != shape.locals
                || shape.context != self.binding.native_context
                || frame.caller != self.object()
                || frame.callee != self.object()
                || frame.code_owner != self.binding.definition.source
                || frame.continuation != 0
                || frame.action_target.is_some()
                || frame.action_icon
                || !(256..=9000).contains(&shape.tree)
                || shape.node > 250
            {
                return Err("native registry Main/direct-call frame shape differs".into());
            }
            let (source, routine) = p.resolve(&frame.code_owner, frame.location.bhav)?;
            let ins = routine
                .body
                .instructions
                .get(frame.location.instruction)
                .ok_or("native registry current instruction absent")?;
            if source != frame.location.source {
                return Err("native registry selected source differs".into());
            }
            if let Some(child) = frames.get(i + 1) {
                if ins.opcode < 256 || ins.opcode & 0x8000 != 0 || ins.opcode != child.location.bhav
                {
                    return Err("native registry parent is not a pure direct call".into());
                }
            } else {
                match &e.leaf {
                    Leaf::BaseTickSleep { argument_index, .. } => {
                        if ins.opcode != 0
                            || *argument_index >= frame.args.len()
                            || usize::try_from(i16::from_le_bytes([
                                ins.operands[0],
                                ins.operands[1],
                            ]))
                            .ok()
                                != Some(*argument_index)
                        {
                            return Err("native registry Sleep leaf differs".into());
                        }
                    }
                    Leaf::PersonAnimatePolling { operands, .. } => {
                        if ins.opcode != 44 || ins.operands != *operands {
                            return Err("native registry Animate polling leaf differs".into());
                        }
                    }
                }
            }
            offset += crate::frame_words(frame.args.len(), frame.locals.len());
        }
        Ok(())
    }
    fn world(&self, world: &World, initial: bool) -> Result<(), String> {
        crate::retained_frames::validate_current_binding(world, &self.binding)?;
        let object = &world.objects[&self.object()];
        let person = matches!(self.evidence.leaf, Leaf::PersonAnimatePolling { .. });
        if object.person.is_some() != person
            || object.cursor.is_some()
            || object.native_tree_error.is_none()
            || object.native_flags.is_none_or(|v| v & 0x80 == 0)
            || object.idle.is_none()
        {
            return Err("native registry current object/profile/activation differs".into());
        }
        let storage = world
            .scheduler_storage
            .as_ref()
            .ok_or("native registry storage absent")?;
        match &self.adapter.topology {
            Topology::NonGroup { .. } => {
                if object.multitile.is_some() {
                    return Err("native registry non-group declaration differs".into());
                }
            }
            Topology::LiveGroup { members, .. } => {
                if person
                    || crate::multitile::chain(world, self.object())?
                        != members.iter().map(|m| m.object).collect::<Vec<_>>()
                {
                    return Err("native registry group topology membership differs".into());
                }
                for member in members {
                    let value = world
                        .objects
                        .get(&member.object)
                        .ok_or("native registry group member absent")?;
                    if storage.handle(member.object) != Some(member.registry_lifetime)
                        || value.definition.as_ref() != Some(&member.definition)
                        || value.person.is_some()
                        || value.cursor.is_some()
                        || value.native_flags.is_none_or(|v| v & 0x80 == 0)
                        || value.multitile.is_none()
                        || initial
                            && (value.multitile.as_ref() != Some(&member.part)
                                || value.idle.as_ref().is_none_or(|i| i.flags & 1 == 0))
                    {
                        return Err("native registry retained group lifetime/part differs".into());
                    }
                }
            }
        }
        Ok(())
    }
    fn validate(&self, p: &Program, world: &World, t: &Thread, depth: usize) -> Result<(), String> {
        self.frames(
            p,
            t.stack
                .get(..depth)
                .ok_or("native registry parent depth exceeds stack")?,
        )?;
        self.shape(world, t, depth)
    }
    fn shape(&self, world: &World, t: &Thread, depth: usize) -> Result<(), String> {
        self.validate_evidence()?;
        let frames = t
            .stack
            .get(..depth)
            .ok_or("native registry parent depth exceeds stack")?;
        if crate::retained_frames::registry_binding(t, depth)? != self.binding
            || frames.len() != self.evidence.frame_shapes.len()
            || frames
                .iter()
                .zip(&self.evidence.frame_shapes)
                .any(|(f, s)| {
                    f.storage_offset != s.offset_words
                        || f.location.bhav != s.tree
                        || f.location.instruction != s.node
                        || f.args.len() != s.argc
                        || f.locals.len() != s.locals
                })
        {
            return Err("native registry retained frame owner changed".into());
        }
        self.world(world, false)
    }
}

pub(crate) fn contains(t: &Thread) -> bool {
    matches!(t.status, Status::NativeRegistryReentry(_))
        || t.callback
            .iter()
            .flat_map(|b| b.chain())
            .any(|b| matches!(b.resume_status, Status::NativeRegistryReentry(_)))
        || t.service
            .iter()
            .flat_map(|s| s.chain())
            .any(|s| matches!(s.resume_status, Status::NativeRegistryReentry(_)))
}
pub(crate) fn validate_thread(p: &Program, t: &Thread) -> Result<(), String> {
    if let Status::NativeRegistryReentry(owner) = &t.status {
        owner.frames(p, &t.stack)?;
        if t.check
            || t.callback.is_some()
            || t.service.is_some()
            || t.dispatch.is_some()
            || t.autonomy.is_some()
            || !t.find_best.is_empty()
            || !t.completed_actions.is_empty()
        {
            return Err("native registry reentry overlaps active continuation/service".into());
        }
    }
    for b in t.callback.iter().flat_map(|b| b.chain()) {
        if let Status::NativeRegistryReentry(owner) = &b.resume_status {
            if b.resume_check {
                return Err("native registry callback must restore check=false".into());
            }
            owner.frames(
                p,
                t.stack
                    .get(..b.base_depth)
                    .ok_or("native registry callback depth exceeds stack")?,
            )?;
        }
    }
    if t.service
        .iter()
        .flat_map(|s| s.chain())
        .any(|s| matches!(s.resume_status, Status::NativeRegistryReentry(_)))
    {
        return Err("native registry target-service parent is unsupported".into());
    }
    Ok(())
}
pub(crate) fn validate_vm(p: &Program, vm: &Vm) -> Result<(), String> {
    for (&id, t) in &vm.threads {
        if !contains(t) {
            continue;
        }
        validate_thread(p, t)?;
        if matches!(t.status, Status::NativeRegistryReentry(_))
            && (t.check
                || t.callback.is_some()
                || t.service.is_some()
                || t.dispatch.is_some()
                || t.autonomy.is_some()
                || !t.find_best.is_empty()
                || !t.completed_actions.is_empty())
        {
            return Err("native registry reentry overlaps active continuation/service".into());
        }
        if t.service
            .iter()
            .flat_map(|s| s.chain())
            .any(|s| matches!(s.resume_status, Status::NativeRegistryReentry(_)))
        {
            return Err("native registry target-service parent is unsupported".into());
        }
        for (status, depth, check) in std::iter::once((&t.status, t.stack.len(), t.check)).chain(
            t.callback
                .iter()
                .flat_map(|b| b.chain())
                .map(|b| (&b.resume_status, b.base_depth, b.resume_check)),
        ) {
            let Status::NativeRegistryReentry(owner) = status else {
                continue;
            };
            owner.validate(p, &vm.world, t, depth)?;
            let l = vm
                .lifecycles
                .get(&owner.object())
                .ok_or("native registry lifecycle absent")?;
            if check
                || vm.object_threads.get(&owner.object()) != Some(&id)
                || l.thread != id
                || l.phase != Phase::Main
                || l.run_init != owner.closed_group_init_permission()
                || l.owner != owner.binding.definition.source
                || l.definition != owner.binding.definition.object
                || l.init != owner.evidence.cached_init
                || l.main != owner.evidence.cached_main
                || vm.tick < owner.evidence.boundary.tick
            {
                return Err("native registry Main/lifetime/Init permission differs".into());
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
        if let Status::NativeRegistryReentry(owner) = status
            && (owner.adapter.fresh.source_key != source || owner.evidence.boundary.tick != tick)
        {
            return Err("native registry adapter epoch/capture tick differs".into());
        }
    }
    Ok(())
}
impl Vm {
    pub(crate) fn validate_native_registry_reentry_reset(
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
        let Status::NativeRegistryReentry(owner) = &t.status else {
            return Err("native registry Reset retains an active callback/continuation".into());
        };
        owner.shape(&self.world, t, t.stack.len())?;
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
                "native registry Reset retains an active callback/continuation/service".into(),
            );
        }
        Ok(())
    }
    pub(crate) fn prepare_native_registry_reentry_tick(
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
        if !matches!(t.status, Status::NativeRegistryReentry(_)) {
            return Ok(());
        }
        self.validate_native_registry_reentry_reset(object)?;
        let value = &self.world.objects[&object];
        let idle = value.idle.as_ref().unwrap();
        if idle.flags & 1 == 0 || idle.countdown > 0 || value.native_tree_error != Some(0) {
            return Err(
                "native registry reentry has not passed registry eligibility/error gate".into(),
            );
        }
        Ok(())
    }
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[rustfmt::skip]
mod tests;
