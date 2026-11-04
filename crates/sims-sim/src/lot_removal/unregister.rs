//! Headless adapter for 44123f's 447d8b prelude. Our presenter has no native
//! preview or held-object UI owners (these are distinct from selected_person).
//! Native UI ownership must use its own service adapter when implemented.
use crate::Simulation;
use sims_vm::{ObjectId, Program, indicator_unregister as native};

struct Host<'a> {
    sim: &'a mut Simulation,
    program: &'a Program,
}
impl native::Host for Host<'_> {
    fn enabled(&self) -> bool {
        self.sim.native_lot.is_some() && !self.sim.vm.world.registry_ui_disabled
    }
    fn kind(&self, object: ObjectId) -> Result<i16, String> {
        let d = self
            .sim
            .vm
            .world
            .objects
            .get(&object)
            .and_then(|o| o.definition.as_ref())
            .ok_or("unregister definition absent")?;
        Ok(sims_vm::construction::definition_words(self.program, d)?[9])
    }
    fn guid(&self, object: ObjectId) -> Result<u32, String> {
        Ok(self
            .sim
            .vm
            .world
            .objects
            .get(&object)
            .and_then(|o| o.definition.as_ref())
            .ok_or("unregister definition absent")?
            .guid)
    }
    fn cast_person(&mut self, object: ObjectId) -> Result<Option<ObjectId>, String> {
        Ok(self
            .sim
            .vm
            .world
            .objects
            .get(&object)
            .is_some_and(|o| o.person.is_some())
            .then_some(object))
    }
    fn preview_person(&mut self) -> Result<Option<ObjectId>, String> {
        Ok(None)
    }
    fn held_object(&mut self) -> Result<Option<ObjectId>, String> {
        Ok(None)
    }
    fn clear_ui(&mut self) -> Result<(), String> {
        Err("native preview/held UI disposal requires an owning presenter".into())
    }
    fn indicators(
        &mut self,
    ) -> (
        &mut sims_vm::load_indicators::Indicators,
        &mut sims_vm::indicator_pools::Pools,
    ) {
        (
            &mut self.sim.native_lot.as_mut().unwrap().indicators,
            &mut self.sim.native_indicator_pools,
        )
    }
    fn queues(&mut self) -> &mut native::Queues {
        &mut self.sim.vm.world.indicator_queues
    }
}
impl Simulation {
    pub(crate) fn unregister_immediate_removal(
        &mut self,
        p: &Program,
        target: ObjectId,
    ) -> Result<(), String> {
        native::unregister(
            &mut Host {
                sim: self,
                program: p,
            },
            Some(target),
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_refresh_queue_validation_preserves_nullable_order_and_rejects_missing_owners() {
        let (_, mut s) = crate::lot_runtime::tests::fixture();
        // A null star notification dirties the registry without an entry;
        // skipping this field during save must not lose that pending pass.
        s.vm.world.indicator_queues.dirty = true;
        let saved = serde_json::to_value(&s).unwrap();
        assert_eq!(saved["vm"]["world"]["indicator_queues"]["dirty"], true);
        let restored: crate::Simulation = serde_json::from_value(saved).unwrap();
        assert_eq!(restored, s);
        s.vm.world.indicator_queues.dates = vec![None, Some(2), Some(2), Some(4)];
        s.vm.world.indicator_queues.stars = vec![Some(1), None];
        let before = s.clone();
        s.validate_native_lot().unwrap();
        assert_eq!(s, before);
        s.vm.world.indicator_queues.stars.push(Some(99));
        assert_eq!(
            s.validate_native_lot().unwrap_err(),
            "native indicator refresh queue references an absent object"
        );
    }
}
