//! Generic Sims Call (opcode 1): one of several dozen built-in routines,
//! picked by operand word 0.
//!
//! Ported from the old prototype's `generic_person_call`
//! (legacy/crates/sims-vm/src/person.rs:627). Only the routines scripts have
//! reached so far are here; the rest fail loudly with their number.
//!
//! - 28: classify a lot. Temp 0 holds a lot number and receives 0 for a
//!   residential lot, 1 for a community lot and -1 for a lot the zoning
//!   table does not know. Lot 0 and the Magic Town lots 90 to 92 are
//!   residential and 93 to 98 community whatever the table says.
//! - 10 and 11: add the running object to the lot's list of burning
//!   fires, or take it off (0x44fd29, 0x44fd3a). Ported from the old
//!   prototype's `fires::State` (legacy/crates/sims-vm/src/fires.rs).
//! - 43: raise the magic controller's attributes to the selected family's
//!   magic history (0x410af6). Ported from the old prototype's `restore`
//!   (legacy/crates/sims-vm/src/magic_history.rs). It does nothing unless
//!   the game holds a family's history, that family is the one in global
//!   9, and the lot in global 10 is lot 0 or zoned residential (the raw
//!   zoning table, without routine 28's fixed lots). Then it asks for the
//!   instances of the controller's type, which makes that type's
//!   instance list if it has none (0x445857), and each of the first
//!   instance's first six attributes becomes the larger of itself and the
//!   family's word.

use crate::fault::Trap;
use crate::interpreter::{Machine, word};

impl Machine<'_, '_> {
    pub(crate) fn generic_call(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        match word(operands, 0) {
            28 => {
                let zoning = &self.program.zoning;
                let temps = &mut self.me_mut()?.temps;
                let lot = temps
                    .first_mut()
                    .ok_or_else(|| Trap::Invalid("object has no temps".into()))?;
                *lot = match *lot {
                    0 | 90..=92 => 0,
                    93..=98 => 1,
                    number => match zoning.get(&number) {
                        Some(1) => 0,
                        Some(2) => 1,
                        // A lot missing from the table reads as zone 0.
                        _ => -1,
                    },
                };
                Ok(true)
            }
            10 => {
                self.world.fires.push(self.me);
                Ok(true)
            }
            11 => {
                let me = self.me;
                if let Some(place) = self.world.fires.iter().position(|&id| id == me) {
                    self.world.fires.remove(place);
                }
                Ok(true)
            }
            43 => {
                self.restore_magic_history();
                Ok(true)
            }
            routine => Err(Trap::Unsupported(format!(
                "Generic Sims Call routine {routine}"
            ))),
        }
    }
}

/// The type of the object that keeps a lot's magic state.
const MAGIC_CONTROLLER: u32 = 0xb6c9_0029;

impl Machine<'_, '_> {
    fn restore_magic_history(&mut self) {
        let world = &mut *self.world;
        let Some(history) = &world.magic_history else {
            return;
        };
        let (family, lot) = (world.globals[9], world.globals[10]);
        if family == 0 || history.family == 0 || i32::from(family) != history.family {
            return;
        }
        if lot != 0 && self.program.zoning.get(&lot) != Some(&1) {
            return;
        }
        let values = history.values;
        let objects = &world.objects;
        let instances = world.instances.entry(MAGIC_CONTROLLER).or_insert_with(|| {
            objects
                .iter()
                .filter(|(_, object)| object.guid == MAGIC_CONTROLLER)
                .map(|(&id, _)| id)
                .collect()
        });
        let Some(controller) = instances.first().and_then(|id| world.objects.get_mut(id)) else {
            return;
        };
        for (attribute, value) in controller.attributes.iter_mut().zip(values) {
            *attribute = (*attribute).max(value);
        }
    }
}
