//! Explicit host/view-boundary room updates for imported households.
use super::{
    Tuning,
    manager::{Manager, Settings},
};
use crate::{Simulation, clock::ClockEvent};
use serde::{Deserialize, Serialize};
use sims_vm::{Program, person_loading::RoomEffect};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Live {
    pub manager: Manager,
    /// Other consumers retain the ordered notifications in the lot/clock logs.
    pub room_effects_consumed: usize,
    pub clock_events_consumed: usize,
}

impl Simulation {
    pub(crate) fn validate_original_environment(&self, program: &Program) -> Result<(), String> {
        let Some(live) = &self.room_environment else {
            return Ok(());
        };
        let lot = self
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("saved environment has no original lot")?;
        let native = self
            .native_lot
            .as_ref()
            .ok_or("saved environment has no native state")?;
        let household = self
            .household
            .as_ref()
            .ok_or("saved environment has no clock")?;
        let refresh = self
            .vm
            .world
            .refresh
            .as_ref()
            .ok_or("saved environment has no refresh directory")?;
        let m = &live.manager;
        let expected = Manager::new(lot, Tuning::from_program(program)?, m.settings)?;
        if live.room_effects_consumed > native.room_effects.len()
            || live.clock_events_consumed > household.clock_events.len()
            || m.size != lot.size
            || m.tuning != expected.tuning
            || m.light_maps.len() != lot.floors.len()
            || m.light_maps.iter().any(|map| map.cells.len() != 4096)
            || m.light_bytes.len() != lot.floors.len()
            || m.light_bytes
                .iter()
                .any(|map| map.len() != lot.size * lot.size)
            || !m.rooms.keys().eq(expected.rooms.keys())
            || !m.rooms.keys().eq(refresh.rooms.keys())
        {
            return Err("invalid saved room environment context".into());
        }
        for (&id, room) in &m.rooms {
            let expected = &expected.rooms[&id];
            if room.tiles != expected.tiles
                || room.outdoors != expected.outdoors
                || room.models.len() > 6
                || room.positions.len() != room.models.len()
                || room.models != m.settings.models[..room.models.len()]
                || room
                    .positions
                    .iter()
                    .flatten()
                    .any(|&v| !f32::from_bits(v).is_finite())
            {
                return Err("invalid saved room state".into());
            }
        }
        Ok(())
    }
    /// Install after the ordered original load/reset tail. Apply recorded
    /// occupancy changes before the initial DD and explicit room update.
    pub fn enable_original_environment(
        &mut self,
        program: &Program,
        settings: Settings,
    ) -> Result<(), String> {
        if self.room_environment.is_some() {
            return Err("room environment already enabled".into());
        }
        let lot = self
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("original lot absent")?;
        let household = self.household.as_ref().ok_or("household clock absent")?;
        let period = household.clock.period;
        let live = Live {
            manager: Manager::new(lot, Tuning::from_program(program)?, settings)?,
            room_effects_consumed: 0,
            clock_events_consumed: household.clock_events.len(),
        };
        self.room_environment = Some(live);
        self.apply_original_room_effects()?;
        self.room_environment
            .as_mut()
            .unwrap()
            .manager
            .lighting_changed(&mut self.vm, period)?;
        self.update_original_environment(program)?;
        Ok(())
    }

    pub(crate) fn apply_original_room_effects(&mut self) -> Result<(), String> {
        let Some(live) = &mut self.room_environment else {
            return Ok(());
        };
        let native = self
            .native_lot
            .as_ref()
            .ok_or("original room membership absent")?;
        if live.room_effects_consumed > native.room_effects.len() {
            return Err("room event cursor outside log".into());
        }
        while let Some(effect) = native.room_effects.get(live.room_effects_consumed) {
            if let RoomEffect::Occupied { room, occupied } = *effect {
                live.manager.occupied(&mut self.vm, room, occupied)?;
            }
            live.room_effects_consumed += 1;
        }
        Ok(())
    }

    /// Called by the host between updates, not implicitly by Environment
    /// queries. RoomMembership's occupancy publication remains synchronous.
    pub fn update_original_environment(&mut self, program: &Program) -> Result<bool, String> {
        self.apply_original_room_effects()?;
        let live = self
            .room_environment
            .as_mut()
            .ok_or("room environment absent")?;
        let household = self.household.as_ref().ok_or("household clock absent")?;
        if live.clock_events_consumed > household.clock_events.len() {
            return Err("room clock cursor outside log".into());
        }
        for event in &household.clock_events[live.clock_events_consumed..] {
            if matches!(event, ClockEvent::Signal { kind: 0xdd, .. }) {
                live.manager
                    .lighting_changed(&mut self.vm, household.clock.period)?;
            }
        }
        live.clock_events_consumed = household.clock_events.len();
        let lot = self
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("original lot absent")?;
        let native = self.native_lot.as_ref().ok_or("original links absent")?;
        live.manager.update(
            &mut self.vm,
            program,
            lot,
            &native.placement.links,
            household.clock.period,
        )
    }
}
