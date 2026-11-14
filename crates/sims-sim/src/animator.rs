//! What a person's animator keeps that the simulation reads.
//!
//! Every person has an animator (person +0xadc, vtable 0x671ee4). The
//! person's tick calls its tick (virtual function 0x50, 0x58dc72) and the
//! Animate Sim primitive (0x58950b) starts and polls its clips. The
//! animator plays clips on layers of the person's skin (the list at skin
//! +0x7c). Three of its slots matter here: the foreground clip a script
//! plays (+0xc), the base clip of the person's posture (+0x14), and the
//! clip of something carried (+0x13c).
//!
//! What is kept is what scripts can see or what decides what they see:
//! the animator's clock, the speed it plays at, its state, whether it has
//! placed itself on the person's tile, the posture and carry bookkeeping,
//! Animate Sim's progress through a clip's events, the queue of events,
//! the foreground layer's clock, from which "done" is decided, and
//! whether the person's appearance is to be read again before the next
//! clip.
//!
//! What is left out is presentation as far as the original's code shows:
//! the posed bones and the skin (0x5994c3, 0x5a15bc, 0x5a0ef5), the head's
//! turn (0x590768 past the countdown it keeps in person words 42 and 45),
//! the draw state (0x434101), the face (0x58faf6), what a carried object's
//! position follows (0x590b27 and the tail of 0x58dc72), and the base
//! layer's own clock, whose clip has no events. That nothing the
//! simulation reads depends on these is **unverified**.

use serde::{Deserialize, Serialize};
use sims_core::{Ext, F32};
use std::cmp::Ordering;

/// A position or direction the animator has not been given yet
/// (0x671edc).
pub const UNSET: F32 = F32::from_bits(0xc479_c000);

/// The comparison tolerance of the animator's and the layers' code
/// (0x67343c).
pub const EPSILON: F32 = F32::from_bits(0x3586_37bd);

/// What a layer is given as its duration when a clip that plays once has
/// ended (0x597e11): 1e8.
const ENDED_DURATION: F32 = F32::from_bits(0x4cbe_bc20);

/// The animator's state when the load leaves it (person +0xadc, vtable
/// 0x671ee4), before the person's first tick: a new animator
/// ([`Animator::new`]) after the person's reset in load step 13 has reset
/// it ([`Animator::reset`], 0x589220, called from the person's reset
/// 0x45e791).
///
/// What the load does to the animator between the two (reading the saved
/// animations, from 0x45cf2d) is not modeled; the reset clears what it
/// could have left that the simulation reads: the clips, the posture, the
/// position and the event queue. Three values are **inferred** instead:
///
/// - The clock is 0. The constructor reads the clock when the person is
///   made, and the first tick overwrites it whatever it holds.
/// - The speed and its target are 1.0, as the constructor leaves them: the
///   first tick sets the target to 1.4 and moves the speed up by at most
///   0.2, and the capture holds 1.2 after it.
/// - The skin was updated at clock 0 and nothing changed it since. The
///   first tick updates the skin whatever these hold, since its clock is
///   not 0.
///
/// The destination is the constructor's 0. The first tick finds the
/// position unset and places the animator, which writes the destination
/// too, so it never shows.
pub const LOADED: Animator = Animator {
    clock: 0,
    rate: F32::from_bits(0x3f80_0000),
    target: F32::from_bits(0x3f80_0000),
    state: 3,
    arrival: 0,
    position: [UNSET; 3],
    destination: [F32::ZERO; 3],
    head_clock: 0,
    posture: -1,
    base: false,
    pickup: 0,
    carry_mode: 0,
    carry_selection: -1,
    selection: -1,
    playing: false,
    seen: 0,
    slow: false,
    follow_rate: false,
    promote: true,
    events: Vec::new(),
    foreground: None,
    skin_clock: 0,
    skin_dirty: false,
    profile_stale: true,
};

