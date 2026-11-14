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
//!    A change clears the carry slot, which is always empty here, and a
//!    change to clip 0 starts its clip (0x26), which is not ported.
//! 4. The base layer is brought up to date (virtual function 0x58,
//!    0x590430): a posture other than the one it was started for clears
//!    it; then, unless the definition's word 102 has bit 2 set, or the
//!    layer is there and the person's kind (word 65) is at most 1, the
//!    posture is noted and, for postures 0 to 2, the posture's clip
//!    started. Starting it first looks the clip up (virtual function
//!    0xf4, 0x592625), which reads the person's appearance again if the
//!    animator says so ([`Animator::profile_stale`]); then the clip goes
//!    into the base slot (0x592726) to repeat at priority -10, which
//!    marks the skin's layers changed. Only posture 0 is ported: its clip
//!    was found every time the capture shows (a base layer after each
//!    start), while the clips of postures 1 and 2 and a clip the lookup
//!    does not find (0x59db2f, which leaves the slot empty) were never
//!    seen and fail.
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
//!    2). Any other state, and an animator in state 3 that has to turn or
//!    walk, is not ported.
//! 8. The tail: the skin's layers are updated once at the clock (0x5992b3,
//!    from 0x5a0ef5 or from 0x58f8bf: the second call finds the skin
//!    updated at the same clock and does nothing), the animator places
//!    itself if it has not, and the head's countdown runs (0x590768):
//!    person word 45 goes down by 1 when above 0, and reaching 0 sets
//!    word 42 to 8. With bit 0 of word 42 set the head follows something
//!    and may change word 41 (0x590768, 0x5904d2), which is not ported.
//!
//! Left out as presentation, as far as the original's code shows: the
//! draw state (0x434101), the face (0x58faf6), the pose (0x5994c3,
//! 0x5a15bc), what a carried object's position follows (0x590b27 and the
//! tail of 0x58dc72), the head's turn, and the route's own position
//! fields (+0x18 to +0x28) which only states 6 to 10 read. That none of
//! this changes what the simulation reads is **unverified**.
//!
//! The person's tick passes the animator a flag (byte 0x6e48c0 is 0)
//! that picks which skin routine runs first (0x5a0ffd or 0x5a0ef5); either
//! way the layers are updated once, and which routine ran changes only the
//! pose.

use crate::fault::Trap;
use crate::interpreter::Interpreter;
use crate::people::read_appearance_if_stale;
use crate::placement::{Place, on_lot, pool_tile};
use sims_core::{Ext, F32};
use sims_sim::World;
use sims_sim::animator::{Animator, EPSILON, LOADED, Layer, UNSET};
use std::cmp::Ordering;

