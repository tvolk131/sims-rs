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
use sims_sim::animator::{Advance, Animator, EPSILON, LOADED, Layer, UNSET};

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
/// The turn clip of half a turn the other way (0x58f4fa).
const HALF_TURN: usize = 5;
/// The timed event a clip may send that the simulation does not see
/// (0x5915e1).
const FOOTSTEP: &str = "footstep";
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
                4 => {
                    turn(world, id, a)?;
                    Next::Done
                }
                9 => self.follow(world, id, a)?,
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
    /// become 2), or, with any other route result, follows the route
    /// (state 9). One arrived that has to turn starts a turn (0x58df01).
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
        self.start_clip(world, id, a, selection, now)?;
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
    /// end mode is 0.
    ///
    /// The layer's timed events are dispatched as it plays (0x5912fd).
    /// A footstep event only picks a sound for the floor under the
    /// person (0x58900f, which reads the lot) and plays it through the
    /// sound events (0x57a54c); sound is not modeled, so a clip whose
    /// only events are footsteps plays here. That the sound events
    /// change nothing the simulation keeps is **unverified** beyond the
    /// capture's random number streams, which they would advance.
    ///
    /// Fails for what is not ported: a foreground slot that holds a layer
    /// (0x592878), a clip that is not found (the original then reads
    /// through a null pointer) and a clip with timed events other than
    /// footsteps.
    fn start_clip(
        &self,
        world: &mut World,
        id: u16,
        a: &mut Animator,
        selection: usize,
        now: i32,
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
        if clip.event_keys.iter().any(|key| key != FOOTSTEP) {
            return Err(unsupported(
                "0x5912fd",
                &format!("plays the clip {name:?}, whose timed events other than footsteps"),
            ));
        }
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
            false,
            now,
        );
        if a.promote {
            a.promote = false;
            layer.priority += 5;
        }
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
    /// +0x200, which the animator's constructor (0x58831d) sets to half
    /// the distance of the posture's clip 0x14 (clip +0x30, virtual
    /// function 0x120, 0x588ed7) times 16/3.
    ///
    /// The value is worked out here when it is needed, not kept: it is
    /// the constructor's while the clip's name is, which holds as long as
    /// the person's words 58, 64 and 65 and its type stay as they were.
    fn look_ahead(&self, world: &World, id: u16) -> Result<Ext, Trap> {
        let name = self
            .posture_clip(world, id, WALK)?
            .ok_or_else(|| unsupported("0x588ed7", "has no name for its walk clip"))?;
        let clip = self
            .program
            .animations
            .clip(&name)
            .map_err(|e| unsupported("0x588ed7", &format!("finds no clip {name:?}: {e}")))?;
        let k = (Ext::from_i32(16) / Ext::from_i32(3)).single();
        let half = (F32::from_bits(clip.distance).ext() * HALF.ext()).single();
        let stride = (half * k).single();
        Ok((stride * HALF.ext()).single())
    }

    /// State 9 (0x58e4e0): the animator follows the person's path. From
    /// the path point it is heading for (+0x180), it looks ahead along the
    /// segment from the point before, from where its position projects on
    /// the segment (+0x1ec, 0 at the point before and 1 at the point), by
    /// steps of one unit, to the first rounded point at least the look-
    /// ahead distance away; a segment it has passed moves it to the next.
    /// Then it heads there (0x58eb29).
    ///
    /// Fails for what is not ported: the last segment (0x58e740), a
    /// foreground clip (0x58e584), a position beyond either end of the
    /// segment, and a person whose data word 17 is set (another
    /// look-ahead).
    fn follow(&self, world: &mut World, id: u16, a: &mut Animator) -> Result<Next, Trap> {
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
                return Err(unsupported(
                    "0x58e740",
                    "follows the last segment of its path",
                ));
            }
            if a.foreground.is_some() {
                return Err(unsupported("0x58e584", "follows a path while a clip plays"));
            }
            a.state = if pace == 1 { 10 } else { 9 };
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
            a.motion.progress = t.to_f32();
            if !at_least(t, zero) {
                return Err(unsupported(
                    "0x58e631",
                    "is behind the start of its segment",
                ));
            }
            if t > ONE.ext() {
                return Err(unsupported("0x58e64d", "is beyond the end of its segment"));
            }
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
                    return self.head(world, id, a, dx, dy, distance);
                }
                a.motion.progress = (inverse + t).single().to_f32();
            }
            a.motion.progress = F32::ZERO;
            a.motion.segment = segment + 1;
        }
        Err(unsupported("0x58e50a", "found no point ahead on its path"))
    }

    /// The animator heads for a point ahead (0x58eb29): its direction
    /// there, in eighths of a turn from 0 to 8 (0x5920c6: the angle of
    /// the offset, plus a turn when negative, times 8 over a turn, plus
    /// 2), and the turn from its own direction, put into -4 to 4. A turn
    /// of 2 eighths or more the negative way is made on the spot first:
    /// the destination becomes the position with the new direction, and
    /// the animator stands (state 3).
    ///
    /// An offset of no length leaves the state as it is. Fails for what
    /// is not ported: a direction out of range, a speed below 1, a person
    /// of kind above 1, and a turn of less than 2 eighths the negative way
    /// or any the positive way, which walk (0x58ec8a, 0x58ec4f).
    fn head(
        &self,
        world: &World,
        id: u16,
        a: &mut Animator,
        dx: Ext,
        dy: Ext,
        distance: Ext,
    ) -> Result<Next, Trap> {
        if distance.abs() <= EPSILON.ext() {
            return Ok(Next::Done);
        }
        let mut angle = Ext::atan2(dy.to_f32(), dx.to_f32());
        let turn = (PI.ext() + PI.ext()).single();
        if !at_least(angle, Ext::ZERO) {
            angle = (angle + turn).single();
        }
        let heading = (((angle * EIGHT.ext()).single() / turn).single() + TWO.ext()).single();
        if !at_least(heading, Ext::ZERO) || at_least(heading, Ext::from_i32(TURN)) {
            return Err(unsupported("0x58eb86", "heads in a direction out of range"));
        }
        let mut d = (heading - a.position[2].ext()).single();
        while d >= FOUR.ext() {
            d = (d - EIGHT.ext()).single();
        }
        if d < (-FOUR.ext()).single() {
            return Err(unsupported(
                "0x58ebe9",
                "turns by less than half a turn back",
            ));
        }
        if !at_least(a.rate.ext(), Ext::from_f64_bits(ONE_DOUBLE)) {
            return Err(unsupported(
                "0x58ec12",
                "heads somewhere at a speed below 1",
            ));
        }
        if person_word(world, id, KIND)? > 1 {
            return Err(unsupported(
                "0x58ec2d",
                "heads somewhere for a person of kind above 1",
            ));
        }
        let limit = TWO.ext();
        if d >= limit {
            return Err(unsupported(
                "0x58ec4f",
                "turns 2 eighths or more the positive way",
            ));
        }
        if d > -limit {
            return Err(unsupported("0x58ec8a", "starts walking"));
        }
        a.destination = [a.position[0], a.position[1], heading.to_f32()];
        a.state = 3;
        Ok(Next::Again)
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
        a.state = 3;
        world.animators.insert(id, a);
        Ok(())
    }
}