/// A person's animator: the fields of the object at person +0xadc that
/// the simulation reads. Field offsets are the original's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Animator {
    /// +0xb4: the clock the last tick read (virtual function 0x34,
    /// 0x585483).
    pub clock: i32,
    /// +0x228: the speed the animator plays and walks at.
    pub rate: F32,
    /// +0x22c: the speed it moves towards (0x588fa0).
    pub target: F32,
    /// +0x190: what the animator is doing, 0 to 12. 0 plays a script's
    /// clip, 3 stands. The others turn, walk and route, which is not
    /// ported.
    pub state: i32,
    /// +0x188: how the last route ended.
    pub arrival: i32,
    /// +0x80, +0x84, +0x88: where the animator is, and its direction in
    /// eighths of a turn. [`UNSET`] until it places itself.
    pub position: [F32; 3],
    /// +0x1c4, +0x1c8, +0x1cc: where it is going.
    pub destination: [F32; 3],
    /// +0x164: the clock at the head's last turn (0x590768).
    pub head_clock: i32,
    /// +0x198: the posture the base layer was started for.
    pub posture: i32,
    /// Whether the base slot (+0x14) holds a layer.
    pub base: bool,
    /// +0x1a4: set while a clip that picks something up plays, which keeps
    /// the carry selection as it is.
    pub pickup: i32,
    /// +0x1a8: 1 while the person carries something, 6 while a script's
    /// carry clip plays.
    pub carry_mode: i32,
    /// +0x1ac: the carry clip chosen, or -1 for none.
    pub carry_selection: i32,
    /// +0x98: written -1 when a script's clip starts.
    pub selection: i32,
    /// +0xa0: whether Animate Sim has a clip playing.
    pub playing: bool,
    /// +0xa4: how many of the clip's numbered events Animate Sim has seen.
    pub seen: i32,
    /// Byte +0xa8: the clock runs at 42 per tick instead of 50.
    pub slow: bool,
    /// Byte +0x29c: the foreground layer follows the animator's speed.
    pub follow_rate: bool,
    /// +0x1a0: the next foreground layer's priority goes up by 5.
    pub promote: bool,
    /// +0x2c: the events the clips have sent and Animate Sim has not
    /// taken, oldest first.
    pub events: Vec<i32>,
    /// The foreground slot (+0xc).
    pub foreground: Option<Layer>,
    /// Skin +0xb8: the clock at the skin's last update (0x5992b3).
    pub skin_clock: i32,
    /// Skin +0x94: the layers changed since the skin's last update.
    pub skin_dirty: bool,
    /// Byte +0xd8: the person's appearance has to be read again from its
    /// type's body strings (virtual function 0x14, 0x5888d3) before the
    /// next clip is looked up. A new animator and a reset set it; looking
    /// up a clip (0x5893b3 for Animate Sim, virtual function 0xf4,
    /// 0x592625, for the posture's clip) reads the appearance and clears
    /// it.
    pub profile_stale: bool,
}

/// What a layer (0x5978ef, 0x597a88) keeps that decides when its clip
/// ends and which of its events are due. Field offsets are the layer's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layer {
    /// The clip's name.
    pub clip: String,
    /// Clip +0x2c: the clip's duration in milliseconds.
    pub clip_duration: F32,
    /// Whether the layer bound any motion to a bone (layer +8). A layer
    /// with no binding never advances.
    pub bound: bool,
    /// +0x14.
    pub priority: i32,
    /// +0x18: the clock at the last update.
    pub current: i32,
    /// +0x1c: the clock at the update before.
    pub previous: i32,
    /// +0x20: the clip's duration at the layer's speed.
    pub duration: F32,
    /// +0x24: 1.0 forward, -1.0 backward, 0 stopped.
    pub speed: F32,
    /// +0x28: how far through the clip, from 0 to 1.
    pub phase: F32,
    /// +0x2c.
    pub weight: F32,
    /// +0x30: the clip time the last update reached, in milliseconds.
    pub event_time: i32,
    /// +0x34: the clip time the update before reached.
    pub previous_event_time: i32,
    /// +0x38: the frame count of the clip's first motion.
    pub frames: u32,
    /// +0x3c: the last update reached the clip's end.
    pub ended: bool,
    /// +0x40: what happens at the end: 0 stop, 1 repeat, 2 reverse, 3 stop
    /// and fade out.
    pub mode: i32,
    /// +0x44: fading in or out.
    pub fading: bool,
    /// +0x68: an interrupt ends the clip; -1 until Animate Sim sets it.
    pub interruptible: i32,
}

