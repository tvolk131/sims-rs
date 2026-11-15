//! A person's animator tick (virtual function 0x50, 0x58dc72), as far as
//! the simulation reads it.
//!
//! The person's tick (0x45cc8c) calls this after the decay and before the
//! room. In order:
//!
//! 1. The clock (0x585483, [`Animator::clock_at`]) is read and kept
//!    (+0xb4). It comes from the lot's tick count (virtual function 0x38
//!    of the lot), not from the tick the person's tick is given: the
//!    person's first tick, which a new person gets with tick 0 while it
//!    is made, reads the current tick count.
//! 2. A blend layer (+0x10) with no weight is removed. Nothing ported
//!    fills that slot, so it is always empty here.
//! 3. The carry clip is chosen (0x5903a3). Unless a script's carry clip
//!    plays (+0x1a8 is 6), or a clip that picks something up plays
//!    (+0x1a4), the person carries something when its first contained
//!    slot holds an object (0x589fce); then the carry clip is 0, else -1.
//!    A change clears the carry slot at once, and a change to clip 0
//!    starts the posture's clip 0x26 there, forward, to stop at its end,
//!    at priority 10 ([`Interpreter::start_carry`]). The layer is updated
//!    with the skin's others in the tail.
//! 4. The base layer is brought up to date (virtual function 0x58,
//!    0x590430): a posture other than the one it was started for clears
//!    it; then, unless the definition's word 102 has bit 2 set, or the
//!    layer is there and the person's kind (word 65) is at most 1, the
//!    posture is noted and, for postures 0 to 2, the posture's clip
//!    started: the posture's clip 3, 1 or 2 by the count of
//!    [`Interpreter::posture_clip`]. Starting it first looks the clip up
//!    (virtual function 0xf4, 0x592625), which reads the person's
//!    appearance again if the animator says so
//!    ([`Animator::profile_stale`]); then the clip goes into the base slot
//!    (0x592726) to repeat at priority -10, which marks the skin's layers
//!    changed; its playback is kept in [`World::base_layers`] and updated
//!    with the skin's others in the tail, timed events included. A
//!    posture whose clip has no name (0x59db2f gives none) starts nothing
//!    and leaves the slot as it was (0x5904af), to be tried again on the
//!    next tick. A name no file holds fails ([`Interpreter::start_base`]).
//! 5. The speed moves towards the speed wanted (0x588fa0) by at most 0.2
//!    up; down it moves at once.
//! 6. An animator whose person stands off the lot (0x422632) does nothing
//!    more before the tail.
//! 7. By state (the jump table at 0x58faa2): in state 0, a clip that
//!    follows the animator's speed takes it (0x598636), keeping its
//!    direction. In state 3, an animator not placed yet places itself on
//!    its person's tile (virtual function 0x60, 0x592132); one placed
//!    puts both directions into 0 to 8 and, arrived and facing the way it
//!    should, notes the route's end (0x188: 0 stays 0; -1, 2 and 3 become
//!    2). In states 1 and 2 a clip still playing is cleared and the base
//!    layer started again, unless person word 78 holds it in state 1
//!    (0x58f0eb, 0x58f12e; [`Interpreter::seated`]). States 4, 6 and 9
//!    are ported below. Any other state, and an animator in state 3 that
//!    has to walk, is not ported.
//! 8. The tail: the skin's layers are updated once at the clock, in the
//!    skin's order: by priority, which a changed skin sorts its layers by
//!    first (0x5991fd), then [`Animator::carry_first`] (0x5992b3, from
//!    0x5a0ef5 or
//!    from 0x58f8bf: the second call finds the skin updated at the same
//!    clock and does nothing), the animator places
//!    itself if it has not, and the head takes its step, measures where
//!    it and its target are and takes its turn off again (0x590768,
//!    0x5904d2, 0x590b27; see `crate::head`).
//!
//! Left out as presentation, as far as the original's code shows: the
//! draw state (0x434101), the face (0x58faf6), the pose as kept (0x5994c3,
//! 0x5a15bc; it is worked out when the head needs it), what a carried
//! object's position follows (the tail of 0x58dc72), and the route's own
//! position
//! fields (+0x18 to +0x28) which only states 6 to 10 read. That none of
//! this changes what the simulation reads is **unverified**.
//!
//! A person that takes itself off the lot during its own tick (Find
//! Location, [`Interpreter::take_self_off`]) has its animator refreshed
//! there if the house view drew it (0x58da12, animator byte +0x109): a
//! small tick of the clock, the skin forced, the place and the head.
//! Whether the view drew it is not kept, so the take-off checks that the
//! refresh would write nothing but the animator and its base layer
//! ([`Interpreter::refresh_at_take_off`]), and the animator's tick that
//! the refresh then the tick reach what the tick alone does
//! ([`Interpreter::tick_animator_after_take_off`]); anything else fails.
//!
//! The person's tick passes the animator a flag (byte 0x6e48c0 is 0)
//! that picks which skin routine runs first (0x5a0ffd or 0x5a0ef5); either
//! way the layers are updated once, and which routine ran changes only the
//! pose.

use crate::fault::Trap;
use crate::interpreter::Interpreter;
use crate::people::read_appearance_if_stale;
use crate::placement::{Kept, Pending, Place, on_lot, pool_tile};
use sims_core::{Ext, F32};
use sims_sim::World;
use sims_sim::animator::{Animator, Base, EPSILON, LOADED, Layer, UNSET};
use sims_sim::pose;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BTreeSet;

/// Person data words.
const POSTURE: usize = 0;
const KIND: usize = 65;
/// Object data words.
const DIRECTION: usize = 1;
/// The word 0x588fa0 tests for 1 (object +0x6c).
const PACE: usize = 17;
/// The priority of the base slot's layer (0x5904c1, 0x58957e).
const BASE_PRIORITY: i32 = -10;
/// The definition word whose bit 2 keeps the base layer from starting
/// (definition +0xcc).
const DEFINITION_FLAGS: usize = 102;

/// How far the speed moves up per tick (0x6d07e8).
const STEP: F32 = F32::from_bits(0x3e4c_cccd);
/// The speed wanted, before its base (0x6d079c).
const WANTED: F32 = F32::from_bits(0x3f99_999a);
/// What the base is scaled by when it is at least 0 (0x6d07a0), or below
/// (0x6d07a4).
const SCALE: F32 = F32::from_bits(0x3e4c_cccd);
const SCALE_BELOW: F32 = F32::from_bits(0x3ecc_cccd);
/// Eight eighths of a turn (0x66f3c0, a double; 8 in either width).
const TURN: i32 = 8;

impl Interpreter<'_> {
    /// Person `id`'s animator tick (0x58dc72), at the lot's tick count.
    /// See the module documentation.
    pub(crate) fn tick_animator(&self, world: &mut World, id: u16) -> Result<(), Trap> {
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?;
        let person = object
            .person
            .as_ref()
            .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?;
        let word = |data: &[i16], index: usize| data.get(index).copied().unwrap_or(0);
        let kind = word(&person.data, KIND);
        let pace = word(&object.data, PACE);
        let position = object.position;
        let direction = word(&object.data, DIRECTION);
        let carrying = object
            .slots
            .get(1)
            .map(|slot| slot.object != 0)
            .ok_or_else(|| {
                Trap::Unsupported(
                    "a person with no contained slot: the original's carry test (0x589fce) \
                     reads through a null pointer"
                        .into(),
                )
            });
        let flags = self.base_flags(world, id)?;

        let mut a = world.animators.get(&id).cloned().unwrap_or(LOADED);
        let now = a.clock_at(world.ticks as i32);
        a.clock = now;

        // 0x5903a3: the carry clip.
        if a.carry_mode != 6 {
            let mut next = -1;
            if a.pickup == 0 {
                a.carry_mode = i32::from(carrying?);
                if (1..=5).contains(&a.carry_mode) {
                    next = 0;
                }
            }
            if next != a.carry_selection {
                // A change clears the carry slot (virtual function 0x104,
                // 0x592878: at once, which marks the skin's layers
                // changed).
                if a.carry.take().is_some() {
                    a.skin_dirty = true;
                }
                if next == 0 {
                    self.start_carry(world, id, &mut a, now)?;
                }
                a.carry_selection = next;
            }
        }

        // 0x590430: the base layer.
        self.update_base(world, id, &mut a, flags)?;

        // 0x588fa0 and 0x58dd03: the speed. Reading the appearance may
        // have changed the kind.
        let kind = world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .map_or(kind, |p| word(&p.data, KIND));
        let target = wanted_rate(pace, a.state, kind);
        a.target = target.to_f32();
        let rate = a.rate.ext();
        let step = STEP.ext();
        a.rate = if (target - step).single() > rate && target > rate {
            (step + rate).single().to_f32()
        } else {
            target.to_f32()
        };

        if on_lot([position[0], position[1]]) {
            self.dispatch(world, id, &mut a, position, direction, now)?;
        }

        // 0x5992b3: the skin's update.
        self.update_skin(world, id, &mut a, now, false)?;
        place(&mut a, position, direction);

        // 0x590768, 0x5904d2 and 0x590b27: the head.
        self.look(world, id, &mut a)?;
        world.animators.insert(id, a);
        Ok(())
    }
}

