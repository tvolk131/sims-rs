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
//! - **1**: when the slot's object is in another room than the person
//!   (and bit 0 of person word 68 is clear), the next step is 2.
//!   Otherwise the places the person could stand are found (0x48235a);
//!   the next step is 18.
//! - **2**, the way through a door (0x45afd0): once the person is in
//!   the object's room, back to step 1. Otherwise a door tree that
//!   returned false marks its door failed, the door to go through is
//!   chosen (`routing/portals.rs`), and its door tree is pushed on the
//!   person's thread; the step stays 2 while that tree runs.
//! - **18**: the route there is planned (0x45c38e); with a route ready
//!   (2), the next step is 12. With no way found (0) the next step is 6,
//!   which is not ported.
//! - **12**: the animator starts the route (0x589c85); the next step is 3
//!   and the thread yields.
//! - **3**, on the ticks after: while the animator's route is under way
//!   (0x589d47 gives 2), the thread yields. When it reached the path's
//!   end (1) the next step is 9, else 10.
//! - **9**: the animator stops (0x589e2d); the next step is 8.
//! - **8**: the route ends ([`Machine::end_route`]): the result is true.
//!
//! The steps run one after another in a run until one yields. Every run
//! works on the person's last context; its words +0x58, +0x60 and +0x64
//! are put back as they were at the run's start when the run ends.
//!
//! Go To Relative Position (opcode 27, 0x45c634) runs the same steps
//! with a slot record it makes from its operands instead of one of the
//! object's slots ([`relative_slot`]).
//!
//! Ported from the original's code. What the Goth house's routes to
//! update 261 do not reach fails loudly: slots named any way but by a
//! literal, a stack object that is missing or off the lot in a later
//! step, a person with bit 0 of word 68 set or other contexts, a person
//! who wants to be interrupted, a person sitting (posture 1), no door to
//! go through, a route that stops short of the path's end (step 10),
//! every other step and every other result of the planning.

use crate::animator::stop_route;
use crate::fault::{Fault, Trap};
use crate::interpreter::{Flow, Machine, word};
use crate::placement::stands_on_lot;
use crate::routing::slot::{direction, new_context, route_slot};
use sims_sim::route::RouteSlot;

/// Object data words.
const DIRECTION: usize = 1;
const FLAGS: usize = 8;
const ROOM: usize = 29;
/// Person words.
const POSTURE: usize = 0;
const WORD_72: usize = 72;
const NO_ROOM: usize = 68;
/// Takes the route's result as the route ends (0x45b869).
const ROUTE_RESULT: usize = 62;
/// Read and set as the route ends (0x45b885).
const WORD_38: usize = 38;
const WORD_39: usize = 39;
/// The footprint's extension, which the place test reads; the end of a
/// route sets it from word 72 (0x45b709).
const EXTENSION: usize = 73;
/// Object data word 9, which the end of a route reads and clears.
const WORD_9: usize = 9;

/// The route's steps (the frame's primitive state).
const NEW: i32 = 0;
const CANDIDATES: i32 = 1;
const PORTAL: i32 = 2;
const UNDER_WAY: i32 = 3;
const FAILED: i32 = 7;
const SUCCEEDED: i32 = 8;
const ARRIVED: i32 = 9;
const STOPPED: i32 = 10;
const START: i32 = 12;
const PLAN: i32 = 18;

/// An object's type: a door.
const DOOR: i16 = 8;

/// The slot flag that lets the route end near the slot.
const NEAR: i32 = 0x4000;
/// The slot flag a person with bit 0 of word 68 adds to its context.
const NO_ROOM_FLAG: i32 = 0x800;

