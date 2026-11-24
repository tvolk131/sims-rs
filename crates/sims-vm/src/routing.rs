//! Routing: how a person finds its way to an object's slot.
//!
//! Go To Routing Slot ([`crate::primitives`], opcode 45) makes a route
//! context for the slot ([`sims_sim::route::Context`], kept on the person
//! at +0xaf0), finds where the person could stand to use it
//! ([`slot`]), plans a path there ([`planner`]) and hands the path to the
//! person's animator, which walks it tick by tick.
//!
//! The planner searches a graph of free rectangles ([`regions`],
//! [`search`]) among the obstacles of the room's objects and of the room
//! itself ([`rooms`]).
//!
//! Ported from the original's code and checked against the capture of the
//! Goth house's first route (update 14). Only what that route reaches is
//! ported; every other branch fails loudly.
//!
//! When the house changes under the routes, the lot forgets what they
//! worked out ([`Interpreter::forget_routes`], 0x444805): the game going
//! to live mode does so (0x416534), and the lot's move by an offset
//! (0x444991, not ported). A route a person resumes after that starts
//! again from what it has (`primitives/go_to_slot.rs`, 0x45aea5).

mod planner;
pub(crate) mod portals;
mod regions;
mod rooms;
pub(crate) use rooms::walls_cross;
mod search;
pub(crate) mod slot;

use crate::interpreter::Interpreter;
use sims_sim::World;

impl Interpreter<'_> {
    /// The lot forgets what its routes worked out (0x444805). Each person
    /// on its person vector ([`World::people`], lot +0x84) has byte +0x78
    /// of every route context cleared (0x45eddc), which the person's next
    /// run of its route's steps acts on. Then every portal's costs become
    /// 10001, not worked out ([`Interpreter::forget_portal_costs`],
    /// 0x46387c).
    pub(crate) fn forget_routes(&self, world: &mut World) -> Result<(), crate::fault::Trap> {
        for id in world.people.clone() {
            if let Some(route) = world.routing.routes.get_mut(&id) {
                for context in &mut route.contexts {
                    context.byte_78 = false;
                }
            }
        }
        self.forget_portal_costs(world)
    }
}

#[cfg(test)]
mod tests {
    use crate::boot::{Frames, Lot};
    use crate::interpreter::{Interpreter, Stream};
    use std::path::PathBuf;

    /// 10001, a cost not worked out (0x667478).
    const UNKNOWN: f32 = 10001.0;

    fn goth() -> Option<Lot> {
        let root = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
        let house = root.join("UserData/Houses/House05.iff");
        Some(Lot::load(&root, &house, &[3], Frames::default()).unwrap())
    }

    /// Going live (0x416534) makes the lot forget its routes (0x444805):
    /// every route context of a person on the person vector has byte
    /// +0x78 cleared (0x45eddc), and every portal's costs become 10001,
    /// the vectors keeping their columns (0x46387c).
    #[test]
    fn going_live_forgets_the_routes() {
        let Some(lot) = goth() else {
            return;
        };
        let interpreter = Interpreter::new(&lot.program, &lot.catalog, Stream::Global);
        let mut world = lot.world.clone();
        let portals = interpreter.portals(&world).unwrap();
        assert!(!portals.is_empty());
        for &portal in &portals {
            world.objects.get_mut(&portal).unwrap().portal = Some(vec![3.0, 1001.0, 10000.0]);
        }
        let person = world.people[0];
        let mut context = crate::routing::slot::loaded_context();
        context.byte_78 = true;
        let route = world.routing.routes.entry(person).or_default();
        route.contexts = vec![context.clone(), context];
        interpreter.go_live(&mut world).unwrap();
        for &portal in &portals {
            assert_eq!(world.objects[&portal].portal, Some(vec![UNKNOWN; 3]));
        }
        assert!(
            world.routing.routes[&person]
                .contexts
                .iter()
                .all(|context| !context.byte_78)
        );
    }

    /// A door's group taken off and put back (0x4634f2, 0x4630d9) leaves
    /// every portal's costs forgotten (0x46387c).
    #[test]
    fn moving_a_door_forgets_the_portal_costs() {
        let Some(lot) = goth() else {
            return;
        };
        let interpreter = Interpreter::new(&lot.program, &lot.catalog, Stream::Global);
        let mut world = lot.world.clone();
        let portals = interpreter.portals(&world).unwrap();
        let worked_out = |world: &mut sims_sim::World| {
            for &portal in &portals {
                world.objects.get_mut(&portal).unwrap().portal = Some(vec![3.0, 1001.0]);
            }
        };
        let forgotten = |world: &sims_sim::World| {
            portals
                .iter()
                .all(|portal| world.objects[portal].portal == Some(vec![UNKNOWN; 2]))
        };
        let door = portals[0];
        let [x, y, level] = world.objects[&door].position;
        worked_out(&mut world);
        interpreter.take_off(&mut world, door, true).unwrap();
        assert!(forgotten(&world));
        worked_out(&mut world);
        let place = crate::placement::Place {
            on_lot: true,
            position: [x, y],
            container: None,
            slot: -1,
            level,
        };
        interpreter.put(&mut world, door, place, true).unwrap();
        assert!(forgotten(&world));
        assert_eq!(world.objects[&door].position, [x, y, level]);
    }
}
