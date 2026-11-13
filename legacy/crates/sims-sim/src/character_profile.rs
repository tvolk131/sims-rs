//! Profile invalidation shared by 5893b3 and 592625 skill lookups.
use crate::{Simulation, character_update::AnimatorRequest};
use serde::{Deserialize, Serialize};
use sims_vm::{AnimationRequest, ObjectId, Status, ThreadId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Origin {
    Ordinary(AnimatorRequest),
    Command {
        thread: ThreadId,
        request: AnimationRequest,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub origin: Origin,
    pub started: bool,
    pub tick: u64,
}
impl Simulation {
    pub(crate) fn defer_skill_profile(
        &mut self,
        object: ObjectId,
        origin: Origin,
    ) -> Result<(), String> {
        let resident = self
            .characters
            .get_mut(&object)
            .ok_or("profile character absent")?;
        if !resident.profile_dirty || resident.skill_profile.is_some() {
            return Err("profile reload already consumed".into());
        }
        // Both helpers clear +d8 before calling 5888d3.
        resident.profile_dirty = false;
        resident.skill_profile = Some(Pending {
            origin,
            started: false,
            tick: self.vm.tick,
        });
        Ok(())
    }
    pub fn pending_skill_profiles(&self) -> Vec<(ObjectId, bool)> {
        self.characters
            .iter()
            .filter_map(|(&id, r)| r.skill_profile.as_ref().map(|p| (id, p.started)))
            .collect()
    }
    pub fn begin_skill_profile(&mut self, object: ObjectId) -> Result<(), String> {
        let p = self
            .characters
            .get_mut(&object)
            .and_then(|r| r.skill_profile.as_mut())
            .ok_or("profile continuation absent")?;
        if p.started {
            return Err("profile already started".into());
        }
        p.started = true;
        Ok(())
    }
    pub fn complete_skill_profile(&mut self, object: ObjectId) -> Result<(), String> {
        let r = self
            .characters
            .get_mut(&object)
            .ok_or("profile character absent")?;
        if r.skill_profile.as_ref().is_none_or(|p| !p.started) {
            return Err("profile service is not pending".into());
        }
        r.skill_profile = None;
        Ok(())
    }
    pub(crate) fn validate_skill_profiles(&self) -> Result<(), String> {
        let updates = self.pending_character_updates();
        for (&object, r) in &self.characters {
            if let Some(p) = &r.skill_profile {
                let bound = match &p.origin {
                    Origin::Ordinary(request) => updates.contains(&(object, request.clone())),
                    Origin::Command { thread, request } => request.caller == object
                        && self.vm.threads.get(thread).is_some_and(
                            |t| matches!(&t.status, Status::AwaitingAnimation(r) if r == request),
                        ),
                };
                if !bound || r.profile_dirty || p.tick != self.vm.tick {
                    return Err("unbound native skill profile continuation".into());
                }
            }
        }
        Ok(())
    }
}
