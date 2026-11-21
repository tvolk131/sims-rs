//! Go To Routing Slot (opcode 45, 0x45abce): a person walks to a routing
//! slot of the stack object.
//!
//! The first run (the frame's primitive state 0) picks the slot and
//! notes on the object's slot table whether the route may end anywhere
//! near it (slot flag 0x4000, set when bit 0 of operand byte 4 is clear).
//! Then it runs the route's steps (0x45ad63), each run going on from the
//! step the frame's primitive state holds:
//!
//! - **0**: with the stack object gone or off the lot the run gives false
//!   (0x45ae65). Else a route context for the slot is made (0x48163e) and
//!   pushed on the person's contexts (+0xaf0); the next step is 1.
//!
//! Every run then takes the person's last context (0x45c349). With none
//! (a reset erased them) it gives false. A context that names no object
//! (one the load made for a person saved part way through a route) is
//! popped, the step goes back to 0 and the thread yields (0x45ae71).
//! - **1**: when the slot's object is in another room than the person
//!   (and bit 0 of person word 68 is clear), the next step is 2.
//!   Otherwise, a person who wants to be interrupted (0x45d7c0) and whose
//!   animator may stop ([`sims_sim::animator::Animator::interruptible`],
//!   0x589ead) fails with result 4, the next step 7 (0x45b18b). Else the
//!   places the person could stand are found (0x48235a); the next step is
//!   18.
//! - **2**, the way through a door (0x45afd0): a person who wants to be
//!   interrupted and whose animator may stop fails with result 4, the
//!   next step 7, as in step 1. Once the person is in the object's room,
//!   back to step 1. Otherwise a door tree that
//!   returned false marks its door failed, the door to go through is
//!   chosen (`routing/portals.rs`), and its door tree is pushed on the
//!   person's thread; the step stays 2 while that tree runs.
//! - **18**: the route there is planned (0x45c38e); with a route ready
//!   (2), the next step is 12. With no way found (0) the next step is 6.
//!   With the person already in the chosen seat (1) the next step is 8.
//!   For a place beside a seat, the seat's Sit tree is pushed on the
//!   person's thread with the seat as its stack object (0x45b96b); the
//!   next step is 4.
//! - **4**, sitting (0x45b3fa): the Sit tree returned true and the next
//!   step is 8; else the context's result becomes person word 62, or 5
//!   when that is 0, and the next step is 16.
//! - **6**, trying again (0x45b066), by the context's retry count
//!   (+0x58, as a 16-bit number): at 0 the person's place is compared
//!   with the one noted (+0x60, +0x64) and noted in its place; moved
//!   more than 8 units (squared 0x40), the count stays 0 and the next
//!   step is 1, else the count becomes 1 and step 6 runs again. At 1 the
//!   count becomes 2; with no person asked yet (context +0x80) another
//!   person in the way is looked for and asked to move
//!   (`primitives/in_way.rs`): asked, the next step is 14, else 16. At 2
//!   the count becomes 3 and the next step is 1; at 3 the next step is
//!   16; any other count goes to step 7.
//! - **14**, waiting for the person asked (0x45b475): a person who
//!   wants to be interrupted and whose animator may stop fails with
//!   result 4, the next step 7, as in step 1. With none asked
//!   the next step is 1 (and the count goes back to 0 when the asked
//!   person moved, context byte +0x84). Else, the first time, "route ask
//!   to move animation" is pushed on the thread with the person asked as
//!   its stack object and the tick noted (+0x5c); for the next 100 ticks
//!   "route waiting animation" is pushed; after that the noted tick is
//!   cleared and the next step is 6. The step stays 14 while those trees
//!   run.
//! - **16**, giving up (0x45aef7): the context's result becomes 1 when
//!   it was 0, the next step is 7 and person word 62 takes the result.
//!   When the route may end near the slot (slot flag 0x4000) and the
//!   slot's object's file has a "Route failure feedback" tree (0x45af2a),
//!   that tree is pushed on the person's thread with the object as its
//!   stack object and the arguments (the result, context word +0x8c, 0,
//!   0), the person starts using the object, person word 72 is cleared
//!   and the next step is 17 (0x45b6ba).
//! - **17**, back from that tree (0x45afb7), whatever it returned: person
//!   word 62 takes the result again and the next step is 7.
//! - **7**: the route ends as failed ([`Machine::end_route`]).
//! - **12**: the animator starts the route (0x589c85); the next step is 3
//!   and the thread yields.
//! - **3**, on the ticks after (0x45b2b7): a person who wants to be
//!   interrupted and whose animator may stop takes result 4 and the next
//!   step is 11. Otherwise, while the animator's route is under way
//!   (0x589d47 gives 2), the thread yields. When it reached the path's
//!   end (1) the next step is 9, else 10.
//! - **9**: the animator stops (0x589e2d); the next step is 8.
//! - **10**: the animator stops short of the path's end, as when its
//!   next step was blocked (0x45b371); the next step is 6.
//! - **11**, interrupted on the way (0x45b3c2): the animator stops; the
//!   next step is 7.
//! - **8**: the route ends ([`Machine::end_route`]): the result is true.
//! - **19** and **20**, the stack object lost (0x45ae8d): on every run
//!   after the first, a stack object that is gone or off the lot sends
//!   the route to step 20 if it was under way (step 3), else to step 19.
//!   Step 20 asks the animator to stop (0x45b352, 0x589e2d), with no
//!   sound: once it stops, the next step is 19; while it does not, the
//!   run yields in step 20. The next run tests the stack object first,
//!   so if it is still lost the saved step 20 becomes 19 and the route
//!   fails without waiting for the animator any longer. Step 19 fails the
//!   route: the next step is 7 (0x45afc4).
//! - With byte +0x78 of the context clear, a run at any step but 2 or 3
//!   only sets it again (0x45aebb); steps 2 and 3 fail loudly
//!   (0x45aea5).
//!
//! The steps run one after another in a run until one yields or pushes a
//! tree. Every run works on the person's last context. Its retry count
//! (+0x58) and noted place (+0x60, +0x64) are read into the run at its
//! start and written back when the run ends, before a door tree or an
//! animation is pushed, and not after the route ended (0x45b9a0).
//!
//! When a route ends, every other person's last context that asked this
//! person to move with this person's running action forgets it (byte
//! +0x84 noting whether that route was under way), and the action this
//! route asked another person to do is taken out of that person's queue
//! (0x45d5f8), the action's object told it was cancelled; if that person
//! is already running it, it is cancelled as the player cancels a running
//! action (0x45d617).
//!
//! Go To Relative Position (opcode 27, 0x45c634) runs the same steps
//! with a slot record it makes from its operands instead of one of the
//! object's slots ([`relative_slot`]).
//!
//! The slot is named by its index (operand word 0) in a routing slot
//! table, which holds only the SLOT entries of kind 3. Operand word 1
//! says which table and how (0x45ac22): 1, the object's
//! ([`crate::Program::routing_slot`]) by the literal index; 0, the
//! object's by the index in that parameter; 2, the lot's global routing
//! slots ([`crate::Program::global_routing_slot`]) by the literal index,
//! still placed by the stack object. A missing slot, or any other table,
//! is native error 0x25. Flag 0x4000 is written into the lot's one copy
//! of a global slot, shared by every object, kept here under
//! [`GLOBAL_SLOTS`].
//!
//! A slot record an object holds in place of its type's (the move slot
//! of the marker made to ask a person to move, `world.routing.slots`) is
//! used instead of the type's slot.
//!
//! A person sitting (posture 1) is there if it sits in the seat beside a
//! candidate; otherwise it runs its seat's Stand tree first (step 5,
//! 0x45b1d7 to 0x45b6d4). No door to go through gives up with result 2
//! (0x45b036). Giving up on a slot the route may end near runs the stack
//! object's "Route failure feedback" tree (step 17, 0x45af2a).
//!
//! Ported from the original's code. What the Goth house's routes to tick
//! 9906 do not reach fails loudly: a parameter number past the frame's
//! parameters, a person with bit 0 of word 68 set or other contexts,
//! every other step and every other result of the planning. Past tick 1000 the routes
//! are **unverified**: there is no capture.

