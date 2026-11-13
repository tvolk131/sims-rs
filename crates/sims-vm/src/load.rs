//! The load steps that run scripts. `docs/roadmap.md` lists every step of
//! the original's lot load in order; the ones that only change state live
//! in `sims_sim::load`.

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use sims_sim::World;

/// The two indicator objects the original keeps on every lot: the marker
/// over the selected Sim and the one over a date (0x445e4a).
pub const INDICATORS: [u32; 2] = [0x2f72_6fff, 0x0baa_71c5];

/// Definition word 42: nonzero for an object the lot has one of, wherever
/// its tiles are. With type 7 or 12 (word 9) that makes a lot-wide
/// controller.
const GLOBAL_WORD: usize = 42;

/// Global 27: the area of the lot's indoor rooms, in tiles.
const INDOOR_AREA: usize = 27;

impl Interpreter<'_> {
    /// Creates the two indicator objects through the factory and hides
    /// them (0x445e4a). The original does this at the end of every load,
    /// so a lot the game holds always has them. Returns their IDs.
    ///
    /// Only the creation is ported. The original also records the IDs for
    /// the interface and attaches the markers to Sims later.
    pub fn create_indicators(&self, world: &mut World) -> Result<[u16; 2], Fault> {
        let mut ids = [0; 2];
        for (id, guid) in ids.iter_mut().zip(INDICATORS) {
            *id = self.create(world, guid)?;
            if let Some(object) = world.objects.get_mut(id) {
                // Hidden: flag 0x4000 of data word 8.
                object.data[8] |= 0x4000;
            }
        }
        Ok(ids)
    }

    /// Resizes the lot the game is holding to the size the house asks for
    /// in global 23, where 0 means the full 64 tiles (0x415e20). Call it
    /// after the house's simulator state is read and the context applied,
    /// and before anything else of the house is read.
    ///
    /// A resize rebuilds the lot's rooms, and the object registry answers
    /// that by creating every lot-wide controller the lot lacks
    /// ([`Interpreter::reconcile_controllers`]). Returns the IDs it
    /// created. A lot that already has the size, or a request below 4
    /// tiles, changes nothing and creates nothing.
    ///
    /// Ported from the old prototype's `prepare_cleared_inner`
    /// (legacy/crates/sims-content/src/house_startup.rs), which reproduced a
    /// capture of the original at the end of this step. Only the case of
    /// that capture is ported: a bare lot, one with no arrays in
    /// [`World::arrays`], shrinking, with every object off the lot. Then
    /// there is nothing to shrink but [`World::size`], no object to
    /// delete, and no indoor room, so the rebuild leaves global 27 (the
    /// lot's indoor area) at 0. Anything else fails. The rooms themselves
    /// and their tile counts are not modeled.
    pub fn resize(&self, world: &mut World) -> Result<Vec<u16>, Fault> {
        let fail = |message: &str| Fault {
            object: 0,
            at: None,
            trap: Trap::Unsupported(message.into()),
        };
        if world.globals.len() != 64 {
            return Err(fail("the load context has not been applied"));
        }
        let requested = match world.globals[23] {
            0 => sims_sim::FULL_SIZE,
            size => size,
        };
        if requested < 4 || requested == world.size {
            return Ok(Vec::new());
        }
        if requested > world.size {
            return Err(fail("growing a lot is not ported"));
        }
        if world.objects.values().any(|o| o.position[0] >= 0) {
            return Err(fail("resizing a lot with objects on it is not ported"));
        }
        if !world.arrays.is_empty() {
            return Err(fail(
                "rebuilding the rooms of a lot with arrays is not ported",
            ));
        }
        world.size = requested;
        // The room rebuild writes the lot's indoor area to global 27
        // (0x466c3c). A bare lot has no indoor room.
        world.globals[INDOOR_AREA] = 0;
        self.reconcile_controllers(world)
    }

    /// Makes the lot hold exactly one of every lot-wide controller
    /// (0x442b9f): every definition in the catalog of type 7 or 12 with a
    /// nonzero global word, in catalog order. An existing instance is
    /// kept; a missing one is created through the factory, which runs its
    /// Init and its first turn before the next definition is looked at.
    /// Returns the IDs created, in creation order.
    ///
    /// The original takes its list of existing instances once, so a
    /// controller created here never counts as existing. It deletes later
    /// duplicates of an instance; deletion is not ported, so a duplicate
    /// fails. It also notes the ID of the animation tester (GUID
    /// 0x3EEC206C) for the interface, which is not simulation state.
    ///
    /// Ported from the old prototype (legacy/crates/sims-vm/src/global_load.rs,
    /// `reconcile_global_controllers` in
    /// legacy/crates/sims-content/src/save_storage.rs).
    pub fn reconcile_controllers(&self, world: &mut World) -> Result<Vec<u16>, Fault> {
        let fail = |trap| Fault {
            object: 0,
            at: None,
            trap,
        };
        // Existing instances by definition, in registration order.
        let mut existing = Vec::new();
        for &id in &world.order {
            let Some(object) = world.objects.get(&id) else {
                continue;
            };
            let definition = self.catalog.object(object.guid).ok_or_else(|| {
                fail(Trap::Invalid(format!(
                    "object {id} has no definition for GUID 0x{:08X}",
                    object.guid
                )))
            })?;
            existing.push((definition.resource, definition.chunk));
        }
        let mut created = Vec::new();
        for definition in &self.catalog.objects {
            let Some(words) = self
                .program
                .definition(definition.resource, definition.chunk)
            else {
                return Err(fail(Trap::Invalid(format!(
                    "the program does not hold the definition of GUID 0x{:08X}; \
                     link the whole catalog",
                    definition.guid
                ))));
            };
            if !matches!(words[9], 7 | 12) || words[GLOBAL_WORD] == 0 {
                continue;
            }
            let identity = (definition.resource, definition.chunk);
            match existing.iter().filter(|&&e| e == identity).count() {
                0 => {}
                1 => continue,
                _ => {
                    return Err(fail(Trap::Unsupported(format!(
                        "the lot has duplicates of controller 0x{:08X}; deleting them is not \
                         ported",
                        definition.guid
                    ))));
                }
            }
            // Objects carry a GUID, not a definition. Two definitions with
            // one GUID cannot be told apart here.
            if self
                .catalog
                .object(definition.guid)
                .is_none_or(|d| (d.resource, d.chunk) != identity)
            {
                return Err(fail(Trap::Unsupported(format!(
                    "controller GUID 0x{:08X} names more than one definition",
                    definition.guid
                ))));
            }
            created.push(self.create(world, definition.guid)?);
        }
        Ok(created)
    }
}
