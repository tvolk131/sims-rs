//! Go To Routing Slot (opcode 45, 0x45abce): a person walks to a routing
//! slot of the stack object.
//!
//! The first run (the frame's primitive state 0) picks the slot and
//! notes on the object's slot table whether the route may end anywhere
//! near it (slot flag 0x4000, set when bit 0 of operand byte 4 is clear).
//! Then it runs the route's steps (0x45ad63), each run going on from the
//! step the frame's primitive state holds:
//!
//! - **0**: a route context for the slot is made (0x48163e) and pushed on
//!   the person's contexts (+0xaf0); the next step is 1.
//! - **1**: the places the person could stand are found (0x48235a); the
//!   next step is 18.
//! - **18**: the route there is planned (0x45c38e); with a route ready
//!   (2), the next step is 12.
//! - **12**: the animator starts the route (0x589c85); the next step is 3
//!   and the thread yields.
//! - **3**, on the ticks after: while the animator's route is under way
//!   (0x589d47 gives 2), the thread yields.
//!
//! The steps run one after another in a run until one yields. Every run
//! works on the person's last context; its words +0x58, +0x60 and +0x64
//! are put back as they were at the run's start when the run ends.
//!
//! Go To Relative Position (opcode 27, 0x45c634) runs the same steps
//! with a slot record it makes from its operands instead of one of the
//! object's slots ([`relative_slot`]).
//!
//! Ported from the original's code. What the Goth house's first route
//! does not reach fails loudly: slots named any way but by a literal,
//! a stack object that is missing or off the lot in a later step, a
//! person with bit 0 of word 68 set or other contexts, a person who wants
//! to be interrupted, a person sitting (posture 1), every other step and
//! every other result of the planning and of the route.

use crate::fault::{Fault, Trap};
use crate::interpreter::{Flow, Machine, word};
use crate::placement::stands_on_lot;
use crate::routing::slot::{new_context, route_slot};
use sims_sim::route::RouteSlot;

/// Object data words.
const FLAGS: usize = 8;
const ROOM: usize = 29;
/// Person words.
const POSTURE: usize = 0;
const NO_ROOM: usize = 68;

/// The route's steps (the frame's primitive state).
const NEW: i32 = 0;
const CANDIDATES: i32 = 1;
const UNDER_WAY: i32 = 3;
const START: i32 = 12;
const PLAN: i32 = 18;

/// The slot flag that lets the route end near the slot.
const NEAR: i32 = 0x4000;
/// The slot flag a person with bit 0 of word 68 adds to its context.
const NO_ROOM_FLAG: i32 = 0x800;

fn unsupported(at: &str, what: &str) -> Trap {
    Trap::Unsupported(format!(
        "a route (Go To Routing Slot or Go To Relative Position) {what} ({at}), which is \
         not ported"
    ))
}

