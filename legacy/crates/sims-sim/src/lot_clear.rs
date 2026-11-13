//! Frozen44279a/445f84 load teardown. Ordinary scheduling never runs here.
//! Errors leave an incomplete loader, which must not be published as play.
use crate::{Simulation, Step, StepStop, lot_removal::immediate::Removal};
use sims_vm::{ObjectId, Program};

impl Simulation {
    /// Clear through the normal physical removal, Reset and destruction path.
    /// The host services pending assets, never acknowledges an unknown request.
    /// This synchronous loader is not an arbitrary-instruction save boundary.
    pub fn clear_for_house_read(
        &mut self,
        program: &Program,
        host: &mut impl FnMut(&mut Self, &Program) -> Result<(), String>,
    ) -> Result<Vec<ObjectId>, String> {
        if self.synchronous_removal.is_some()
            || !self.deletion.is_empty()
            || self.vm.scheduled_object_visit.is_some()
            || self.native_lot.is_none()
        {
            return Err("house clear requires a frozen, available registry".into());
        }
        let tick = self.vm.tick;
        self.vm.world.registry_ui_disabled = true;
        let mut removed = vec![];
        //445f84 reads each pointer live, nulling it only after its destructor.
        for primary in [true, false] {
            let indicators = &self.native_lot.as_ref().unwrap().indicators;
            let id = if primary {
                indicators.primary
            } else {
                indicators.secondary
            };
            if let Some(id) = id {
                let removal = Removal::indicator(self, id)?;
                self.finish_frozen_removal(program, removal, host)?;
                removed.push(id);
                let indicators = &mut self.native_lot.as_mut().unwrap().indicators;
                if primary {
                    indicators.primary = None;
                } else {
                    indicators.secondary = None;
                }
            }
        }
        for active in [true, false] {
            let pool = if active {
                &self.native_indicator_pools.active
            } else {
                &self.native_indicator_pools.spare
            };
            let count = pool.len(); // captured native end, not a live growing end
            for n in 0..count {
                let pool = if active {
                    &self.native_indicator_pools.active
                } else {
                    &self.native_indicator_pools.spare
                };
                let id = pool
                    .get(n)
                    .copied()
                    .flatten()
                    .ok_or("null or shortened teardown indicator pool")?;
                let removal = Removal::indicator(self, id)?;
                self.finish_frozen_removal(program, removal, host)?;
                removed.push(id);
            }
            if active {
                self.native_indicator_pools.active.clear();
            } else {
                self.native_indicator_pools.spare.clear();
            }
        }
        // Callbacks may remove another object or create a new head. Re-read the
        // actual linked registry after every complete synchronous destruction.
        loop {
            let next = self
                .vm
                .world
                .person
                .as_ref()
                .ok_or("clear registry absent")?
                .object_order
                .first()
                .copied();
            let Some(id) = next else {
                break;
            };
            if removed.len() >= 100_000 {
                return Err("house-clear diagnostic budget exceeded".into());
            }
            let removal = Removal::new(self, program, id)?;
            self.finish_frozen_removal(program, removal, host)?;
            removed.push(id);
        }
        if !self.vm.world.objects.is_empty() || !self.vm.threads.is_empty() || self.vm.tick != tick
        {
            return Err("house clear failed to drain registry at frozen time".into());
        }
        //44279a preserves the deferred ID queue even while destroying its
        //objects. The outer house loader explicitly clears it via428679 after
        //415eff returns; recycled IDs must retain that intervening behavior.
        self.vm
            .world
            .scheduler_storage
            .as_mut()
            .ok_or("clear scheduler storage absent")?
            .clear_registry()?;
        self.vm.world.registry_ui_disabled = false;
        self.sync_spatial_world();
        self.validate_native_lot()?;
        Ok(removed)
    }

    fn finish_frozen_removal(
        &mut self,
        program: &Program,
        removal: Removal,
        host: &mut impl FnMut(&mut Self, &Program) -> Result<(), String>,
    ) -> Result<(), String> {
        self.retain_frozen_removal(program, removal)?;
        let mut step = Step {
            stop: StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        for _ in 0..1000 {
            let mut remaining = 100_000;
            if self.drive_frozen_removal(program, &mut remaining, &mut step)? {
                self.take_synchronous_removal()?;
                return Ok(());
            }
            host(self, program)?;
        }
        Err("house-clear service budget exceeded".into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn clear_runs_reset_callbacks_in_indicator_then_registry_order_and_retains_rng() {
        use sims_formats::behavior::Instruction;
        let (mut p, mut s) = crate::lot_runtime::tests::fixture();
        s.vm.world.scheduler_storage = Some(Default::default());
        s.vm.world.refresh = Some(sims_vm::refresh::RefreshState {
            draw: (1..=4).map(|id| (id, Default::default())).collect(),
            ..Default::default()
        });
        s.vm.world.globals.extend([(20, 0), (36, 0), (37, 0)]);
        for id in 1..=4 {
            let mut o = s.vm.world.objects.remove(&id).unwrap();
            o.native_flags = Some(0);
            s.vm.world.register_scheduler_object(id, &mut o).unwrap();
            s.vm.world.objects.insert(id, o);
        }
        let m = p.modules.get_mut("objects").unwrap();
        m.object_functions.get_mut(&1).unwrap()[3].action = 4097;
        let mut reset = m.routines[&4096].clone();
        reset.body.instructions = vec![
            Instruction {
                opcode: 2,
                on_true: 1,
                on_false: 255,
                operands: [36, 0, 1, 0, 0, 3, 6, 7],
            },
            Instruction {
                opcode: 8,
                on_true: 254,
                on_false: 255,
                operands: [0, 0, 8, 0, 100, 0, 7, 0],
            },
        ];
        m.routines.insert(4097, reset);
        for id in [1, 2, 3] {
            s.vm.start_object_lifecycle(&p, id).unwrap();
        }
        s.native_lot.as_mut().unwrap().indicators.primary = Some(4);
        s.vm.tick = 123;
        s.vm.lot_rng_state = Some(456);
        s.vm.crt_rng_state = Some(789);
        let mut expected_rng = s.vm.rng_state;
        for _ in 0..4 {
            sims_vm::rng::next(&mut expected_rng);
        }
        let removed = s
            .clear_for_house_read(&p, &mut |_, _| Err("unexpected asset work".into()))
            .unwrap();
        assert_eq!(removed, [4, 2, 1, 3]);
        assert_eq!(s.vm.world.globals[&36], 4);
        assert_eq!(
            (
                s.vm.tick,
                s.vm.rng_state,
                s.vm.lot_rng_state,
                s.vm.crt_rng_state
            ),
            (123, expected_rng, Some(456), Some(789))
        );
        assert!(s.vm.world.objects.is_empty() && s.vm.threads.is_empty());
        assert!(!s.vm.world.registry_ui_disabled);
        let before = s.vm.clone();
        assert!(
            s.clear_for_house_read(&p, &mut |_, _| unreachable!())
                .unwrap()
                .is_empty()
        );
        assert_eq!(s.vm, before);
    }
}
