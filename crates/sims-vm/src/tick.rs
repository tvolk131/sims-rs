//! Ticks: one lot update after another.
//!
//! A tick is one call of the original's lot update (0x46d345):
//!
//! 1. The game's random stream is put aside and the lot's installed
//!    (0x46f447, 0x46f44d): scripts draw from [`World::lot_rng`]. The
//!    scheduler makes a pass over the objects (0x46d553,
//!    [`Interpreter::scheduler_pass`]). Then the lot's stream is saved and
//!    the game's restored: from here scripts draw from
//!    [`World::global_rng`].
//! 2. After a full pass, global 6 (seconds) goes up by 2 and the tick
//!    count by 1. At 60 seconds the seconds go back by 60 and global 5
//!    (minutes) goes up, and signal 0xdb is sent. Each further rollover
//!    (the hour, the day, the month) is not ported and fails.
//! 3. The registry's pass at the end of the update (0x4412d1,
//!    [`Interpreter::end_of_update`]): the dialogs, then the queued
//!    removals, then a check that a Sim is selected.
//! 4. The signal from step 2, if any, goes to the game's eight
//!    subscribers (0x40760d). Of those only the interface takes signal
//!    0xdb (0x4f65bf; a sound when the minutes are 0), so it changes
//!    nothing here.
//!
//! **Not modeled**, being presentation as far as the original's code
//! shows: an object's animated sprites (the list at object 0x148, stepped
//! after each thread run and redrawn), the parts of a person's animator
//! that only pose and draw the person (see [`crate::animator`]), the
//! graphics work every tenth tick (0x5a0932), and signal 0xf7, which a person's tick sends when the
//! selected Sim changes room and which only the house viewer takes
//! (0x4a82db, 0x4ab992). That none of these changes simulation state is
//! **unverified**. The lot's subscriber forwards each signal to a second
//! subscriber at lot 0x10 (0x416393) before its own cases; what that one
//! is, and what it does with 0xdb and 0xf7, is **unverified**. The
//! registry's pass also opens the game's special panels (registry words
//! 0xe8 to 0x108) when a script asked for one; no ported primitive asks,
//! so they never open here. When the pass showed a modal dialog the lot
//! update clears two simulator words (0xb8, 0xbc) that are not modeled.
//!
//! Read from the original's code; the old prototype
//! (legacy/crates/sims-vm/src/lib.rs, legacy/crates/sims-sim/src/clock.rs,
//! legacy/crates/sims-sim/src/removal.rs) was the guide. What a capture of
//! the original running these ticks confirms is in `docs/decisions.md`.

use crate::fault::{Fault, Trap};
use crate::interpreter::{Interpreter, Status, Stream};
use crate::primitives::dialog::{self, DONE, NEW, SHOWN};
use crate::selection::selected;
use sims_sim::motives::{self, Tuning};
use sims_sim::rooms::Room;
use sims_sim::{PersonTick, World};
use std::collections::VecDeque;
use std::fmt;

/// How many instructions a thread may run in one tick. The original
/// counts each thread's instructions from 0 at every run (object field 4,
/// 0x474ea1) and raises error 0x44d past the value at 0x6c4d70, which
/// was 458,605 in a memory image of the running game. Here a thread that
/// passes it fails; the original's handling of the error (the object's
/// virtual function 4) is not ported. Whether the value is the same in
/// every run is **unverified**.
pub const TICK_BUDGET: usize = 458_605;

/// Globals the clock uses.
const HOURS: usize = 0;
const MINUTES: usize = 5;
const SECONDS: usize = 6;
/// Globals that make the scheduler's pass partial: 17, or 19 and 10 both.
const PAUSED: usize = 17;
const MODE: usize = 19;
const LOT: usize = 10;

/// Scheduler flags (the registry word's high half).
const READY: u16 = 1;
const IN_PARTIAL: u16 = 2;
const CHECK_ERROR: u16 = 4;

/// Object data words.
const ROOM: usize = 29;
const HIDDEN: usize = 34;

