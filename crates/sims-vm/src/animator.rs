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
use crate::placement::on_lot;
use sims_core::{Ext, F32};
use sims_sim::World;
use sims_sim::animator::{Advance, Animator, EPSILON, LOADED, UNSET};

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
        let posture = i32::from(word(&person.data, POSTURE));
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
        let guid = object.guid;
        let flags = self.machine(world, id, 0, false).definition_words(guid)?[DEFINITION_FLAGS];

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
            match a.state {
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
                }
                3 => {
                    if a.position.iter().any(|&p| p.ext() == UNSET.ext()) {
                        place(&mut a, position, direction);
                    } else {
                        stand(&mut a)?;
                    }
                }
                state => {
                    return Err(Trap::Unsupported(format!(
                        "the animator in state {state} (0x58faa2), which is not ported"
                    )));
                }
            }
        }

        // 0x5992b3: the skin's update.
        if a.skin_dirty || now != a.skin_clock {
            a.skin_dirty = false;
            if let Some(layer) = a.foreground.as_mut() {
                match layer.update(now).map_err(Trap::Unsupported)? {
                    Advance::Unbound => {}
                    Advance::Silent | Advance::Events => layer.commit(),
                }
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
fn place(a: &mut Animator, position: [i32; 3], direction: i16) {
    let own = [position[0], position[1], i32::from(direction)];
    for (i, own) in own.into_iter().enumerate() {
        if (a.position[i].ext() - UNSET.ext()).single().abs() <= EPSILON.ext() {
            let value = Ext::from_i32(own).single().to_f32();
            a.position[i] = value;
            a.destination[i] = value;
        }
    }
}

/// State 3 for a placed animator (0x58ddbf to 0x58df01): both directions
/// are put into 0 to 8, then an animator arrived and facing its
/// destination's direction notes the route's end.
fn stand(a: &mut Animator) -> Result<(), Trap> {
    a.destination[2] = wrap(a.destination[2]);
    a.position[2] = wrap(a.position[2]);
    let eps = EPSILON.ext();
    let near = |to: F32, at: F32| (to.ext() - at.ext()).single().abs() <= eps;
    let aligned = near(a.destination[2], a.position[2]);
    let arrived = near(a.destination[0], a.position[0]) && near(a.destination[1], a.position[1]);
    if !(aligned && arrived) {
        return Err(Trap::Unsupported(
            "the animator in state 3 has to turn or walk to its destination (0x58df01), which \
             is not ported"
                .into(),
        ));
    }
    match a.arrival {
        0 => {}
        -1 | 2 | 3 => a.arrival = 2,
        _ => {
            return Err(Trap::Unsupported(
                "the animator arrived with a route result that sends it to state 9 (0x58def2), \
                 which is not ported"
                    .into(),
            ));
        }
    }
    Ok(())
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