/// The slot record Go To Relative Position routes to (0x45c634), from its
/// operands: byte 2 the location (a signed byte), byte 3 the direction
/// to face, byte 6 options.
///
/// The record starts as the original's constructor makes it (0x46dd48):
/// no offset, number -1, snap target -1, gradient 0.1875, facing -2,
/// resolution 16. Then (0x46de84, 0x46de4b) it stands, and its minimum,
/// optimal and maximum proximity are 16. The slot flags:
///
/// - 0x4000 (the route may end near the place) unless option bit 1 is
///   set; 0x8000 if option bit 2 is set;
/// - by the location: -2 none of the eight directions; 0, 2, 4 or 6
///   that direction; 1, 3 or 5 the directions on either side; 7
///   directions 6 and 0; any other location directions 0, 2, 4 and 6,
///   with an optimal proximity of 32 (0x45c701).
///
/// The facing is the direction byte for 0 to 7, -2 for 0xfe and -3 for
/// 0xff. Any other direction is an error the original reports (0x4544dc)
/// before it takes the false branch; that fails loudly.
pub(crate) fn relative_slot(operands: &[u8; 8]) -> Result<RouteSlot, Trap> {
    const SIXTEENTH: i32 = 16;
    let mut slot = RouteSlot {
        offset: [0; 3],
        number: -1,
        standing: 1,
        sitting: 0,
        ground: 0,
        flags: 0,
        snap_target: -1,
        proximity: [SIXTEENTH; 3],
        gradient: 0x3e40_0000,
        facing: -2,
        resolution: SIXTEENTH,
    };
    let options = operands[6];
    if options & 2 == 0 {
        slot.flags |= NEAR;
    }
    if options & 4 != 0 {
        slot.flags |= 0x8000;
    }
    let direction = |d: i32| 1 << d;
    match operands[2] as i8 {
        -2 => slot.flags &= !0xff,
        location @ (0 | 2 | 4 | 6) => slot.flags |= direction(i32::from(location)),
        location @ (1 | 3 | 5) => {
            let location = i32::from(location);
            slot.flags |= direction(location - 1) | direction(location + 1);
        }
        7 => slot.flags |= direction(6) | direction(0),
        _ => {
            slot.flags |= direction(0) | direction(2) | direction(4) | direction(6);
            slot.proximity = [SIXTEENTH, 2 * SIXTEENTH, SIXTEENTH];
        }
    }
    slot.facing = match operands[3] {
        0xfe => -2,
        0xff => -3,
        facing @ 0..=7 => i32::from(facing),
        other => {
            return Err(Trap::Unsupported(format!(
                "Go To Relative Position facing {other}, an error the original reports \
                 (0x4544dc), which is not ported"
            )));
        }
    };
    Ok(slot)
}

impl Machine<'_, '_> {
    pub(crate) fn go_to_slot(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        if self.frame()?.primitive_state != NEW {
            return self.route_steps(None);
        }
        if self.me()?.person.is_none() {
            return Err(unsupported("0x45abce", "on an object that is not a person"));
        }
        let target = self.frame()?.stack_object;
        let target = u16::try_from(target)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
            .ok_or(Trap::native(0x34, "Go To Routing Slot has no stack object"))?;
        if !stands_on_lot(self.world, target)? {
            return Ok(Flow::Branch(false));
        }
        let source = word(operands, 2);
        if source != 1 {
            return Err(unsupported(
                "0x45ac2e",
                &format!("naming its slot from source {source}"),
            ));
        }
        let guid = self.world.objects[&target].guid;
        let definition = self.definition(guid)?;
        let slots = self.program.slots(definition.resource, definition.chunk);
        let index = usize::try_from(word(operands, 0))
            .ok()
            .filter(|&i| i < slots.len())
            .ok_or(Trap::native(0x25, "Go To Routing Slot names no slot"))?;
        let key = (target, index as u16);
        let flags = self
            .world
            .routing
            .slot_flags
            .get(&key)
            .copied()
            .unwrap_or(slots[index].flags as i32);
        let mut flags = flags & !NEAR;
        if operands[4] & 1 == 0 {
            flags |= NEAR;
        }
        if flags == slots[index].flags as i32 {
            self.world.routing.slot_flags.remove(&key);
        } else {
            self.world.routing.slot_flags.insert(key, flags);
        }
        let slot = route_slot(&slots[index], index, Some(flags));
        self.route_steps(Some(slot))
    }

    /// Go To Relative Position (opcode 27, 0x45c634): the route's steps
    /// with the slot record [`relative_slot`] makes, on the first run.
    pub(crate) fn go_to_relative(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        if self.frame()?.primitive_state != NEW {
            return self.route_steps(None);
        }
        if self.me()?.person.is_none() {
            return Err(unsupported("0x45c634", "on an object that is not a person"));
        }
        let slot = relative_slot(operands)?;
        self.route_steps(Some(slot))
    }

