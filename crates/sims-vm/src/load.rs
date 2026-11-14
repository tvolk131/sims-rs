//! The load steps that run scripts. `docs/roadmap.md` lists every step of
//! the original's lot load in order; the ones that only change state live
//! in `sims_sim::load` and `sims_sim::rooms`.

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::program::{Callback, Tree};
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

/// Data word 29: the room an object stands in.
const ROOM: usize = 29;

/// What [`Interpreter::assign_rooms`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roomed {
    /// The objects whose room word changed in the first pass, in
    /// registration order: the ID, the old room and the new one.
    pub moved: Vec<(u16, i16, i16)>,
    /// The objects whose function 7 tree ran in the second pass, in the
    /// order they ran.
    pub asked: Vec<u16>,
}

/// What [`Interpreter::reconcile_controllers`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciled {
    /// The IDs of the controllers created, in creation order.
    pub created: Vec<u16>,
    /// The IDs of the duplicates removed, in removal order.
    pub removed: Vec<u16>,
}

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
    /// ([`Interpreter::reconcile_controllers`]). Returns what that did. A
    /// lot that already has the size, or a request below 4 tiles, changes
    /// nothing and creates nothing.
    ///
    /// Ported from the old prototype's `prepare_cleared_inner`
    /// (legacy/crates/sims-content/src/house_startup.rs), which reproduced a
    /// capture of the original at the end of this step. Only the case of
    /// that capture is ported: a bare lot, one with no arrays in
    /// [`World::arrays`], shrinking, with every object off the lot. Then
    /// there is nothing to shrink but [`World::size`], no object to
    /// delete, and no indoor room, so the rebuild leaves global 27 (the
    /// lot's indoor area) at 0. Anything else fails.
    ///
    /// The bare lot's rooms are not built, because the lot has no arrays
    /// to work them out from: [`World::rooms`] stays as it was until the
    /// load works the rooms out from the house's walls
    /// ([`World::rebuild_rooms`]). In a trace of the original the lot has
    /// two rooms after this step, one for each level. No script that runs
    /// before the load's own rebuild reads a room.
    pub fn resize(&self, world: &mut World) -> Result<Reconciled, Fault> {
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
            return Ok(Reconciled::default());
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

    /// Load step 10: gives every object its room and then tells every
    /// object that the rooms changed (0x442659). The original runs this
    /// whenever the rooms have been worked out again
    /// ([`World::rebuild_rooms`]), so call it after that.
    ///
    /// Two passes over the registration list. The first puts an object
    /// whose level is 0 on level 1 and writes the room it stands in
    /// ([`World::room_at`]) into its data word 29. The second runs each
    /// object's function 7 tree, as a callback: synchronously, on top of
    /// whatever the object's thread holds, with the object's temps as
    /// arguments and no stack object. An object whose definition names no
    /// such tree, or names one that is missing, runs nothing. A tree may
    /// create objects, which go to the head of the list and are not
    /// visited. If a tree removes the object being visited, the original
    /// would follow a freed link; here that fails.
    ///
    /// For the Goth house this was compared with a trace of the original:
    /// the original asks all 382 objects in registration order, the same
    /// 62 have a tree here as change their thread there, no data word
    /// changes in either pass, and the lot afterwards equals the trace's.
    /// See `docs/decisions.md`.
    ///
    /// Not modeled: the first pass also tells the room a person stands in
    /// that it is occupied, which lights the room. The original's level
    /// argument, which limits both passes to one level, is not ported:
    /// this is the call for both levels.
    ///
    /// Ported from the old prototype's `update_loaded_rooms`
    /// (legacy/crates/sims-content/src/save_storage.rs).
    pub fn assign_rooms(&self, world: &mut World) -> Result<Roomed, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let mut done = Roomed::default();
        for id in world.order.clone() {
            let Some(object) = world.objects.get(&id) else {
                continue;
            };
            let mut position = object.position;
            if position[2] == 0 {
                position[2] = 1;
            }
            let room = world
                .room_at(position)
                .map_err(|error| fail(id, Trap::Unsupported(error.to_string())))?;
            if let Some(object) = world.objects.get_mut(&id) {
                object.position = position;
                let word = object
                    .data
                    .get_mut(ROOM)
                    .ok_or_else(|| fail(id, Trap::Invalid("object has no data word 29".into())))?;
                if *word != room {
                    done.moved.push((id, *word, room));
                    *word = room;
                }
            }
        }
        let mut current = world.order.first().copied();
        while let Some(id) = current {
            let object = world
                .objects
                .get(&id)
                .ok_or_else(|| fail(id, Trap::native(10, "missing object")))?;
            let definition = self
                .catalog
                .object(object.guid)
                .ok_or_else(|| fail(id, Trap::Invalid("object has no definition".into())))?;
            let tree = self.program.callback(definition, Callback::RoomsChanged);
            let exists =
                tree != 0 && !matches!(self.program.tree(definition.resource, tree), Tree::Missing);
            self.callback(world, id, Callback::RoomsChanged as usize, 0)?;
            if exists {
                done.asked.push(id);
            }
            // The next object is read from the list after the tree ran.
            let place = world.order.iter().position(|&other| other == id);
            let place = place.ok_or_else(|| {
                fail(
                    id,
                    Trap::Unsupported(
                        "an object removed itself while it was told the rooms changed".into(),
                    ),
                )
            })?;
            current = world.order.get(place + 1).copied();
        }
        Ok(done)
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
    /// nonzero global word, in catalog order. The first instance in the
    /// registration list is kept. Each later one is removed at once
    /// ([`Interpreter::remove_now`], 0x44123f), which runs its Reset tree.
    /// A missing one is created through the factory, which runs its Init
    /// and its first turn. All of that is done for one definition before
    /// the next is looked at.
    ///
    /// This is load step 8, run among the saved objects, and it is what a
    /// resize runs (step 3).
    ///
    /// The original takes its list of existing instances once, at the
    /// start: the objects whose definition has a nonzero global word, in
    /// registration order. A controller created here never joins it. After
    /// it removes a duplicate it scans the list again from the start for
    /// the same definition. If a script run on the way has removed an
    /// object that list names, the original would use freed memory; here
    /// that fails.
    ///
    /// The original also notes the ID of the animation tester (GUID
    /// 0x3EEC206C) for the interface, which is not simulation state.
    ///
    /// For the Goth house the creation was compared with captures of the
    /// original, on a bare lot (step 3) and among the saved objects and
    /// people (step 8); see `docs/decisions.md`. That house has no
    /// duplicate. Removing duplicates is **unverified** against the
    /// running game: it follows the old prototype, which compared its
    /// selection loop with the original's 0x442b9f run alone on made-up
    /// catalogs, with the factory and the removal stubbed out.
    ///
    /// Ported from the old prototype (`reconcile` in
    /// legacy/crates/sims-vm/src/global_load.rs,
    /// `reconcile_global_controllers` in
    /// legacy/crates/sims-content/src/save_storage.rs).
    pub fn reconcile_controllers(&self, world: &mut World) -> Result<Reconciled, Fault> {
        let fail = |trap| Fault {
            object: 0,
            at: None,
            trap,
        };
        let words = |definition: &sims_content::Object| {
            self.program
                .definition(definition.resource, definition.chunk)
                .ok_or_else(|| {
                    fail(Trap::Invalid(format!(
                        "the program does not hold the definition of GUID 0x{:08X}; \
                         link the whole catalog",
                        definition.guid
                    )))
                })
        };
        // The instances of every definition with a global word, in
        // registration order, each with its definition's place in the
        // files.
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
            if words(definition)?[GLOBAL_WORD] != 0 {
                existing.push((id, object.guid, (definition.resource, definition.chunk)));
            }
        }
        let mut result = Reconciled::default();
        for definition in &self.catalog.objects {
            let words = words(definition)?;
            if !matches!(words[9], 7 | 12) || words[GLOBAL_WORD] == 0 {
                continue;
            }
            let identity = (definition.resource, definition.chunk);
            let mut kept = false;
            let mut at = 0;
            while at < existing.len() {
                let (id, guid, place) = existing[at];
                if place != identity {
                    at += 1;
                    continue;
                }
                if world.objects.get(&id).is_none_or(|o| o.guid != guid) {
                    return Err(fail(Trap::Unsupported(format!(
                        "controller {id} was removed by a script while the controllers were \
                         reconciled"
                    ))));
                }
                if !kept {
                    kept = true;
                    at += 1;
                    continue;
                }
                self.remove_now(world, id)?;
                result.removed.push(id);
                existing.remove(at);
                // The original starts the scan over after each removal.
                kept = false;
                at = 0;
            }
            if kept {
                continue;
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
            result.created.push(self.create(world, definition.guid)?);
        }
        Ok(result)
    }
}