impl Interpreter<'_> {
    /// The skin's update at clock `now` (0x5992b3): unless `force`d, only
    /// when its layers changed or it was last updated at another clock.
    fn update_skin(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        now: i32,
        force: bool,
    ) -> Result<(), Trap> {
        if !force && !a.skin_dirty && now == a.skin_clock {
            return Ok(());
        }
        // 0x5991fd: a changed skin first sorts its layers by priority,
        // lowest first, keeping the order of layers of the same priority.
        // Every change marks the skin changed, so the layers are always in
        // that order when they update: the list's own order
        // (Animator::carry_first) decides only a tie.
        let carry_first = carry_first(a);
        a.skin_dirty = false;
        // The base layer, at priority -10, is the lowest.
        self.update_base_layer(world, id, a, now)?;
        if carry_first {
            self.update_carry(world, id, a, now)?;
            self.update_foreground(world, id, a, now)?;
        } else {
            self.update_foreground(world, id, a, now)?;
            self.update_carry(world, id, a, now)?;
        }
        a.skin_clock = now;
        self.note_translations(world, id, a)
    }

    /// Notes in [`World::translated_bones`] the bones whose translation a
    /// layer of person `id`'s skin could have written at its update:
    /// every motion with positions of each layer with something bound,
    /// frames and weight (0x597c4c, 0x598226). The original samples only
    /// when its update is asked to pose (0x59820a), and a motion's
    /// positions only up to the clip's count; neither is told apart here,
    /// so the set may hold more than the original moved.
    fn note_translations(&self, world: &mut World, id: u16, a: &Animator) -> Result<(), Trap> {
        let base = world.base_layers.get(&id);
        let layers = [base, a.foreground.as_ref(), a.carry.as_ref()];
        let mut moved = Vec::new();
        for layer in layers.into_iter().flatten() {
            if !layer.bound || layer.frames == 0 || layer.weight.ext() <= Ext::ZERO {
                continue;
            }
            let clip = self.program.animations.clip(&layer.clip).map_err(|e| {
                Trap::Unsupported(format!(
                    "person {id}'s skin samples a clip {:?} that cannot be found (0x59dc32): {e}",
                    layer.clip
                ))
            })?;
            moved.extend(
                clip.channels
                    .iter()
                    .filter(|c| c.translation.is_some())
                    .map(|c| &c.bone),
            );
        }
        if moved.is_empty() {
            return Ok(());
        }
        let known = world.translated_bones.entry(id).or_default();
        for bone in moved {
            if !known.contains(bone) {
                known.insert(bone.clone());
            }
        }
        Ok(())
    }

    /// Person `id`'s animator refresh as a take-off makes it when the
    /// house view drew the person (0x58da12 with +0x109 set; see
    /// [`Interpreter::take_self_off`]): the clock read again, the skin's
    /// update forced at it, the animator placed, and the head's step,
    /// measurement and restoration. What the original also does there and
    /// the animator's tick leaves out (the draw state, the face, the pose)
    /// is left out here too, and so is the redraw of what the head slot
    /// holds (`crate::placement::head_slot_redraw`), which is checked
    /// before.
    fn refresh_animator(&self, world: &mut World, id: u16) -> Result<(), Trap> {
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?;
        let position = object.position;
        let direction = object.data.get(DIRECTION).copied().unwrap_or(0);
        let mut a = world.animators.get(&id).cloned().unwrap_or(LOADED);
        let now = a.clock_at(world.ticks as i32);
        a.clock = now;
        self.update_skin(world, id, &mut a, now, true)?;
        place(&mut a, position, direction);
        self.look(world, id, &mut a)?;
        world.animators.insert(id, a);
        Ok(())
    }

    /// Person `id`'s animator refresh at its taking itself off the lot
    /// ([`Interpreter::take_self_off`]), worked out on a copy of `world`
    /// as the take-off leaves it. It fails unless the refresh writes
    /// nothing but the person's animator and base layer and plays no
    /// sound, so that what runs before the person's animator tick sees
    /// the same lot whether or not the original made it. The sounds of
    /// timed events are not modeled, so a timed event with a sound key
    /// fails here ([`crate::clip_events`]).
    pub(crate) fn refresh_at_take_off(&self, world: &World, id: u16) -> Result<Pending, Trap> {
        let sounds = RefCell::new(Vec::new());
        let apart = Interpreter {
            sounds: self.sounds.map(|_| &sounds),
            take_off_refresh: true,
            ..*self
        };
        let mut refreshed = world.clone();
        apart.refresh_animator(&mut refreshed, id)?;
        let pending = Pending {
            left: Kept::of(world, id),
            refreshed: Kept::of(&refreshed, id),
        };
        pending.left.restore(&mut refreshed, id);
        if refreshed.encode() != world.encode() || !sounds.borrow().is_empty() {
            return Err(Trap::Unsupported(format!(
                "person {id} took itself off the lot, and its animator's refresh there (0x58da12), \
                 made only if the house view drew it (animator byte +0x109, which is not kept), \
                 would write more than its animator or play a sound"
            )));
        }
        Ok(pending)
    }

    /// Person `id`'s animator tick after it took itself off the lot in its
    /// thread's run, with its animator's refresh there worked out
    /// (`pending`, [`Interpreter::refresh_at_take_off`]). Whether the
    /// original made the refresh is not known, so both branches must
    /// agree:
    ///
    /// 1. The animator and its base layer are as the take-off left them,
    ///    so nothing in between wrote them (the primitives allowed in
    ///    between, [`crate::placement::reads_no_animator`], read none of
    ///    them).
    /// 2. The tick from the refreshed animator and base layer, on the lot
    ///    as it is now, reaches the same world, and the same sounds, as
    ///    the tick alone.
    ///
    /// The tick alone is what is kept. Anything else fails.
    pub(crate) fn tick_animator_after_take_off(
        &self,
        world: &mut World,
        id: u16,
        pending: Pending,
    ) -> Result<(), Trap> {
        let refused = |what: &str| {
            Trap::Unsupported(format!(
                "person {id} took itself off the lot and {what}, so whether the house view drew \
                 it, which decides its animator's refresh at the take-off (0x58da12, animator \
                 byte +0x109), matters; that is not kept"
            ))
        };
        if Kept::of(world, id) != pending.left {
            return Err(refused(
                "its animator or base layer changed before its animator's tick",
            ));
        }
        let sounds = RefCell::new(Vec::new());
        let apart = Interpreter {
            sounds: self.sounds.map(|_| &sounds),
            ..*self
        };
        let mut refreshed = world.clone();
        pending.refreshed.restore(&mut refreshed, id);
        apart.tick_animator(&mut refreshed, id)?;
        let before = self.sounds.map_or(0, |s| s.borrow().len());
        self.tick_animator(world, id)?;
        let same_sounds = self
            .sounds
            .is_none_or(|s| s.borrow()[before..] == sounds.borrow()[..]);
        if refreshed.encode() != world.encode() || !same_sounds {
            return Err(refused(
                "the refresh then the tick reach another state than the tick alone",
            ));
        }
        Ok(())
    }

    /// Updates the foreground layer, if there is one, at clock `now`
    /// ([`Interpreter::update_layer`]).
    fn update_foreground(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        now: i32,
    ) -> Result<(), Trap> {
        if let Some(mut layer) = a.foreground.take() {
            let updated = self.update_layer(world, id, a, &mut layer, now);
            a.foreground = Some(layer);
            updated?;
        }
        Ok(())
    }

    /// Updates the base slot's layer, if there is one, at clock `now`
    /// ([`Interpreter::update_layer`]). A filled slot with no layer kept
    /// is a broken state.
    fn update_base_layer(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        now: i32,
    ) -> Result<(), Trap> {
        match (a.base.is_filled(), world.base_layers.remove(&id)) {
            (false, None) => Ok(()),
            (true, Some(mut layer)) => {
                let updated = self.update_layer(world, id, a, &mut layer, now);
                world.base_layers.insert(id, layer);
                updated
            }
            (filled, _) => Err(Trap::Invalid(format!(
                "person {id}'s base slot is {} but its layer is {}",
                if filled { "filled" } else { "empty" },
                if filled { "missing" } else { "kept" },
            ))),
        }
    }

    /// Updates the carry layer, if there is one, at clock `now`
    /// ([`Interpreter::update_layer`]).
    fn update_carry(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        now: i32,
    ) -> Result<(), Trap> {
        if let Some(mut layer) = a.carry.take() {
            let updated = self.update_layer(world, id, a, &mut layer, now);
            a.carry = Some(layer);
            updated?;
        }
        Ok(())
    }
}

/// The speed the animator moves towards (0x588fa0): 1.2 plus a fifth of a
/// base, or two fifths of a base below 0. The base is 1, except for a
/// person whose object word 17 is 1: 8 in state 0, and in states 4 to 9 8
/// for a person of kind (word 65) at most 1 and 0 for any other.
pub(crate) fn wanted_rate(pace: i16, state: i32, kind: i16) -> Ext {
    let one = Ext::from_i32(1);
    let eight = Ext::from_i32(8);
    let base = if pace != 1 {
        one
    } else if state == 0 {
        eight
    } else if state <= 3 || state > 9 {
        one
    } else if kind <= 1 {
        eight
    } else {
        Ext::ZERO
    };
    let scale = if base >= Ext::ZERO {
        SCALE
    } else {
        SCALE_BELOW
    };
    ((scale.ext() * base).single() + WANTED.ext()).single()
}

/// Virtual function 0x60 (0x592132), as far as the simulation reads it:
/// each component of the position not given yet, compared within
/// [`EPSILON`], takes the person's own (object +0xf8, +0xf4 and data word
/// 1), and so does the destination's.
pub(crate) fn place(a: &mut Animator, position: [i32; 3], direction: i16) {
    let own = [position[0], position[1], i32::from(direction)];
    for (i, own) in own.into_iter().enumerate() {
        if (a.position[i].ext() - UNSET.ext()).single().abs() <= EPSILON.ext() {
            let value = Ext::from_i32(own).single().to_f32();
            a.position[i] = value;
            a.destination[i] = value;
        }
    }
}

/// What a state's code does next: the tail, or the code of the state it
/// set (0x58f089).
enum Next {
    Done,
    Again,
}

/// How many times one tick may go from state to state before it fails.
/// The original has no limit; the Goth house's first route goes from
/// state 3 to 9, to 3 and to 4.
const REDISPATCH: usize = 16;

/// The global tables of clip names (0x6f7698, filled by 0x59d860), by
/// their index there: 0 is string table 150, 1 the child's (151), 2 and 3
/// the adult's by sex (152, 153).
const NAMES: u16 = 150;
const CHILD_NAMES: u16 = 151;
const ADULT_NAMES: u16 = 152;
/// The type's own table of clip names, read first (0x457b84: STR# 0x96
/// of the definition's file).
const OWN_NAMES: u16 = 150;
/// Person words the name lookup reads (0x59db2f).
const AGE: usize = 58;
const SPECIAL: usize = 64;
/// Person word 78 (+0x628): while set, a sitting animator (state 1) keeps
/// its foreground clip (0x58f0f6).
const SEATED_HOLD: usize = 78;
/// The posture clip whose distance gives the walk's look-ahead (0x58831d,
/// virtual function 0x120 with 0x14).
const WALK: usize = 0x14;
/// The posture clip the animator carries with (0x5903fe).
const CARRY: usize = 0x26;
/// The largest turn made walking, for a person of kind above 1
/// (0x667454); otherwise it is 2 (0x6d07b4).
const SMALL_TURN: F32 = F32::from_bits(0x3f4c_cccd);
/// How far a walk turns per tick (0x6d07b8), and for a person of kind
/// above 1 (0x58ec99).
const STEER: F32 = F32::from_bits(0x3e4c_cccd);
const SMALL_STEER: F32 = F32::from_bits(0x3d69_78d5);
/// What a walk's step is worked out with (0x668390, 0x666a54): a third,
/// 16, and the animator's milliseconds per tick (virtual function 0x38,
/// 0x585502).
const THIRD: F32 = F32::from_bits(0x3eaa_aaab);
const SIXTEEN: F32 = F32::from_bits(0x4180_0000);
const TICK_MILLISECONDS: i32 = 42;
/// The route's end (+0x188) when the walk's next step is blocked
/// (0x58edeb, 0x58f682).
const BLOCKED: i32 = 3;
/// Bounds of the turn clips (0x6d07c4, 0x6d07c8; 3 is 0x6d07cc).
const TURN_HALF: F32 = F32::from_bits(0x3f00_0000);
const TURN_ONE_AND_HALF: F32 = F32::from_bits(0x3fc0_0000);
/// Layer end modes (layer +0x40): stop at the end, or repeat.
const STOP: i32 = 0;
const REPEAT: i32 = 1;
/// The posture's clips whose distances set how far a walk goes before it
/// stops (0x58831d: +0x1f8 from clip 0x13, +0x204 from clip 0x16, with
/// data word 17 set).
const STOP_WALK: usize = 0x13;
const STOP_RUN: usize = 0x16;
/// The stopping distance at the end of a path (0x6d07d8, or 0x6d07dc
/// with data word 17 set), and what it is scaled by (0x668654).
const STOP_DISTANCE: F32 = F32::from_bits(0x4000_0000);
const STOP_RUN_DISTANCE: F32 = F32::from_bits(0x4040_0000);
const STOP_SCALE: F32 = F32::from_bits(0x3fc0_0000);
/// Object data word 9: set, the end of a path stops at the stopping
/// distance (0x58e7c9).
const STOPS: usize = 9;
/// 0x666a28: the least share of the stride a stop fades the walk to.
const MIN_SHARE: F32 = F32::from_bits(0x3e4c_cccd);
/// 0x6d07d4: the least time, in milliseconds, a stop takes.
const MIN_STOP_TIME: i32 = 100;
/// The walk clip with data word 17 set (state 10).
const RUN: usize = 0x15;
/// Object data word 22, which the animator's scale writes (0x59209f).
const SCALE_WORD: usize = 22;

/// What state 6 works its direction's angle out with (0x666cbc,
/// 0x667430, 0x66742c): 6, 45 and 1/180.
const SIX: F32 = F32::from_bits(0x40c0_0000);
const FORTY_FIVE: F32 = F32::from_bits(0x4234_0000);
const DEGREE: F32 = F32::from_bits(0x3bb6_0b61);
/// The phase of clip 0x13 at which state 6 ends (0x672020).
const STOP_WALK_END: F32 = F32::from_bits(0x3f73_3333);

/// Constants of the animator's code.
const ONE: F32 = F32::from_bits(0x3f80_0000);
const HALF: F32 = F32::from_bits(0x3f00_0000);
const QUARTER: F32 = F32::from_bits(0x3e80_0000);
const TWO: F32 = F32::from_bits(0x4000_0000);
const THREE: F32 = F32::from_bits(0x4040_0000);
const FOUR: F32 = F32::from_bits(0x4080_0000);
const EIGHT: F32 = F32::from_bits(0x4100_0000);
const PI: F32 = F32::from_bits(0x4049_0fdb);
const SCALE_UNIT: F32 = F32::from_bits(0x4380_0000);
/// 0.5 and 1.0 as doubles (0x667270, 0x667280).
const HALF_DOUBLE: u64 = 0x3fe0_0000_0000_0000;
const ONE_DOUBLE: u64 = 0x3ff0_0000_0000_0000;

/// Where state 6 carries a person (0x58e125): from `position` (the
/// direction third, in eighths of a turn) one clip's distance times a
/// third times 16 straight ahead, at the angle (direction + 6) times 45
/// times pi times 1/180. The cosine is stored as a float first; the sine
/// is used as the FPU leaves it (here libm's double), which is
/// **unverified** at rounding boundaries.
fn stop_walk_destination(position: [F32; 3], clip_distance: F32) -> [F32; 2] {
    let distance = ((clip_distance.ext() * THIRD.ext()).single() * SIXTEEN.ext()).single();
    let angle = ((((position[2].ext() + SIX.ext()).single() * FORTY_FIVE.ext()).single()
        * PI.ext())
    .single()
        * DEGREE.ext())
    .single()
    .to_f32();
    let cosine = Ext::cos(angle).single();
    let sine = Ext::sin(angle);
    [
        ((cosine * distance).single() + position[0].ext())
            .single()
            .to_f32(),
        ((sine * distance).single() + position[1].ext())
            .single()
            .to_f32(),
    ]
}

/// One tick of state 6 (0x58e1e4): while the clip's phase `t` is below
/// 0.95 (not a number counts as below), the point (1 - t) times `from`
/// plus t times `to`; None from 0.95 on, when the walk ends.
fn stop_walk_step(from: [F32; 2], to: [F32; 2], t: F32) -> Option<[F32; 2]> {
    let t = t.ext();
    if at_least(t, STOP_WALK_END.ext()) {
        return None;
    }
    let u = (ONE.ext() - t).single();
    let mix = |from: F32, to: F32| {
        ((u * from.ext()).single() + (t * to.ext()).single())
            .single()
            .to_f32()
    };
    Some([mix(from[0], to[0]), mix(from[1], to[1])])
}

/// The clip of each posture's base layer, as [`Interpreter::posture_clip`]
/// counts them, for postures 0 to 2 (0x59048c).
const BASE_CLIPS: [usize; 3] = [3, 1, 2];

/// The clip of `posture`'s base layer among the posture's clips
/// (0x59048c to 0x5904a1), or None for a posture with no base layer.
pub(crate) fn base_selection(posture: i32) -> Option<usize> {
    usize::try_from(posture)
        .ok()
        .and_then(|p| BASE_CLIPS.get(p))
        .copied()
}

