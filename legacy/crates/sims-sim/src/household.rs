//! Headless lot clock and person-update integration. Enabling this service is
//! explicit while authored laboratory fixtures coexist with household scenarios.
use crate::{
    Simulation,
    clock::{ClockEvent, LotClock},
};
use serde::{Deserialize, Serialize};
use sims_vm::{
    ObjectId, Program, Status, ThreadId,
    motives::{Tuning, UpdateState},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Household {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valuation: Option<crate::house_valuation::Retained>,
    #[serde(default)]
    pub house_statistics: Option<crate::house_statistics::State>,
    pub clock: LotClock,
    pub tuning: Tuning,
    pub people: BTreeMap<ObjectId, PersonUpdate>,
    /// Pending world/presentation notifications. Their consumers remain separate
    /// from the clock and must not cause a second calendar advance.
    pub clock_events: Vec<ClockEvent>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonUpdate {
    pub started: Option<u64>,
    pub finished: Option<u64>,
}

impl Simulation {
    pub fn enable_household(
        &mut self,
        program: &Program,
        mut clock: LotClock,
    ) -> Result<(), String> {
        if self.household.is_some() {
            return Err("household updates already enabled".into());
        }
        clock.validate()?;
        clock.ticks = self.vm.tick as u32;
        let tuning = Tuning::from_program(program)?;
        let mut people = BTreeMap::new();
        for (&id, object) in &self.vm.world.objects {
            let Some(person) = &object.person else {
                continue;
            };
            if !self.vm.object_threads.contains_key(&id) {
                return Err("household person has no registered Main thread".into());
            }
            if [3, 6, 27, 29, 58, 65]
                .iter()
                .any(|key| !person.contains_key(key))
                || !object.data.contains_key(&34)
            {
                return Err("household person update fields are incomplete".into());
            }
            people.insert(
                id,
                PersonUpdate {
                    started: None,
                    finished: None,
                },
            );
        }
        for &id in people.keys() {
            let object = self.vm.world.objects.get_mut(&id).unwrap();
            let previous = object.motives.ok_or("household person has no motives")?;
            object.motive_update.get_or_insert(UpdateState {
                previous,
                last_decay: self.vm.tick as i32,
            });
        }
        self.household = Some(Household {
            valuation: None,
            house_statistics: None,
            clock,
            tuning,
            people,
            clock_events: vec![],
        });
        self.publish_clock();
        self.publish_budget_statistics();
        Ok(())
    }

    pub(crate) fn begin_person_update(&mut self, thread: ThreadId) {
        let Some(household) = &mut self.household else {
            return;
        };
        let Some((&actor, _)) = self.vm.object_threads.iter().find(|(_, id)| **id == thread) else {
            return;
        };
        let Some(update) = household.people.get_mut(&actor) else {
            return;
        };
        let explicit_visit = self
            .vm
            .scheduled_object_visit
            .as_ref()
            .is_some_and(|v| v.object() == actor && v.thread() == thread);
        if (!explicit_visit && update.finished == Some(self.vm.tick))
            || update.started == Some(self.vm.tick)
        {
            return;
        }
        let t = &self.vm.threads[&thread];
        if t.check
            || matches!(t.status, Status::Returned(_) | Status::Faulted(_))
            || (!self.characters.contains_key(&actor)
                && matches!(t.status,Status::Sleeping{wake_tick} if wake_tick > self.vm.tick))
        {
            return;
        }
        if self.vm.world.objects[&actor]
            .idle
            .as_ref()
            .is_some_and(|idle| idle.countdown > 0 && idle.last_run != Some(self.vm.tick))
        {
            return;
        }
        if explicit_visit {
            update.finished = None;
        }
        update.started = Some(self.vm.tick);
    }

    pub(crate) fn finish_person_update(&mut self, thread: ThreadId) -> Result<(), String> {
        let Some(household) = &mut self.household else {
            return Ok(());
        };
        let Some((&actor, _)) = self.vm.object_threads.iter().find(|(_, id)| **id == thread) else {
            return Ok(());
        };
        let Some(update) = household.people.get_mut(&actor) else {
            return Ok(());
        };
        if update.started != Some(self.vm.tick) {
            return Ok(());
        }
        let object = self.vm.world.objects.get_mut(&actor).unwrap();
        let fields = object
            .person
            .as_mut()
            .ok_or("person update lost its fields")?;
        let mut person = [0i16; 256];
        for (&key, &value) in fields.iter() {
            if let Some(dest) = person.get_mut(key as usize) {
                *dest = value;
            }
        }
        object
            .motive_update
            .as_mut()
            .ok_or("missing person motive update state")?
            .finish(
                object
                    .motives
                    .as_mut()
                    .ok_or("person update lost motives")?,
                &mut person,
                object.data[&34] != 0,
                self.vm.tick as i32,
                household.clock.hour,
                &household.tuning,
                // Until room scoring is implemented this uses native room -5's
                // explicit preserve-environment path, not a fabricated room score.
                None,
                &object.motive_changes,
            );
        fields.insert(27, person[27]);
        update.started = None;
        update.finished = Some(self.vm.tick);
        Ok(())
    }

    pub(crate) fn advance_household_clock(&mut self, complete: bool) {
        if let Some(h) = &mut self.household {
            let events = h.clock.finish_tick(complete);
            for event in &events {
                if matches!(event, ClockEvent::Signal { kind: 0xdc, .. })
                    && let Some(statistics) = &mut h.house_statistics
                {
                    statistics.new_day();
                }
            }
            // The event remains for other subscribers, including valuation.
            h.clock_events.extend(events);
            if let Some(b) = &mut self.vm.world.budget {
                b.expenses[..8].copy_from_slice(&h.clock.statistics[5]);
            }
        }
        self.publish_clock();
    }
    pub(crate) fn publish_budget_statistics(&mut self) {
        if let (Some(h), Some(b)) = (&mut self.household, &self.vm.world.budget) {
            h.clock.statistics[5].copy_from_slice(&b.expenses[..8]);
        }
    }
    fn publish_clock(&mut self) {
        let Some(h) = &self.household else { return };
        let c = &h.clock;
        for (index, value) in [
            (0, c.hour),
            (1, c.day),
            (4, c.period),
            (5, c.minute),
            (6, c.second),
            (7, c.month),
            (8, c.year),
            (28, c.elapsed_days),
            (29, c.month_days),
        ] {
            self.vm.world.globals.insert(index, value);
        }
    }
    pub(crate) fn validate_household(&self, program: &Program) -> Result<(), String> {
        let Some(h) = &self.household else {
            return Ok(());
        };
        h.clock.validate()?;
        if self
            .vm
            .world
            .budget
            .as_ref()
            .is_some_and(|b| b.expenses[..8] != h.clock.statistics[5])
        {
            return Err("household budget statistics mismatch".into());
        }
        h.tuning.validate_cached()?;
        if self.vm.world.scheduler_storage.is_none() {
            // Authored fixture profiles bind their tuning to Program inputs.
            // Native caches instead retain the actual normalized globals and
            // loaded curves (420c36/42138d), which may differ from resources.
            let mut expected_tuning = Tuning::from_program(program)?;
            expected_tuning.update_precision = h.tuning.update_precision;
            if h.tuning != expected_tuning {
                return Err("household clock or tuning binding mismatch".into());
            }
        }
        if h.clock.ticks != self.vm.tick as u32 {
            return Err("household clock or tuning binding mismatch".into());
        }
        if h.clock_events.len() > 1_000_000 {
            return Err("oversized household clock event queue".into());
        }
        let c = &h.clock;
        for (index, value) in [
            (0, c.hour),
            (1, c.day),
            (4, c.period),
            (5, c.minute),
            (6, c.second),
            (7, c.month),
            (8, c.year),
            (28, c.elapsed_days),
            (29, c.month_days),
        ] {
            if self.vm.world.globals.get(&index) != Some(&value) {
                return Err("household clock globals mismatch".into());
            }
        }
        if self
            .vm
            .world
            .objects
            .iter()
            .any(|(id, o)| o.person.is_some() != h.people.contains_key(id))
        {
            return Err("household person registry mismatch".into());
        }
        for (&actor, u) in &h.people {
            let object = self
                .vm
                .world
                .objects
                .get(&actor)
                .ok_or("missing household person")?;
            if object.person.is_none()
                || object.motives.is_none()
                || object
                    .person
                    .as_ref()
                    .is_some_and(|p| [3, 6, 27, 29, 58, 65].iter().any(|k| !p.contains_key(k)))
                || !object.data.contains_key(&34)
                || (!self.vm.object_threads.contains_key(&actor)
                    && !self.person_factories.contains_key(&actor))
                || u.started.is_some_and(|t| t != self.vm.tick)
                || u.finished.is_some_and(|t| t > self.vm.tick)
                || (u.started.is_some() && u.finished == u.started)
                || object.motive_update.is_none()
            {
                return Err("invalid household person update".into());
            }
        }
        Ok(())
    }
}

impl Simulation {
    /// The authored household advertises newly placed objects just as it does
    /// its initial furnishings. Native registry-flag construction remains a host boundary.
    pub(crate) fn advertise_placed_object(&mut self, id: ObjectId) {
        if let Some(body) = self.bodies.get(&id) {
            if let Some(c) = self
                .vm
                .world
                .person
                .as_mut()
                .and_then(|p| p.autonomy.as_mut())
            {
                if let Some(set) = &mut c.advertised {
                    set.insert(id);
                }
                c.placements.insert(
                    id,
                    sims_vm::autonomy::Placement {
                        x: body.position.y,
                        y: body.position.x,
                        level: body.level,
                    },
                );
            }
        }
    }
    /// Enable the residential autonomy service on an authored residential lot.
    /// The host supplies advertisement-registry membership; this is not imported
    /// native registry construction. Physical poses remain owned by Simulation.
    pub fn enable_residential_autonomy(
        &mut self,
        advertised: impl IntoIterator<Item = ObjectId>,
    ) -> Result<(), String> {
        let advertised: std::collections::BTreeSet<_> = advertised.into_iter().collect();
        if self.household.is_none()
            || advertised
                .iter()
                .any(|id| !self.vm.world.objects.contains_key(id))
        {
            return Err("residential autonomy requires an initialized household and existing advertised objects".into());
        }
        let state = self
            .vm
            .world
            .person
            .as_mut()
            .ok_or("person world unavailable")?;
        if state.autonomy.is_some() {
            return Err("autonomy context already supplied".into());
        }
        state.autonomy = Some(sims_vm::autonomy::Context {
            residential: true,
            advertised: Some(advertised),
            full_scan: Some(Default::default()),
            placements: self
                .bodies
                .iter()
                .map(|(&id, b)| {
                    (
                        id,
                        sims_vm::autonomy::Placement {
                            x: b.position.y,
                            y: b.position.x,
                            level: b.level,
                        },
                    )
                })
                .collect(),
            ..Default::default()
        });
        self.vm.world.tree_advertisements.resize(16, [0; 3]);
        state.free_will = true;
        Ok(())
    }
}