/// Person data words.
mod person {
    /// Something for the interface to act on (0x4a69aa), cleared after.
    pub const REQUEST: usize = 1;
    /// Counted up every tick.
    pub const TICKS: usize = 27;
    /// Nonzero stops the motives decaying.
    pub const NO_DECAY: usize = 29;
    /// 0 for a member of the household that lives on the lot.
    pub const VISITOR: usize = 32;
    /// The kind of person: above 1 the room's enter and leave are skipped.
    pub const KIND: usize = 65;
}

/// The motive that tells whether a person is awake (0 and above).
const ALERTNESS: usize = 11;

/// What the player does when the game asks. A dialog that stops the game
/// (a modal one) is shown during the registry's pass, and the game waits
/// there for the answer (0x436da0).
pub trait Player {
    /// The answer to a dialog, as the status its button gives (2 for the
    /// first button, 3 the second, 4 the third), or None to give none.
    fn answer(&mut self, shown: &Shown) -> Option<u8>;
}

/// A dialog the registry shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The tick count when it is shown.
    pub tick: u32,
    /// The object that asked for it.
    pub object: u16,
    /// The answers it offers.
    pub answers: &'static [u8],
}

/// A player who answers from a list, in order. A dialog with one button
/// can only be answered with it, so it takes nothing from the list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answers {
    pub queue: VecDeque<u8>,
}

impl Player for Answers {
    fn answer(&mut self, shown: &Shown) -> Option<u8> {
        match shown.answers {
            [only] => Some(*only),
            _ => self.queue.pop_front(),
        }
    }
}

/// Why a tick could not be run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickError {
    /// A script or a service on an object failed.
    Fault(Fault),
    /// A dialog was shown and the player gave no answer, or one it does
    /// not offer.
    Unanswered { shown: Shown, given: Option<u8> },
    /// Something the lot update does that is not ported.
    Unsupported(String),
}

impl fmt::Display for TickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fault(fault) => fault.fmt(f),
            Self::Unanswered { shown, given } => {
                write!(
                    f,
                    "object {} shows a dialog that waits for an answer ({}); ",
                    shown.object,
                    shown
                        .answers
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )?;
                match given {
                    None => f.write_str("none was given"),
                    Some(answer) => write!(f, "{answer} was given"),
                }
            }
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
        }
    }
}

impl std::error::Error for TickError {}

impl From<Fault> for TickError {
    fn from(fault: Fault) -> Self {
        Self::Fault(fault)
    }
}

/// What one lot update did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Update {
    /// Whether the scheduler's pass was a full one, which advances the
    /// clock.
    pub full: bool,
    /// The objects whose countdown was counted down.
    pub counted: Vec<u16>,
    /// The objects ticked, in order.
    pub ticked: Vec<u16>,
    /// The dialogs shown and how each was answered.
    pub answered: Vec<(u16, u8)>,
    /// The objects the registry removed.
    pub removed: Vec<u16>,
}