fn unsupported(at: &str, what: &str) -> Trap {
    Trap::Unsupported(format!("the animator {what} ({at}), which is not ported"))
}

impl Interpreter<'_> {
    /// The state's code (the jump table at 0x58faa2), run again each time
    /// a state hands over to another (0x58f089).
    fn dispatch(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        position: [i32; 3],
        direction: i16,
        now: i32,
    ) -> Result<(), Trap> {
        for _ in 0..REDISPATCH {
            let next = match a.state {
                0 => {
                    if a.follow_rate {
                        let layer = a.foreground.as_mut().ok_or_else(|| {
                            Trap::Unsupported(
                                "the animator follows its speed with no foreground clip; the \
                                 original reads through a null pointer (0x58f0b6)"
                                    .into(),
                            )
                        })?;
                        let speed = if layer.speed.ext() >= Ext::ZERO {
                            a.rate
                        } else {
                            (-a.rate.ext()).to_f32()
                        };
                        layer.set_speed(speed);
                    }
                    Next::Done
                }
                1 | 2 => self.seated(world, id, a)?,
                3 => {
                    if a.position.iter().any(|&p| p.ext() == UNSET.ext()) {
                        place(a, position, direction);
                        Next::Done
                    } else {
                        self.stand(world, id, a, now)?
                    }
                }
                4 => turn(world, id, a)?,
                6 => self.stop_walk(world, id, a, now)?,
                9 => self.follow(world, id, a, now)?,
                state => {
                    return Err(Trap::Unsupported(format!(
                        "the animator in state {state} (0x58faa2), which is not ported"
                    )));
                }
            };
            if let Next::Done = next {
                return Ok(());
            }
        }
        Err(Trap::Unsupported(format!(
            "the animator changed state {REDISPATCH} times in one tick"
        )))
    }

    /// The base layer brought up to date (virtual function 0x58,
    /// 0x590430; the module documentation, step 4). `flags` is the
    /// definition's word 102.
    pub(crate) fn update_base(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        flags: i16,
    ) -> Result<(), Trap> {
        let words = &world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
            .data;
        let posture = i32::from(words.get(POSTURE).copied().unwrap_or(0));
        let kind = words.get(KIND).copied().unwrap_or(0);
        if a.posture != posture {
            // 0x592878 removes the base slot's layer at once (0x5a0a7b),
            // which marks the skin's layers changed (0x5991d4). A clip
            // Animate Sim put there goes the same way.
            clear_base(world, id, a);
        }
        // A layer in the slot stays for a person of kind at most 1,
        // whoever started it; for any other kind the posture's clip
        // replaces it, a clip Animate Sim put there included.
        if flags & 4 == 0 && !(a.base.is_filled() && kind <= 1) {
            a.posture = posture;
            if let Some(selection) = base_selection(posture) {
                read_appearance_if_stale(world, self.program, id, &mut a.profile_stale)?;
                // 0x5904af: with no name the lookup gives no clip, and
                // nothing starts.
                let Some(name) = self.posture_clip(world, id, selection)? else {
                    return Ok(());
                };
                let now = a.clock_at(world.ticks as i32);
                self.start_base(world, id, a, &name, false, now)?;
                a.base = Base::Posture;
            }
        }
        Ok(())
    }

    /// Starts clip `name` in person `id`'s base slot at clock `now`
    /// (virtual function 0x100, 0x592726, then +0x40 = 1 and 0x59867f
    /// with -10, as 0x5904b9 and Animate Sim's 0x58957e both do): the
    /// slot is emptied first (virtual function 0x104, 0x592878, at once)
    /// and the new layer made at priority 1 at the clock, forward from
    /// phase 0 or, with `reverse`, backward from phase 1 (0x597a88,
    /// 0x59281b); then it repeats (end mode 1) at priority -10. Either
    /// step marks the skin's layers changed. The caller says what the
    /// slot holds ([`Animator::base`]); the layer goes to
    /// [`World::base_layers`].
    ///
    /// Fails for what is not ported: a name no file holds, which the
    /// lookup (0x592625, 0x59dc32) or the start (0x592726 failing at
    /// 0x592748) would pass over.
    pub(crate) fn start_base(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        name: &str,
        reverse: bool,
        now: i32,
    ) -> Result<(), Trap> {
        let clip =
            self.program.animations.clip(name).map_err(|e| {
                unsupported("0x59dc32", &format!("finds no base clip {name:?}: {e}"))
            })?;
        let mut layer = Layer::start(
            name,
            F32::from_bits(clip.duration),
            clip.frames,
            clip.motions > 0,
            clip.tracks.len(),
            reverse,
            now,
        );
        layer.mode = 1;
        layer.priority = BASE_PRIORITY;
        world.base_layers.insert(id, layer);
        a.skin_dirty = true;
        Ok(())
    }

    /// The definition word 102 of person `id`'s type, whose bit 2 keeps
    /// the base layer from starting.
    pub(crate) fn base_flags(&self, world: &mut World, id: u16) -> Result<i16, Trap> {
        let guid = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?
            .guid;
        Ok(self.machine(world, id, 0, false).definition_words(guid)?[DEFINITION_FLAGS])
    }

    /// States 1 and 2, sitting and the posture after it (0x58f0eb,
    /// 0x58f12e): a foreground clip that still plays is cleared (virtual
    /// function 0xfc, 0x586b63; see [`clear_foreground`]), the base
    /// layer's posture forgotten (+0x198 = -1) and the base layer brought
    /// up to date (virtual function 0x58, [`Interpreter::update_base`]),
    /// except in state 1 while person word 78 (+0x628) is set. Either way
    /// the animator stops walking ([`Animator::walking`], 0x58f117); the
    /// route's position fields (+0x24, +0x28) and +0x290, cleared with
    /// it, are not kept.
    fn seated(&self, world: &mut World, id: u16, a: &mut Animator) -> Result<Next, Trap> {
        let held = a.state == 1 && person_word(world, id, SEATED_HOLD)? != 0;
        if a.foreground.is_some() && !held {
            clear_foreground(a);
            a.posture = -1;
            let flags = self.base_flags(world, id)?;
            self.update_base(world, id, a, flags)?;
        }
        a.walking = false;
        Ok(Next::Done)
    }

    /// State 3 for a placed animator (0x58ddbf). Both directions are put
    /// into 0 to 8. An animator arrived and facing its destination's
    /// direction notes the route's end (+0x188: 0 stays 0; -1, 2 and 3
    /// become 2, 0x58f135) and stops walking ([`Animator::walking`] and
    /// +0x24, +0x28 and +0x290 cleared, 0x58f117), or, with any other
    /// route result, follows the route (state 9). One arrived that has to
    /// turn starts a turn, or turns at once by less than a step (0x58df01,
    /// [`Interpreter::start_turn`]).
    fn stand(&self, world: &mut World, id: u16, a: &mut Animator, now: i32) -> Result<Next, Trap> {
        a.destination[2] = wrap(a.destination[2]);
        a.position[2] = wrap(a.position[2]);
        let eps = EPSILON.ext();
        let near = |to: F32, at: F32| (to.ext() - at.ext()).single().abs() <= eps;
        let aligned = near(a.destination[2], a.position[2]);
        let arrived =
            near(a.destination[0], a.position[0]) && near(a.destination[1], a.position[1]);
        if aligned && arrived {
            match a.arrival {
                0 => {}
                -1 | 2 | 3 => a.arrival = 2,
                _ => {
                    a.state = 9;
                    return Ok(Next::Again);
                }
            }
            a.walking = false;
            return Ok(Next::Done);
        }
        if !arrived {
            return Err(unsupported(
                "0x58df1e",
                "in state 3 walks to its destination",
            ));
        }
        self.start_turn(world, id, a, now)
    }

    /// The start of a turn on the spot (0x58df01 to 0x58f5f9), for an
    /// animator that has arrived.
    ///
    /// The turn d is the destination's direction less the position's, put
    /// into -4 to 4 by whole turns (0x58e013 to 0x58e041). The smallest
    /// turn worth a clip, the step, is a quarter of an eighth over the
    /// speed. The posture's clip and the turn it makes (0x58f4ee to
    /// 0x58f674), with the bounds the original compares against (0x6d07c4
    /// = 0.5, 0x6d07c8 = 1.5 and 0x6d07cc = 3):
    ///
    /// | d             | clip | turn |
    /// |---------------|------|------|
    /// | below -3      | 5    | -4   |
    /// | -3 to -1.5    | 7    | -2   |
    /// | -1.5 to -0.5  | 9    | -1   |
    /// | -0.5 to 0.5   | 10   | 0    |
    /// | 0.5 to 1.5    | 8    | 1    |
    /// | 1.5 to 3      | 6    | 2    |
    /// | above 3       | 4    | 4    |
    ///
    /// A bound belongs to the row nearer 0 (-1.5 and 1.5 to the rows of
    /// clips 9 and 8, -0.5 and 0.5 to clip 10).
    ///
    /// A turn of less than a step either way is made at once (0x58dfc0).
    /// The distance to the destination, 0 for an animator that has
    /// arrived (0x58df1a), is below 0.5 (0x6d07e4), so the animator is put
    /// at its destination (0x58dfcf): the position takes the
    /// destination's two coordinates, and the direction takes the
    /// destination's (virtual function 0xd0, 0x58488b), which goes to
    /// object data word 1 (0x5920e8, [`direction_word`]). The state stays
    /// 3 and its code runs again (0x58f089), which finds the animator
    /// arrived and facing its destination's direction
    /// ([`Interpreter::stand`]). No clip plays. The original also sets
    /// global 0x6f6d64 to 4, which is not kept. Only an animator that has
    /// not arrived can be 0.5 or more away. It would take a step by
    /// heading (0x58f312) or turn with a clip (0x58f4ee), and its state 3
    /// fails at 0x58df1e.
    ///
    /// The turn is noted (+0x1f0, the clip's turn +0x1f4, the destination
    /// +0x1dc to +0x1e4, the position +0x1d0 to +0x1d8); the destination's
    /// direction becomes the position's plus d less the clip's turn; the
    /// clip starts in the foreground (0x5926d7) at the animator's speed
    /// and priority 5 (0x59867f), and the animator turns (state 4). The
    /// original also sets global 0x6f6d6c to 2, which is not kept.
    ///
    /// Fails for what is not ported: a person of kind above 1, whose
    /// bounds are scaled (0x58df72), and a turn that is not a number, for
    /// which the original would loop forever (0x58e039).
    fn start_turn(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        now: i32,
    ) -> Result<Next, Trap> {
        let eight = EIGHT.ext();
        let mut d = (a.destination[2].ext() - a.position[2].ext()).single();
        if d.is_nan() {
            return Err(unsupported("0x58e039", "turns by what is not a number"));
        }
        while d >= FOUR.ext() {
            d = (d - eight).single();
        }
        while d < (-FOUR.ext()).single() {
            d = (d + eight).single();
        }
        let step = ((ONE.ext() * QUARTER.ext()).single() / a.rate.ext()).single();
        if person_word(world, id, KIND)? > 1 {
            return Err(unsupported("0x58df72", "turns a person of kind above 1"));
        }
        if d < step && d > (-step).single() {
            // 0x58dfc0: arrived, so 0 away, below 0.5 (0x6d07e4).
            a.position[0] = a.destination[0];
            a.position[1] = a.destination[1];
            a.position[2] = a.destination[2];
            set_datum(world, id, DIRECTION, direction_word(a.position[2]))?;
            return Ok(Next::Again);
        }
        let [half, one_and_half, three] = [TURN_HALF, TURN_ONE_AND_HALF, THREE].map(F32::ext);
        let (selection, clip_turn) = if !at_least(d, (-three).single()) {
            (5, -4.0)
        } else if d > three {
            (4, 4.0)
        } else if d >= (-half).single() && d <= half {
            (10, 0.0)
        } else if d > half {
            if d > one_and_half { (6, 2.0) } else { (8, 1.0) }
        } else if d >= (-one_and_half).single() {
            (9, -1.0)
        } else {
            (7, -2.0)
        };
        let clip_turn = F32::from_f32(clip_turn);
        let m = &mut a.motion;
        m.turn_wanted = d.to_f32();
        m.turn_destination = a.destination;
        m.turn_from = a.position;
        a.destination[2] = ((d - clip_turn.ext()).single() + a.position[2].ext())
            .single()
            .to_f32();
        m.turn_clip = clip_turn;
        self.start_clip(world, id, a, selection, now, STOP)?;
        let rate = a.rate;
        let layer = a.foreground.as_mut().ok_or_else(|| {
            unsupported(
                "0x58f5e1",
                "found no clip to turn with: it reads through a null pointer",
            )
        })?;
        layer.set_speed(rate);
        // 0x59867f: the priority, which marks the skin's layers changed.
        layer.priority = 5;
        a.skin_dirty = true;
        a.state = 4;
        Ok(Next::Done)
    }

    /// Plays one of the posture's clips in the foreground (virtual
    /// function 0xf8, 0x5926d7), unless the foreground already plays the
    /// clip of that number. The clip is looked up by name (0x592625,
    /// which first reads the person's appearance again if the animator
    /// says so) and started (0x592726) at the clock, forward, at speed 1:
    /// the next foreground clip's raised priority is used up (+5, which
    /// marks the skin's layers changed) and the layer joins the skin. Its
    /// end mode (layer +0x40) becomes `mode`; a clip already playing keeps
    /// its own.
    ///
    /// The layer's timed events are handled as it plays
    /// (`clip_events.rs`).
    ///
    /// A layer the foreground slot already holds, of another clip, leaves
    /// the skin first (virtual function 0x104, 0x592878, called at
    /// 0x59275c), as in [`clear_foreground`].
    ///
    /// Fails for what is not ported: a clip that is not found (the
    /// original then reads through a null pointer).
    fn start_clip(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        selection: usize,
        now: i32,
        mode: i32,
    ) -> Result<(), Trap> {
        if a.foreground.is_some() && a.selection == selection as i32 {
            return Ok(());
        }
        read_appearance_if_stale(world, self.program, id, &mut a.profile_stale)?;
        let name = self.posture_clip(world, id, selection)?;
        a.selection = selection as i32;
        let name = name.ok_or_else(|| {
            unsupported("0x592726", &format!("has no name for its clip {selection}"))
        })?;
        let clip = self
            .program
            .animations
            .clip(&name)
            .map_err(|e| unsupported("0x59dc32", &format!("finds no clip {name:?}: {e}")))?;
        // 0x59275c: a layer already in the slot is cleared first (virtual
        // function 0x104, 0x592878; see [`clear_foreground`]).
        if a.foreground.take().is_some() {
            a.skin_dirty = true;
        }
        let mut layer = Layer::start(
            &name,
            F32::from_bits(clip.duration),
            clip.frames,
            clip.motions > 0,
            clip.tracks.len(),
            false,
            now,
        );
        if a.promote {
            a.promote = false;
            layer.priority += 5;
        }
        layer.mode = mode;
        a.skin_dirty = true;
        a.foreground = Some(layer);
        // A new layer goes in before the carry layer (priority 10).
        a.carry_first = false;
        Ok(())
    }

    /// Starts the animator's own carry clip in the carry slot, the slot
    /// already empty (0x5903fc to 0x590421). The posture's clip 0x26 is
    /// looked up (virtual function 0xf4, 0x592625, which first reads the
    /// person's appearance again if the animator says so) and started in
    /// the slot (virtual function 0x100, 0x592726) at the clock, forward,
    /// at speed 1, to stop at its end (end mode 0, 0x592823): made at
    /// priority 1, it goes in before the first layer of the skin whose
    /// priority is above that (0x599128). Then it takes priority 10
    /// (0x59867f), which marks the skin's layers changed. Unlike a
    /// script's carry clip, the carry mode stays as the tick set it, so
    /// the next tick chooses again.
    ///
    /// Fails for what is not ported: a clip that is not found. The
    /// original leaves the slot empty then (0x59040a, or 0x592726 failing
    /// at 0x59274a) and still notes the carry clip chosen.
    fn start_carry(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        now: i32,
    ) -> Result<(), Trap> {
        read_appearance_if_stale(world, self.program, id, &mut a.profile_stale)?;
        let name = self.posture_clip(world, id, CARRY)?.ok_or_else(|| {
            unsupported(
                "0x59db2f",
                &format!("has no name for its carry clip {CARRY:#x}"),
            )
        })?;
        let clip =
            self.program.animations.clip(&name).map_err(|e| {
                unsupported("0x59dc32", &format!("finds no carry clip {name:?}: {e}"))
            })?;
        let mut layer = Layer::start(
            &name,
            F32::from_bits(clip.duration),
            clip.frames,
            clip.motions > 0,
            clip.tracks.len(),
            false,
            now,
        );
        a.carry_first = a.foreground.as_ref().is_some_and(|l| l.priority > 1);
        layer.priority = 10;
        a.skin_dirty = true;
        a.carry = Some(layer);
        Ok(())
    }

    /// The name of one of the posture's clips (0x59db2f), counted from 0:
    /// the type's own table first; then, for a person whose word 64 is
    /// set, the special tables (not ported); then the global table for the
    /// person's age (word 58 at most 17) or, for an adult, sex (bit 0 of
    /// word 65); then the global table 150. An empty name counts as none.
    /// The old prototype's `posture_name`
    /// (legacy/crates/sims-vm/src/animation.rs) reads the same tables.
    pub(crate) fn posture_clip(
        &self,
        world: &World,
        id: u16,
        selection: usize,
    ) -> Result<Option<String>, Trap> {
        if person_word(world, id, SPECIAL)? != 0 {
            return Err(unsupported(
                "0x59db44",
                "names a clip for a person whose word 64 is set",
            ));
        }
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?;
        let definition = self.catalog.object(object.guid).ok_or_else(|| {
            Trap::Invalid(format!("no definition for GUID 0x{:08X}", object.guid))
        })?;
        let lookup = |resource: usize, table: u16| {
            self.program
                .libraries
                .get(&resource)
                .and_then(|l| l.strings.get(&table))
                .and_then(|s| s.get(selection))
                .filter(|name| !name.is_empty())
                .cloned()
        };
        if let Some(name) = lookup(definition.resource, OWN_NAMES) {
            return Ok(Some(name));
        }
        let table = if person_word(world, id, AGE)? <= 17 {
            CHILD_NAMES
        } else {
            ADULT_NAMES + (person_word(world, id, KIND)? as u16 & 1)
        };
        let global = self.program.global;
        Ok(lookup(global, table).or_else(|| lookup(global, NAMES)))
    }

    /// The distance the walk looks ahead, as far as it is ported: half of
    /// +0x200, the stride of the posture's clip 0x14 halved
    /// ([`Interpreter::stride`]).
    fn look_ahead(&self, world: &World, id: u16) -> Result<Ext, Trap> {
        Ok((self.stride(world, id, WALK, true)? * HALF.ext()).single())
    }

    /// State 9 (0x58e4e0): the animator follows the person's path. From
    /// the path point it is heading for (+0x180), it looks ahead along the
    /// segment from the point before, from where its position projects on
    /// the segment (+0x1ec, 0 at the point before and 1 at the point), by
    /// steps of one unit, to the first rounded point at least the look-
    /// ahead distance away; a segment it has passed moves it to the next.
    /// Then it heads there ([`Interpreter::head`], 0x58eb29).
    ///
    /// On each segment but the last the state becomes 9 (10 with data
    /// word 17 set, 0x58e58a), unless a foreground clip plays that may not
    /// be interrupted (layer +0x68 is 0, 0x58e584).
    ///
    /// The position's projection is held to the segment.
    ///
    /// The point before the path's last is where the path ends; the last
    /// point is only the way to face there ([`Interpreter::end_of_path`]).
    ///
    /// Fails for what is not ported: a person whose data word 17 is set
    /// (another look-ahead).
    fn follow(&self, world: &mut World, id: u16, a: &mut Animator, now: i32) -> Result<Next, Trap> {
        let path = world
            .routing
            .routes
            .get(&id)
            .map(|r| r.path.clone())
            .unwrap_or_default();
        let pace = datum(world, id, PACE)?;
        let count = path.len() as i64;
        let eps = EPSILON.ext();
        let zero = Ext::ZERO;
        let one_double = Ext::from_f64_bits(ONE_DOUBLE);
        let half_double = Ext::from_f64_bits(HALF_DOUBLE);
        let [x, y] = [a.position[0].ext(), a.position[1].ext()];
        let mut reach = None;
        for _ in 0..path.len() + 1 {
            let segment = a.motion.segment;
            let point = |i: i32| {
                usize::try_from(i)
                    .ok()
                    .and_then(|i| path.get(i))
                    .copied()
                    .ok_or_else(|| unsupported("0x58e519", "follows a path past its end"))
            };
            let p = point(segment)?;
            let q = if segment == 0 { p } else { point(segment - 1)? };
            let [p0, p1] = p.map(Ext::from_i32);
            let [q0, q1] = q.map(Ext::from_i32);
            let along = (p1 - q1).single();
            let across = (p0 - q0).single();
            let length = ((across * across).single() + (along * along).single())
                .single()
                .sqrt()
                .single();
            if i64::from(segment) == count - 2 {
                let last = point(segment + 1)?;
                return self.end_of_path(world, id, a, p, last, now);
            }
            if a.foreground.as_ref().is_none_or(|l| l.interruptible != 0) {
                a.state = if pace == 1 { 10 } else { 9 };
            }
            let inverse = if length.abs() <= eps {
                Ext::from_i32(1)
            } else {
                (ONE.ext() / length).single()
            };
            let f1 = (q1 - p1).single();
            let f0 = (q0 - p0).single();
            let g1 = (x - p1).single();
            let g0 = (y - p0).single();
            let square = ((f0 * f0).single() + (f1 * f1).single()).single();
            let t = if square > zero {
                let dot = ((g0 * f0).single() + (g1 * f1).single()).single();
                (ONE.ext() - (dot / square).single()).single()
            } else {
                zero
            };
            // A position behind the segment's start, or not a number,
            // looks from the start (0x58e631); one beyond its end from the
            // end (0x58e64d).
            a.motion.progress = if !at_least(t, zero) {
                F32::ZERO
            } else if t > ONE.ext() {
                ONE
            } else {
                t.to_f32()
            };
            while length > zero && a.motion.progress.ext() < one_double {
                let t = a.motion.progress.ext();
                let u = (one_double - t).single();
                let round = |p: Ext, q: Ext| {
                    (((p * t).single() + (q * u).single()).single() + half_double)
                        .single()
                        .to_i32_truncate()
                };
                let ahead1 = round(p1, q1);
                let ahead0 = round(p0, q0);
                let dx = (Ext::from_i32(ahead1) - x).single();
                let dy = (Ext::from_i32(ahead0) - y).single();
                if pace != 0 {
                    return Err(unsupported(
                        "0x58e6d5",
                        "looks ahead for a person whose data word 17 is set",
                    ));
                }
                let reach = match reach {
                    Some(reach) => reach,
                    None => {
                        let s = self.look_ahead(world, id)?;
                        *reach.insert((s * s).single())
                    }
                };
                let distance = ((dy * dy).single() + (dx * dx).single()).single();
                if distance >= reach {
                    return self.head(world, id, a, [dx, dy], distance, now);
                }
                a.motion.progress = (inverse + t).single().to_f32();
            }
            a.motion.progress = F32::ZERO;
            a.motion.segment = segment + 1;
        }
        Err(unsupported("0x58e50a", "found no point ahead on its path"))
    }

    /// The end of the path (0x58e740): the animator is heading for the
    /// point before the path's last, `end`. The last point, `last`, gives
    /// the direction to face at the end: the angle from `end` to `last`
    /// ([`heading`], as the walk's heading is worked out).
    ///
    /// For a person whose object data word 9 is set (0x58e7d2): while the
    /// animator has not come near the end yet (+0x288 clear) and is at
    /// least the stopping distance from it (2, or 3 with data word 17 set,
    /// times 1.5), it keeps walking, straight at the end point (state 9,
    /// or 10 with data word 17 set; [`Interpreter::head`]). Otherwise, for
    /// a person whose word 64 is clear (0x58e846): the destination becomes
    /// the position, +0x288 and +0x28c are set, the route result becomes
    /// -1, the progress 1, and the animator stands (state 3). With a
    /// foreground clip it first takes its last step once more (0x58e897,
    /// [`Interpreter::walk_step`]), and then the destination becomes the new
    /// position, direction included (0x58f057). The state's code runs
    /// again (0x58f089). A blocked step goes to the update's tail
    /// instead (0x58f682).
    ///
    /// For a person whose object data word 9 is clear the animator slows
    /// to a stop on a timer ([`Interpreter::stop_on_timer`], 0x58e8be).
    ///
    /// The original also sets global 0x6f6d68 to 13 and then 14, and a
    /// local byte from 0x6f6d70 that only matters when it is 1 (0x58f6eb);
    /// 0x6f6d70 is 0 in a memory image of the running game. Neither is
    /// kept.
    ///
    /// Fails for what is not ported: a person whose word 64 is set
    /// (0x58e83b and 0x58e93c).
    #[allow(clippy::too_many_arguments)]
    fn end_of_path(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        end: [i32; 2],
        last: [i32; 2],
        now: i32,
    ) -> Result<Next, Trap> {
        let [p0, p1] = end.map(Ext::from_i32);
        let dx = (p1 - a.position[0].ext()).single();
        let dy = (p0 - a.position[1].ext()).single();
        let left = ((dy * dy).single() + (dx * dx).single())
            .single()
            .sqrt()
            .single();
        // 0x58e753 to 0x58e7c7: the direction to face at the end. Path
        // points hold the second coordinate first.
        let end_x = Ext::from_i32(end[1]).single();
        let end_y = Ext::from_i32(end[0]).single();
        let facing = heading(
            (Ext::from_i32(last[1]) - end_x).single(),
            (Ext::from_i32(last[0]) - end_y).single(),
        );
        if datum(world, id, STOPS)? == 0 {
            return self.stop_on_timer(world, id, a, [end_x, end_y, facing], [dx, dy], left, now);
        }
        let pace = datum(world, id, PACE)?;
        let stop = if pace == 1 {
            STOP_RUN_DISTANCE
        } else {
            STOP_DISTANCE
        };
        let stop = (stop.ext() * STOP_SCALE.ext()).single();
        // 0x58e810: below the stopping distance, or not a number, stops.
        if a.motion.near_end == 0 && at_least(left, stop) {
            a.state = if pace == 1 { 10 } else { 9 };
            let square = ((dy * dy).single() + (dx * dx).single()).single();
            return self.head(world, id, a, [dx, dy], square, now);
        }
        if person_word(world, id, SPECIAL)? != 0 {
            return Err(unsupported(
                "0x58e83b",
                "stops at the end of its path for a person whose word 64 is set",
            ));
        }
        a.destination[0] = a.position[0];
        a.destination[1] = a.position[1];
        a.motion.near_end = 1;
        a.arrival = -1;
        a.state = 3;
        a.motion.finishing = 1;
        a.motion.progress = ONE;
        if a.foreground.is_none() {
            return Ok(Next::Again);
        }
        let [sx, sy] = a.motion.step.map(F32::ext);
        let square = ((sy * sy).single() + (sx * sx).single()).single();
        if !self.walk_step(world, id, a, [sx, sy], square)? {
            return Ok(Next::Done);
        }
        a.destination = a.position;
        Ok(Next::Again)
    }

    /// The end of the path for a person whose object data word 9 is clear
    /// (0x58e8be to 0x58eb03): the animator walks on while the walk clip
    /// slows down, and stops on the end point when a timer (+0x1c0,
    /// [`sims_sim::animator::Motion::timer`]) runs out. `end` is the end
    /// point and the direction to face there, `dx` and `dy` how far the
    /// end point is and `left` the distance ([`Interpreter::end_of_path`]).
    ///
    /// With no timer set (0):
    ///
    /// 1. The phase of the foreground clip (0 with none) picks the next
    ///    footfall of the walk ([`Interpreter::footfalls`]): the first
    ///    (+0x174) if the phase is from it up to the second (+0x170), else
    ///    the second; and the stride that ends there (+0x17c for the first,
    ///    +0x178 for the second). With data word 17 set and person word 64
    ///    clear the stride is doubled.
    /// 2. Further from the end than the stride, the animator walks on
    ///    (0x58eb0a, [`Interpreter::head`]).
    /// 3. Otherwise +0x288 is set. A foreground clip that may not be
    ///    interrupted (layer +0x68 is 0) and is not done (0x586d41) walks
    ///    on.
    /// 4. Otherwise the timer is set: the share of the stride left (at
    ///    least 0.2), and the part of the clip's cycle to the footfall
    ///    (that phase less the clip's, plus 1 when below 1) times the
    ///    clip's duration at the animator's speed, truncated (0x636128),
    ///    at least 100: the timer is the clock plus that, and the clip's
    ///    weight fades from what it is to the share in that time (0x597a3e).
    ///    With no foreground clip the time is 100 and nothing fades. The
    ///    animator walks on.
    ///
    /// With a timer set: if data word 17 is set, the end is nearer than 2,
    /// or the clock has come to the timer, the timer becomes -1 and +0x28c
    /// is cleared. A timer still above 0 walks on. A timer of -1 stops
    /// the animator: the route result becomes -1, the position and the
    /// destination the end point, the destination's direction the
    /// direction to face, the timer 0, +0x28c clear, the progress 1, the
    /// foreground clip is cleared (virtual function 0xfc, 0x586b63), and
    /// it stands (state 3); the state's code runs again (0x58f089).
    ///
    /// The original also clears +0x290, which only code that is not
    /// ported reads (see [`Interpreter::walk`]).
    ///
    /// Fails for what is not ported: a person whose word 64 is set
    /// (0x58e936).
    #[allow(clippy::too_many_arguments)]
    fn stop_on_timer(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        [end_x, end_y, facing]: [Ext; 3],
        [dx, dy]: [Ext; 2],
        left: Ext,
        now: i32,
    ) -> Result<Next, Trap> {
        let pace = datum(world, id, PACE)?;
        let special = person_word(world, id, SPECIAL)?;
        let walk_on = |this: &Self, world: &mut World, a: &mut Animator| {
            let square = ((dy * dy).single() + (dx * dx).single()).single();
            this.head(world, id, a, [dx, dy], square, now)
        };
        if a.motion.timer == 0 {
            // Whichever way the stride decides, the animator walks on with
            // nothing else changed when +0x288 is already set, person word
            // 64 is clear and a foreground clip that may not be interrupted
            // has not finished (0x58e92a, 0x58e948). The footfalls are
            // then not needed.
            let walks_on = a.motion.near_end == 1
                && special == 0
                && a.foreground
                    .as_ref()
                    .is_some_and(|l| l.interruptible == 0 && !l.finished());
            if !walks_on {
                let feet = self.footfalls(world, id)?;
                slow_down(a, &feet, left, special != 0, pace, now)?;
            }
            return walk_on(self, world, a);
        }
        if a.motion.timer > 0
            && (pace == 1 || !at_least(left, STOP_DISTANCE.ext()) || now >= a.motion.timer)
        {
            a.motion.timer = -1;
            a.motion.finishing = 0;
        }
        if a.motion.timer >= 0 {
            return walk_on(self, world, a);
        }
        a.arrival = -1;
        a.position[0] = end_x.to_f32();
        a.destination[0] = a.position[0];
        a.motion.timer = 0;
        a.motion.finishing = 0;
        a.position[1] = end_y.to_f32();
        a.destination[1] = a.position[1];
        a.destination[2] = facing.to_f32();
        a.state = 3;
        a.motion.progress = ONE;
        clear_foreground(a);
        Ok(Next::Again)
    }

    /// The animator heads for a point ahead (0x58eb29), `dx` and `dy` away
    /// along its position's first and second coordinates, `distance` the
    /// square of the way there. An offset of no length (within
    /// [`EPSILON`]) leaves the state as it is.
    ///
    /// The heading is the offset's angle (virtual function 0x98,
    /// 0x5920c6) in eighths of a turn, plus 2, put into 0 to 8; the turn
    /// d is the heading less the position's direction, put into -4 to 4.
    /// Both are loops of whole turns.
    ///
    /// A turn of 2 eighths or more either way (0.8 for a person of kind
    /// above 1; at a speed below 1, 2 over the speed) is made on the spot
    /// first (0x58ec51): the destination becomes the position with the
    /// heading, the walk flag is cleared and the animator stands (state
    /// 3). A smaller turn walks ([`Interpreter::walk`]).
    fn head(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        offset: [Ext; 2],
        distance: Ext,
        now: i32,
    ) -> Result<Next, Trap> {
        if distance.abs() <= EPSILON.ext() {
            return Ok(Next::Done);
        }
        let [dx, dy] = offset;
        let heading = heading(dx, dy);
        let mut d = (heading - a.position[2].ext()).single();
        while at_least(d, FOUR.ext()) {
            d = (d - EIGHT.ext()).single();
        }
        while !at_least(d, (-FOUR.ext()).single()) {
            d = (d + EIGHT.ext()).single();
        }
        let slow = !at_least(a.rate.ext(), Ext::from_f64_bits(ONE_DOUBLE));
        let small = person_word(world, id, KIND)? > 1;
        let mut limit = TWO.ext();
        if slow {
            limit = (limit / a.rate.ext()).single();
        }
        if small {
            limit = SMALL_TURN.ext();
        }
        // Beyond the limit either way, or not a number.
        let above = matches!(d.partial_cmp(&(-limit).single()), Some(Ordering::Greater));
        if at_least(d, limit) || !above {
            a.destination = [a.position[0], a.position[1], heading.to_f32()];
            a.state = 3;
            a.walking = false;
            return Ok(Next::Again);
        }
        self.walk(world, id, a, d, offset, distance, now)
    }

    /// State 6 (0x58e125): the person starts its walk with the posture's
    /// clip 0x13, which carries it one clip's distance straight ahead.
    ///
    /// When that clip is not the selection yet, it starts (virtual
    /// function 0xf8, [`Interpreter::start_clip`]) to stop at its end; the
    /// speed and the speed wanted both become the speed wanted (0x588fa0,
    /// [`wanted_rate`], in state 6) and the clip takes it (0x598636). The
    /// start (+0x1d0, +0x1d4) becomes the position, and the destination
    /// (+0x1c4, +0x1c8) the position plus the clip's distance times a
    /// third times 16 (0x668390, 0x666a54) along the direction: its angle
    /// is (direction + 6) times 45 times pi times 1/180, in radians. The
    /// cosine is stored as a float first; the sine is used as the FPU
    /// leaves it (here libm's double), which is **unverified** at
    /// rounding boundaries.
    ///
    /// Then, each tick: the walk flag is cleared (and +0x290, not kept)
    /// and the clip takes the animator's speed. While the clip's phase t
    /// is below 0.95 (0x672020; not a number counts as below) the
    /// position goes from the start to the destination in step with it,
    /// (1 - t) times the start plus t times the destination. From 0.95 on
    /// the clip is cleared (virtual function 0xfc), the position becomes
    /// the destination, the destination's direction the position's, the
    /// walk flag is set and the animator follows its route again (state
    /// 9, whose code runs, 0x58e242). The original also sets global
    /// 0x6f6d68 to 10, which is not kept.
    fn stop_walk(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        now: i32,
    ) -> Result<Next, Trap> {
        if a.selection != STOP_WALK as i32 {
            self.start_clip(world, id, a, STOP_WALK, now, STOP)?;
            let pace = datum(world, id, PACE)?;
            let kind = person_word(world, id, KIND)?;
            let wanted = wanted_rate(pace, a.state, kind).to_f32();
            a.rate = wanted;
            a.target = wanted;
            let layer = a.foreground.as_mut().ok_or_else(|| {
                unsupported(
                    "0x58e15a",
                    "starts its walk with no clip: it reads through a null pointer",
                )
            })?;
            layer.set_speed(wanted);
            let clip = self.clip(&layer.clip)?;
            a.motion.turn_from[0] = a.position[0];
            a.motion.turn_from[1] = a.position[1];
            let [x, y] = stop_walk_destination(a.position, F32::from_bits(clip.distance));
            a.destination[0] = x;
            a.destination[1] = y;
        }
        a.walking = false;
        let rate = a.rate;
        let layer = a.foreground.as_mut().ok_or_else(|| {
            unsupported(
                "0x58e1e4",
                "walks with no clip: it reads through a null pointer",
            )
        })?;
        layer.set_speed(rate);
        let from = [a.motion.turn_from[0], a.motion.turn_from[1]];
        let to = [a.destination[0], a.destination[1]];
        if let Some([x, y]) = stop_walk_step(from, to, layer.phase) {
            a.position[0] = x;
            a.position[1] = y;
            return Ok(Next::Done);
        }
        clear_foreground(a);
        a.position[0] = a.destination[0];
        a.position[1] = a.destination[1];
        a.destination[2] = a.position[2];
        a.state = 9;
        a.walking = true;
        Ok(Next::Again)
    }

    /// The walk (0x58ec8a to 0x58f089), one tick of it, turning by `d`
    /// towards the point ahead at `offset` (squared distance `distance`).
    ///
    /// 1. The turn is held to a fifth of an eighth either way (0.057 for a
    ///    person of kind above 1; at a speed below 1, divided by the
    ///    speed) and added to the direction (virtual function 0xd0,
    ///    0x58488b), which goes to object data word 1 (0x5920e8).
    /// 2. A walk not yet going ([`Animator::walking`] clear and the
    ///    selection not a walk clip, 0x14 or 0x15) whose turn was not
    ///    held stops instead when the point is further than the stopping
    ///    distance (+0x1f8, or +0x204 with data word 17 set), and so does
    ///    any for a person whose word 64 is set: it goes to state 6, or
    ///    11 with data word 17 set (0x58ed52), and that state's code runs
    ///    ([`Interpreter::stop_walk`]). State 11 is not ported and fails.
    /// 3. The walk flag is set. The clip (0x14 in state 9, 0x15 in state
    ///    10, where data word 17 clear makes it state 9 first, else 0)
    ///    plays in the foreground, repeating ([`Interpreter::start_clip`]
    ///    with end mode 1), at the animator's speed (0x598636), from the
    ///    phase of the clip playing before if that one moves the person
    ///    ([`sims_content::Clip::moving`]), else from 0.
    /// 4. The step: the clip's distance times 16/3 over its duration at
    ///    the layer's speed in ticks of 42 milliseconds (virtual function
    ///    0x38, 0x585502), along the offset made one long. Its end,
    ///    rounded to whole units, is moved onto the person's path where
    ///    the path is nearer ([`snap_to_path`], 0x48313e), and the person must fit
    ///    there (virtual function 0x54, [`Interpreter::fits`]). A person
    ///    who does not stays put: the route's end becomes 3 (0x58f682)
    ///    and the update goes to its tail. Otherwise the person is put
    ///    there (virtual function 0x88, 0x45e4e9: the
    ///    placement, 0x433340, telling the registry, then the room,
    ///    0x45e43d) and the animator's position becomes the step's end,
    ///    unrounded.
    ///
    /// The step is kept (+0x294, +0x298) for the end of the path
    /// ([`Interpreter::end_of_path`]). The original also sets +0x290,
    /// which only code that is not ported reads (0x58ef8e). With +0x28c
    /// set (the end of a path sets it) the walk goes on without asking
    /// whether to stop (0x58ed15). The original sets global 0x6f6d64 to
    /// 3, which is not kept.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        d: Ext,
        offset: [Ext; 2],
        distance: Ext,
        now: i32,
    ) -> Result<Next, Trap> {
        let pace = datum(world, id, PACE)?;
        let mut limit = if person_word(world, id, KIND)? > 1 {
            SMALL_STEER
        } else {
            STEER
        }
        .ext();
        if !at_least(a.rate.ext(), Ext::from_f64_bits(ONE_DOUBLE)) {
            limit = (limit / a.rate.ext()).single();
        }
        let low = -limit;
        let (d, held) = if !at_least(d, low) {
            (low, true)
        } else if d > limit {
            (limit, true)
        } else {
            (d, false)
        };
        a.position[2] = (d + a.position[2].ext()).single().to_f32();
        set_datum(world, id, DIRECTION, direction_word(a.position[2]))?;
        let going = a.motion.finishing != 0
            || a.walking
            || a.selection == WALK as i32
            || a.selection == RUN as i32;
        if !going {
            let stop = |clip| -> Result<bool, Trap> {
                let stop = self.stride(world, id, clip, false)?;
                Ok(distance > (stop * stop).single())
            };
            if person_word(world, id, SPECIAL)? != 0
                || (!held && stop(if pace == 1 { STOP_RUN } else { STOP_WALK })?)
            {
                // 0x58ed52: state 6, or 11 with data word 17 set; +0x24
                // and +0x28 become 0 (not kept); that state's code runs.
                if pace == 1 {
                    return Err(unsupported(
                        "0x58ed52",
                        "goes to state 11 to start a walk with data word 17 set",
                    ));
                }
                a.state = 6;
                return Ok(Next::Again);
            }
        }

        a.walking = true;
        if a.state == 10 && pace == 0 {
            a.state = 9;
        }
        let selection = match a.state {
            9 => WALK,
            10 => RUN,
            _ => 0,
        };
        let phase = match &a.foreground {
            Some(layer) if self.clip(&layer.clip)?.moving => layer.phase,
            _ => F32::ZERO,
        };
        self.start_clip(world, id, a, selection, now, REPEAT)?;
        let rate = a.rate;
        let layer = a.foreground.as_mut().ok_or_else(|| {
            unsupported(
                "0x58edd8",
                "found no clip to walk with: it reads through a null pointer",
            )
        })?;
        layer.set_speed(rate);
        layer.phase = phase;
        self.walk_step(world, id, a, offset, distance)?;
        Ok(Next::Done)
    }

    /// One step of the walk (0x58edec to 0x58f04d): the clip's distance
    /// times 16/3 over its duration at the layer's speed in ticks of 42
    /// milliseconds, along `offset` made one long (`distance` is the
    /// square of its length; one within [`EPSILON`] counts as 1). The
    /// step is kept ([`sims_sim::animator::Motion::step`]). Its end,
    /// rounded to whole units, is moved onto the person's path, the
    /// person must fit there, and is put there; the animator's position
    /// becomes the step's end, unrounded. Returns false, with the route's
    /// end set to 3 and nothing moved, when the person does not fit
    /// there. See [`Interpreter::walk`].
    fn walk_step(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        offset: [Ext; 2],
        distance: Ext,
    ) -> Result<bool, Trap> {
        let layer = a.foreground.as_ref().ok_or_else(|| {
            unsupported(
                "0x58edf6",
                "steps with no foreground clip: it reads through a null pointer",
            )
        })?;
        let clip = self.clip(&layer.clip)?;
        let duration = (F32::from_bits(clip.duration).ext() / layer.speed.ext()).single();
        let length =
            ((F32::from_bits(clip.distance).ext() * THIRD.ext()).single() * SIXTEEN.ext()).single();
        let ticks = (duration / Ext::from_i32(TICK_MILLISECONDS)).single();
        let speed = (length / ticks).single();
        let square = if distance.abs() > EPSILON.ext() || distance.is_nan() {
            distance
        } else {
            Ext::from_i32(1)
        };
        let norm = square.sqrt().single();
        let [dx, dy] = offset;
        let step_x = ((speed * dx).single() / norm).single();
        let step_y = ((speed * dy).single() / norm).single();
        a.motion.step = [step_x.to_f32(), step_y.to_f32()];
        let x = (step_x + a.position[0].ext()).single();
        let y = (step_y + a.position[1].ext()).single();
        let round = |v: Ext| (v + HALF.ext()).single().to_i32_truncate();
        let path = world
            .routing
            .routes
            .get(&id)
            .map(|r| r.path.clone())
            .unwrap_or_default();
        // Path points hold the second coordinate first.
        let [second, first] = snap_to_path(&path, [round(y), round(x)], a.motion.segment)?;
        let level = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?
            .position[2];
        let to = [first, second];
        if !self.fits(world, id, to, level).map_err(|f| f.trap)? {
            // 0x58eecb to 0x58f682: the route ends blocked (3), and the
            // update goes to its tail.
            a.arrival = BLOCKED;
            return Ok(false);
        }
        // What the placement runs may read the animator; it sees it as the
        // original's would be.
        world.animators.insert(id, a.clone());
        self.put_person(world, id, to, level)?;
        if let Some(now) = world.animators.get(&id) {
            *a = now.clone();
        }
        a.position[0] = x.to_f32();
        a.position[1] = y.to_f32();
        // 0x58eeeb: a stop asked for while the route was under way
        // ([`stop_route`]) ends it here, once the person is put, if the
        // route is still on its way to the end (+0x188 is 1) and the
        // foreground clip may be interrupted (layer +0x68).
        if world.stop_requests.contains(&id)
            && a.arrival == 1
            && a.foreground.as_ref().is_none_or(|l| l.interruptible != 0)
        {
            if person_word(world, id, SPECIAL)? != 0 {
                return Err(unsupported(
                    "0x58ef2f",
                    "stops a route asked to stop under way for a person whose word 64 is set \
                     (state 7 and the stop clip 0x13, 0x58f010)",
                ));
            }
            // 0x58ef29 to 0x58f682: the route ends blocked (3), and the
            // update goes to its tail.
            a.arrival = BLOCKED;
        }
        Ok(true)
    }

    /// A walking person put on a tile (the person's virtual function
    /// 0x88, 0x45e4e9): the placement, then the room it is in noted.
    pub(crate) fn put_person(
        &self,
        world: &mut World,
        id: u16,
        to: [i32; 2],
        level: i32,
    ) -> Result<(), Trap> {
        let place = Place {
            on_lot: on_lot(to),
            position: to,
            container: None,
            slot: -1,
            level,
        };
        self.put(world, id, place, true).map_err(|f| f.trap)?;
        self.note_room(world, id)
    }

    /// How the animator's route went (0x589d47), as the route's step 3
    /// reads it: 2 while it is under way (the route's end, +0x188, is -1
    /// or 1), 0 for an end of 0 or any end but 2.
    ///
    /// For an end of 2 (the animator reached the path's end) the person
    /// is put where the animator stands, and the result is 1:
    ///
    /// 1. The direction goes to object data word 1 (0x5920e8).
    /// 2. The position, each coordinate rounded half up and truncated,
    ///    is moved onto the path ([`snap_to_path`], all its segments).
    /// 3. If the person fits there (virtual function 0x54) it is put there
    ///    (virtual function 0x88, [`Self::put_person`]). A person that does
    ///    not fit would have the animator's position unset (virtual
    ///    function 0x20) and be put back on its own tile with result 0;
    ///    that fails loudly.
    pub(crate) fn route_result(&self, world: &mut World, id: u16) -> Result<i32, Trap> {
        let a = world
            .animators
            .get(&id)
            .ok_or_else(|| Trap::Invalid(format!("person {id} has no animator")))?;
        match a.arrival {
            -1 | 1 => return Ok(2),
            2 => {}
            _ => return Ok(0),
        }
        let [x, y, direction] = a.position;
        set_datum(world, id, DIRECTION, direction_word(direction))?;
        let round = |v: F32| (v.ext() + HALF.ext()).single().to_i32_truncate();
        let path = world
            .routing
            .routes
            .get(&id)
            .map(|r| r.path.clone())
            .unwrap_or_default();
        // Path points hold the second coordinate first.
        let [second, first] = snap_to_path(&path, [round(y), round(x)], -1)?;
        let level = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?
            .position[2];
        let to = [first, second];
        if !self.fits(world, id, to, level).map_err(|f| f.trap)? {
            return Err(unsupported(
                "0x589dee",
                "ends its route where its person does not fit",
            ));
        }
        self.put_person(world, id, to, level)?;
        Ok(1)
    }

    /// A person's virtual function 0x74 (0x45e519), which the end of a
    /// route runs after it turns the person:
    ///
    /// 1. The animator's position is unset (its virtual function 0x24,
    ///    0x5893a9, which runs virtual function 0x20 with 1, 0x589361):
    ///    both coordinates and the direction become [`UNSET`], and byte
    ///    +0xd9 is set. As in the frame's draw (`frame.rs`), +0xd9 is not
    ///    kept; the animator's tick places an unset animator again.
    /// 2. Person word 64 becomes 0, then 1 if the person stands on the lot
    ///    (0x422632) on a pool tile (0x478f76).
    pub(crate) fn refresh_person(&self, world: &mut World, id: u16) -> Result<(), Trap> {
        let a = world
            .animators
            .get_mut(&id)
            .ok_or_else(|| Trap::Invalid(format!("person {id} has no animator")))?;
        a.position = [UNSET; 3];
        let position = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?
            .position;
        let [a, b, _] = World::tile(position);
        let pool = on_lot([position[0], position[1]]) && pool_tile(world, [a, b], position[2])?;
        let word = world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
            .data
            .get_mut(SPECIAL)
            .ok_or_else(|| Trap::Invalid(format!("person data has no word {SPECIAL}")))?;
        *word = i16::from(pool);
        world.routing.unknown_words.remove(&(id, SPECIAL));
        Ok(())
    }

    /// One of the strides the animator's constructor (0x58831d) keeps: the
    /// distance of the posture's clip `selection` (clip +0x30, virtual
    /// function 0x120, 0x588ed7), halved when `half`, times 16/3.
    ///
    /// The value is worked out here when it is needed, not kept: it is
    /// the constructor's while the clip's name is, which holds as long as
    /// the person's words 58, 64 and 65 and its type stay as they were.
    /// The original takes 0 for a clip it does not find; here that fails.
    fn stride(&self, world: &World, id: u16, selection: usize, half: bool) -> Result<Ext, Trap> {
        let name = self.posture_clip(world, id, selection)?.ok_or_else(|| {
            unsupported("0x588ed7", &format!("has no name for its clip {selection}"))
        })?;
        let mut distance = F32::from_bits(self.clip(&name)?.distance).ext();
        if half {
            distance = (distance * HALF.ext()).single();
        }
        let k = (Ext::from_i32(16) / Ext::from_i32(3)).single();
        Ok((distance * k).single())
    }

    /// The phases of the walk's footfalls, as the animator's constructor
    /// (0x58831d) keeps them: the walk clip (the posture's clip 0x14) at
    /// its events `anchor` = `1` (+0x170) and `anchor` = `2` (+0x174), each
    /// event's time over the clip's duration (0x5885b6), in that order.
    /// The events are found as virtual function 0x128 (0x588df8) finds
    /// them ([`anchor_event`]).
    ///
    /// The values are worked out here when they are needed, not kept, as
    /// [`Interpreter::stride`] is.
    pub fn footfall_phases(&self, world: &World, id: u16) -> Result<[F32; 2], Trap> {
        let name = self
            .posture_clip(world, id, WALK)?
            .ok_or_else(|| unsupported("0x58852c", &format!("has no name for its clip {WALK}")))?;
        let clip = self.clip(&name)?;
        let duration = F32::from_bits(clip.duration).ext();
        let phase = |value: &str| {
            let (_, time) = anchor_event(clip, value).ok_or_else(|| {
                unsupported(
                    "0x588df8",
                    &format!("finds no event anchor = {value} in its walk clip {name:?}"),
                )
            })?;
            Ok::<_, Trap>((Ext::from_i32(time) / duration).single().to_f32())
        };
        Ok([phase("1")?, phase("2")?])
    }

    /// The walk's footfalls ([`Footfalls`]): the phases
    /// ([`Interpreter::footfall_phases`]) and the strides
    /// ([`Interpreter::strides`]) that end at them.
    fn footfalls(&self, world: &World, id: u16) -> Result<Footfalls, Trap> {
        let [second, first] = self.footfall_phases(world, id)?;
        let [first_stride, second_stride] = self.strides(world, id)?;
        Ok(Footfalls {
            first,
            second,
            first_stride,
            second_stride,
        })
    }

    /// The walk's strides: animator +0x17c, the one that ends at the
    /// footfall of event `anchor` = `2`, then +0x178, the one that ends at
    /// the footfall of `anchor` = `1`.
    ///
    /// The constructor (0x58852c to 0x58876a) poses the person's skeleton
    /// with the walk clip alone, at weight 1, at each footfall's phase:
    /// first `anchor` = `2`'s, then `anchor` = `1`'s (0x5885e8, 0x58869f;
    /// [`sims_sim::pose`]). Each time it takes the origins of the two
    /// anchor events' bones (0x595f32, 0x5c39a0), along their third
    /// coordinate, times 16/3 (0x588418). With `one` and `two` those
    /// products for the bones of `anchor` = `1` and `2`, +0x17c is `two`
    /// less `one` at the first pose and +0x178 is `one` less `two` at the
    /// second; `one` is stored as a float before the subtraction.
    ///
    /// The original works the strides out once, when it builds the
    /// person, and keeps them. This works them out from the same clip and
    /// skeleton each time they are needed, which is the same while the
    /// person's walk clip and skeleton stay the same.
    ///
    /// The layer's time since its previous pose is taken as 0, so the
    /// next frame is always blended in, and the clip as looping. Neither
    /// is read from the original; the strides this gives equal the ones
    /// it stored for every person in the capture (see
    /// `crates/sims-vm/tests/resize.rs`).
    pub fn strides(&self, world: &World, id: u16) -> Result<[F32; 2], Trap> {
        let name = self
            .posture_clip(world, id, WALK)?
            .ok_or_else(|| unsupported("0x58852c", &format!("has no name for its clip {WALK}")))?;
        let clip = self.clip(&name)?;
        let guid = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?
            .guid;
        let skeleton = self.program.people.skeletons.get(&guid).ok_or_else(|| {
            Trap::Unsupported(format!(
                "no skeleton was supplied for the person type 0x{guid:08X}"
            ))
        })?;
        let samples = self
            .program
            .animations
            .samples(clip)
            .map_err(|e| unsupported("0x5992b3", &format!("cannot read its samples: {e}")))?;
        let floats3 = |v: &[f32; 3]| v.map(F32::from_f32);
        let floats4 = |v: &[f32; 4]| v.map(F32::from_f32);
        let translations: Vec<[F32; 3]> = samples.translations.iter().map(floats3).collect();
        let rotations: Vec<[F32; 4]> = samples.rotations.iter().map(floats4).collect();
        let place = |bone: &str| skeleton.bones.iter().position(|b| b.name == bone);
        let anchor = |value: &str| {
            let (bone, _) = anchor_event(clip, value).ok_or_else(|| {
                unsupported(
                    "0x588df8",
                    &format!("finds no event anchor = {value} in its walk clip {name:?}"),
                )
            })?;
            place(bone).ok_or_else(|| {
                unsupported(
                    "0x5990e0",
                    &format!("finds no bone {bone:?} in the person's skeleton"),
                )
            })
        };
        let anchors = [anchor("1")?, anchor("2")?];
        let parents: Vec<Option<usize>> = skeleton.bones.iter().map(|b| b.parent).collect();
        let rest: Vec<pose::Bone> = skeleton
            .bones
            .iter()
            .map(|b| pose::Bone {
                translation: b.translation.map(F32::from_bits),
                rotation: b.rotation.map(F32::from_bits),
            })
            .collect();
        let bad = |e: pose::Error| unsupported("0x5992b3", &format!("cannot pose: {}", e.0));
        let duration = F32::from_bits(clip.duration);
        let k = (Ext::from_i32(16) / Ext::from_i32(3)).single();
        let heights = |phase: F32| -> Result<[Ext; 2], Trap> {
            let at =
                pose::Selection::at(phase, clip.frames, duration, F32::ZERO, true).map_err(bad)?;
            let mut locals = rest.clone();
            for channel in &clip.channels {
                let Some(bone) = place(&channel.bone) else {
                    continue;
                };
                let from = |start: Option<u32>, len: usize| {
                    start.map(|s| s as usize).filter(|&s| s <= len)
                };
                let motion = pose::Motion {
                    translations: from(channel.translation, translations.len())
                        .map(|s| &translations[s..]),
                    rotations: from(channel.rotation, rotations.len()).map(|s| &rotations[s..]),
                };
                pose::sample(&mut locals[bone], motion, at).map_err(bad)?;
            }
            let placed = pose::absolute(&parents, &locals).map_err(bad)?;
            Ok(anchors.map(|b| (placed[b].translation[2].ext() * k).single()))
        };
        let footfalls = self.footfall_phases(world, id)?;
        let [one, two] = heights(footfalls[1])?;
        let first = (two - one).single().to_f32();
        let [one, two] = heights(footfalls[0])?;
        let second = (one - two).single().to_f32();
        Ok([first, second])
    }

    /// A clip by name, as the animator looks it up (0x59dc32).
    fn clip(&self, name: &str) -> Result<&sims_content::Clip, Trap> {
        self.program
            .animations
            .clip(name)
            .map_err(|e| unsupported("0x59dc32", &format!("finds no clip {name:?}: {e}")))
    }

    /// The start of a route (0x589c85), from Go To Routing Slot: the
    /// position and destination become the person's tile position and
    /// direction (object +0xf8, +0xf4 and data word 1); the scale becomes
    /// 1 (virtual function 0x90, 0x59209f, which writes object data word
    /// 22 as the scale times 256, truncated); the route result 1 (+0x188),
    /// the segment and progress 0, and the animator stands (state 3).
    ///
    /// The original also sets two globals (0x6f6d64 and 0x6f6d68) to 6 and
    /// zeroes fields nothing ported reads ([`sims_sim::animator::Motion`]);
    /// the scale itself (+0x7c) is left out, as nothing ported reads it.
    pub(crate) fn start_route(&self, world: &mut World, id: u16) -> Result<(), Trap> {
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::native(10, "missing object"))?;
        let own = [
            object.position[0],
            object.position[1],
            i32::from(datum(world, id, DIRECTION)?),
        ]
        .map(|v| Ext::from_i32(v).single().to_f32());
        let mut a = world.animators.get(&id).cloned().unwrap_or(LOADED);
        a.position = own;
        a.destination = own;
        let scale = (ONE.ext() * SCALE_UNIT.ext()).single().to_i32_truncate();
        set_datum(world, id, SCALE_WORD, scale as i16)?;
        a.arrival = 1;
        a.motion.progress = F32::ZERO;
        a.motion.segment = 0;
        a.motion.near_end = 0;
        a.state = 3;
        world.animators.insert(id, a);
        // 0x589d38: a stop asked for before is forgotten.
        world.stop_requests.remove(&id);
        Ok(())
    }
}

