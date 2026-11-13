//! The load steps that run scripts. `docs/roadmap.md` lists every step of
//! the original's lot load in order; the ones that only change state live
//! in `sims_sim::load`.

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::program::Callback;
use sims_formats::lot::{ObjectType, ObjectTypes};
use sims_sim::World;
use std::collections::BTreeMap;

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
    /// so a lot the game holds always has them. Their IDs go to
    /// [`World::indicators`] and are returned.
    ///
    /// Only the creation is ported. The original also attaches the markers
    /// to Sims later.
    pub fn create_indicators(&self, world: &mut World) -> Result<[u16; 2], Fault> {
        let mut ids = [0; 2];
        for (id, guid) in ids.iter_mut().zip(INDICATORS) {
            *id = self.create(world, guid)?;
            if let Some(object) = world.objects.get_mut(id) {
                // Hidden: flag 0x4000 of data word 8.
                object.data[8] |= 0x4000;
            }
        }
        world.indicators = ids;
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

    /// Whether a saved type's scripts changed since the house was saved
    /// (0x43a516), or None for a GUID the catalog has no definition for.
    ///
    /// The save holds a signature of the type's Init tree and one of its
    /// Main tree ([`crate::Program::signature`]). The type changed if the
    /// linked Init signature differs from the saved one, or else if the
    /// Main signatures differ. The original does not work out the Main
    /// signature when Init already differs.
    ///
    /// A type table older than version 1 holds no signatures, and there
    /// the original flags exactly the people (type 2). The reader takes
    /// only versions 2 and 3, so that rule is not ported.
    pub fn type_changed(&self, saved: &ObjectType) -> Option<bool> {
        let definition = self.catalog.object(saved.guid)?;
        let signature = |callback| {
            self.program.signature(
                definition.resource,
                self.program.callback(definition, callback),
            )
        };
        Some(
            signature(Callback::Init) != saved.init_signature
                || signature(Callback::Main) != saved.main_signature,
        )
    }

    /// Reads a house's type table onto the lot (0x43dd76) and marks the
    /// types whose scripts changed since the save (0x43a516). The original
    /// does both in one pass, after it has read the lot arrays and before
    /// it reads the objects.
    ///
    /// For each saved type with a definition, in the file's order, the
    /// definition takes the saved type ID and its changed flag is set or
    /// cleared in [`World::changed_types`] ([`Interpreter::type_changed`]).
    /// A type the table does not list keeps the flag it had. A saved type
    /// with no definition gets no flag and no entry.
    ///
    /// Then [`World::types`] is rebuilt from the definitions' saved IDs, in
    /// catalog order. When two saved types name one definition, the
    /// definition keeps the later ID and the earlier ID names nothing.
    ///
    /// The lot must hold no objects, because the frames of an object name
    /// their code by the table this replaces. Returns the GUIDs flagged as
    /// changed, in the file's order.
    ///
    /// Ported from the old prototype's `Execution::step`
    /// (legacy/crates/sims-sim/src/saved_types.rs), which was compared with
    /// the original's 0x43a516 run in isolation on authored records. Three
    /// results were also compared with a capture of the original loading
    /// the Goth house; see `docs/decisions.md`. The rest is
    /// **unverified**.
    ///
    /// Not ported: a GUID translation table in the house file
    /// (`sims_content::House::guid_table`), which the caller must refuse;
    /// and type ID 0, which in the original
    /// ends up naming the last definition in the catalog that no saved type
    /// names.
    pub fn read_types(&self, world: &mut World, saved: &ObjectTypes) -> Result<Vec<u32>, Fault> {
        if let Some((&id, _)) = world.objects.first_key_value() {
            return Err(Fault {
                object: id,
                at: None,
                trap: Trap::Unsupported(
                    "reading a type table onto a lot that holds objects".into(),
                ),
            });
        }
        // Each definition's saved type ID, by its place in the catalog.
        let mut saved_ids = BTreeMap::new();
        let mut changed = Vec::new();
        for entry in &saved.entries {
            let Some(&place) = self.catalog.by_guid.get(&entry.guid) else {
                continue;
            };
            saved_ids.insert(place, entry.id);
            if self.type_changed(entry) == Some(true) {
                world.changed_types.insert(entry.guid);
                changed.push(entry.guid);
            } else {
                world.changed_types.remove(&entry.guid);
            }
        }
        world.types = saved_ids
            .into_iter()
            .map(|(place, id)| (id, self.catalog.objects[place].guid))
            .collect();
        Ok(changed)
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