impl Layer {
    /// A new layer (0x5978ef, then 0x597a88 from 0x592726): priority 1,
    /// weight 1, at the clip's start or, backward, at its end, stopped
    /// at the end of the clip. `frames` is the frame count of the clip's
    /// first motion, and `bound` says whether a motion binds to a bone of
    /// the skeleton.
    pub fn start(
        clip: &str,
        clip_duration: F32,
        frames: u32,
        bound: bool,
        reverse: bool,
        now: i32,
    ) -> Self {
        let mut layer = Self {
            clip: clip.to_owned(),
            clip_duration,
            bound,
            priority: 1,
            current: now,
            previous: now,
            duration: F32::ZERO,
            speed: F32::ZERO,
            phase: F32::ZERO,
            weight: F32::from_bits(0x3f80_0000),
            event_time: 0,
            previous_event_time: 0,
            frames,
            ended: false,
            mode: 0,
            fading: false,
            interruptible: -1,
        };
        layer.set_speed(F32::from_bits(if reverse {
            0xbf80_0000
        } else {
            0x3f80_0000
        }));
        if reverse {
            layer.phase = F32::from_bits(0x3f80_0000);
        }
        layer
    }

    /// Sets the speed (0x598636): the duration becomes the clip's divided
    /// by the speed, made positive, or 0 for a speed within
    /// [`EPSILON`] of 0.
    pub fn set_speed(&mut self, speed: F32) {
        self.speed = speed;
        let s = speed.ext();
        let duration = if s.abs() > EPSILON.ext() {
            (self.clip_duration.ext() / s).single()
        } else {
            Ext::ZERO
        };
        self.duration = if duration < Ext::ZERO {
            (-duration).to_f32()
        } else {
            duration.to_f32()
        };
    }

    /// Whether the clip is done (0x586d41): it ended, or it is stopped.
    pub fn finished(&self) -> bool {
        self.ended || self.speed.ext() == Ext::ZERO
    }

    /// The layer's update at clock `now` (0x597c4c), up to and including
    /// the event times. Arithmetic is the FPU's under the game's control
    /// word (24-bit significands).
    ///
    /// The layer's events are dispatched between the event times and
    /// [`Layer::commit`] (0x5912fd, through the callback at 0x6f7690);
    /// that is not ported, so the caller refuses a clip with events. The
    /// pose the layer samples is presentation.
    ///
    /// Fails for what is not ported: a layer fading in or out, and the
    /// end of a clip that repeats, reverses or fades out.
    pub fn update(&mut self, now: i32) -> Result<Advance, String> {
        self.previous = self.current;
        self.current = now;
        let elapsed = Ext::from_i32(now.wrapping_sub(self.previous)).single();
        if !self.bound || self.frames == 0 {
            return Ok(Advance::Unbound);
        }
        let zero = Ext::ZERO;
        let one = Ext::from_i32(1);
        let speed = self.speed.ext();
        let mut increment = if speed.abs() > EPSILON.ext() {
            (elapsed / self.duration.ext()).single()
        } else {
            zero
        };
        // A speed that is not a number counts as backward.
        if !matches!(
            speed.partial_cmp(&zero),
            Some(Ordering::Greater | Ordering::Equal)
        ) {
            increment = -increment;
        }
        let phase = (increment + self.phase.ext()).single();
        self.phase = phase.to_f32();
        if elapsed.abs() > EPSILON.ext() {
            self.ended = if speed >= zero {
                phase >= one
            } else {
                !matches!(phase.partial_cmp(&zero), Some(Ordering::Greater))
            };
            if self.ended {
                if self.mode != 0 {
                    return Err(format!(
                        "a layer with end mode {} reached its clip's end (0x597d28), which is \
                         not ported",
                        self.mode
                    ));
                }
                self.duration = ENDED_DURATION;
                self.speed = F32::ZERO;
                if phase < zero {
                    self.phase = F32::ZERO;
                } else if phase > one {
                    self.phase = F32::from_bits(0x3f80_0000);
                }
            }
        }
        if self.fading {
            return Err("a layer fading in or out (0x597e3c) is not ported".into());
        }
        self.event_time = (self.clip_duration.ext() * self.phase.ext())
            .single()
            .to_i32_truncate();
        Ok(if self.weight.ext() > zero {
            Advance::Events
        } else {
            Advance::Silent
        })
    }

