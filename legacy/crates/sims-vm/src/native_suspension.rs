//! Timestamp-free ordinary native suspension at an explicitly supplied stopped
//! boundary. This is not an imported timer, a completed host reply, or a claim
//! that all other Thread controls have been recovered.
use crate::{Definition, Frame, Location, Program, Status, Thread, ThreadId, Vm};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const IMAGE: &str = "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321";
const ENTRY_CODE_SHA256: [u8; 32] = [
    0xbc, 0x7d, 0x60, 0x50, 0xcd, 0xf9, 0xbb, 0xc6, 0x72, 0xa9, 0xa5, 0x5f, 0xdf, 0xb7, 0xe9, 0x6f,
    0xa9, 0x83, 0x33, 0xea, 0x06, 0x85, 0x8f, 0x00, 0x13, 0x6e, 0x35, 0x07, 0x02, 0x0e, 0x50, 0xfc,
];

/// Required source/caller evidence. The VM verifies the supplied identity and
/// bytes, not the authenticity of an external capture file. Authored tests must
/// label their supplied witnesses accordingly. No fields have serde defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub image_sha256: String,
    pub entry_pc: u32,
    pub caller_return: u32,
    pub entry_code: [u8; 32],
}
impl Evidence {
    fn validate(&self) -> Result<(), String> {
        if self.image_sha256 != IMAGE
            || self.entry_pc != 0x415e20
            || self.caller_return != 0x415f92
            || Sha256::digest(self.entry_code)[..] != ENTRY_CODE_SHA256
        {
            return Err("unsupported native ordinary suspension source/caller evidence".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn fingerprint_itself_is_not_an_accepted_instruction_witness() {
        let evidence = Evidence {
            image_sha256: IMAGE.into(),
            entry_pc: 0x415e20,
            caller_return: 0x415f92,
            entry_code: ENTRY_CODE_SHA256,
        };
        assert!(evidence.validate().is_err());
    }

    #[test]
    fn matching_metadata_does_not_admit_synthetic_instruction_bytes() {
        let evidence = Evidence {
            image_sha256: IMAGE.into(),
            entry_pc: 0x415e20,
            caller_return: 0x415f92,
            entry_code: [0; 32],
        };
        assert!(evidence.validate().is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Shape {
    code_owner: String,
    location: Location,
    caller: crate::ObjectId,
    callee: crate::ObjectId,
    continuation: u32,
    args: usize,
    locals: usize,
    offset: usize,
}
impl Shape {
    fn of(f: &Frame) -> Self {
        Self {
            code_owner: f.code_owner.clone(),
            location: f.location.clone(),
            caller: f.caller,
            callee: f.callee,
            continuation: f.continuation,
            args: f.args.len(),
            locals: f.locals.len(),
            offset: f.storage_offset,
        }
    }
}

/// Source-bound owner, constructed only from RetainedStack::native_suspension.
/// Private fields are still validated on import/restore: deserialization is not
/// authority to invent a context, lifetime, caller, or active frame shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    evidence: Evidence,
    binding: crate::retained_frames::ContextBinding,
    ordinary: Vec<Shape>,
}
impl Owner {
    pub(crate) fn new(
        evidence: Evidence,
        binding: crate::retained_frames::ContextBinding,
        frames: &[Frame],
    ) -> Result<Self, String> {
        evidence.validate()?;
        Ok(Self {
            evidence,
            binding,
            ordinary: frames.iter().map(Shape::of).collect(),
        })
    }
    pub fn object(&self) -> crate::ObjectId {
        self.binding.object
    }
    pub fn definition(&self) -> &Definition {
        &self.binding.definition
    }
    fn shape(&self, t: &Thread, depth: usize) -> Result<(), String> {
        self.evidence.validate()?;
        let prefix = t
            .stack
            .get(..depth)
            .ok_or("native suspension parent stack absent")?;
        if self.ordinary.len() != 2
            || prefix.iter().map(Shape::of).collect::<Vec<_>>() != self.ordinary
        {
            return Err("native suspension parent shape changed".into());
        }
        Ok(())
    }
    fn validate(&self, p: &Program, t: &Thread, depth: usize) -> Result<(), String> {
        self.shape(t, depth)?;
        if crate::retained_frames::native_binding(p, t, depth)? != self.binding {
            return Err("native suspension context/lifetime binding changed".into());
        }
        Ok(())
    }
}

pub(crate) fn validate(p: &Program, t: &Thread) -> Result<(), String> {
    crate::registry_suspension::validate(p, t)?;
    crate::native_registry_reentry::validate_thread(p, t)?;
    if let Status::NativeSuspended(owner) = &t.status {
        owner.validate(p, t, t.stack.len())?;
        if t.check
            || t.callback.is_some()
            || t.dispatch.is_some()
            || t.autonomy.is_some()
            || !t.find_best.is_empty()
            || !t.completed_actions.is_empty()
            || t.service.as_ref().is_some_and(|s| s.result.is_none())
        {
            return Err("native suspension overlaps an active synchronous owner".into());
        }
        // A completed service reply and stale callback_result are NOT erased.
        // Their explicit supplied values remain independent Thread controls.
    }
    for boundary in t.callback.iter().flat_map(|b| b.chain()) {
        if let Status::NativeSuspended(owner) = &boundary.resume_status {
            if boundary.resume_check {
                return Err("native suspension callback must restore ordinary check=false".into());
            }
            owner.validate(p, t, boundary.base_depth)?;
        }
    }
    // This bounded source seam supports synchronous callbacks, not creating a
    // new yielding target-service parent from the imported suspension. Reject
    // that unsupported transition explicitly rather than discarding its reply.
    if t.service
        .iter()
        .flat_map(|s| s.chain())
        .any(|s| matches!(s.resume_status, Status::NativeSuspended(_)))
    {
        return Err("native suspension target-service parent is unsupported".into());
    }
    Ok(())
}

pub(crate) fn contains(t: &Thread) -> bool {
    matches!(t.status, Status::NativeSuspended(_))
        || t.callback
            .iter()
            .flat_map(|b| b.chain())
            .any(|b| matches!(b.resume_status, Status::NativeSuspended(_)))
        || t.service
            .iter()
            .flat_map(|s| s.chain())
            .any(|s| matches!(s.resume_status, Status::NativeSuspended(_)))
}

impl Vm {
    /// Common replacement boundary, before storage reset publication or spawn.
    /// Only native-owned threads acquire these additional guards; historical
    /// Ready/Sleeping lifecycle semantics remain unchanged.
    pub(crate) fn native_replacement_preflight(
        &self,
        p: &Program,
        object: crate::ObjectId,
        existing: Option<ThreadId>,
    ) -> Result<(), String> {
        let Some(id) = existing else { return Ok(()) };
        let Some(t) = self.threads.get(&id) else {
            return Ok(());
        };
        if !contains(t) && !crate::registry_suspension::contains(t) && !crate::native_registry_reentry::contains(t) {
            return Ok(());
        }
        if self.object_threads.get(&object) != Some(&id) {
            return Err("native suspension replacement lost its registered thread".into());
        }
        crate::retained_frames::validate_world(p, &self.world, t)?;
        self.validate_native_suspension_reset(object)?;
        if t.interrupted {
            return Err("native suspension replacement retains a pending host interrupt".into());
        }
        Ok(())
    }
    pub(crate) fn reject_native_suspension_run(&self, thread: ThreadId) -> Result<(), String> {
        if self.threads.get(&thread).is_some_and(|t| {
            matches!(
                t.status,
                Status::NativeSuspended(_) | Status::RegistrySuspended(_) | Status::NativeRegistryReentry(_)
            )
        }) {
            return Err("native suspension requires an eligible native registry Tick".into());
        }
        Ok(())
    }
    pub(crate) fn validate_native_suspension_reset(
        &self,
        object: crate::ObjectId,
    ) -> Result<(), String> {
        self.validate_registry_suspension_reset(object)?;
        self.validate_native_registry_reentry_reset(object)?;
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
        let check_owner = |owner: &Owner, depth: usize| -> Result<(), String> {
            owner.shape(t, depth)?;
            if owner.object() != object {
                return Err("native suspension Reset receiver differs".into());
            }
            crate::retained_frames::native_binding_current(&self.world, t, &owner.binding)
        };
        if let Status::NativeSuspended(owner) = &t.status {
            check_owner(owner, t.stack.len())?;
        }
        for b in t.callback.iter().flat_map(|b| b.chain()) {
            if let Status::NativeSuspended(owner) = &b.resume_status {
                if b.resume_check {
                    return Err(
                        "native suspension callback must restore ordinary check=false".into(),
                    );
                }
                check_owner(owner, b.base_depth)?;
            }
        }
        for s in t.service.iter().flat_map(|s| s.chain()) {
            if let Status::NativeSuspended(owner) = &s.resume_status {
                check_owner(owner, s.base_depth)?;
            }
        }
        if !matches!(t.status, Status::NativeSuspended(_))
            || t.check
            || t.callback.is_some()
            || t.service.is_some()
            || t.dispatch.is_some()
            || t.autonomy.is_some()
            || !t.find_best.is_empty()
            || !t.completed_actions.is_empty()
        {
            return Err(
                "native suspension Reset retains an active callback, continuation or service reply"
                    .into(),
            );
        }
        Ok(())
    }
    /// Only the native Tick API calls this read-only preflight. The scheduler
    /// owns pause/partial-pass/cursor eligibility; the VM independently rejects
    /// a disabled/still-counting object or a changed registry lifetime.
    pub(crate) fn prepare_native_suspension_tick(
        &self,
        object: crate::ObjectId,
    ) -> Result<(), String> {
        self.prepare_registry_suspension_tick(object)?;
        self.prepare_native_registry_reentry_tick(object)?;
        let Some(t) = self
            .object_threads
            .get(&object)
            .and_then(|id| self.threads.get(id))
        else {
            return Ok(());
        };
        let Status::NativeSuspended(owner) = &t.status else {
            return Ok(());
        };
        owner.shape(t, t.stack.len())?;
        if owner.object() != object {
            return Err("native suspension Tick receiver differs".into());
        }
        crate::retained_frames::native_binding_current(&self.world, t, &owner.binding)?;
        let value = self
            .world
            .objects
            .get(&object)
            .ok_or("native suspension object absent")?;
        let idle = value
            .idle
            .as_ref()
            .ok_or("native suspension registry word absent")?;
        if idle.flags & 1 == 0 || idle.countdown > 0 || value.native_tree_error != Some(0) {
            return Err(
                "native suspension has not passed the registry eligibility/error gate".into(),
            );
        }
        if t.check
            || t.callback.is_some()
            || t.service.is_some()
            || t.dispatch.is_some()
            || t.autonomy.is_some()
            || !t.find_best.is_empty()
            || !t.completed_actions.is_empty()
        {
            return Err("native suspension still owns synchronous work or a service reply".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[cfg(feature = "private-fixtures")]
#[rustfmt::skip]
mod tests;
