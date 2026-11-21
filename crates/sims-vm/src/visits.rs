//! Load step 13: visiting every object in registration order (0x43fc94).
//!
//! For each object (0x431e2e):
//!
//! 1. Its position is restored (0x433e50): an object on level 0 goes to
//!    level 1, and the object and everything it holds get their position,
//!    footprint and room again.
//! 2. It runs its Load function (2).
//! 3. If its type's scripts changed since the house was saved
//!    ([`World::changed_types`]): every person drops its references to the
//!    object (0x444306), the object resets ([`Interpreter::reset`],
//!    0x431fbe), and then its Init function (0) runs.
//! 4. Two of its scheduler flags are refreshed.
//!
//! A reset runs the object's Reset function (3) if it has one. Otherwise
//! the object is rebuilt ([`Interpreter::rebuild`]): taken off the lot,
//! its storage reset, Init and a first turn run, and put back.
//!
//! A person's visit (0x459bf6) differs in four ways (`people.rs`):
//!
//! - Before anything else the person takes its lasting fields from its
//!   neighborhood record (0x4258a3).
//! - It is not asked its Load function.
//! - It resets in its own way (0x45e791): its thread restarts at Main
//!   first, and then its Reset function runs on top of that.
//! - After its position is restored, and before a reset, it notes
//!   the room it is in (0x45e43d at 0x459c22; [`Interpreter::note_room`]),
//!   which may tell the room it is occupied. A capture of the original
//!   loading the Goth house has each person's noted room (person field
//!   0xafc) equal to its room word at the end of the load.
//!
//! The original then loads the person's motive curves (0x459d23). That is
//! not kept here. Nothing reads the curves until a person's motives are
//! updated. [`crate::curve_table`] says which curves they would be.
//!
//! Ported from the old prototype's visit loop
//! (legacy/crates/sims-content/src/house_loading.rs), `PostLoad`
//! (legacy/crates/sims-vm/src/loading.rs) and `Reset`
//! (legacy/crates/sims-sim/src/lot_reset.rs). `docs/decisions/` says
//! what a trace of the original loading the Goth house confirms.

use crate::factory::TURN_BUDGET;
use crate::fault::{Fault, Trap};
use crate::interpreter::{Asked, Interpreter};
use crate::placement::capture;
use crate::program::Tree;
use sims_content::People;
use sims_sim::World;

/// Places in an object's callback table.
pub(crate) mod function {
    pub const INIT: usize = 0;
    pub const LOAD: usize = 2;
    /// The function a type of a group names to have a tile leave its
    /// group before it is taken off or removed (0x42344e: the first word
    /// of its callback entry 8, 0x457c38, 0x455fa9).
    pub const DYNAMIC_GROUP: usize = 8;
    pub const PLACED: usize = 9;
    pub const LEAVING: usize = 10;
}

/// What [`Interpreter::visit_objects`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Visited {
    /// The objects visited, in order.
    pub objects: Vec<u16>,
    /// Those whose type had changed, which were reset.
    pub reset: Vec<u16>,
    /// The visit that could not be made, and why. The lot is then as the
    /// visit before it left it. None when every object was visited.
    pub stopped: Option<Fault>,
}