/// State 4 (0x58e045): the animator turns on the spot as its foreground
/// clip plays. First it stops walking ([`Animator::walking`] cleared, and
/// +0x24, +0x28 and +0x290, which are not kept). While the clip is short of its end (layer +0x28 below 1),
/// the direction and position go from where the turn started to the
/// destination in step with it; the direction is written to object data
/// word 1, rounded and put into 0 to 7 (0x5920e8, [`direction_word`]), and
/// the clip takes the animator's speed (0x598636).
///
/// At the clip's end (0x58e06f) the turn ends ([`end_turn`]) and the
/// animator stands again.
///
/// Fails for no foreground clip, where the original reads through a null
/// pointer.
fn turn(world: &mut World, id: u16, a: &mut Animator) -> Result<Next, Trap> {
    a.walking = false;
    let layer = a.foreground.as_ref().ok_or_else(|| {
        unsupported(
            "0x58e05a",
            "turns with no clip: it reads through a null pointer",
        )
    })?;
    let t = layer.phase.ext();
    if t >= ONE.ext() {
        end_turn(world, id, a)?;
        return Ok(Next::Again);
    }
    let u = (ONE.ext() - t).single();
    let mix = |from: F32, to: F32| ((u * from.ext()).single() + (t * to.ext()).single()).single();
    let m = a.motion;
    let direction = mix(m.turn_from[2], a.destination[2]);
    a.position[2] = direction.to_f32();
    set_datum(world, id, DIRECTION, direction_word(a.position[2]))?;
    a.position[0] = mix(m.turn_from[0], a.destination[0]).to_f32();
    a.position[1] = mix(m.turn_from[1], a.destination[1]).to_f32();
    let rate = a.rate;
    if let Some(layer) = a.foreground.as_mut() {
        layer.set_speed(rate);
    }
    Ok(Next::Done)
}