use crate::animator::ask_stop;
use crate::fault::{Fault, Trap};
use crate::interpreter::{Flow, Machine, word};
use crate::placement::{capture, stands_on_lot};
use crate::routing::slot::{SlotRoute, direction, new_context, route_slot};
use crate::sounds::SoundEvent;
use sims_sim::animator::LOADED;
use sims_sim::route::RouteSlot;

/// The object ID under which the slot flags of the lot's global routing
/// slots are kept: no object has it.
pub(crate) const GLOBAL_SLOTS: u16 = 0;

/// Object data words.
const DIRECTION: usize = 1;
/// The object functions of a seat: Sit (callback 0x1a) and Stand
/// (callback 0x1b).
const SIT: usize = 26;
const STAND: usize = 27;
const FLAGS: usize = 8;
const ROOM: usize = 29;
/// Person words.
const POSTURE: usize = 0;
const WORD_72: usize = 72;
const NO_ROOM: usize = 68;
/// Takes the route's result as the route ends (0x45b869).
const ROUTE_RESULT: usize = 62;
/// Read and set as the route ends (0x45b885).
/// Person word 37: the seat side a person stood up on (0x45b6de), or the
/// chosen candidate's word +0xe.
const WORD_37: usize = 37;
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
const SITTING: i32 = 4;
/// Step 5: the seat's Stand tree runs before the route (0x45b439).
const STANDING: i32 = 5;
const RETRY: i32 = 6;
const FAILED: i32 = 7;
const SUCCEEDED: i32 = 8;
const ARRIVED: i32 = 9;
const STOPPED: i32 = 10;
const INTERRUPTED: i32 = 11;
const START: i32 = 12;