/// Person data words.
const POSTURE: usize = 0;
const HEAD_MODE: usize = 42;
const HEAD_COUNTDOWN: usize = 45;
const KIND: usize = 65;
/// Object data words.
const DIRECTION: usize = 1;
/// The word 0x588fa0 tests for 1 (object +0x6c).
const PACE: usize = 17;
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
                if next == 0 {
                    return Err(Trap::Unsupported(
                        "a person starts carrying: its carry clip (0x26, 0x5903fc) is not \
                         ported"
                            .into(),
                    ));
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
        if a.skin_dirty || now != a.skin_clock {
            a.skin_dirty = false;
            if let Some(mut layer) = a.foreground.take() {
                let updated = self.update_layer(world, id, &mut a, &mut layer, now);
                a.foreground = Some(layer);
                updated?;
            }
            a.skin_clock = now;
        }
        place(&mut a, position, direction);

        // 0x590768: the head's countdown.
        if a.head_clock != now {
            let person = world
                .objects
                .get_mut(&id)
                .and_then(|o| o.person.as_mut())
                .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?;
            let countdown = person.data.get(HEAD_COUNTDOWN).copied().unwrap_or(0);
            if countdown > 0 {
                person.data[HEAD_COUNTDOWN] = countdown - 1;
                if countdown == 1 {
                    person.data[HEAD_MODE] = 8;
                }
            }
            if person.data.get(HEAD_MODE).copied().unwrap_or(0) & 1 != 0 {
                return Err(Trap::Unsupported(
                    "person word 42 has bit 0 set: the head follows something (0x5907c5, \
                     0x5904d2), which is not ported"
                        .into(),
                ));
            }
            a.head_clock = now;
        }
        world.animators.insert(id, a);
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
/// The posture clip whose distance gives the walk's look-ahead (0x58831d,
/// virtual function 0x120 with 0x14).
const WALK: usize = 0x14;
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
/// The turn clip of half a turn the other way (0x58f4fa).
const HALF_TURN: usize = 5;
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
/// The walk clip with data word 17 set (state 10).
const RUN: usize = 0x15;
/// Object data word 22, which the animator's scale writes (0x59209f).
const SCALE_WORD: usize = 22;

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
                3 => {
                    if a.position.iter().any(|&p| p.ext() == UNSET.ext()) {
                        place(a, position, direction);
                        Next::Done
                    } else {
                        self.stand(world, id, a, now)?
                    }
                }
                4 => turn(world, id, a)?,
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
        if a.posture != posture && a.base {
            // 0x592878 removes the base slot's layer at once (0x5a0a7b),
            // which marks the skin's layers changed (0x5991d4).
            a.base = false;
            a.skin_dirty = true;
        }
        if flags & 4 == 0 && !(a.base && kind <= 1) {
            a.posture = posture;
            match posture {
                0 => {
                    read_appearance_if_stale(world, self.program, id, &mut a.profile_stale)?;
                    // 0x592726 empties the slot first (it is empty here)
                    // and adds the new layer to the skin.
                    a.base = true;
                    a.skin_dirty = true;
                }
                1 | 2 => {
                    return Err(Trap::Unsupported(format!(
                        "the animator starts the base clip of posture {posture} (0x590497), \
                         which has not been seen; not ported"
                    )));
                }
                _ => {}
            }
        }
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

    /// State 3 for a placed animator (0x58ddbf). Both directions are put
    /// into 0 to 8. An animator arrived and facing its destination's
    /// direction notes the route's end (+0x188: 0 stays 0; -1, 2 and 3
    /// become 2, 0x58f135) and stops walking ([`Animator::walking`] and
    /// +0x24, +0x28 and +0x290 cleared, 0x58f117), or, with any other
    /// route result, follows the route (state 9). One arrived that has to
    /// turn starts a turn (0x58df01).
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
        self.start_turn(world, id, a, now)?;
        Ok(Next::Done)
    }

    /// The start of a turn on the spot (0x58df01 to 0x58f5f9), for the
    /// turn that is ported: more than 3 eighths the negative way, which
    /// plays the posture's clip 5 (half a turn) and ends 4 eighths short
    /// of the clip's start.
    ///
    /// The turn d is the destination's direction less the position's, put
    /// into -4 to 4 by whole turns. The smallest turn worth a clip is a
    /// quarter of an eighth over the speed. The turn is noted (+0x1f0,
    /// the clip's turn +0x1f4, the destination +0x1dc to +0x1e4, the
    /// position +0x1d0 to +0x1d8); the destination's direction becomes the
    /// position's plus d less the clip's turn; the clip starts in the
    /// foreground (0x5926d7) at the animator's speed and priority 5
    /// (0x59867f), and the animator turns (state 4).
    fn start_turn(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        now: i32,
    ) -> Result<(), Trap> {
        let eight = EIGHT.ext();
        let mut d = (a.destination[2].ext() - a.position[2].ext()).single();
        while d >= FOUR.ext() {
            d = (d - eight).single();
        }
        if d < (-FOUR.ext()).single() {
            return Err(unsupported(
                "0x58e039",
                "turns by less than half a turn back",
            ));
        }
        let step = ((ONE.ext() * QUARTER.ext()).single() / a.rate.ext()).single();
        if person_word(world, id, KIND)? > 1 {
            return Err(unsupported("0x58df72", "turns a person of kind above 1"));
        }
        if d >= step {
            return Err(unsupported("0x58f4ec", "turns the positive way"));
        }
        if d > -step {
            return Err(unsupported("0x58dfc0", "turns by less than a step"));
        }
        if d >= -THREE.ext() {
            return Err(unsupported("0x58f510", "turns by 3 eighths or less"));
        }
        let selection = HALF_TURN;
        let clip_turn = F32::from_bits(0xc080_0000);
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
        Ok(())
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
    /// Fails for what is not ported: a foreground slot that holds a layer
    /// (0x592878) and a clip that is not found (the original then reads
    /// through a null pointer).
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
        if let Some(layer) = &a.foreground {
            return Err(unsupported(
                "0x592878",
                &format!(
                    "clears the foreground clip {:?} to start another",
                    layer.clip
                ),
            ));
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
        Ok(())
    }

    /// The name of one of the posture's clips (0x59db2f), counted from 0:
    /// the type's own table first; then, for a person whose word 64 is
    /// set, the special tables (not ported); then the global table for the
    /// person's age (word 58 at most 17) or, for an adult, sex (bit 0 of
    /// word 65); then the global table 150. An empty name counts as none.
    /// The old prototype's `posture_name`
    /// (legacy/crates/sims-vm/src/animation.rs) reads the same tables.
    fn posture_clip(
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
                return self.end_of_path(world, id, a, p, now);
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
    /// point before the path's last, `end`. The last point only gives the
    /// direction to face at the end (0x58e77a), which the branch ported
    /// here does not use.
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
    /// again (0x58f089).
    ///
    /// The original also sets global 0x6f6d68 to 13 and then 14, and a
    /// local byte from 0x6f6d70 that only matters when it is 1 (0x58f6eb);
    /// 0x6f6d70 is 0 in a memory image of the running game. Neither is
    /// kept.
    ///
    /// Fails for what is not ported: a person whose data word 9 is clear
    /// (0x58e8be: a timer and a stopping clip), and one whose word 64 is
    /// set (0x58e83b and 0x58e93c).
    fn end_of_path(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        end: [i32; 2],
        now: i32,
    ) -> Result<Next, Trap> {
        if datum(world, id, STOPS)? == 0 {
            return Err(unsupported(
                "0x58e8be",
                "comes to the end of its path with object data word 9 clear",
            ));
        }
        let pace = datum(world, id, PACE)?;
        let [p0, p1] = end.map(Ext::from_i32);
        let dx = (p1 - a.position[0].ext()).single();
        let dy = (p0 - a.position[1].ext()).single();
        let left = ((dy * dy).single() + (dx * dx).single())
            .single()
            .sqrt()
            .single();
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
        self.walk_step(world, id, a, [sx, sy], square)?;
        a.destination = a.position;
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
    ///    any for a person whose word 64 is set (0x58ed52, states 6 and
    ///    11). That is not ported and fails.
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
    ///    there (virtual function 0x54, [`Interpreter::fits`]; a person
    ///    who does not goes to 0x58f682, not ported, and fails). The
    ///    person is put there (virtual function 0x88, 0x45e4e9: the
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
                return Err(unsupported(
                    "0x58ed52",
                    "stops short of a point too far to walk to",
                ));
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
    /// becomes the step's end, unrounded. See [`Interpreter::walk`].
    fn walk_step(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        offset: [Ext; 2],
        distance: Ext,
    ) -> Result<(), Trap> {
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
            return Err(unsupported("0x58f682", "finds its next step blocked"));
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
        Ok(())
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
        Ok(())
    }
}

/// State 4 (0x58e045): the animator turns on the spot as its foreground
/// clip plays. While the clip is short of its end (layer +0x28 below 1),
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

/// The animator stops following its route (0x589e2d), as the route's
/// steps 9 and 10 and its end ask. While the route is under way (its end,
/// +0x188, is 1 or -1) nothing stops: +0x284 is set to 1, which nothing
/// ported reads, and the answer is false. Otherwise the end becomes 0,
/// the segment -1, the destination the position, and the animator
/// stands (state 3); the answer is true.
///
/// The original also sets +0x1c0 to 0, +0x18c to 1 and the global
/// 0x6f6d68 to 7. Nothing ported reads them, and they are not kept.
pub(crate) fn stop_route(a: &mut Animator) -> bool {
    if matches!(a.arrival, 1 | -1) {
        return false;
    }
    a.arrival = 0;
    a.motion.segment = -1;
    a.destination = a.position;
    a.state = 3;
    true
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
            assert!(!stop_route(&mut a));
            assert_eq!(a, before);
        }
        // Any other end: the animator stands where it is.
        a.arrival = 2;
        assert!(stop_route(&mut a));
        assert_eq!((a.arrival, a.motion.segment, a.state), (0, -1, 3));
        assert_eq!(a.destination, a.position);
    }
}