/// A context's door (+0x68) as an object ID.
fn portal_id(word: i32) -> Result<u16, Trap> {
    u16::try_from(word).map_err(|_| Trap::Invalid(format!("a route context names door {word}")))
}

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
/// resolution 16. Then (0x46de84, 0x46de4b) it stands, and its least,
/// greatest and best distances are 16. The slot flags:
///
/// - 0x4000 (the route may end near the place) unless option bit 1 is
///   set; 0x8000 if option bit 2 is set;
/// - by the location: -2 none of the eight directions; 0, 2, 4 or 6
///   that direction; 1, 3 or 5 the directions on either side; 7
///   directions 6 and 0; any other location directions 0, 2, 4 and 6,
///   with a greatest distance of 32 (0x45c701).
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
            // 0x45ae0f: a route inside another (a door's walk) may end near
            // its place only if the person's first route may.
            if let Some(first) = route.contexts.first()
                && first.slot.flags & NEAR == 0
            {
                ctx.slot.flags &= !NEAR;
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
                    if self.person_word(NO_ROOM)? & 1 != 1
                        && self.room(ctx.target)? != self.room(me)?
                    {
                        // 0x45b140: to another room, through a door.
                        if interpreter.kind(self.world, ctx.target)? == DOOR {
                            let routes = &self.world.routing.routes[&me];
                            if routes
                                .contexts
                                .iter()
                                .any(|c| c.word_68 == i32::from(ctx.target))
                            {
                                return Err(unsupported(
                                    "0x45b176",
                                    "to a door one of its routes goes through",
                                ));
                            }
                        }
                        state = PORTAL;
                        continue;
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
                    if planned == 0 {
                        // 0x45b2ab: no way; step 6 (0x45b066) tries again,
                        // and asks a person in the way to move (0x45ba28,
                        // step 14).
                        return Err(unsupported(
                            "0x45b066",
                            "whose planning found no way: step 6, which tries again and asks \
                             a person in the way to move (0x45ba28)",
                        ));
                    }
                    if planned != 2 {
                        return Err(unsupported(
                            "0x45b285",
                            &format!("whose planning gave {planned}"),
                        ));
                    }
                    state = START;
                }
                PORTAL => {
                    // 0x45afd0.
                    if self.wants_interrupt()? {
                        return Err(unsupported(
                            "0x45afd2",
                            "for a person who wants to be interrupted",
                        ));
                    }
                    if self.room(ctx.target)? == self.room(me)? {
                        // Through: person word 72 goes back to what it was
                        // when the route began.
                        self.set_person_word(WORD_72, ctx.word_94)?;
                        state = CANDIDATES;
                        continue;
                    }
                    if ctx.word_68 != 0 && !self.me()?.last_result {
                        // The door tree returned false: the door is marked
                        // failed (0x463e09).
                        let door = portal_id(ctx.word_68)?;
                        interpreter.fail_portal(self.world, me, door)?;
                    }
                    let Some((door, tree)) = self.choose_door(&ctx)? else {
                        return Err(unsupported(
                            "0x45b036",
                            "with no door to go through (the route fails with result 2)",
                        ));
                    };
                    // 0x45b954: the route stays at this step, and person
                    // word 72 is cleared, while the door's tree runs. The
                    // step and the context's saved words are written before
                    // the tree is pushed, as the frame changes with it.
                    ctx.word_68 = i32::from(door);
                    ctx.word_58 = i32::from(saved.0 as i16);
                    ctx.saved = saved.1;
                    self.store_route(&ctx, &path);
                    self.frame_mut()?.primitive_state = PORTAL;
                    self.enter_door(door, tree)?;
                    interpreter.try_portal(self.world, me, door)?;
                    self.set_person_word(WORD_72, 0)?;
                    return Ok(Flow::Pushed);
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
                    // 0x589d47: the animator's route result; 1 if it
                    // reached the path's end and the person was put there.
                    match interpreter.route_result(self.world, me)? {
                        2 => {
                            result = 2;
                            break;
                        }
                        1 => state = ARRIVED,
                        _ => state = STOPPED,
                    }
                }
                ARRIVED | STOPPED => {
                    // 0x45b3de, 0x45b371: the animator stops (0x589e2d).
                    let animator = self
                        .world
                        .animators
                        .get_mut(&me)
                        .ok_or_else(|| Trap::Invalid(format!("person {me} has no animator")))?;
                    if !stop_route(animator) {
                        return Err(unsupported(
                            "0x45b3eb",
                            "whose animator is still under way when it is to stop",
                        ));
                    }
                    if state == STOPPED {
                        return Err(unsupported(
                            "0x45b384",
                            "whose animator's route ended without reaching the path's end \
                             (step 6)",
                        ));
                    }
                    // The original notes the step in its log (0x57a54c),
                    // which is not modeled.
                    state = SUCCEEDED;
                }
                SUCCEEDED => return self.end_route(state, ctx, &path),
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

    /// The end of a route (0x45b6ea), at step 8 (the person reached the
    /// path's end) or 7 (it failed):
    ///
    /// 1. At step 8, person word 73 takes word 72 if that is not 0. If
    ///    the person fits on its own tile (virtual function 0x54), it is
    ///    put there again (virtual function 0x88), and when the path has
    ///    two points or more and object data word 9 is 0, it turns toward
    ///    the path's last point from the one before ([`direction`], into
    ///    data word 1) and its virtual function 0x74 runs
    ///    ([`crate::Interpreter::refresh_person`]). If it does not fit,
    ///    word 73 goes back to 0 (when it was set) and, for a person whose
    ///    word 0 is 0, the route fails: step 7 and context +0x88 = 1.
    /// 2. An animator still on a route (+0x188 not 0) is stopped
    ///    (0x589e2d); the original first reports it (0x4544dc), which is
    ///    not modeled.
    /// 3. The result: at step 7, 0 (false), and context +0x88 becomes 1
    ///    if it is 0; at step 8, 1 (true), and context +0x88 becomes 0.
    ///    Person word 62 takes context +0x88. With one context, word 39
    ///    becomes 1 if word 38 is 0. Object data word 9 becomes 0.
    /// 4. Other people's contexts that wait on this person (+0x80, when
    ///    person +0xa84 is set) would be released, and the context's own
    ///    object +0x80 told (0x45d5f8). Nothing ported sets context +0x80,
    ///    so a context naming one fails loudly.
    /// 5. The context is removed (0x45f8e8); the path stays. The frame
    ///    keeps the step.
    fn end_route(
        &mut self,
        mut state: i32,
        mut ctx: sims_sim::route::Context,
        path: &[[i32; 2]],
    ) -> Result<Flow, Trap> {
        let me = self.me;
        let interpreter = self.interpreter();
        let inner = |fault: Fault| {
            if fault.object == me {
                fault.trap
            } else {
                Trap::Unsupported(format!("the end of a route: {fault}"))
            }
        };
        if state == SUCCEEDED {
            let ignored = self.person_word(WORD_72)?;
            if ignored != 0 {
                self.set_person_word(EXTENSION, ignored)?;
            }
            let [first, second, level] = self.me()?.position;
            if interpreter
                .fits(self.world, me, [first, second], level)
                .map_err(inner)?
            {
                interpreter.put_person(self.world, me, [first, second], level)?;
                if let [.., before, last] = path
                    && self.datum(WORD_9)? == 0
                {
                    self.set_datum(DIRECTION, (direction(*before, *last) & 7) as i16)?;
                    interpreter.refresh_person(self.world, me)?;
                }
            } else {
                if ignored != 0 {
                    self.set_person_word(EXTENSION, 0)?;
                }
                if self.person_word(POSTURE)? == 0 {
                    state = FAILED;
                    ctx.result = 1;
                }
            }
        }
        if let Some(animator) = self.world.animators.get_mut(&me)
            && animator.arrival != 0
        {
            stop_route(animator);
        }
        let result = if state == FAILED {
            if ctx.result == 0 {
                ctx.result = 1;
            }
            false
        } else {
            ctx.result = 0;
            true
        };
        self.set_person_word(ROUTE_RESULT, ctx.result as i16)?;
        let contexts = self
            .world
            .routing
            .routes
            .get(&me)
            .map_or(0, |r| r.contexts.len());
        if contexts == 1 && self.person_word(WORD_38)? == 0 {
            self.set_person_word(WORD_39, 1)?;
        }
        self.set_datum(WORD_9, 0)?;
        if ctx.objects[1] != 0 {
            return Err(unsupported(
                "0x45b91e",
                "whose context names an object at +0x80",
            ));
        }
        let waiting = self
            .world
            .routing
            .routes
            .iter()
            .any(|(&id, r)| id != me && r.contexts.iter().any(|c| c.objects[1] == me));
        if waiting {
            return Err(unsupported(
                "0x45b8be",
                "that another person's context waits on",
            ));
        }
        if let Some(route) = self.world.routing.routes.get_mut(&me) {
            route.contexts.pop();
            route.path = path.to_vec();
        }
        self.frame_mut()?.primitive_state = state;
        Ok(Flow::Branch(result))
    }

    /// An object's room: its data word 29.
    fn room(&self, id: u16) -> Result<i16, Trap> {
        self.world
            .objects
            .get(&id)
            .ok_or(Trap::native(10, "missing object"))?
            .data
            .get(ROOM)
            .copied()
            .ok_or_else(|| Trap::Invalid("object data has no word 29".into()))
    }

    /// The door to go through and its door tree, or None (0x45c7c2).
    ///
    /// On a route's first door (the context names none yet), the person
    /// gets its number for the doors' costs and its costs are forgotten
    /// (0x4638b9). Then a door is chosen (0x463c83); none, or one whose
    /// definition has no door tree, is None.
    fn choose_door(&mut self, ctx: &sims_sim::route::Context) -> Result<Option<(u16, u16)>, Trap> {
        let interpreter = self.interpreter();
        if ctx.word_68 == 0 {
            interpreter.reset_portal_costs(self.world, self.me)?;
        }
        let Some(door) = interpreter.choose_portal(self.world, self.me, ctx.target)? else {
            return Ok(None);
        };
        let tree = interpreter.door_tree(self.world, door);
        Ok((tree != 0).then_some((door, tree)))
    }

    /// Runs the door's tree on the person's thread, as the person's virtual
    /// function 0x34 does (0x45e59b) with no arguments given: the person is
    /// woken, the tree of the door's type is pushed with the door as its
    /// stack object and the person's temps as its arguments (0x452b98,
    /// 0x47484a), and the person starts using the door. A tree that cannot
    /// be pushed, where the original would go on without the door, fails
    /// loudly.
    fn enter_door(&mut self, door: u16, tree: u16) -> Result<(), Trap> {
        let temps = self.me()?.temps.clone();
        self.wake(self.me)?;
        self.enter_on(door, tree, &temps).map_err(|fault| {
            Trap::Unsupported(format!(
                "a door tree {tree:#06x} of object {door} that cannot be pushed, where the \
                 original goes on without the door (0x45c827): {fault}"
            ))
        })?;
        self.count_use(door)
    }

    /// Sets person data word `index` of the running object.
    fn set_person_word(&mut self, index: usize, value: i16) -> Result<(), Trap> {
        let slot = self
            .me_mut()?
            .person
            .as_mut()
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .data
            .get_mut(index)
            .ok_or_else(|| Trap::Invalid(format!("person data has no word {index}")))?;
        *slot = value;
        let me = self.me;
        self.world.routing.unknown_words.remove(&(me, index));
        Ok(())
    }

    /// Object data word `index` of the running object.
    fn datum(&self, index: usize) -> Result<i16, Trap> {
        self.me()?
            .data
            .get(index)
            .copied()
            .ok_or_else(|| Trap::Invalid(format!("object data has no word {index}")))
    }

    /// Sets object data word `index` of the running object.
    fn set_datum(&mut self, index: usize, value: i16) -> Result<(), Trap> {
        *self
            .me_mut()?
            .data
            .get_mut(index)
            .ok_or_else(|| Trap::Invalid(format!("object data has no word {index}")))? = value;
        Ok(())
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
        // Any other location: the four straight directions, with a
        // greatest distance of 32.
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