/// State 4 (0x58e045): the animator turns on the spot as its foreground
/// clip plays. While the clip is short of its end (layer +0x28 below 1),
/// the direction and position go from where the turn started to the
/// destination in step with it; the direction is written to object data
/// word 1, rounded and put into 0 to 7 (0x5920e8), and the clip takes the
/// animator's speed (0x598636).
///
/// Fails for what is not ported: no foreground clip, and the end of the
/// turn (0x58e06f).
fn turn(world: &mut World, id: u16, a: &mut Animator) -> Result<(), Trap> {
    let layer = a.foreground.as_ref().ok_or_else(|| {
        unsupported(
            "0x58e05a",
            "turns with no clip: it reads through a null pointer",
        )
    })?;
    let t = layer.phase.ext();
    if t >= ONE.ext() {
        return Err(unsupported("0x58e06f", "comes to the end of a turn"));
    }
    let u = (ONE.ext() - t).single();
    let mix = |from: F32, to: F32| ((u * from.ext()).single() + (t * to.ext()).single()).single();
    let m = a.motion;
    let direction = mix(m.turn_from[2], a.destination[2]);
    a.position[2] = direction.to_f32();
    // 0x5920e8.
    let mut v = (direction + Ext::from_f64_bits(HALF_DOUBLE))
        .single()
        .to_i32_truncate();
    if v < 0 {
        v = v.wrapping_add(((7i32.wrapping_sub(v) as u32) >> 3) as i32 * 8);
    }
    if v >= 8 {
        v = v.wrapping_sub(((v as u32) >> 3) as i32 * 8);
    }
    set_datum(world, id, DIRECTION, v as i16)?;
    a.position[0] = mix(m.turn_from[0], a.destination[0]).to_f32();
    a.position[1] = mix(m.turn_from[1], a.destination[1]).to_f32();
    let rate = a.rate;
    if let Some(layer) = a.foreground.as_mut() {
        layer.set_speed(rate);
    }
    Ok(())
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