/// The end of a turn (0x58e06f to 0x58e120), when the turn's clip has
/// played to its end:
///
/// - The next foreground clip's priority is raised again (+0x1a0 = 1).
/// - The foreground clip is cleared (virtual function 0xfc, 0x586b63):
///   the layer leaves the skin, which marks the skin's layers changed
///   (0x592878, 0x5a0a7b, 0x5991a0), and the clip's selection becomes -1.
///   0x592878 would instead keep the layer to blend out when byte
///   0x6f6bd0 is set; no code writes that byte, so it is taken to be 0.
///   The layer's fields +0x78 and +0x7c, set to -1 first, go with it.
/// - The animator stands (state 3) where the turn was to end: the
///   position and the destination become the destination noted when the
///   turn started (+0x1dc to +0x1e4; the direction through virtual
///   function 0xd0, 0x58488b), and the direction is written to object
///   data word 1 (0x5920e8).
///
/// The original also sets two globals (0x6f6d64 and 0x6f6d68) to 9; as
/// in [`Interpreter::start_route`] they are not kept.
fn end_turn(world: &mut World, id: u16, a: &mut Animator) -> Result<(), Trap> {
    a.promote = true;
    a.foreground = None;
    a.skin_dirty = true;
    a.selection = -1;
    a.state = 3;
    let to = a.motion.turn_destination;
    a.position[2] = to[2];
    set_datum(world, id, DIRECTION, direction_word(a.position[2]))?;
    a.destination[2] = a.position[2];
    a.position[0] = to[0];
    a.destination[0] = to[0];
    a.position[1] = to[1];
    a.destination[1] = to[1];
    Ok(())
}

