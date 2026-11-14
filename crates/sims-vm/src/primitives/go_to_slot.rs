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
        "Go To Routing Slot {what} ({at}), which is not ported"
    ))
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
                            return Err(unsupported("0x45b140", "to a slot in another room"));
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
