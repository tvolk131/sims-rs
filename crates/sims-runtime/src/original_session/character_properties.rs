//! Retained inputs for 5912fd's property callback. Logical sound position is
//! 432595/4325b5/422632, independent of viewport visibility. This calculation
//! supplies callback inputs; it does not execute the location getter's other
//! physical-slot output/publication operations.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{
    character::Resident,
    pose::{properties, transform::Placement},
};
use sims_vm::{ObjectId, World};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// Current simulator +24, used by the original snow/footstep branches.
    /// This is not the zoning global or an assumed residential constant.
    pub lot_kind: i16,
    /// Actual process owner +5b, read by 5912fd.
    pub debug_sound: bool,
}
impl Source {
    pub fn context(
        self,
        id: ObjectId,
        resident: &Resident,
        world: &World,
    ) -> Result<properties::Context> {
        ensure!(
            resident.context.actor == id,
            "native property animator owner differs"
        );
        self.context_at(
            id,
            &resident.character.placement,
            resident.context.appearance_mode,
            world,
        )
    }

    pub fn context_at(
        self,
        id: ObjectId,
        placement: &Placement,
        appearance_mode: i32,
        world: &World,
    ) -> Result<properties::Context> {
        let object = world
            .objects
            .get(&id)
            .context("native property object absent")?;
        let flags = object
            .native_flags
            .context("native property object flags unknown")?;
        let position = world
            .positions
            .get(&id)
            .context("native property physical position absent")?;
        let size = world
            .map_size
            .as_ref()
            .context("native property map size unknown")?
            .active_size;
        ensure!(size >= 4, "native property map size unsupported");
        let inside = [position.x, position.y].into_iter().all(|n| {
            // 422632 narrows the arithmetic-shifted fixed coordinate to signed
            // char before comparing it against the actual active world size.
            let tile = (n >> 4) as i8 as i32;
            tile > 0 && tile < size - 1
        });
        Ok(properties::Context {
            actor: id,
            sound_visible: inside,
            selected: flags & 2 != 0,
            debug_sound: self.debug_sound,
            position_x: placement.base[0],
            appearance_mode,
            snow: (40..=42).contains(&self.lot_kind),
            lot_kind: self.lot_kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::original_session::completed_simulation::compose::Composed;
    use std::{fs, path::Path};

    #[test]
    fn pre_handoff_context_uses_actual_world_and_appearance_owner() -> Result<()> {
        let mut world = World::default();
        world.map_size = Some(sims_vm::world_map::State { active_size: 64 });
        world.objects.insert(
            7,
            sims_vm::Object {
                native_flags: Some(2),
                ..Default::default()
            },
        );
        world.positions.insert(
            7,
            sims_vm::autonomy::Placement {
                x: 16,
                y: 32,
                level: 1,
            },
        );
        let placement = Placement {
            base: [-999., 0., 0.],
            offset: [0.; 3],
            previous: [0.; 3],
            dirty: true,
        };
        let source = Source {
            lot_kind: 41,
            debug_sound: true,
        };
        let before = postcard::to_allocvec(&world)?;
        let context = source.context_at(7, &placement, -1, &world)?;
        assert!(context.sound_visible && context.selected && context.snow && context.debug_sound);
        assert_eq!(context.position_x, -999.);
        assert_eq!(
            context.appearance_mode, -1,
            "unselected constructed cache must not become outfit0"
        );
        assert_eq!(postcard::to_allocvec(&world)?, before);
        world.positions.get_mut(&7).unwrap().x = -16;
        world.objects.get_mut(&7).unwrap().native_flags = Some(0);
        let context = source.context_at(7, &placement, 5, &world)?;
        assert!(!context.sound_visible && !context.selected);
        assert_eq!(context.appearance_mode, 5);
        world.objects.get_mut(&7).unwrap().native_flags = None;
        assert!(source.context_at(7, &placement, 5, &world).is_err());
        world.objects.get_mut(&7).unwrap().native_flags = Some(0);
        world.positions.remove(&7);
        assert!(source.context_at(7, &placement, 5, &world).is_err());
        Ok(())
    }

    #[test]
    fn captured_current_properties_and_dynamic_world_inputs() -> Result<()> {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(|| -> Result<()> {
                let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let sim = root.join(
                    "fixtures/private/goth-simulation-composition-20261028-first/executed-first",
                );
                if !sim.join("refs.json").exists() {
                    return Ok(());
                }
                let refs: serde_json::Value =
                    serde_json::from_slice(&fs::read(sim.join("refs.json"))?)?;
                let state: Composed = sims_fixtures::Store::open(sim.join("store"))?
                    .get_value(&serde_json::from_value(refs["simulation"].clone())?)?;
                let input: serde_json::Value = serde_json::from_slice(&fs::read(root.join(
                    "fixtures/private/goth-character-residents-20261027-third/residents-input.json",
                ))?)?;
                let input = &input["resident_input"];
                let lot = u16::from_str_radix(input["lot_raw"].as_str().unwrap(), 16)?.swap_bytes()
                    as i16;
                let source = Source {
                    lot_kind: lot,
                    debug_sound: input["debug_raw"].as_u64().unwrap() != 0,
                };
                let world = &state.snapshot.state.vm.world;
                let before = postcard::to_allocvec(world)?;
                for (&id, resident) in &state.snapshot.state.characters {
                    let actual = source.context(id, resident, world)?;
                    ensure!(
                        actual == resident.context,
                        "captured property context changed for {id}"
                    );
                }
                ensure!(
                    before == postcard::to_allocvec(world)?,
                    "property input calculation mutated world"
                );
                let mut changed = world.clone();
                changed.objects.get_mut(&21).unwrap().native_flags = Some(0);
                changed.positions.get_mut(&21).unwrap().x = -16;
                let mut resident = state.snapshot.state.characters[&21].clone();
                resident.character.placement.base[0] = 13.5;
                resident.context.appearance_mode = 5;
                let context = source.context(21, &resident, &changed)?;
                ensure!(
                    !context.selected
                        && !context.sound_visible
                        && context.position_x == 13.5
                        && context.appearance_mode == 5
                        && context.lot_kind == 5,
                    "native property inputs did not follow current state"
                );
                changed.objects.get_mut(&21).unwrap().native_flags = None;
                ensure!(
                    source.context(21, &resident, &changed).is_err(),
                    "unknown flags were guessed"
                );
                Ok(())
            })?
            .join()
            .expect("captured property test panicked")
    }
}