    /// The route's steps (0x45ad63).
    fn route_steps(&mut self, slot: Option<sims_sim::route::RouteSlot>) -> Result<Flow, Trap> {
        let me = self.me;
        let target = self.frame()?.stack_object;
        let target = u16::try_from(target)
            .ok()
            .filter(|id| self.world.objects.contains_key(id));
        let lost = match target {
            Some(id) => !stands_on_lot(self.world, id)?,
            None => true,
        };
        let mut state = self.frame()?.primitive_state;
        if state == NEW {
            let (Some(target), Some(slot), false) = (target, slot, lost) else {
                return Err(unsupported("0x45ae65", "with its stack object gone"));
            };
            let mut ctx = new_context(self.world, me, target, slot);
            if self.person_word(NO_ROOM)? & 1 == 1 {
                ctx.slot.flags |= NO_ROOM_FLAG;
            }
            let route = self.world.routing.routes.entry(me).or_default();
            if !route.contexts.is_empty() {
                return Err(unsupported("0x45ae0f", "for a person with a route context"));
            }
            route.contexts.push(ctx);
            state = CANDIDATES;
        }
        let Some(mut ctx) = self
            .world
            .routing
            .routes
            .get(&me)
            .and_then(|r| r.contexts.last())
            .cloned()
        else {
            return Err(unsupported(
                "0x45ae65",
                "for a person with no route context",
            ));
        };
        if ctx.target == 0 {
            return Err(unsupported("0x45ae71", "whose context lost its object"));
        }
        if lost {
            return Err(unsupported("0x45ae8d", "with its stack object gone"));
        }
        if !ctx.byte_78 {
            return Err(unsupported(
                "0x45aea5",
                "with byte +0x78 of its context clear",
            ));
        }
        let saved = (ctx.word_58, ctx.saved);
        let mut result = 2;
        let interpreter = self.interpreter();
        let mut path = self
            .world
            .routing
            .routes
            .get(&me)
            .map(|r| r.path.clone())
            .unwrap_or_default();
        let inner = |fault: Fault| {
            if fault.object == me {
                fault.trap
            } else {
                Trap::Unsupported(format!("Go To Routing Slot: {fault}"))
            }
        };
        loop {
            match state {
                CANDIDATES => {
                    if self.me()?.data.get(FLAGS).copied().unwrap_or(0) & 8 == 0 {
                        return Err(unsupported(
                            "0x45afc4",
                            "for a person without bit 3 of data word 8",
                        ));
                    }
                    if self.person_word(NO_ROOM)? & 1 != 1 {
                        let room = |id: u16| {
                            self.world
                                .objects
                                .get(&id)
                                .and_then(|o| o.data.get(ROOM).copied())
                        };
                        if room(ctx.target) != room(me) {
                            return Err(unsupported(
                                "0x45b140, then 0x45afd0 and 0x45c7c2",
                                "to an object in another room, which goes through a door the \
                                 routing between rooms chooses (0x46390f)",
                            ));
                        }
                    }
                    if self.wants_interrupt()? {
                        return Err(unsupported(
                            "0x45b196",
                            "for a person who wants to be interrupted",
                        ));
                    }
                    interpreter
                        .find_candidates(self.world, &mut ctx)
                        .map_err(inner)?;
                    state = PLAN;
                    if self.person_word(POSTURE)? == 1 {
                        return Err(unsupported("0x45b1d7", "for a person sitting"));
                    }
                }
                PLAN => {
                    let planned = interpreter
                        .route_to_slot(self.world, &mut ctx, &mut path)
                        .map_err(inner)?;
                    result = planned;
                    if planned != 2 {
                        return Err(unsupported(
                            "0x45b285",
                            &format!("whose planning gave {planned}"),
                        ));
                    }
                    state = START;
                }
                START => {
                    // The original notes the route in its log here
                    // (0x57a54c), which is not modeled.
                    self.store_route(&ctx, &path);
                    interpreter.start_route(self.world, me)?;
                    state = UNDER_WAY;
                    break;
                }
                UNDER_WAY => {
                    if self.wants_interrupt()? {
                        return Err(unsupported(
                            "0x45b2c2",
                            "for a person who wants to be interrupted",
                        ));
                    }
                    if path.is_empty() {
                        return Err(unsupported("0x45b2f7", "with no path"));
                    }
                    // 0x589d47: the animator's route result.
                    let arrival = self.world.animators.get(&me).map_or(0, |a| a.arrival);
                    result = match arrival {
                        -1 | 1 => 2,
                        0 => 0,
                        _ => {
                            return Err(unsupported(
                                "0x589d6e",
                                &format!("whose route ended with {arrival}"),
                            ));
                        }
                    };
                    if result != 2 {
                        return Err(unsupported(
                            "0x45b346",
                            &format!("whose route gave {result}"),
                        ));
                    }
                    break;
                }
                other => {
                    return Err(unsupported("0x45aef0", &format!("at step {other}")));
                }
            }
        }
        // 0x45b9a0: the step is kept in the frame, and the context's words
        // +0x58 (as a 16-bit word), +0x60 and +0x64 put back.
        ctx.word_58 = i32::from(saved.0 as i16);
        ctx.saved = saved.1;
        self.store_route(&ctx, &path);
        self.frame_mut()?.primitive_state = state;
        match result {
            2 => {
                if self.synchronous {
                    return Err(Trap::native(
                        0x44f,
                        "a synchronous tree yielded in Go To Routing Slot",
                    ));
                }
                Ok(Flow::Yield)
            }
            other => Err(unsupported("0x45b9c0", &format!("returning {other}"))),
        }
    }