/// Person `id`'s animator stops following its route (0x589e2d), as the
/// route's steps 9, 10 and 11 and its end ask. While the route is under
/// way (its end, +0x188, is 1 or -1) nothing stops: the stop is only
/// asked for (+0x284 = 1, kept in `requests`, [`World::stop_requests`])
/// and the answer is false; the walk's next step ends the route
/// ([`Interpreter::walk_step`]). Otherwise the end becomes 0, the segment
/// -1, the destination the position, and the animator stands (state 3);
/// the answer is true.
///
/// The timer of a stop at the end of a path (+0x1c0) becomes 0 too
/// (0x589e56). The original also sets +0x18c to 1 and the global
/// 0x6f6d68 to 7. Nothing ported reads them, and they are not kept.
pub(crate) fn stop_route(a: &mut Animator, requests: &mut BTreeSet<u16>, id: u16) -> bool {
    if matches!(a.arrival, 1 | -1) {
        requests.insert(id);
        return false;
    }
    a.arrival = 0;
    a.motion.segment = -1;
    a.motion.timer = 0;
    a.destination = a.position;
    a.state = 3;
    true
}

/// The start of a stop at the end of a path, with no timer set yet
/// (0x58e8c9 to 0x58eaa4; see [`Interpreter::stop_on_timer`]): the next
/// footfall and its stride, and, within the stride of the end, the timer
/// and the fade. `left` is the distance to the end, `special` whether
/// person word 64 is set, `pace` object data word 17. The animator walks
/// on afterwards in every case.
///
/// Fails for what is not ported: a person whose word 64 is set, within
/// the stride (0x58e936).
pub(crate) fn slow_down(
    a: &mut Animator,
    feet: &Footfalls,
    left: Ext,
    special: bool,
    pace: i16,
    now: i32,
) -> Result<(), Trap> {
    let phase = a.foreground.as_ref().map_or(F32::ZERO, |l| l.phase).ext();
    // 0x58e8e0: below the first footfall, from the second on, or not a
    // number, the second is next.
    let second = !at_least(phase, feet.first.ext()) || phase >= feet.second.ext();
    let mut stride = if second {
        feet.second_stride
    } else {
        feet.first_stride
    }
    .ext();
    if !special && pace == 1 {
        stride = (stride + stride).single();
    }
    if left > stride {
        return Ok(());
    }
    if special {
        return Err(unsupported(
            "0x58e936",
            "slows to a stop at the end of its path for a person whose word 64 is set",
        ));
    }
    a.motion.near_end = 1;
    if let Some(layer) = &a.foreground
        && layer.interruptible == 0
        && !layer.finished()
    {
        return Ok(());
    }
    let mut share = (left / stride).single();
    if !at_least(share, MIN_SHARE.ext()) {
        share = MIN_SHARE.ext();
    }
    let footfall = if second { feet.first } else { feet.second };
    let mut cycle = (footfall.ext() - phase).single();
    if !at_least(cycle, ONE.ext()) {
        cycle = (cycle + ONE.ext()).single();
    }
    let length = match &a.foreground {
        Some(layer) => (layer.clip_duration.ext() / a.rate.ext()).single(),
        None => Ext::ZERO,
    };
    let time = (length * cycle)
        .single()
        .to_i32_truncate()
        .max(MIN_STOP_TIME);
    a.motion.timer = time.wrapping_add(now);
    if let Some(layer) = a.foreground.as_mut() {
        let weight = layer.weight;
        layer.start_fade(now, time, weight, share.to_f32());
    }
    Ok(())
}