    /// The end of the update (0x59861f): the event time reached becomes the
    /// one before.
    pub fn commit(&mut self) {
        self.previous_event_time = self.event_time;
    }
}

/// How far an update went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advance {
    /// The layer has nothing bound, so its clock stopped after the clock
    /// values; the update ends without [`Layer::commit`].
    Unbound,
    /// The layer has no weight: no event is due. The caller commits.
    Silent,
    /// The events between the event times are due. The caller dispatches
    /// them and commits.
    Events,
}

impl Animator {
    /// A new animator (0x587d2c, after its base 0x58464f), made with its
    /// person, at clock `clock` (the clock when the person is made,
    /// 0x585483).
    ///
    /// Speed and target 1.0, standing (state 3), not placed (the position
    /// unset, 0x671d64), the destination 0, no posture (-1), no clips, the
    /// selections -1, nothing playing and the appearance to be read. The
    /// constructors write every field kept here but the carry selection
    /// (+0x1ac); that is taken to be -1: the person's first tick sets it
    /// to -1 whatever it holds unless the person carries something, and
    /// the reset that follows the first tick sets it to -1 too. The skin is
    /// made later (0x588dcb) and its clock is not read here; it is taken
    /// as 0. The person's first tick updates the skin whatever its clock
    /// holds, since the posture's clip it starts marks the skin changed.
    pub fn new(clock: i32) -> Self {
        Self {
            clock,
            promote: false,
            ..LOADED
        }
    }

    /// The animator's reset (virtual function 0xc, 0x589220), as far as it
    /// writes what is kept here:
    ///
    /// - The position is unset again (virtual function 0x20, 0x589361);
    ///   the destination stays.
    /// - The posture is -1, the carry selection -1, the carry mode 0, the
    ///   state 3 (standing), the next foreground clip's raised priority is
    ///   set again, and the appearance is to be read again (+0xd8).
    /// - The foreground clip is removed and the selection set to -1
    ///   (virtual function 0xfc, 0x586b63); the blend, carry and base
    ///   slots are emptied (0x592878). Removing a layer marks the skin's
    ///   layers changed.
    /// - The event queue is emptied and nothing plays (+0xa0).
    ///
    /// The speed, its target, the clocks, the pickup flag, the count of
    /// events seen and the slow and follow flags stay as they are.
    ///
    /// Fails for what is not ported: a route result other than 0 (0x589e2d
    /// then ends the route) and a foreground clip, whose removal can fade
    /// it out through the blend slot (0x592887).
    pub fn reset(&mut self) -> Result<(), String> {
        if self.arrival != 0 {
            return Err(format!(
                "resetting an animator whose route ended with {} (0x589e2d) is not ported",
                self.arrival
            ));
        }
        if let Some(layer) = &self.foreground {
            return Err(format!(
                "resetting an animator that plays the clip {:?}: removing it (0x592878) can \
                 fade it out, which is not ported",
                layer.clip
            ));
        }
        self.position = [UNSET; 3];
        self.posture = -1;
        self.carry_selection = -1;
        self.promote = true;
        self.carry_mode = 0;
        self.state = 3;
        self.profile_stale = true;
        self.selection = -1;
        if self.base {
            self.base = false;
            self.skin_dirty = true;
        }
        self.events.clear();
        self.playing = false;
        Ok(())
    }

    /// How far the clock runs per tick (0x585483): 42 times 1.2, truncated,
    /// or 42 when slow.
    pub fn per_tick(&self) -> i32 {
        if self.slow {
            42
        } else {
            (Ext::from_i32(42) * F32::from_bits(0x3f99_999a).ext()).to_i32_truncate()
        }
    }

    /// The clock at lot tick count `ticks` (0x585483): the tick count
    /// times [`Animator::per_tick`], under the game's control word.
    pub fn clock_at(&self, ticks: i32) -> i32 {
        (Ext::from_i32(ticks) * Ext::from_i32(self.per_tick()))
            .single()
            .to_i32_truncate()
    }
}
