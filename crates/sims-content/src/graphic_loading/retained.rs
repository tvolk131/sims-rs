//! Hot ordinary Animator ownership. No Ready/Bound constructor is run here.
//! Surviving allocations are keyed by skin identity, never by reusable VM ID.
use crate::graphic_retirement::observed::{Id, Retained, frame};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::graphic_destruction::Request;
use sims_vm::{Definition, ObjectId, Vm};
use std::collections::{BTreeMap, BTreeSet};

mod owner;
#[cfg(test)]
mod tests;
pub use owner::Owner;
pub mod refresh;

/// Explicit process/view inputs shared by retained Animators. Physical state,
/// camera direction, terrain and tick are read from the owning Simulation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshContext {
    pub floor_vector_bits: [u32; 3],
    pub fp_control: u16,
    pub origin_pose_bits: [u32; 7],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Graphic {
    definition: Definition,
    registry_lifetime: u32,
    ready: u8,
    owner: Owner,
    started: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    refresh_started: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    graphics: BTreeMap<ObjectId, Graphic>,
    survivors: BTreeMap<Id, Owner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_context: Option<RefreshContext>,
}

impl Registry {
    pub fn is_empty(&self) -> bool {
        self.graphics.is_empty() && self.survivors.is_empty() && self.refresh_context.is_none()
    }
    pub fn objects(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.graphics.keys().copied()
    }
    pub fn owner(&self, id: ObjectId) -> Option<&Owner> {
        self.graphics.get(&id).map(|g| &g.owner)
    }
    pub fn contains(&self, id: ObjectId) -> bool {
        self.graphics.contains_key(&id)
    }
    pub fn refresh_started(&self, id: ObjectId) -> bool {
        self.graphics.get(&id).is_some_and(|g| g.refresh_started)
    }
    pub fn pending_updates(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.graphics.iter().filter_map(|(&id, g)| {
            (g.refresh_started || g.owner.frame().is_some_and(frame::Owner::update_pending))
                .then_some(id)
        })
    }
    pub fn survivors(&self) -> &BTreeMap<Id, Owner> {
        &self.survivors
    }
    pub fn started(&self, id: ObjectId) -> bool {
        self.graphics.get(&id).is_some_and(|g| g.started)
    }
    pub(super) fn ready(&self, id: ObjectId) -> Option<u8> {
        self.graphics.get(&id).map(|g| g.ready)
    }
    pub(super) fn insert(
        &mut self,
        id: ObjectId,
        definition: Definition,
        registry_lifetime: u32,
        ready: u8,
        input: Retained,
    ) -> Result<()> {
        ensure!(
            id > 0 && !self.graphics.contains_key(&id),
            "duplicate retained graphic object"
        );
        let graphic = Graphic {
            definition,
            registry_lifetime,
            ready,
            owner: Owner::legacy(input)?,
            started: false,
            refresh_started: false,
        };
        let mut candidate = self.clone();
        candidate.graphics.insert(id, graphic);
        candidate.validate_storage()?;
        *self = candidate;
        Ok(())
    }
    pub(super) fn insert_frame(
        &mut self,
        binding: frame::Binding,
        ready: u8,
        input: frame::Input,
        context: RefreshContext,
    ) -> Result<()> {
        let id = binding.object;
        ensure!(!self.contains(id), "duplicate retained graphic object");
        ensure!(
            self.refresh_context.as_ref().is_none_or(|c| c == &context),
            "retained Refresh shared context differs"
        );
        let graphic = Graphic {
            definition: binding.definition.clone(),
            registry_lifetime: binding.registry_lifetime,
            ready,
            owner: Owner::live(input, binding)?,
            started: false,
            refresh_started: false,
        };
        let mut candidate = self.clone();
        candidate.refresh_context = Some(context);
        candidate.graphics.insert(id, graphic);
        candidate.validate_storage()?;
        *self = candidate;
        Ok(())
    }
    pub(super) fn validate_storage(&self) -> Result<()> {
        let mut actors = BTreeSet::new();
        let mut allocations = BTreeSet::new();
        for (&id, g) in &self.graphics {
            ensure!(
                id > 0 && g.registry_lifetime != 0 && g.ready <= 1,
                "invalid retained graphic registration"
            );
            ensure!(
                !g.owner.complete()
                    && !g.owner.outer_released()
                    && (g.started || g.owner.events().is_empty()),
                "retained graphic lost its destructor owner"
            );
            ensure!(
                (!g.refresh_started || (!g.started && g.owner.frame().is_some()))
                    && (!g.owner.retiring() || g.started)
                    && g.owner.frame().is_none_or(|f| {
                        !g.started && (g.refresh_started || !f.update_pending())
                    })
                    && (g.owner.destructor().is_none() || !g.refresh_started),
                "retained graphic Refresh/destructor phases overlap"
            );
            if let Some(prelude) = g.owner.prelude() {
                let binding = prelude.binding();
                ensure!(
                    self.refresh_context.is_some()
                        && binding.object == id
                        && binding.definition == g.definition
                        && binding.registry_lifetime == g.registry_lifetime
                        && binding.native_actor == g.owner.retained().actor,
                    "retained Refresh binding differs from registry owner"
                );
            }
        }
        for (&skin, owner) in &self.survivors {
            ensure!(
                skin == owner.retained().skin.id && owner.complete() && owner.outer_released(),
                "retained survivor is still an active Animator"
            );
        }
        for owner in self
            .graphics
            .values()
            .map(|g| &g.owner)
            .chain(self.survivors.values())
        {
            owner.validate()?;
            // The first-load profile observes a null shared base-focus owner.
            // A live shared alias needs its own importer/service, not copies
            // silently diverging across two independently retained Animators.
            ensure!(
                owner.global_focus().is_none(),
                "retained shared base-focus owner unsupported"
            );
            ensure!(
                actors.insert(owner.retained().actor),
                "duplicate retained actor identity"
            );
            for &allocation in owner.retained().allocations.keys() {
                ensure!(
                    allocations.insert(allocation),
                    "aliased retained allocation ownership"
                );
            }
        }
        Ok(())
    }
    pub(super) fn validate(
        &self,
        vm: &Vm,
        retired: &BTreeMap<ObjectId, sims_sim::removal::graphic::Retired>,
    ) -> Result<()> {
        self.validate_storage()?;
        for (&id, g) in &self.graphics {
            let storage = vm
                .world
                .scheduler_storage
                .as_ref()
                .context("retained graphic registry lifetime owner absent")?;
            if let Some(object) = vm.world.objects.get(&id) {
                ensure!(
                    object.person.is_none()
                        && object.definition.as_ref() == Some(&g.definition)
                        && storage.handle(id) == Some(g.registry_lifetime)
                        && !g.started
                        && !retired.contains_key(&id),
                    "retained graphic live object binding differs"
                );
                if let Some(f) = g.owner.frame() {
                    f.validate_vm(vm)?;
                }
            } else {
                ensure!(
                    retired
                        .get(&id)
                        .is_some_and(|r| r.definition == g.definition
                            && r.asset_service_started == g.started
                            && r.registry_lifetime == Some(g.registry_lifetime))
                        && storage.retired_handle(g.registry_lifetime)
                        && !g.refresh_started
                        && g.owner
                            .frame()
                            .is_none_or(|f| f.complete() && f.pending().is_none()),
                    "retained graphic lost unlinked object ownership"
                );
            }
        }
        Ok(())
    }
    pub(super) fn begin(&mut self, id: ObjectId, focus: bool) -> Result<()> {
        self.validate_storage()?;
        let mut g = self
            .graphics
            .get(&id)
            .context("retained graphic absent")?
            .clone();
        ensure!(
            !g.started && !g.refresh_started && !focus,
            "retained graphic retirement/focus mismatch"
        );
        g.owner.begin_retirement()?;
        g.started = true;
        self.graphics.insert(id, g);
        Ok(())
    }
    pub(super) fn advance(
        &mut self,
        id: ObjectId,
        focus: bool,
        mut notify: impl FnMut(Request) -> Result<()>,
    ) -> Result<bool> {
        self.validate_storage()?;
        let mut g = self
            .graphics
            .get(&id)
            .context("retained graphic absent")?
            .clone();
        ensure!(
            g.started && !focus,
            "retained graphic retirement/focus mismatch"
        );
        let skin = g.owner.retained().skin.id;
        ensure!(
            !self.survivors.contains_key(&skin),
            "retained skin already retired"
        );
        let before = g.owner.service_count();
        let returned = g.owner.advance()?;
        //The last storage acknowledgement can itself complete584898. Keep
        //that boundary atomic with the caller's outer allocation release.
        let complete = returned || g.owner.complete();
        if g.owner.service_count() != before {
            if let Some(request @ (Request::DestroyModel(_) | Request::ClearFocus)) =
                g.owner.last_service()
            {
                notify(request)?;
            }
        }
        if complete {
            //584898 has returned;432387 releases +120's allocation next.
            //The Simulation acknowledgement clears the object's graphics owner.
            g.owner.release_animator()?;
            self.graphics.remove(&id);
            self.survivors.insert(skin, g.owner);
        } else {
            self.graphics.insert(id, g);
        }
        Ok(complete)
    }
}