/// The heading of an offset `dx`, `dy` along the position's first and
/// second coordinates (0x58eb47 to 0x58ebbe, and 0x58e76c to 0x58e7c7
/// for the end of a path): the offset's angle (virtual function 0x98,
/// 0x5920c6: the arctangent, plus a whole turn when below 0) in eighths
/// of a turn, plus 2, put into 0 to 8 by whole turns. The offset is
/// passed as two stored floats.
fn heading(dx: Ext, dy: Ext) -> Ext {
    let mut angle = Ext::atan2(dy.to_f32(), dx.to_f32());
    let turn = (PI.ext() + PI.ext()).single();
    if !at_least(angle, Ext::ZERO) {
        angle = (angle + turn).single();
    }
    let eight = Ext::from_i32(TURN);
    let mut heading = (((angle * EIGHT.ext()).single() / turn).single() + TWO.ext()).single();
    while !at_least(heading, Ext::ZERO) {
        heading = (heading + eight).single();
    }
    while at_least(heading, eight) {
        heading = (heading - eight).single();
    }
    heading
}

/// Whether the skin's update reaches the carry layer before the
/// foreground layer: the lower priority first (0x5991fd), and between
/// equal priorities the order of the skin's list
/// ([`Animator::carry_first`]).
pub(crate) fn carry_first(a: &Animator) -> bool {
    match (&a.carry, &a.foreground) {
        (Some(carry), Some(foreground)) => match carry.priority.cmp(&foreground.priority) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal => a.carry_first,
        },
        _ => a.carry_first,
    }
}

