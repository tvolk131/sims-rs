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
//! A person's visit (0x459bf6) differs in three ways (`people.rs`):
//!
//! - Before anything else the person takes its lasting fields from its
//!   neighborhood record (0x4258a3).
//! - It is not asked its Load function.
//! - It resets in its own way (0x45e791): its thread restarts at Main
//!   first, and then its Reset function runs on top of that.
//!
//! After the visit the original also notes the room the person is in,
//! which tells the room it is occupied, and loads the person's motive
//! curves (0x45e43d, 0x459d23). Neither is kept here. An outdoor room
//! ignores being told, and the room update fails for an indoor room
//! where it would matter (`sims_sim::rooms::RoomError::Occupancy`).
//! Nothing reads the curves until a person's motives are updated.
//! [`crate::curve_table`] says which curves they would be.
//!
//! Ported from the old prototype's visit loop
//! (legacy/crates/sims-content/src/house_loading.rs), `PostLoad`
//! (legacy/crates/sims-vm/src/loading.rs) and `Reset`
//! (legacy/crates/sims-sim/src/lot_reset.rs). `docs/decisions.md` says
//! what a trace of the original loading the Goth house confirms.

use crate::factory::TURN_BUDGET;
use crate::fault::{Fault, Trap};
use crate::interpreter::{Asked, Interpreter};
use crate::placement::capture;
use crate::program::Tree;
use crate::removal::refers_to;
use sims_content::People;
use sims_sim::World;

/// Places in an object's callback table.
pub(crate) mod function {
    pub const INIT: usize = 0;
    pub const LOAD: usize = 2;
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
        let definition = self
            .catalog
            .object(object.guid)
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
        if matches!(self.program.tree(definition.resource, tree), Tree::Missing) {
            // The original finds the tree missing only after it has
            // written its mark above the top frame.
            self.machine(world, id, 0, false)
                .write_mark()
                .map_err(fail)?;
            return Ok(false);
        }
        let temps = object.temps.clone();
        self.call(world, id, tree, stack_object, &temps, 8, TURN_BUDGET)
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
        if !person {
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

    /// Tells every person that an object is about to reset (0x444306). A
    /// person that does not refer to the object changes nothing. One that
    /// does must cancel actions and unwind its thread, which is not ported
    /// and fails.
    pub(crate) fn drop_references(&self, world: &World, id: u16) -> Result<(), Fault> {
        for (&other, object) in &world.objects {
            if let Some(person) = &object.person
                && other != id
                && refers_to(person, id)
            {
                return Err(Fault {
                    object: id,
                    at: None,
                    trap: Trap::Unsupported(format!(
                        "person {other} refers to object {id}; making a person drop an object \
                         that resets is not ported"
                    )),
                });
            }
        }
        Ok(())
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
