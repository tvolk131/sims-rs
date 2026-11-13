//! Ordinary 3D storage retained across the native post-unlink destructor.
use crate::Simulation;
use serde::{Deserialize, Serialize};
use sims_vm::{Definition, ObjectId, Program};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retired {
    pub definition: Definition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_lifetime: Option<u32>,
    pub tick: u64,
    pub asset_service_started: bool,
}
impl Simulation {
    pub(crate) fn drive_removal_placement(
        &mut self,
        p: &Program,
        e: &mut crate::lot_placement::live::Execution,
        remaining: &mut usize,
        step: &mut crate::Step,
    ) -> Result<bool, String> {
        let mut graphics = self.retained_ordinary_graphics.clone();
        if self.native_character_settings.is_some() {
            for (&id, object) in &self.vm.world.objects {
                if object.person.is_none()
                    && object
                        .definition
                        .as_ref()
                        .and_then(|d| {
                            p.modules
                                .get(&d.source)?
                                .object_definition_words
                                .get(&d.object)
                        })
                        .and_then(|w| w.get(102))
                        .is_some_and(|w| w & 1 != 0)
                {
                    graphics.insert(id);
                }
            }
        }
        self.drive_live_placement_with_graphics(p, e, remaining, step, &graphics)
    }
    fn removal_graphic_placement(&mut self) -> Option<&mut crate::lot_placement::live::Execution> {
        let pending = |e: &crate::lot_placement::live::Execution| {
            matches!(
                e.pending,
                Some(crate::lot_placement::Request::RefreshAnimator(_))
            )
        };
        // Innermost physical detachment precedes its destructor and caller.
        if let Some(e) = self.native_lot.as_mut().and_then(|n| n.removal.as_mut()) {
            if e.placement.graphic().is_some() {
                return e.placement.graphic_mut();
            }
        }
        if self.deletion.last().is_some_and(|d| {
            matches!(&d.base_reset, Some(super::base::Execution::Detach(e)) if e.graphic().is_some())
        }) {
            let Some(super::base::Execution::Detach(e)) =
                self.deletion.last_mut().and_then(|d| d.base_reset.as_mut())
            else { unreachable!() };
            return e.graphic_mut();
        }
        if let Some(index) = self
            .deletion
            .iter()
            .rposition(|d| d.cursor.as_ref().is_some_and(|e| e.graphic().is_some()))
        {
            return self.deletion[index].cursor.as_mut().unwrap().graphic_mut();
        }
        if let Some(e) = self.synchronous_removal.as_mut() {
            if !e.removal.deleting && e.removal.placement.graphic().is_some() {
                return e.removal.placement.graphic_mut();
            }
        }
        if let Some(e) = self
            .object_pickup
            .as_mut()
            .and_then(|e| e.placement.as_mut())
        {
            if pending(e) {
                return Some(e);
            }
        }
        if let Some(e) = self.object_placement.as_mut() {
            if e.graphic().is_some() {
                return e.graphic_mut();
            }
        }
        if let Some(f) = self.cursor_factory.as_mut() {
            return f.placement.as_mut().and_then(|e| e.graphic_mut());
        }
        if let Some(e) = self.cursor_cleanup.as_mut() {
            return e.graphic_mut();
        }
        if let Some(e) = self.cursor_release.as_mut() {
            return e.graphic_mut();
        }
        self.cursor_rotation
            .as_mut()
            .and_then(|r| r.execution.graphic_mut())
    }
    pub fn pending_removal_graphic(&self) -> Option<(ObjectId, bool)> {
        let candidates = [
            self.object_placement.as_ref().and_then(|e| e.graphic()),
            self.deletion
                .iter()
                .rev()
                .find_map(|d| d.cursor.as_ref().and_then(|e| e.graphic())),
            self.cursor_cleanup.as_ref().and_then(|e| e.graphic()),
            self.cursor_release.as_ref().and_then(|e| e.graphic()),
            self.cursor_rotation
                .as_ref()
                .and_then(|r| r.execution.graphic()),
            self.cursor_factory
                .as_ref()
                .and_then(|f| f.placement.as_ref())
                .and_then(|e| e.graphic()),
            self.object_pickup
                .as_ref()
                .and_then(|e| e.placement.as_ref()),
            self.native_lot
                .as_ref()
                .and_then(|n| n.removal.as_ref())
                .and_then(|r| r.placement.graphic()),
            self.deletion.last().and_then(|d| match &d.base_reset {
                Some(super::base::Execution::Detach(e)) => e.graphic(),
                _ => None,
            }),
            self.synchronous_removal
                .as_ref()
                .filter(|r| !r.removal.deleting)
                .and_then(|r| r.removal.placement.graphic()),
        ];
        candidates.into_iter().flatten().find_map(|e| {
            let Some(crate::lot_placement::Request::RefreshAnimator(id)) = e.pending else {
                return None;
            };
            self.vm
                .world
                .objects
                .get(&id)
                .is_some_and(|o| o.person.is_none())
                .then_some((id, e.graphic_started))
        })
    }
    // Mirror the existing mutating acknowledgement chooser without cloning its
    // Simulation or placement state. A mismatched nested owner must be rejected
    // before any asset or VM effect, rather than discovered at final ack.
    fn removal_graphic_placement_ref(&self) -> Option<&crate::lot_placement::live::Execution> {
        if let Some(e) = self.native_lot.as_ref().and_then(|n| n.removal.as_ref()) {
            if e.placement.graphic().is_some() {
                return e.placement.graphic();
            }
        }
        if let Some(super::base::Execution::Detach(e)) =
            self.deletion.last().and_then(|d| d.base_reset.as_ref())
        {
            if e.graphic().is_some() {
                return e.graphic();
            }
        }
        if let Some(e) = self
            .deletion
            .iter()
            .rev()
            .find_map(|d| d.cursor.as_ref().and_then(|e| e.graphic()))
        {
            return Some(e);
        }
        if let Some(e) = self.synchronous_removal.as_ref() {
            if !e.removal.deleting && e.removal.placement.graphic().is_some() {
                return e.removal.placement.graphic();
            }
        }
        if let Some(e) = self
            .object_pickup
            .as_ref()
            .and_then(|e| e.placement.as_ref())
        {
            if matches!(
                e.pending,
                Some(crate::lot_placement::Request::RefreshAnimator(_))
            ) {
                return Some(e);
            }
        }
        if let Some(e) = &self.object_placement {
            if e.graphic().is_some() {
                return e.graphic();
            }
        }
        if let Some(f) = &self.cursor_factory {
            return f.placement.as_ref().and_then(|e| e.graphic());
        }
        if let Some(e) = &self.cursor_cleanup {
            return e.graphic();
        }
        if let Some(e) = &self.cursor_release {
            return e.graphic();
        }
        self.cursor_rotation
            .as_ref()
            .and_then(|r| r.execution.graphic())
    }
    /// Read-only join of the actual physical/removal owners before an external
    /// retained Animator service. This performs the corresponding existing
    /// snapshot checks using borrowed state, without advancing a continuation.
    pub fn validate_removal_graphic_owner(
        &self,
        p: &Program,
        id: ObjectId,
        started: bool,
    ) -> Result<(), String> {
        if self.pending_removal_graphic() != Some((id, started))
            || self.removal_graphic_placement_ref().is_none_or(|e| {
                e.pending != Some(crate::lot_placement::Request::RefreshAnimator(id))
                    || e.graphic_started != started
                    || e.complete
            })
            || !self.retained_ordinary_graphics.contains(&id)
            || self.retired_graphics.contains_key(&id)
            || self.retired_characters.contains_key(&id)
            || self
                .vm
                .world
                .objects
                .get(&id)
                .is_none_or(|o| o.person.is_some())
        {
            return Err("retained ordinary Refresh lost physical ownership".into());
        }
        self.validate_native_lot()?;
        self.validate_removals(p)?;
        if let Some(e) = &self.synchronous_removal {
            e.validate(self, p)?;
        }
        if let Some(e) = &self.object_placement {
            e.validate(self, p)?;
        }
        if let Some(e) = &self.object_pickup {
            e.validate(self, p)?;
        }
        if let Some(e) = &self.cursor_factory {
            e.validate(self, p)?;
        }
        if let Some(e) = &self.cursor_rotation {
            e.validate(self, p)?;
        }
        if let Some(e) = &self.cursor_release {
            e.validate(self, p)?;
        }
        if let Some(e) = &self.cursor_cleanup {
            e.validate(self, p)?;
        }
        Ok(())
    }
    pub fn start_removal_graphic(&mut self, id: ObjectId) -> Result<(), String> {
        if self.pending_removal_graphic() != Some((id, false)) {
            return Err("ordinary removal refresh lost ownership".into());
        }
        self.removal_graphic_placement().unwrap().graphic_started = true;
        Ok(())
    }
    pub fn complete_removal_graphic(&mut self, id: ObjectId) -> Result<(), String> {
        if self.pending_removal_graphic() != Some((id, true)) {
            return Err("stale ordinary removal refresh".into());
        }
        let e = self.removal_graphic_placement().unwrap();
        e.graphic_started = false;
        e.pending = None;
        Ok(())
    }
    pub(crate) fn ordinary_graphic_required(
        &self,
        p: &Program,
        id: ObjectId,
    ) -> Result<bool, String> {
        if self.retained_ordinary_graphics.contains(&id) {
            if self
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .and_then(|s| s.handle(id))
                .is_none()
            {
                return Err("retained ordinary graphic registry lifetime absent".into());
            }
            if self
                .vm
                .world
                .objects
                .get(&id)
                .is_none_or(|o| o.person.is_some())
            {
                return Err("retained ordinary graphic object absent or person".into());
            }
            return Ok(true);
        }
        if self.native_character_settings.is_none() {
            return Ok(false);
        }
        let object = self
            .vm
            .world
            .objects
            .get(&id)
            .ok_or("ordinary graphic object absent")?;
        if object.person.is_some() {
            return Ok(false);
        }
        let definition = object
            .definition
            .as_ref()
            .ok_or("ordinary graphic definition absent")?;
        let words = sims_vm::construction::definition_words(p, definition)?;
        Ok(words.get(102).is_some_and(|w| w & 1 != 0))
    }
    pub fn begin_graphic_retirement(&mut self, id: ObjectId) -> Result<(), String> {
        self.validate_graphic_retirement_owner(id)?;
        let r = self
            .retired_graphics
            .get_mut(&id)
            .ok_or("retired ordinary graphic absent")?;
        if r.asset_service_started {
            return Err("ordinary retirement already started".into());
        }
        r.asset_service_started = true;
        Ok(())
    }
    pub fn complete_graphic_retirement(&mut self, id: ObjectId) -> Result<(), String> {
        self.validate_graphic_retirement_owner(id)?;
        if !self
            .retired_graphics
            .get(&id)
            .is_some_and(|r| r.asset_service_started)
        {
            return Err("ordinary retirement not started".into());
        }
        self.deletion.last_mut().unwrap().assets_released = true;
        self.retired_graphics.remove(&id);
        self.retained_ordinary_graphics.remove(&id);
        Ok(())
    }
    /// Read-only asset-host preflight, before any external owner is mutated.
    pub fn validate_graphic_retirement_owner(&self, id: ObjectId) -> Result<(), String> {
        if self
            .deletion
            .last()
            .is_none_or(|d| d.target != id || !d.storage_unlinked || d.assets_released)
            || self.vm.world.objects.contains_key(&id)
            || self.retired_characters.contains_key(&id)
        {
            return Err("ordinary retirement lost deletion ownership".into());
        }
        Ok(())
    }
    pub(crate) fn validate_retired_graphics(&self, p: &Program) -> Result<(), String> {
        for &id in &self.retained_ordinary_graphics {
            let storage = self
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .ok_or("retained ordinary graphic registry lifetime owner absent")?;
            if self.vm.world.objects.contains_key(&id) {
                if storage.handle(id).is_none() {
                    return Err("retained ordinary graphic registry lifetime absent".into());
                }
            } else if !self
                .retired_graphics
                .get(&id)
                .and_then(|r| r.registry_lifetime)
                .is_some_and(|h| storage.retired_handle(h))
            {
                return Err("retained ordinary graphic retired lifetime absent".into());
            }
            let definition = self
                .vm
                .world
                .objects
                .get(&id)
                .and_then(|o| {
                    o.person
                        .is_none()
                        .then_some(o.definition.as_ref())
                        .flatten()
                })
                .or_else(|| self.retired_graphics.get(&id).map(|r| &r.definition))
                .ok_or("retained ordinary graphic lost object or destructor owner")?;
            let words = sims_vm::construction::definition_words(p, definition)?;
            if id <= 0 || words[9] == 2 {
                return Err("invalid retained ordinary graphic owner".into());
            }
        }
        for (&id, r) in &self.retired_graphics {
            self.validate_graphic_retirement_owner(id)?;
            let words = sims_vm::construction::definition_words(p, &r.definition)?;
            if id <= 0
                || r.tick != self.vm.tick
                || (self.native_character_settings.is_none()
                    && !self.retained_ordinary_graphics.contains(&id))
                || words[9] == 2
                || (!self.retained_ordinary_graphics.contains(&id)
                    && !words.get(102).is_some_and(|w| w & 1 != 0))
            {
                return Err("invalid retired ordinary graphic".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod retained_tests {
    #[test]
    fn observed_animator_ownership_survives_absent_person_settings_and_changed_flags() {
        let (mut p, mut s) = crate::lot_runtime::tests::fixture();
        // Authored registration lifetimes, not imported native history.
        s.vm.world.scheduler_storage = Some(Default::default());
        let objects = std::mem::take(&mut s.vm.world.objects);
        for (id, mut object) in objects {
            s.vm.world
                .register_scheduler_object(id, &mut object)
                .unwrap();
            s.vm.world.objects.insert(id, object);
        }
        p.modules
            .get_mut("objects")
            .unwrap()
            .object_definition_words
            .get_mut(&1)
            .unwrap()[102] = 0;
        assert!(s.native_character_settings.is_none());
        assert!(!s.ordinary_graphic_required(&p, 1).unwrap());
        s.retained_ordinary_graphics.insert(1);
        let before = s.vm.clone();
        assert!(s.ordinary_graphic_required(&p, 1).unwrap());
        s.validate_retired_graphics(&p).unwrap();
        let restored: crate::Simulation =
            serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(
            restored.retained_ordinary_graphics,
            s.retained_ordinary_graphics
        );
        assert!(restored.ordinary_graphic_required(&p, 1).unwrap());
        assert_eq!(s.vm, before);
    }

    #[test]
    fn stale_positive_graphic_identity_is_rejected() {
        let (p, mut s) = crate::lot_runtime::tests::fixture();
        s.retained_ordinary_graphics.insert(9999);
        assert!(s.ordinary_graphic_required(&p, 9999).is_err());
        assert!(s.validate_retired_graphics(&p).is_err());
    }
}