/// Empties person `id`'s base slot, if it holds a layer (virtual
/// function 0x104, 0x592878): the base slot's layer is always removed at
/// once (0x592890, 0x5a0a7b), which marks the skin's layers changed
/// (0x5991d4).
pub(crate) fn clear_base(world: &mut World, id: u16, a: &mut Animator) {
    if a.base.is_filled() {
        a.base = Base::Empty;
        a.skin_dirty = true;
    }
    world.base_layers.remove(&id);
}

/// The foreground clip is cleared (virtual function 0xfc, 0x586b63): a
/// layer in the slot leaves the skin, which marks the skin's layers
/// changed (virtual function 0x104, 0x592878, then 0x5a0a7b), and the
/// clip's selection becomes -1.
///
/// 0x592878 would instead keep the layer to blend out when byte 0x6f6bd0
/// is set. No code writes that byte, and it is 0 in the memory images of
/// the running game taken at updates 14, 15, 47 and 580, so it is taken
/// to be 0 (see [`end_turn`]).
fn clear_foreground(a: &mut Animator) {
    if a.foreground.take().is_some() {
        a.skin_dirty = true;
    }
    a.selection = -1;
}

/// The walk's footfalls ([`Interpreter::footfalls`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Footfalls {
    /// +0x174: the phase of the walk clip's event `anchor` = `2`.
    pub first: F32,
    /// +0x170: the phase of the walk clip's event `anchor` = `1`.
    pub second: F32,
    /// +0x17c: the stride that ends at the first footfall.
    pub first_stride: F32,
    /// +0x178: the stride that ends at the second footfall.
    pub second_stride: F32,
}

/// The first event of a clip with the property `anchor` and the given
/// value, as the animator's virtual function 0x128 (0x588df8) finds it:
/// the clip's motions in order, each motion's events from the last, and
/// each event's properties in order. The answer is the motion's bone and
/// the event's time.
fn anchor_event<'c>(clip: &'c sims_content::Clip, value: &str) -> Option<(&'c str, i32)> {
    clip.tracks.iter().find_map(|track| {
        track
            .events
            .iter()
            .rev()
            .find(|e| {
                e.properties
                    .iter()
                    .any(|(k, v)| k == "anchor" && v == value)
            })
            .map(|e| (track.bone.as_str(), e.time))
    })
}

/// A direction as object data word 1 holds it (0x5920e8): the direction
/// plus a half, truncated toward zero, then put into 0 to 7 by whole
/// turns.
fn direction_word(direction: F32) -> i16 {
    let mut v = (direction.ext() + Ext::from_f64_bits(HALF_DOUBLE))
        .single()
        .to_i32_truncate();
    if v < 0 {
        v = v.wrapping_add(((7i32.wrapping_sub(v) as u32) >> 3) as i32 * 8);
    }
    if v >= 8 {
        v = v.wrapping_sub(((v as u32) >> 3) as i32 * 8);
    }
    v as i16
}

fn datum(world: &World, id: u16, index: usize) -> Result<i16, Trap> {
    world
        .objects
        .get(&id)
        .ok_or_else(|| Trap::native(10, "missing object"))?
        .data
        .get(index)
        .copied()
        .ok_or_else(|| Trap::Invalid(format!("object data has no word {index}")))
}

fn set_datum(world: &mut World, id: u16, index: usize, value: i16) -> Result<(), Trap> {
    *world
        .objects
        .get_mut(&id)
        .ok_or_else(|| Trap::native(10, "missing object"))?
        .data
        .get_mut(index)
        .ok_or_else(|| Trap::Invalid(format!("object data has no word {index}")))? = value;
    Ok(())
}

fn person_word(world: &World, id: u16, index: usize) -> Result<i16, Trap> {
    world
        .objects
        .get(&id)
        .and_then(|o| o.person.as_ref())
        .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
        .data
        .get(index)
        .copied()
        .ok_or_else(|| Trap::Invalid(format!("person data has no word {index}")))
}

/// Whether `a` is at least `b`: false when either is not a number, as
/// the x87's compare leaves it.
fn at_least(a: Ext, b: Ext) -> bool {
    a >= b
}

/// A direction put into 0 to 8 by whole turns, each step stored to single
/// precision.
fn wrap(direction: F32) -> F32 {
    let turn = Ext::from_i32(TURN);
    let mut d = direction.ext();
    while d < Ext::ZERO {
        d = (d + turn).single();
    }
    while d >= turn {
        d = (d - turn).single();
    }
    d.to_f32()
}

/// A point moved onto the person's path where the path is nearer
/// (0x48313e), as the walk's step does. Points hold the coordinate at
/// object +0xf4 first, as the path's do.
///
/// The segments looked at are, by `index` (the path point the animator
/// heads for, +0x180): below 0, all from the first to the one that ends
/// at the next to last point; 0, the first; above 0, the one that ends at
/// point `index`. If they do not lie from the first segment to the one
/// ending at the next to last point (compared without sign), the point
/// stays. Otherwise the nearest of the path's first looked-at point and,
/// for each segment, the segment's point nearest to the point (0x45ed5f:
/// each coordinate rounded half up and truncated) replaces it, an earlier
/// one winning a tie. Squared distances are whole numbers, and the best
/// is kept in single precision. Fails where the original would read past
/// the path's end.
pub fn snap_to_path(path: &[[i32; 2]], point: [i32; 2], index: i32) -> Result<[i32; 2], Trap> {
    let last_start = (path.len() as i32).wrapping_sub(3);
    let (first, last) = match index {
        i if i < 0 => (0, last_start),
        0 => (0, 0),
        i => (i - 1, i - 1),
    };
    if first < 0 || first as u32 > last_start as u32 || last < 0 || last as u32 > last_start as u32
    {
        return Ok(point);
    }
    let at = |i: i32| {
        path.get(i as usize)
            .copied()
            .ok_or_else(|| unsupported("0x4831f1", "moves a step onto a path past the path's end"))
    };
    let square = |p: [i32; 2], q: [i32; 2]| {
        let d0 = p[0].wrapping_sub(q[0]);
        let d1 = p[1].wrapping_sub(q[1]);
        d0.wrapping_mul(d0).wrapping_add(d1.wrapping_mul(d1))
    };
    let mut best = at(first)?;
    let mut nearest = Ext::from_i32(square(best, point)).single();
    for i in first..=last {
        let (a, b) = (at(i)?, at(i + 1)?);
        let f1 = Ext::from_i32(a[1].wrapping_sub(b[1]));
        let f0 = Ext::from_i32(a[0].wrapping_sub(b[0]));
        let g0 = Ext::from_i32(point[0].wrapping_sub(b[0]));
        let g1 = Ext::from_i32(point[1].wrapping_sub(b[1]));
        let dot = ((g0 * f0).single() + (g1 * f1).single()).single();
        let length = ((f0 * f0).single() + (f1 * f1).single()).single();
        let mut t = (dot / length).single();
        if !at_least(t, Ext::ZERO) {
            t = Ext::ZERO;
        } else if t > Ext::from_i32(1) {
            t = Ext::from_i32(1);
        }
        let along = |k: usize| {
            let v = (Ext::from_i32(a[k].wrapping_sub(b[k])) * t).single();
            ((v + Ext::from_i32(b[k])).single() + HALF.ext())
                .single()
                .to_i32_truncate()
        };
        let o = [along(0), along(1)];
        let d = square(o, point);
        if Ext::from_i32(d) < nearest {
            nearest = Ext::from_i32(d).single();
            best = o;
        }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_under_way_does_not_stop() {
        let mut a = sims_sim::animator::LOADED;
        a.position = [1.0, 2.0, 3.0].map(F32::from_f32);
        a.destination = [5.0, 6.0, 7.0].map(F32::from_f32);
        a.state = 9;
        a.motion.segment = 2;
        for arrival in [1, -1] {
            a.arrival = arrival;
            let before = a.clone();
            let mut requests = BTreeSet::new();
            assert!(!stop_route(&mut a, &mut requests, 7));
            assert_eq!(a, before);
            // The stop is asked for.
            assert_eq!(requests, BTreeSet::from([7]));
        }
        // Any other end: the animator stands where it is, and no stop is
        // asked for.
        a.arrival = 2;
        let mut requests = BTreeSet::new();
        assert!(stop_route(&mut a, &mut requests, 7));
        assert!(requests.is_empty());
        assert_eq!((a.arrival, a.motion.segment, a.state), (0, -1, 3));
        assert_eq!(a.destination, a.position);
    }

    fn feet() -> Footfalls {
        Footfalls {
            first: F32::from_f32(0.25),
            second: F32::from_f32(0.75),
            first_stride: F32::from_f32(8.0),
            second_stride: F32::from_f32(10.0),
        }
    }

    fn walking(phase: f32, interruptible: i32) -> Animator {
        let mut a = sims_sim::animator::LOADED;
        let mut layer = Layer::start("walk", F32::from_f32(1000.0), 38, true, 0, true, 0);
        layer.phase = F32::from_f32(phase);
        layer.interruptible = interruptible;
        a.foreground = Some(layer);
        a
    }

    #[test]
    fn the_walk_start_goes_one_clip_distance_ahead() {
        let f = F32::from_f32;
        // Direction 0 is (0 + 6) * 45 = 270 degrees: straight down the
        // second coordinate. 0.75 / 3 * 16 is 4.
        let to = stop_walk_destination([f(100.0), f(50.0), f(0.0)], f(0.75));
        assert_eq!(to, [f(100.0), f(46.0)]);
        // Direction 2 is a full turn: along the first coordinate.
        let to = stop_walk_destination([f(100.0), f(50.0), f(2.0)], f(0.75));
        assert_eq!(to[0], f(104.0));
        assert!((to[1].to_f32() - 50.0).abs() < 1e-5);
    }

    #[test]
    fn the_walk_start_follows_the_clip_until_its_phase_is_095() {
        let f = F32::from_f32;
        let from = [f(0.0), f(10.0)];
        let to = [f(8.0), f(2.0)];
        assert_eq!(stop_walk_step(from, to, f(0.0)), Some(from));
        assert_eq!(stop_walk_step(from, to, f(0.5)), Some([f(4.0), f(6.0)]));
        assert_eq!(stop_walk_step(from, to, STOP_WALK_END), None);
        assert_eq!(stop_walk_step(from, to, f(1.0)), None);
        // Just below the end it still mixes.
        let below = F32::from_bits(STOP_WALK_END.to_bits() - 1);
        assert!(stop_walk_step(from, to, below).is_some());
    }

    #[test]
    fn a_walk_far_from_the_end_does_not_slow() {
        // Phase 0.5 is between the footfalls: the stride is the first's, 8.
        let mut a = walking(0.5, 1);
        let before = a.clone();
        let left = Ext::from_i32(9);
        slow_down(&mut a, &feet(), left, false, 0, 50).unwrap();
        assert_eq!(a, before);
        // At pace 1 the stride is doubled, and 9 is within it.
        slow_down(&mut a, &feet(), left, false, 1, 50).unwrap();
        assert_eq!(a.motion.near_end, 1);
    }

    #[test]
    fn a_walk_within_a_stride_times_its_stop() {
        // Phase 0.5: the next footfall is the second, at 0.75, a quarter
        // of the clip on; less than a whole clip, so a whole clip more.
        // 1.25 times the clip's 1000 ms at speed 1 is 1250 ms. The fade
        // takes the weight from 1 to the share of the stride left, 4 of 8.
        let mut a = walking(0.5, 1);
        slow_down(&mut a, &feet(), Ext::from_i32(4), false, 0, 50).unwrap();
        assert_eq!((a.motion.near_end, a.motion.timer), (1, 1300));
        let layer = a.foreground.as_ref().unwrap();
        assert!(layer.fading);
        assert_eq!(
            (
                layer.fade.start,
                layer.fade.length,
                layer.fade.from,
                layer.fade.to
            ),
            (50, 1250, F32::from_f32(1.0), F32::from_f32(0.5))
        );
        // A clip that may not be interrupted and has not finished: only
        // the flag is set.
        let mut a = walking(0.5, 0);
        slow_down(&mut a, &feet(), Ext::from_i32(4), false, 0, 50).unwrap();
        assert_eq!((a.motion.near_end, a.motion.timer), (1, 0));
        assert!(!a.foreground.unwrap().fading);
        // With no clip the time is the least, 100 ms.
        let mut a = sims_sim::animator::LOADED;
        slow_down(&mut a, &feet(), Ext::from_i32(4), false, 0, 50).unwrap();
        assert_eq!((a.motion.near_end, a.motion.timer), (1, 150));
        // A person with word 64 set is not ported.
        let mut a = walking(0.5, 1);
        assert!(slow_down(&mut a, &feet(), Ext::from_i32(4), true, 0, 50).is_err());
    }
}