/// The sound events a route plays as it starts and after it stops
/// (0x6c4530, 0x6c4570).
const BEGIN_ROUTE: &str = "begin_route";
const END_ROUTE: &str = "end_route";
const ASK: i32 = 14;
const GIVE_UP: i32 = 16;
const FEEDBACK: i32 = 17;
const PLAN: i32 = 18;
/// Step 19: the stack object was lost; the route fails (0x45afc4).
const LOST: i32 = 19;
/// Step 20: the stack object was lost while the route was under way; the
/// animator stops before step 19 (0x45b352).
const LOST_UNDER_WAY: i32 = 20;

/// The trees a person asking another to move plays (0x6c4554, 0x6c453c).
const ASK_TREE: &str = "route ask to move animation";
const WAIT_TREE: &str = "route waiting animation";
/// The tree of the slot's object a person giving up on a slot it may end
/// near plays (0x6c457c).
const FEEDBACK_TREE: &str = "Route failure feedback";
/// How many ticks a person waits for one asked to move (0x45b56e).
const WAIT: i32 = 100;
/// The squared distance a person must have moved since the last try for
/// step 6 to plan again at once (0x45b0a1).
const MOVED: i32 = 0x40;

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
        // 0x40743b and 0x432595 (through 0x4325b5 and 0x422632) read the
        // stack object's own fields only: its position (+0xf4), its
        // container (+0x66, +0x7e) and +0x110. No animator is read before
        // this test, so a first run whose stack object is off the lot
        // gives false whether or not an animator refresh is pending, and
        // neither is one read naming the slot (native error 0x25 at
        // 0x45ace4 comes first). The route that follows reads the person's
        // animator, so the step's checks for an unsettled animator apply
        // from there.
        if !stands_on_lot(self.world, target)? {
            return Ok(Flow::Branch(false));
        }
        let source = word(operands, 2);
        let number = word(operands, 0);
        let (owner, index) = match source {
            // Its number from a parameter (0x45ac2e). The original's bound
            // check (0x45ac34) never fails, and reads past the parameters
            // for a number out of their range.
            0 => {
                let parameters = &self.frame()?.parameters;
                let index = usize::try_from(number)
                    .ok()
                    .and_then(|n| parameters.get(n).copied())
                    .ok_or_else(|| {
                        unsupported(
                            "0x45ac5c",
                            &format!(
                                "naming its slot by parameter {number} of {}, past the \
                                 parameters",
                                parameters.len()
                            ),
                        )
                    })?;
                (target, index)
            }
            1 => (target, number),
            // One of the lot's global routing slots (0x45acf0).
            2 => (GLOBAL_SLOTS, number),
            _ => return Err(Trap::native(0x25, "Go To Routing Slot names no slot")),
        };
        let (entry, type_slot) = if owner == GLOBAL_SLOTS {
            usize::try_from(index)
                .ok()
                .and_then(|i| self.program.global_routing_slot(i))
        } else {
            let guid = self.world.objects[&target].guid;
            let definition = self.definition(guid)?;
            usize::try_from(index).ok().and_then(|i| {
                self.program
                    .routing_slot(definition.resource, definition.chunk, i)
            })
        }
        .ok_or(Trap::native(0x25, "Go To Routing Slot names no slot"))?;
        self.refuse_unsettled_animator(45)?;
        let type_slot = type_slot.clone();
        let key = (owner, index as u16);
        let near = |flags: i32| {
            if operands[4] & 1 == 0 {
                flags | NEAR
            } else {
                flags & !NEAR
            }
        };
        if let Some(record) = self.world.routing.slots.get_mut(&key) {
            // A record written over (a marker's first slot, 0x45c2ab).
            record.flags = near(record.flags);
            let slot = *record;
            return self.route_steps(Some(slot));
        }
        let flags = self
            .world
            .routing
            .slot_flags
            .get(&key)
            .copied()
            .unwrap_or(type_slot.flags as i32);
        let flags = near(flags);
        if flags == type_slot.flags as i32 {
            self.world.routing.slot_flags.remove(&key);
        } else {
            self.world.routing.slot_flags.insert(key, flags);
        }
        let slot = route_slot(&type_slot, entry, Some(flags));
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
        // The slot record the first run is given; later runs are given
        // none (0x45ad54), and every run reads the context's copy.
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
            // 0x45ae65: with its stack object gone or off the lot, the run
            // gives false and writes nothing.
            let (Some(target), Some(slot), false) = (target, slot, lost) else {
                return Ok(Flow::Branch(false));
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
            // 0x45c349 finds no context (a reset erased them): the run
            // gives false and writes nothing (0x45ae65).
            return Ok(Flow::Branch(false));
        };
        if ctx.target == 0 {
            // 0x45ae71: a context that names no object, as the load makes
            // them (`crate::routing::slot::loaded_context`): the step goes
            // back to 0, the context is popped (0x45f8d9) and the thread
            // yields. The next run starts the route again.
            self.frame_mut()?.primitive_state = NEW;
            if let Some(route) = self.world.routing.routes.get_mut(&me) {
                route.contexts.pop();
            }
            if self.synchronous {
                return Err(Trap::native(
                    0x44f,
                    "a synchronous tree yielded in Go To Routing Slot",
                ));
            }
            return Ok(Flow::Yield);
        }
        if lost {
            // 0x45ae8d: with the stack object gone or off the lot, a route
            // under way stops its animator first (step 20); any other step
            // goes to step 19, which fails the route.
            state = if state == UNDER_WAY {
                LOST_UNDER_WAY
            } else {
                LOST
            };
        }
        if !ctx.byte_78 {
            // 0x45aea5: steps 2 and 3 do more first (context +0x68, or
            // step 15), which is not ported; every other step, the lost
            // steps 19 and 20 among them, only sets the byte again
            // (0x45aebb), written back with the context.
            if matches!(state, PORTAL | UNDER_WAY) {
                return Err(unsupported(
                    "0x45aea5",
                    &format!("at step {state} with byte +0x78 of its context clear"),
                ));
            }
            ctx.byte_78 = true;
        }
        // The retry stage (+0x58) and the place of the last try (+0x60,
        // +0x64) are the run's own; they are written back when the run
        // yields or pushes a tree.
        let mut retry = ctx.word_58;
        let mut spot = ctx.saved;
        let mut result = 2;
        let interpreter = self.interpreter();
        let mut path = self
            .world
            .routing
            .routes
            .get(&me)
            .map(|r| r.path.clone())
            .unwrap_or_default();
        // A check tree's fault keeps where it happened, on this person's
        // thread too.
        let inner = |fault: Fault| {
            if fault.object == me && !fault.holds_check() {
                fault.trap
            } else {
                fault.unsupported_in("Go To Routing Slot")
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
                        // 0x45b140: to another room, through a door; but
                        // a route to a door one of the person's routes
                        // goes through (context +0x68) fails (0x45b176).
                        state = PORTAL;
                        if interpreter.kind(self.world, ctx.target)? == DOOR {
                            let routes = &self.world.routing.routes[&me];
                            if routes
                                .contexts
                                .iter()
                                .any(|c| c.word_68 == i32::from(ctx.target))
                            {
                                state = FAILED;
                            }
                        }
                        continue;
                    }
                    // 0x45b18b: a person who wants to be interrupted, and
                    // whose animator may stop (0x589ead), fails with
                    // context +0x88 = 4.
                    if self.interrupted()? {
                        ctx.result = 4;
                        state = FAILED;
                        continue;
                    }
                    interpreter
                        .find_candidates(self.world, &mut ctx)
                        .map_err(inner)?;
                    state = PLAN;
                    if self.person_word(POSTURE)? == 1 {
                        // 0x45b1d7: a person sitting in a seat beside a
                        // candidate is there; else it stands up first.
                        if interpreter
                            .seated_beside_candidate(self.world, &ctx)
                            .map_err(inner)?
                        {
                            state = SUCCEEDED;
                            continue;
                        }
                        // 0x45b1ef: no seat, or one without a Sit and a
                        // Stand function, fails the route.
                        state = FAILED;
                        let Some(seat) = capture(self.world, me)?.container else {
                            continue;
                        };
                        let tree = interpreter.function_tree(self.world, seat, STAND);
                        if !interpreter.has_function(self.world, seat, SIT) || tree == 0 {
                            continue;
                        }
                        // 0x45b6d4: the route waits at step 5 while the
                        // seat's Stand tree runs, person word 37 takes the
                        // context's third start word and word 72 is
                        // cleared (0x45b962). As for the Sit tree, the
                        // step and the context's saved words are written
                        // before the tree is pushed, and put back if it
                        // cannot be.
                        let side = ctx.start[2] as i16;
                        let kept = (
                            self.me()?.clone(),
                            self.world.routing.routes.get(&me).cloned(),
                        );
                        ctx.word_58 = i32::from(retry as i16);
                        ctx.saved = spot;
                        self.store_route(&ctx, &path);
                        self.frame_mut()?.primitive_state = STANDING;
                        if let Err(trap) = self.stand_up(seat, tree, side) {
                            let (object, route) = kept;
                            self.world.objects.insert(me, object);
                            match route {
                                Some(route) => self.world.routing.routes.insert(me, route),
                                None => self.world.routing.routes.remove(&me),
                            };
                            return Err(trap);
                        }
                        self.set_person_word(WORD_37, side)?;
                        self.world.routing.unknown_words.remove(&(me, WORD_37));
                        self.set_person_word(WORD_72, 0)?;
                        return Ok(Flow::Pushed);
                    }
                }
                STANDING => {
                    // 0x45b439: person word 72 goes back to what it was
                    // when the route began. The Stand tree returned true
                    // (person +0x1c is 0xfe, 0x474e79) and the person
                    // stands (posture 0): the candidates are found again.
                    // Else it gives up with result 6.
                    self.set_person_word(WORD_72, ctx.word_94)?;
                    if self.me()?.last_result && self.person_word(POSTURE)? == 0 {
                        state = CANDIDATES;
                    } else {
                        ctx.result = 6;
                        state = GIVE_UP;
                    }
                }
                PLAN => {
                    let planned = interpreter
                        .route_to_slot(self.world, &mut ctx, &mut path)
                        .map_err(inner)?;
                    match planned {
                        SlotRoute::Answer(0) => {
                            // 0x45b2ab: no way.
                            result = 0;
                            state = RETRY;
                            continue;
                        }
                        // 0x45b1e3: already in the seat.
                        SlotRoute::Answer(1) => {
                            result = 1;
                            state = SUCCEEDED;
                        }
                        SlotRoute::Answer(2) => {
                            result = 2;
                            state = START;
                        }
                        SlotRoute::Answer(other) => {
                            return Err(unsupported(
                                "0x45b285",
                                &format!("whose planning gave {other}"),
                            ));
                        }
                        SlotRoute::Sit { seat, tree } => {
                            // 0x45b96b: the route waits at step 4 while the
                            // seat's Sit tree runs, and person word 72 is
                            // cleared (0x45b962). The step and the
                            // context's saved words are written before the
                            // tree is pushed, as the frame changes with it.
                            // A Sit tree that cannot be pushed fails loudly,
                            // so what was written for it is put back first.
                            let kept = (
                                self.me()?.clone(),
                                self.world.routing.routes.get(&me).cloned(),
                            );
                            ctx.word_58 = i32::from(retry as i16);
                            ctx.saved = spot;
                            self.store_route(&ctx, &path);
                            self.frame_mut()?.primitive_state = SITTING;
                            if let Err(trap) = self.sit(seat, tree) {
                                let (object, route) = kept;
                                self.world.objects.insert(me, object);
                                match route {
                                    Some(route) => self.world.routing.routes.insert(me, route),
                                    None => self.world.routing.routes.remove(&me),
                                };
                                return Err(trap);
                            }
                            self.set_person_word(WORD_72, 0)?;
                            return Ok(Flow::Pushed);
                        }
                    }
                }
                SITTING => {
                    // 0x45b3fa: the Sit tree returned true (person +0x1c
                    // is 0xfe, the target of the return's branch, so the
                    // same as the last result) and the route succeeded;
                    // else it gives up with person word 62 as its result,
                    // or 5 for none.
                    if self.me()?.last_result {
                        state = SUCCEEDED;
                    } else {
                        state = GIVE_UP;
                        ctx.result = match self.person_word(ROUTE_RESULT)? {
                            0 => 5,
                            word => i32::from(word),
                        };
                    }
                }
                PORTAL => {
                    // 0x45afd0: a person who wants to be interrupted, and
                    // whose animator may stop, fails with result 4
                    // (0x45afea), as in step 1.
                    if self.interrupted()? {
                        ctx.result = 4;
                        state = FAILED;
                        continue;
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
                        // 0x45b036: no door to go through; the route gives
                        // up with result 2.
                        ctx.result = 2;
                        state = GIVE_UP;
                        continue;
                    };
                    // 0x45b954: the route stays at this step, and person
                    // word 72 is cleared, while the door's tree runs. The
                    // step and the context's saved words are written before
                    // the tree is pushed, as the frame changes with it.
                    ctx.word_68 = i32::from(door);
                    ctx.word_58 = i32::from(retry as i16);
                    ctx.saved = spot;
                    self.store_route(&ctx, &path);
                    self.frame_mut()?.primitive_state = PORTAL;
                    self.enter_door(door, tree)?;
                    interpreter.try_portal(self.world, me, door)?;
                    self.set_person_word(WORD_72, 0)?;
                    return Ok(Flow::Pushed);
                }
                START => {
                    // 0x45b989: "begin_route" plays for me.
                    self.sound(SoundEvent::Play {
                        object: me as i16,
                        name: BEGIN_ROUTE.into(),
                    });
                    self.store_route(&ctx, &path);
                    interpreter.start_route(self.world, me)?;
                    state = UNDER_WAY;
                    break;
                }
                UNDER_WAY => {
                    // 0x45b2b7: a person who wants to be interrupted, and
                    // whose animator may stop, stops with result 4
                    // (0x45b1ac) and step 11.
                    if self.interrupted()? {
                        ctx.result = 4;
                        state = INTERRUPTED;
                        continue;
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
                ARRIVED | STOPPED | INTERRUPTED => {
                    // 0x45b3de, 0x45b371, 0x45b3c2: the animator stops
                    // (0x589e2d).
                    // A route under way does not stop: the request is kept
                    // for the walk's next step (+0x284), and the run
                    // yields at this step to try again (0x45b9a0).
                    if !ask_stop(self.world, me)? {
                        break;
                    }
                    // 0x45b3a0: "end_route" plays for me. A route that
                    // stopped short of the path's end (a blocked step)
                    // tries again (0x45b384); one interrupted fails
                    // (0x45b3d5).
                    self.sound(SoundEvent::Play {
                        object: me as i16,
                        name: END_ROUTE.into(),
                    });
                    state = match state {
                        STOPPED => RETRY,
                        INTERRUPTED => FAILED,
                        _ => SUCCEEDED,
                    };
                }
                RETRY => match retry as i16 {
                    // 0x45b066: once the person has moved more than 8
                    // since the last try, plan again at once; else go on.
                    0 => {
                        let [first, second, _] = self.me()?.position;
                        let at = [second, first];
                        let dx = at[0].wrapping_sub(spot[0]);
                        let dy = at[1].wrapping_sub(spot[1]);
                        spot = at;
                        if dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) > MOVED {
                            retry = 0;
                            state = CANDIDATES;
                        } else {
                            retry = 1;
                        }
                    }
                    // 0x45b0b9: ask whoever is in the way to move.
                    1 => {
                        retry = 2;
                        state = if ctx.objects[1] == 0 && self.find_in_way(&mut ctx)? {
                            ASK
                        } else {
                            GIVE_UP
                        };
                    }
                    2 => {
                        retry = 3;
                        state = CANDIDATES;
                    }
                    3 => state = GIVE_UP,
                    _ => state = FAILED,
                },
                ASK => {
                    // 0x45b475: a person who wants to be interrupted, and
                    // whose animator may stop, fails with result 4
                    // (0x45b48d to 0x45afea), as in step 1.
                    if self.interrupted()? {
                        ctx.result = 4;
                        state = FAILED;
                        continue;
                    }
                    if ctx.objects[1] == 0 {
                        // Released by the person asked (0x45b8f4).
                        state = CANDIDATES;
                        if ctx.byte_84 {
                            retry = 0;
                        }
                        ctx.time = 0;
                        continue;
                    }
                    let clock = self.world.ticks as i32;
                    let (name, stack_object) = if ctx.time == 0 {
                        (ASK_TREE, Some(ctx.objects[1]))
                    } else if clock.wrapping_sub(ctx.time) < WAIT {
                        (WAIT_TREE, None)
                    } else {
                        ctx.time = 0;
                        state = RETRY;
                        continue;
                    };
                    let owner = self.definition(self.me()?.guid)?.resource;
                    let Some(tree) = self
                        .program
                        .named_tree(owner, name)
                        .map_err(Trap::Unsupported)?
                    else {
                        ctx.time = 0;
                        state = RETRY;
                        continue;
                    };
                    // The step stays 14 while the tree runs. The context
                    // and the step are written before the tree is pushed,
                    // as the frame changes with it.
                    if stack_object.is_some() {
                        ctx.time = clock;
                    }
                    ctx.word_58 = i32::from(retry as i16);
                    ctx.saved = spot;
                    self.store_route(&ctx, &path);
                    self.frame_mut()?.primitive_state = ASK;
                    self.play(tree, stack_object)?;
                    return Ok(Flow::Pushed);
                }
                GIVE_UP => {
                    // 0x45aef7.
                    if ctx.result == 0 {
                        ctx.result = 1;
                    }
                    state = FAILED;
                    self.set_person_word(ROUTE_RESULT, ctx.result as i16)?;
                    // 0x45af1d: the slot record here is the context's
                    // copy (0x5c354f, set at 0x45aec6 on every run), not
                    // the one the first run is given.
                    if ctx.slot.flags & NEAR != 0
                        && let Some((object, tree)) = self.feedback_tree()?
                    {
                        // 0x45af2a: the slot's object's "Route failure
                        // feedback" tree, with the result and word +0x8c
                        // as its first two arguments (0x45afac). Only
                        // once it is pushed does the step become 17 and
                        // person word 72 get cleared (0x45b6ba), and the
                        // context get written (0x45b9a0), so a tree that
                        // cannot be pushed leaves the route as it was.
                        // The routing frame is now the one below the top.
                        let args = [ctx.result as i16, ctx.word_8c, 0, 0];
                        self.give_feedback(object, tree, &args)?;
                        self.set_person_word(WORD_72, 0)?;
                        ctx.word_58 = i32::from(retry as i16);
                        ctx.saved = spot;
                        self.store_route(&ctx, &path);
                        let stack = &mut self.me_mut()?.stack;
                        let below = stack.len().checked_sub(2);
                        below
                            .and_then(|at| stack.get_mut(at))
                            .ok_or_else(|| {
                                Trap::Invalid("no routing frame below the feedback tree".into())
                            })?
                            .primitive_state = FEEDBACK;
                        return Ok(Flow::Pushed);
                    }
                }
                FEEDBACK => {
                    // 0x45afb7: back from the feedback tree, whatever it
                    // returned.
                    self.set_person_word(ROUTE_RESULT, ctx.result as i16)?;
                    state = FAILED;
                }
                LOST_UNDER_WAY => {
                    // 0x45b352: the animator is asked to stop (0x589e2d),
                    // with no sound; while it does not, the run yields at
                    // this step (0x45b9a0).
                    if !ask_stop(self.world, me)? {
                        break;
                    }
                    state = LOST;
                }
                // 0x45afc4.
                LOST => state = FAILED,
                FAILED | SUCCEEDED => return self.end_route(state, ctx, &path),
                other => {
                    return Err(unsupported("0x45aef0", &format!("at step {other}")));
                }
            }
        }
        // 0x45b9a0: the step is kept in the frame, and the run's retry
        // stage (as a 16-bit word) and place written to the context.
        ctx.word_58 = i32::from(retry as i16);
        ctx.saved = spot;
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
    /// 4. With a running action (person +0xa84 not 0), every other
    ///    person's last context that asked this person (+0x80) to do it
    ///    (+0x90) forgets the person and the action, its byte +0x84 noting
    ///    whether this route was under way (state 8) (0x45b89b). When
    ///    this context asked a person to move, that action is taken out
    ///    of the person's queue (0x45d5f8,
    ///    [`Machine::release_action`]); a person already running it has
    ///    the running action cancelled instead (0x45d617).
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
        // A check tree's fault keeps where it happened, on this person's
        // thread too.
        let inner = |fault: Fault| {
            if fault.object == me && !fault.holds_check() {
                fault.trap
            } else {
                fault.unsupported_in("the end of a route")
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
        if self
            .world
            .animators
            .get(&me)
            .is_some_and(|animator| animator.arrival != 0)
        {
            ask_stop(self.world, me)?;
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
        // 0x45b89b: with a running action, release the people whose last
        // context waits on this person for it.
        let running = self
            .me()?
            .person
            .as_ref()
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .active
            .uid;
        if running != 0 {
            for (_, route) in self
                .world
                .routing
                .routes
                .iter_mut()
                .filter(|(id, _)| **id != me)
            {
                if let Some(last) = route.contexts.last_mut()
                    && last.objects[1] == me
                    && last.action == running
                {
                    last.byte_84 = state == SUCCEEDED;
                    last.objects[1] = 0;
                    last.action = 0;
                }
            }
        }
        // 0x45b914: the person this one asked to move no longer needs to.
        if ctx.objects[1] != 0 {
            self.release_action(ctx.objects[1], ctx.action)?;
        }
        if let Some(route) = self.world.routing.routes.get_mut(&me) {
            route.contexts.pop();
            route.path = path.to_vec();
        }
        self.frame_mut()?.primitive_state = state;
        Ok(Flow::Branch(result))
    }

    /// An object's room: its data word 29.
    /// The route's interrupt test (0x45b18b, 0x45afd0, 0x45b2b7,
    /// 0x45b475): the person wants to be interrupted (0x45d7c0) and its
    /// animator may stop (virtual function 0x30, 0x589ead).
    fn interrupted(&self) -> Result<bool, Trap> {
        Ok(self.wants_interrupt()?
            && self
                .world
                .animators
                .get(&self.me)
                .unwrap_or(&LOADED)
                .interruptible())
    }

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
        self.count_use(door, false)
    }

    /// Runs a seat's Sit tree on the person's thread, as the person's
    /// virtual function 0x34 does (0x45e59b) with no arguments given and
    /// no flag (0x45c49d): the person is woken, the tree of the seat's
    /// type is pushed with the seat as its stack object and the person's
    /// temps as its arguments, and the person starts using the seat. A
    /// tree that cannot be pushed, where the original's route gives the
    /// context's result 5 and plans again (0x45c4a8), fails loudly.
    fn sit(&mut self, seat: u16, tree: u16) -> Result<(), Trap> {
        let temps = self.me()?.temps.clone();
        self.wake(self.me)?;
        self.enter_on(seat, tree, &temps).map_err(|fault| {
            Trap::Unsupported(format!(
                "a Sit tree {tree:#06x} of seat {seat} that cannot be pushed, where the \
                 original's route gives result 5 (0x45c4a8): {fault}"
            ))
        })?;
        self.count_use(seat, false)
    }

    /// Runs a seat's Stand tree on the person's thread, as the person's
    /// virtual function 0x34 does (0x45e59b) with no flag (0x45b26a): the
    /// person is woken, the tree of the seat's type is pushed with the
    /// seat as its stack object and the arguments `side`, 0, 0, 0
    /// (0x45b247), and the person starts using the seat. A tree that
    /// cannot be pushed, where the original's route fails (step 7), fails
    /// loudly, as does one with more than four arguments (0x47484a would
    /// read them past the four it is given).
    fn stand_up(&mut self, seat: u16, tree: u16, side: i16) -> Result<(), Trap> {
        self.push_on(seat, tree, &[side, 0, 0, 0], "0x45b26f")
    }

    /// Runs tree `tree` of object `target`'s type on the person's thread
    /// with the arguments `args`, as the person's virtual function 0x34
    /// does (0x45e59b) with no flag: the person is woken, the tree pushed
    /// with `target` as its stack object, and the person starts using
    /// `target`. A tree that cannot be pushed, where the original's route
    /// fails (at `address`), fails loudly, as does one with more
    /// arguments than `args` holds (0x47484a would read past them).
    fn push_on(
        &mut self,
        target: u16,
        tree: u16,
        args: &[i16; 4],
        address: &str,
    ) -> Result<(), Trap> {
        self.wake(self.me)?;
        self.enter_on(target, tree, args).map_err(|fault| {
            Trap::Unsupported(format!(
                "a tree {tree:#06x} of object {target} that cannot be pushed, where the \
                 original's route fails ({address}): {fault}"
            ))
        })?;
        self.count_use(target, false)
    }

    /// Plays a tree of the person's own on its thread, as its virtual
    /// function 0x34 does (0x45e59b) with no arguments given: the person is
    /// woken, the tree pushed with the person as its stack object and its
    /// temps as the arguments, and the person counts a use of itself. Then
    /// the new frame's stack object becomes `stack_object` if given
    /// (0x45b53a). A tree that cannot be pushed, where the original goes
    /// on to step 6, fails loudly.
    fn play(&mut self, tree: u16, stack_object: Option<u16>) -> Result<(), Trap> {
        let me = self.me;
        let temps = self.me()?.temps.clone();
        self.wake(me)?;
        self.enter_on(me, tree, &temps).map_err(|fault| {
            Trap::Unsupported(format!(
                "a route's tree {tree:#06x} that cannot be pushed, where the original goes on \
                 to step 6 (0x45b51e): {fault}"
            ))
        })?;
        self.count_use(me, false)?;
        if let Some(id) = stack_object {
            self.frame_mut()?.stack_object = id as i16;
        }
        Ok(())
    }

    /// The "Route failure feedback" tree of the frame's stack object, the
    /// slot's object (0x45af5b, 0x4021f2 in the object's own file), with
    /// that object; None when its file has no tree of that name, where the
    /// route goes on to step 7. A stack object that is gone, which the
    /// original would read through a null pointer, fails loudly.
    fn feedback_tree(&self) -> Result<Option<(u16, u16)>, Trap> {
        let stack_object = self.frame()?.stack_object;
        let object = u16::try_from(stack_object)
            .ok()
            .filter(|id| self.world.objects.contains_key(id))
            .ok_or_else(|| {
                unsupported(
                    "0x45af5b",
                    &format!("a route giving up with its stack object {stack_object} gone"),
                )
            })?;
        let owner = self.definition(self.world.objects[&object].guid)?.resource;
        let tree = self
            .program
            .named_tree(owner, FEEDBACK_TREE)
            .map_err(Trap::Unsupported)?;
        Ok(tree.map(|tree| (object, tree)))
    }

    /// Plays the slot's object's feedback tree on the person's thread, as
    /// the person's virtual function 0x34 does (0x45e59b) with arguments
    /// and no flag: the person is woken, the tree of the object's type is
    /// pushed with the object as its stack object and the four arguments
    /// (0x452b98, 0x47484a), and the person starts using the object. A
    /// tree that cannot be pushed, where the original goes on to step 7
    /// (0x45afb2), fails loudly, as does one with more than four
    /// arguments (0x47484a would read past the four it is given).
    fn give_feedback(&mut self, object: u16, tree: u16, args: &[i16; 4]) -> Result<(), Trap> {
        self.wake(self.me)?;
        self.enter_on(object, tree, args).map_err(|fault| {
            Trap::Unsupported(format!(
                "a Route failure feedback tree {tree:#06x} of object {object} that cannot be \
                 pushed, where the original goes on to step 7 (0x45afb2): {fault}"
            ))
        })?;
        self.count_use(object, false)
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
    pub(super) fn set_datum(&mut self, index: usize, value: i16) -> Result<(), Trap> {
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