    /// Writes the context back as the person's last, with the path.
    fn store_route(&mut self, ctx: &sims_sim::route::Context, path: &[[i32; 2]]) {
        let route = self.world.routing.routes.entry(self.me).or_default();
        if let Some(last) = route.contexts.last_mut() {
            *last = ctx.clone();
        }
        route.path = path.to_vec();
    }

    /// Person data word `index` of the running object.
    pub(super) fn person_word(&self, index: usize) -> Result<i16, Trap> {
        self.me()?
            .person
            .as_ref()
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .data
            .get(index)
            .copied()
            .ok_or_else(|| Trap::Invalid(format!("person data has no word {index}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The operands with location `location`, direction `facing` and
    /// options `options`.
    fn operands(location: u8, facing: u8, options: u8) -> [u8; 8] {
        [0, 0, location, facing, 0, 0, options, 0]
    }

    #[test]
    fn a_relative_slot_starts_as_the_constructor_makes_it() {
        let slot = relative_slot(&operands(0, 0xfe, 0)).unwrap();
        assert_eq!(slot.offset, [0; 3]);
        assert_eq!((slot.number, slot.snap_target), (-1, -1));
        assert_eq!((slot.standing, slot.sitting, slot.ground), (1, 0, 0));
        assert_eq!(slot.proximity, [16; 3]);
        assert_eq!((slot.gradient, slot.resolution), (0x3e40_0000, 16));
        assert_eq!(slot.facing, -2);
        assert_eq!(slot.flags, NEAR | 1);
    }

    #[test]
    fn the_location_picks_the_directions() {
        let flags = |location: u8| relative_slot(&operands(location, 0, 0)).unwrap().flags & 0xff;
        assert_eq!(flags(0xfe), 0);
        assert_eq!(flags(2), 1 << 2);
        assert_eq!(flags(6), 1 << 6);
        assert_eq!(flags(1), 0b101);
        assert_eq!(flags(5), 0b101_0000);
        assert_eq!(flags(7), 0b100_0001);
        // Any other location: the four straight directions, with an
        // optimal proximity of 32.
        let slot = relative_slot(&operands(0xff, 0, 0)).unwrap();
        assert_eq!(slot.flags & 0xff, 0b101_0101);
        assert_eq!(slot.proximity, [16, 32, 16]);
        assert_eq!(flags(8), 0b101_0101);
    }

    #[test]
    fn the_options_and_the_facing() {
        let slot = |facing: u8, options: u8| relative_slot(&operands(0xfe, facing, options));
        assert_eq!(slot(0, 2).unwrap().flags, 0);
        assert_eq!(slot(0, 4).unwrap().flags, NEAR | 0x8000);
        assert_eq!(slot(0xff, 0).unwrap().facing, -3);
        assert_eq!(slot(7, 0).unwrap().facing, 7);
        assert!(slot(8, 0).is_err());
    }
}