impl Interpreter<'_> {
    /// Asks an object to run one of its functions (0x422d46): the tree its
    /// callback table names at `function`, run synchronously on top of
    /// whatever its thread holds, with the object's temps as arguments.
    /// Returns the tree's result, or false when the table names no tree or
    /// names one that is missing.
    pub(crate) fn callback(
        &self,
        world: &mut World,
        id: u16,
        function: usize,
        stack_object: i16,
    ) -> Result<bool, Fault> {
        self.callback_with(world, id, function, stack_object, None)
    }

    /// [`Interpreter::callback`] with the caller's argument buffer (0x422d46
    /// with a third argument that is not null): the tree takes its
    /// arguments from `args`, and a tree that takes more words than the
    /// buffer holds is refused. With `None`, the object's temps.
    pub(crate) fn callback_with(
        &self,
        world: &mut World,
        id: u16,
        function: usize,
        stack_object: i16,
        args: Option<&[i16]>,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        if let Some(asked) = self.asked {
            asked.borrow_mut().push(Asked {
                object: id,
                function,
            });
        }
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let definition = crate::interpreter::object_definition(self.catalog, world, object.guid)
            .ok_or_else(|| fail(Trap::Invalid("object has no definition".into())))?;
        let tree = self
            .program
            .libraries
            .get(&definition.resource)
            .and_then(|l| l.callbacks.get(&definition.chunk))
            .and_then(|table| table.get(function))
            .map_or(0, |f| f.action);
        if tree == 0 {
            return Ok(false);
        }
        let temps = object.temps.clone();
        let resource = definition.resource;
        let before = self
            .callbacks_must_be_pure
            .then(|| (world.clone(), crate::both_ways::Cells::of(self)));
        let result = self.run_callback_tree(world, id, resource, tree, stack_object, args, &temps);
        if let Some((lot, cells)) = before
            && !(crate::both_ways::same_lot(&lot, world)
                && cells.same(&crate::both_ways::Cells::of(self)))
        {
            return Err(fail(Trap::Unsupported(format!(
                "object {id}'s function {function} (tree {tree}) changed the lot or the machine's \
                 cells while a dialog's portrait searched for a tile from every start (0x4347f0): \
                 which the original runs depends on the house view's centre (0x4a677c)"
            ))));
        }
        result
    }

    /// The rest of [`Interpreter::callback_with`] once the table names
    /// tree `tree`.
    #[allow(clippy::too_many_arguments)]
    fn run_callback_tree(
        &self,
        world: &mut World,
        id: u16,
        resource: usize,
        tree: u16,
        stack_object: i16,
        args: Option<&[i16]>,
        temps: &[i16],
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        if matches!(self.program.tree(resource, tree), Tree::Missing) {
            // The original finds the tree missing only after it has
            // written its mark above the top frame.
            self.machine(world, id, 0, false)
                .write_mark()
                .map_err(fail)?;
            return Ok(false);
        }
        match args {
            Some(args) => self.call(world, id, tree, stack_object, args, args.len(), TURN_BUDGET),
            None => self.call(world, id, tree, stack_object, temps, 8, TURN_BUDGET),
        }
    }

    /// Load step 13: visits every object in registration order
    /// ([`Interpreter::visit`]). The next object is read from the list
    /// after each visit, so an object a visit creates, which goes to the
    /// head of the list, is not visited.
    ///
    /// A visit that fails stops the step. The lot is left as the last
    /// complete visit left it, which is a state the original is in between
    /// two visits, and [`Visited::stopped`] says which visit failed and
    /// why.
    ///
    /// `people` supplies each person's neighborhood record and profile.
    pub fn visit_objects(&self, world: &mut World, people: &People) -> Visited {
        let mut done = Visited::default();
        let mut current = world.order.first().copied();
        while let Some(id) = current {
            let changed = world
                .objects
                .get(&id)
                .is_some_and(|o| world.changed_types.contains(&o.guid));
            let mut next = world.clone();
            let result = self.visit(&mut next, id, people).and_then(|()| {
                next.order
                    .iter()
                    .position(|&other| other == id)
                    .ok_or_else(|| Fault {
                        object: id,
                        at: None,
                        trap: Trap::Unsupported("an object was deleted during its visit".into()),
                    })
            });
            match result {
                Ok(place) => {
                    *world = next;
                    done.objects.push(id);
                    if changed {
                        done.reset.push(id);
                    }
                    current = world.order.get(place + 1).copied();
                }
                Err(fault) => {
                    done.stopped = Some(fault);
                    break;
                }
            }
        }
        done
    }

    /// Visits one object (0x431e2e), or one person (0x459bf6). See the
    /// module documentation.
    pub fn visit(&self, world: &mut World, id: u16, people: &People) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let changed = world.changed_types.contains(&object.guid);
        let person = object.person.is_some();
        let mut position = object.position;
        if person {
            self.attach_neighbor(world, id).map_err(fail)?;
        }
        if position[2] == 0 {
            position[2] = 1;
        }
        self.move_tree(world, id, position).map_err(fail)?;
        if person {
            self.note_room(world, id).map_err(fail)?;
        } else {
            self.callback(world, id, function::LOAD, 0)?;
        }
        if changed {
            self.drop_references(world, id)?;
            if person {
                self.reset_person(world, id, people)?;
            } else {
                self.reset(world, id, false)?;
            }
            self.callback(world, id, function::INIT, 0)?;
        }
        self.machine(world, id, 0, false)
            .refresh_flags()
            .map_err(fail)
    }

    /// Resets an object that has no Reset tree (0x431fbe, with 0x431be1):
    ///
    /// 1. Where it is is noted, and it is taken off the lot
    ///    ([`Interpreter::take_off`]). A multi-tile object takes its whole
    ///    group off.
    /// 2. The storage of the object, or of every tile of its group, is
    ///    reset and its thread restarts at Main. The branch its last
    ///    primitive took is kept.
    /// 3. For the object, or each tile in turn: Init runs, the object gets
    ///    a first turn, and its scheduler flags are refreshed.
    /// 4. If it stood on the lot, it is put back where it was
    ///    ([`Interpreter::put`]).
    pub(crate) fn rebuild(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let saved = capture(world, id).map_err(fail)?;
        self.take_off(world, id, true)?;
        let parts = self.parts(world, id).map_err(fail)?;
        for &part in &parts {
            let fail = |trap| Fault {
                object: part,
                at: None,
                trap,
            };
            let mut machine = self.machine(world, part, 0, false);
            let last = machine.me().map_err(fail)?.last_result;
            let guid = machine.me().map_err(fail)?.guid;
            let words = machine.definition_words(guid).map_err(fail)?;
            machine.reset(words).map_err(fail)?;
            machine.install_main().map_err(fail)?;
            machine.me_mut().map_err(fail)?.last_result = last;
        }
        if parts.len() > 1 || world.objects[&id].multipart.is_some() {
            self.update_offsets(world, id).map_err(fail)?;
        }
        for &part in &parts {
            let fail = |trap| Fault {
                object: part,
                at: None,
                trap,
            };
            self.init(world, part)?;
            self.turn(world, part, TURN_BUDGET)?;
            let mut machine = self.machine(world, part, 0, false);
            machine.end_turn().map_err(fail)?;
            machine.refresh_flags().map_err(fail)?;
        }
        if saved.on_lot {
            self.put(world, id, saved, true)?;
        }
        Ok(())
    }
}
