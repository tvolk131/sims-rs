//! Character storage retained after registry unlink, until the asset host has
//! executed the native destructor. It is never a schedulable household actor.
use crate::{Simulation, character::Resident};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Retired {
    pub resident: Resident,
    pub person: BTreeMap<i16, i16>,
    pub tick: u64,
    pub asset_service_started: bool,
}

impl Simulation {
    pub fn begin_character_retirement(&mut self, object: ObjectId) -> Result<(), String> {
        if self.deletion.last().is_none_or(|cursor| {
            cursor.target != object || !cursor.storage_unlinked || cursor.assets_released
        }) {
            return Err("character retirement lost its deletion cursor".into());
        }
        let retired = self
            .retired_characters
            .get_mut(&object)
            .ok_or("retired character absent")?;
        if retired.asset_service_started {
            return Err("character retirement already started".into());
        }
        retired.asset_service_started = true;
        Ok(())
    }

    /// Called only after actual model/appearance services have finished. The
    /// deletion cursor remains suspended until its matching owner is released.
    pub fn complete_character_retirement(&mut self, object: ObjectId) -> Result<(), String> {
        let retired = self
            .retired_characters
            .get(&object)
            .ok_or("retired character absent")?;
        let cursor = self
            .deletion
            .last_mut()
            .ok_or("retirement deletion absent")?;
        if !retired.asset_service_started
            || cursor.target != object
            || !cursor.storage_unlinked
            || cursor.assets_released
        {
            return Err("stale character retirement completion".into());
        }
        cursor.assets_released = true;
        self.retired_characters.remove(&object);
        Ok(())
    }

    pub(crate) fn validate_retired_characters(&self, program: &Program) -> Result<(), String> {
        for (&id, retired) in &self.retired_characters {
            if retired.tick != self.vm.tick
                || self.native_character_settings.is_none()
                || self.characters.contains_key(&id)
                || self.vm.world.objects.contains_key(&id)
                || retired.resident.context.actor != id
                || retired.resident.source.is_none()
                || retired.resident.character.busy()
                || retired.resident.ordinary_update.is_some()
                || retired.resident.person_update.is_some()
                || retired.resident.refresh_tick.is_some()
                || self
                    .deletion
                    .last()
                    .is_none_or(|d| d.target != id || !d.storage_unlinked || d.assets_released)
            {
                return Err("invalid retired character owner".into());
            }
            retired.resident.character.validate(program)?;
        }
        Ok(())
    }
}