impl Interpreter<'_> {
    /// One lot update (0x46d345). See the module's documentation.
    ///
    /// On an error the world is left as the failing step left it.
    pub fn lot_update(
        &self,
        world: &mut World,
        tuning: &Tuning,
        player: &mut dyn Player,
    ) -> Result<Update, TickError> {
        let mut update = Update::default();
        let lot = Interpreter {
            stream: Stream::Lot,
            ..*self
        };
        update.full = lot.scheduler_pass(world, tuning, &mut update)?;
        let game = Interpreter {
            stream: Stream::Global,
            ..*self
        };
        let mut signal = None;
        if update.full {
            world.ticks = world.ticks.wrapping_add(1);
            let seconds = global(world, SECONDS)?.wrapping_add(2);
            set_global(world, SECONDS, seconds)?;
            if seconds >= 60 {
                set_global(world, SECONDS, seconds.wrapping_sub(60))?;
                let minutes = global(world, MINUTES)?.wrapping_add(1);
                set_global(world, MINUTES, minutes)?;
                signal = Some(0xdb);
                if minutes >= 60 {
                    return Err(TickError::Unsupported(format!(
                        "the hour turns at tick {} (globals 0 and 5, the time of day, the \
                         lighting, signals 0xf8 and 0xdd, and the day's rollover), which is \
                         not ported",
                        world.ticks
                    )));
                }
            }
        }
        game.end_of_update(world, !update.full, player, &mut update)?;
        // Signal 0xdb reaches only the interface among the subscribers
        // (see the module's documentation).
        let _ = signal;
        Ok(update)
    }

    /// The scheduler's pass over the objects (0x46d553). Returns whether
    /// it was a full one.
    ///
    /// The pass is partial if global 17 is not 0, or if globals 19 and 10
    /// both are not. It goes by ID from the cursor (or 1) up to the
    /// highest ID it saw when it started; an object made during the pass
    /// with a higher ID waits for the next pass. For each object:
    ///
    /// 1. If its scheduler flags have bit 2 (value 4), the pass would stop
    ///    on an object with an error (object word 0x28); no object holds
    ///    one here, so the flag is just cleared.
    /// 2. Without flag 1 it is skipped, and in a partial pass so is one
    ///    without flag 2.
    /// 3. A countdown above 0 is counted down by 1. Otherwise the object
    ///    is ticked ([`Interpreter::tick_object`], or
    ///    [`Interpreter::tick_person`] for a person) with the tick count.
    ///    A tick that returns false stops the pass with the cursor on the
    ///    object.
    ///
    /// After the last object the cursor goes back to 0.
    ///
    /// The original's table of objects can hold IDs past the highest
    /// object it holds (an ID freed at the end and not reused); here the
    /// pass stops at the highest ID held. The difference shows only for
    /// an object made during a pass with such an ID: **unverified**.
    pub fn scheduler_pass(
        &self,
        world: &mut World,
        tuning: &Tuning,
        update: &mut Update,
    ) -> Result<bool, TickError> {
        let partial =
            global(world, PAUSED)? != 0 || (global(world, MODE)? != 0 && global(world, LOT)? != 0);
        let end = world.objects.keys().next_back().copied().unwrap_or(0);
        if world.scheduler_cursor == 0 {
            world.scheduler_cursor = 1;
        }
        let start = u16::try_from(world.scheduler_cursor).map_err(|_| {
            TickError::Unsupported(format!("a scheduler cursor of {}", world.scheduler_cursor))
        })?;
        for id in start..=end {
            world.scheduler_cursor = id as i16;
            let Some(object) = world.objects.get_mut(&id) else {
                continue;
            };
            if object.flags() & CHECK_ERROR != 0 {
                object.set_flags(CHECK_ERROR, false);
            }
            let flags = object.flags();
            if flags & READY == 0 || (partial && flags & IN_PARTIAL == 0) {
                continue;
            }
            let countdown = object.countdown().unwrap_or(0);
            if countdown > 0 {
                object.set_countdown(countdown - 1);
                update.counted.push(id);
                continue;
            }
            let tick = world.ticks as i32;
            let is_person = object.person.is_some();
            update.ticked.push(id);
            let carried_on = if is_person {
                self.tick_person(world, id, tick, tuning)?
            } else {
                self.tick_object(world, id)?
            };
            if !carried_on {
                return Err(TickError::Unsupported(format!(
                    "object {id}'s tick returned false, which stops the scheduler's pass with \
                     the cursor kept; carrying on in the next update is not ported"
                )));
            }
        }
        world.scheduler_cursor = 0;
        Ok(!partial)
    }

    /// An object's tick (0x4521f7): its thread runs (0x474e88) until it
    /// yields. If that returns true, data word 25 is counted down when it
    /// is not 0, and the object's animated sprites step (not modeled; see
    /// the module's documentation).
    ///
    /// The thread returns true when it yields. Returning from its
    /// outermost frame is error 0x3e9 in the original, which the object's
    /// error handler deals with; that fails here. No ported primitive
    /// returns the codes that make a thread return false.
    pub fn tick_object(&self, world: &mut World, id: u16) -> Result<bool, Fault> {
        let fail = |code, message| Fault {
            object: id,
            at: None,
            trap: Trap::native(code, message),
        };
        match self.turn(world, id, TICK_BUDGET)? {
            Status::Yielded => {}
            Status::Returned(_) => {
                return Err(fail(
                    0x3e9,
                    "the thread returned from its outermost frame; the original's error \
                     handling (the object's virtual function 4) is not ported",
                ));
            }
            Status::Running => {
                return Err(fail(
                    0x44d,
                    "the thread ran past the instruction limit of one tick; the original's \
                     error handling (the object's virtual function 4) is not ported",
                ));
            }
        }
        // The tail of 0x4521f7, as the factory runs it after an object's
        // first turn.
        let mut machine = self.machine(world, id, 0, false);
        machine.end_turn().map_err(|trap| machine.fault(trap))?;
        Ok(true)
    }

    /// A person's tick (0x45cc8c):
    ///
    /// 1. The object's tick ([`Interpreter::tick_object`]). If it returns
    ///    false, so does this.
    /// 2. Once 60 ticks have passed since the last decay (person field
    ///    0x810, 0 when the person is made), the decay tick is noted and,
    ///    if person data word 29 and data word 34 are 0, the motives decay
    ///    ([`Tuning::decay`], 0x45d145).
    /// 3. The person's animator ticks (its virtual function 0x50, 0x58dc72;
    ///    see [`crate::animator`]).
    /// 4. The room the person was last seen in (person field 0xafc) is
    ///    brought up to date from data word 29 (0x45e43d). When it changes
    ///    a person whose kind (person word 65) is at most 1 leaves the old
    ///    room, if no other person was last seen there (0x46886c), and
    ///    enters the new one (0x468802). An outdoor room ignores both.
    ///    Entering or leaving an indoor room (its occupied mark, its light
    ///    and signal 0xf1) is not ported and fails. The original then
    ///    sends signal 0xf7 for the selected Sim (not modeled).
    /// 5. Unless the room is still unknown (-5), motive 13 takes the
    ///    room's score (0x45cef4): the lot's outdoor score for an outdoor
    ///    room, the room's own score for an indoor one, 0 for a room that
    ///    does not exist. In room 0 on a lot that is not residential a
    ///    score below 0 counts as 0.
    /// 6. Person word 27 goes up by 1.
    /// 7. The motive changes the person's actions set run once
    ///    ([`motives::apply_changes`], 0x45cdb3).
    /// 8. Every tenth tick, for an awake member of the household who is
    ///    not hidden, the house's usage counts take a sample (0x4165e3,
    ///    [`sims_sim::HouseUsage::sample`]), which counts whether the
    ///    person has a route context (0x45c349). A nonzero person word 1
    ///    goes to 0x4a69aa and is cleared; that is not ported and fails.
    pub fn tick_person(
        &self,
        world: &mut World,
        id: u16,
        tick: i32,
        tuning: &Tuning,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        if !self.tick_object(world, id)? {
            return Ok(false);
        }
        let hour = world.globals.get(HOURS).copied().unwrap_or(0);
        let lot = world.globals.get(LOT).copied().unwrap_or(0);
        let residential = self.program.zoning.get(&lot) == Some(&1);
        let mut state = world
            .person_ticks
            .get(&id)
            .copied()
            .unwrap_or(PersonTick::CONSTRUCTED);
        {
            let object = world
                .objects
                .get_mut(&id)
                .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
            let hidden = object.data.get(HIDDEN).copied().unwrap_or(0);
            let person = object
                .person
                .as_mut()
                .ok_or_else(|| fail(Trap::native(0x1c, "object is not a person")))?;
            if tick >= state.last_decay.wrapping_add(60) {
                state.last_decay = tick;
                if person.data.get(person::NO_DECAY).copied().unwrap_or(0) == 0 && hidden == 0 {
                    tuning.decay(person, hour);
                }
            }
        }
        self.tick_animator(world, id).map_err(fail)?;
        let outdoor_score = world.rooms.outdoor_score;
        // The score of a room that exists (0x45cef4).
        let score = |room: &Room| {
            if room.outdoors {
                outdoor_score
            } else {
                room.score
            }
        };
        // 0x46886c: how many other people were last seen in the old
        // room, leaving out those with bit 0 of person word 84 or 68
        // set (0x4351ef). A person not ticked yet has the
        // constructor's -5.
        let others = world
            .objects
            .iter()
            .filter(|&(&other, _)| other != id)
            .filter_map(|(other, o)| Some((other, o.person.as_ref()?)))
            .filter(|(_, p)| {
                let bit = |index: usize| p.data.get(index).is_some_and(|w| w & 1 != 0);
                !bit(84) && !bit(68)
            })
            .filter(|(other, _)| {
                world
                    .person_ticks
                    .get(other)
                    .map_or(PersonTick::CONSTRUCTED.room, |t| t.room)
                    == state.room
            })
            .count();
        let object = world
            .objects
            .get_mut(&id)
            .ok_or_else(|| fail(Trap::native(10, "missing object")))?;
        let word = |data: &[i16], index: usize| data.get(index).copied().unwrap_or(0);
        let hidden = word(&object.data, HIDDEN);
        let room = word(&object.data, ROOM);
        let person = object
            .person
            .as_mut()
            .ok_or_else(|| fail(Trap::native(0x1c, "object is not a person")))?;
        if room != state.room {
            let kind = word(&person.data, person::KIND);
            for (old, new) in [(state.room, false), (room, true)] {
                let exists = !(-5..=-1).contains(&old);
                let indoors = exists
                    && world_room(&world.rooms.rooms, old).is_some_and(|room| !room.outdoors);
                if kind <= 1 && indoors && (new || others == 0) {
                    return Err(fail(Trap::Unsupported(format!(
                        "a person {} indoor room {old} (0x468802), which is not ported",
                        if new { "entering" } else { "leaving" }
                    ))));
                }
            }
            state.room = room;
        }
        if state.room != -5 {
            let mut value =
                world_room(&world.rooms.rooms, state.room).map_or(sims_core::F32::ZERO, score);
            // `jae` past the zeroing (0x45cd9c): a value that is not a
            // number is zeroed too.
            if state.room == 0 && !residential && below_zero(value) {
                value = sims_core::F32::ZERO;
            }
            person.motives[motives::ROOM] = value.to_f32();
        }
        if let Some(count) = person.data.get_mut(person::TICKS) {
            *count = count.wrapping_add(1);
        }
        motives::apply_changes(person);
        // `jb` past the call (0x45ce41): a value that is not a number
        // counts as asleep.
        let awake = !below_zero(sims_core::F32::from_f32(person.motives[ALERTNESS]));
        if tick % 10 == 0 && word(&person.data, person::VISITOR) == 0 && hidden == 0 && awake {
            // 0x45c349: the person's last route context, if it has any.
            // Only the routing primitives (Go To Routing Slot, 0x45abce;
            // Go To Relative Position, 0x45c634) push and pop them, and
            // they are not ported, so the count is the one the load read.
            let routing = person.routing_frames != 0;
            world.usage.sample(routing);
        }
        if word(&person.data, person::REQUEST) != 0 {
            return Err(fail(Trap::Unsupported(
                "person word 1 is set, which a person's tick hands to 0x4a69aa; not ported".into(),
            )));
        }
        world.person_ticks.insert(id, state);
        Ok(true)
    }

    /// The registry's pass at the end of a lot update (0x4412d1):
    ///
    /// 1. The dialogs, from the first record, starting over after each
    ///    one it acts on: a new record is shown (0x436da0: status 1) and,
    ///    if it stops the game, answered by the player then and there; a
    ///    finished one (status 5) is destroyed (0x448663).
    /// 2. After a full pass, the queued removals, oldest first: each ID is
    ///    taken off the queue and, if it still names an object, the
    ///    object is removed at once ([`Interpreter::remove_now`],
    ///    0x44123f). A tile of a multi-tile object is not ported.
    /// 3. If no Sim is selected, or the selected one is a visitor on a
    ///    residential lot, the game selects another (0x445501), which is
    ///    not ported and fails.
    pub fn end_of_update(
        &self,
        world: &mut World,
        partial: bool,
        player: &mut dyn Player,
        update: &mut Update,
    ) -> Result<(), TickError> {
        loop {
            let records = &mut world.dialogs.records;
            let Some(index) = records
                .iter()
                .position(|record| record.status == NEW || record.status == DONE)
            else {
                break;
            };
            if records[index].status == DONE {
                records.remove(index);
                continue;
            }
            let record = &mut records[index];
            record.status = SHOWN;
            let answers = dialog::answers(record.operands[5]).ok_or_else(|| {
                TickError::Unsupported(format!(
                    "object {}'s dialog is of a kind that is not ported",
                    record.caller
                ))
            })?;
            if !dialog::modal(&record.operands) {
                return Err(TickError::Unsupported(format!(
                    "object {} shows a dialog that does not stop the game; answering one \
                     while the game runs is not ported",
                    record.caller
                )));
            }
            let shown = Shown {
                tick: world.ticks,
                object: record.caller,
                answers,
            };
            let given = player.answer(&shown);
            let Some(answer) = given.filter(|answer| answers.contains(answer)) else {
                return Err(TickError::Unanswered { shown, given });
            };
            // 0x4370be: the button's result, or done at once for a dialog
            // that only informs.
            record.status = if dialog::informs(&record.operands) {
                DONE
            } else {
                answer
            };
            update.answered.push((record.caller, answer));
        }
        if !partial {
            while !world.removals.is_empty() {
                let id = world.removals.remove(0);
                let Some(object) = world.objects.get(&id) else {
                    continue;
                };
                let guid = object.guid;
                let master = self
                    .catalog
                    .object(guid)
                    .and_then(|d| self.program.definition(d.resource, d.chunk))
                    .map(|words| words[10])
                    .ok_or_else(|| Fault {
                        object: id,
                        at: None,
                        trap: Trap::Invalid(format!("no definition for GUID 0x{guid:08X}")),
                    })?;
                if master != 0 {
                    return Err(TickError::Unsupported(format!(
                        "removing object {id}, a tile of a multi-tile object, at the end of the \
                         lot update is not ported"
                    )));
                }
                if world.dialogs.records.iter().any(|r| r.caller == id) {
                    return Err(TickError::Unsupported(format!(
                        "removing object {id}, which owns a dialog, is not ported"
                    )));
                }
                self.remove_now(world, id)?;
                update.removed.push(id);
            }
        }
        let lot = global(world, LOT)?;
        let reselect = match selected(world) {
            None => true,
            Some(sim) => {
                let visitor = world
                    .objects
                    .get(&sim)
                    .and_then(|o| o.person.as_ref())
                    .and_then(|p| p.data.get(person::VISITOR))
                    .copied()
                    .unwrap_or(0);
                // The original also asks that a byte at 0x6e4f64 be 0,
                // which it is not known to set: **unverified**.
                self.program.zoning.get(&lot) == Some(&1) && visitor != 0
            }
        };
        if reselect {
            return Err(TickError::Unsupported(
                "no Sim of the household is selected at the end of the lot update, and \
                 selecting another (0x445501) is not ported"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// Below 0 or not a number: the carry flag an x87 compare with 0 sets.
fn below_zero(value: sims_core::F32) -> bool {
    !matches!(
        value.ext().partial_cmp(&sims_core::Ext::ZERO),
        Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
    )
}

/// A room by ID (0x466fcc).
fn world_room(rooms: &std::collections::BTreeMap<i16, Room>, id: i16) -> Option<&Room> {
    rooms.get(&id)
}

fn global(world: &World, index: usize) -> Result<i16, TickError> {
    world
        .globals
        .get(index)
        .copied()
        .ok_or_else(|| TickError::Unsupported(format!("the lot has no global {index}")))
}

fn set_global(world: &mut World, index: usize, value: i16) -> Result<(), TickError> {
    let slot = world
        .globals
        .get_mut(index)
        .ok_or_else(|| TickError::Unsupported(format!("the lot has no global {index}")))?;
    *slot = value;
    Ok(())
}
