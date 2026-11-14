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

/// How far past the clip's end an event may lie (0x66d738): 40.0.
const EVENT_SLACK: F32 = F32::from_bits(0x4220_0000);

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
///
/// Byte +0xdb ([`Animator::undrawn`]) is the constructor's 1: the reset
/// does not touch it. The house view's first frame clears it. In a
/// capture of the original loading the Goth house, a watchpoint saw a
/// frame drawn before the first lot update, and person 18's +0xd8 to
/// +0xdb already held 0 then. The walking flag is the reset's 0
/// (0x589262).
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
    motion: Motion::NONE,
    undrawn: true,
    walking: false,
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
    /// Where the animator is along a route and through a turn.
    pub motion: Motion,
    /// Byte +0xdb: the house view has not drawn the person yet. The
    /// constructors set it (0x584766, 0x584dab) and nothing else does;
    /// the first draw clears it, unsets the position and places the
    /// animator again (`frame.rs` in sims-vm, 0x590c05). The animator's
    /// bytes +0xd9 and +0xda, which the constructors also set and the
    /// draw goes through after this one, are not kept: see there.
    pub undrawn: bool,
    /// +0x19c: the animator walks. Starting or going on with a walk sets
    /// it (0x58ed72); a turn on the spot, its start (0x58df01), standing
    /// at its destination (0x58f11d), a new animator (0x587de2) and the
    /// reset (0x589262) clear it. The walk's
    /// start reads it (0x58ed1d): while it is set, a walk goes on without
    /// asking whether to stop.
    pub walking: bool,
}

/// What the animator keeps while it follows a route (states 3 to 10):
/// where it is along the person's path and where a turn started. A new
/// animator (0x587d2c) holds -1 in the segment and 0 in the rest
/// ([`Motion::NONE`]), and its reset (0x589220) sets the segment to -1
/// again; a route's start (0x589c85) sets the segment and the progress to
/// 0, and a turn's start (0x58f559) writes the rest before the turn
/// (state 4) reads them.
///
/// The walk keeps its last step, and the end of a path sets two flags
/// (+0x288, +0x28c). A new animator and the reset (0x589262) clear both
/// flags; the route start clears the first.
///
/// The route start also writes +0x8c to +0x94, +0x18c and +0x284, and
/// the turn +0x19c, +0x24, +0x28 and +0x290, all to 0. The walk sets
/// +0x290 to 1, and only the end of a path for a person whose word 64 is
/// set reads it (0x58ef8e), which is not ported. Nothing ported reads the
/// others, and they are left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Motion {
    /// +0x180: the index of the path point the animator is heading for.
    pub segment: i32,
    /// +0x1ec: how far along the segment the animator looks ahead, from 0
    /// (the path point before) to 1 (the point it heads for).
    pub progress: F32,
    /// +0x1d0, +0x1d4, +0x1d8: the position and direction when the turn
    /// started.
    pub turn_from: [F32; 3],
    /// +0x1dc, +0x1e0, +0x1e4: the destination when the turn started.
    pub turn_destination: [F32; 3],
    /// +0x1f0: the turn wanted, in eighths of a turn.
    pub turn_wanted: F32,
    /// +0x1f4: the turn the clip makes, in eighths of a turn.
    pub turn_clip: F32,
    /// +0x294, +0x298: the walk's last step along the position's first
    /// and second coordinates (0x58ee57, 0x58ee6c). The end of a path
    /// takes it once more (0x58e897).
    pub step: [F32; 2],
    /// +0x288: the animator came within its stopping distance of the
    /// path's end (0x58e85c). It then stops however far it still is.
    pub near_end: i32,
    /// +0x28c: the walk goes on without asking whether to stop (0x58e883,
    /// read at 0x58ed15).
    pub finishing: i32,
}

impl Motion {
    /// No route, as a new animator holds it (0x587d2c): the segment -1
    /// and every other field 0.
    pub const NONE: Self = Self {
        segment: -1,
        progress: F32::ZERO,
        turn_from: [F32::ZERO; 3],
        turn_destination: [F32::ZERO; 3],
        turn_wanted: F32::ZERO,
        turn_clip: F32::ZERO,
        step: [F32::ZERO; 2],
        near_end: 0,
        finishing: 0,
    };
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
    /// A timed event can set it too (0x5912fd).
    pub interruptible: i32,
    /// For each motion of the clip that has timed events, in the order of
    /// the motions, the index of the next event its binding's cursor
    /// points at (binding +8, cursor +4). A new layer's cursors point at
    /// the first event (0x594778).
    pub cursors: Vec<i32>,
}

impl Layer {
    /// A new layer (0x5978ef, then 0x597a88 from 0x592726): priority 1,
    /// weight 1, at the clip's start or, backward, at its end, stopped
    /// at the end of the clip. `frames` is the frame count of the clip's
    /// first motion, `bound` says whether a motion binds to a bone of
    /// the skeleton, and `tracks` is how many motions have timed events.
    pub fn start(
        clip: &str,
        clip_duration: F32,
        frames: u32,
        bound: bool,
        tracks: usize,
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
            cursors: vec![0; tracks],
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
    /// When it returns [`Advance::Events`], the caller asks [`Layer::due`]
    /// for each motion's events and handles them (0x5912fd, through the
    /// callback at 0x6f7690), then calls [`Layer::commit`]. The pose the
    /// layer samples is presentation.
    ///
    /// At the clip's end the layer is marked ended. A clip that stops
    /// (end mode 0) stops at its end: speed 0, the phase held to 0 to 1.
    /// A clip that repeats (end mode 1) goes on at the same speed, from
    /// the part of the phase past a whole number (0x597dca, through the C
    /// runtime's floor, 0x636ac9) when the phase went below 0 or above 1.
    ///
    /// Fails for what is not ported: a layer fading in or out, and the
    /// end of a clip that reverses or fades out (end modes 2 and 3).
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
            if self.ended && self.mode == 1 {
                // 0x597dca: a clip that repeats keeps its speed and
                // starts over at the part of the phase past a whole
                // number, when the phase is below 0 or above 1.
                let fraction = (phase - phase.floor()).single();
                // Below 0 or not a number: the comparison is not "at
                // least 0".
                let below = !matches!(
                    phase.partial_cmp(&zero),
                    Some(Ordering::Greater | Ordering::Equal)
                );
                if below || phase > one {
                    self.phase = fraction.to_f32();
                }
            } else if self.ended {
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

    /// Which events of the clip's motion `track` are due in this update,
    /// in the order they are handled, as indices into `events`, each
    /// given as its time and whether it has any property. Moves the
    /// motion's cursor past them (0x597ff9 to 0x598208).
    ///
    /// The original visits the layer's bindings in order and runs this
    /// for each binding whose motion has events, whose bone's priority
    /// (bone +0x68) is at most the layer's (+0x14). A bone's priority is
    /// -10000 unless a layer that overrides it binds the bone (layer
    /// +0x58, 0x5991fd), and no layer ported does, so every binding takes
    /// part.
    ///
    /// An event is handled only if its time is from 0 to the clip's
    /// duration plus 40 (0x66d738), truncated, and it has a property;
    /// other events are passed over. Forward (a speed of at least 0):
    ///
    /// 1. If the event time went back since the update before, the clip
    ///    went round: every event from the cursor to the end is handled,
    ///    and the cursor goes back to the first event (0x5947c3).
    /// 2. From the cursor, every event up to the event time is handled.
    ///    The cursor stops at the first one after it.
    ///
    /// Backward the same with the directions swapped, with one
    /// difference the code has: going round, the cursor steps back
    /// (0x5947bb) before each event, not after, and then goes to the last
    /// event (0x5947cc).
    ///
    /// The cursor steps after its event is handled, and the handler
    /// (0x5912fd) never moves it, so working out the events first and
    /// handling them after gives the same order.
    pub fn due(&mut self, track: usize, events: &[(i32, bool)]) -> Result<Vec<usize>, String> {
        let upper = (self.clip_duration.ext() + EVENT_SLACK.ext())
            .single()
            .to_i32_truncate();
        let (event_time, previous) = (self.event_time, self.previous_event_time);
        let forward = matches!(
            self.speed.ext().partial_cmp(&Ext::ZERO),
            Some(Ordering::Greater | Ordering::Equal)
        );
        let cursor = self
            .cursors
            .get_mut(track)
            .ok_or_else(|| format!("the clip {:?} has no motion {track} with events", self.clip))?;
        let valid = |c: i32| usize::try_from(c).ok().filter(|&c| c < events.len());
        let handled = |(time, any): (i32, bool)| (0..=upper).contains(&time) && any;
        let mut due = Vec::new();
        if forward {
            if event_time < previous {
                while let Some(i) = valid(*cursor) {
                    if handled(events[i]) {
                        due.push(i);
                    }
                    *cursor += 1;
                }
                *cursor = 0;
            }
            while let Some(i) = valid(*cursor) {
                let (time, _) = events[i];
                if (0..=upper).contains(&time) {
                    if time > event_time {
                        break;
                    }
                    if handled(events[i]) {
                        due.push(i);
                    }
                }
                *cursor += 1;
            }
        } else {
            if event_time > previous {
                loop {
                    *cursor -= 1;
                    let Some(i) = valid(*cursor) else { break };
                    if handled(events[i]) {
                        due.push(i);
                    }
                }
                *cursor = events.len() as i32 - 1;
            }
            while let Some(i) = valid(*cursor) {
                let (time, _) = events[i];
                if (0..=upper).contains(&time) {
                    if time < event_time {
                        break;
                    }
                    if handled(events[i]) {
                        due.push(i);
                    }
                }
                *cursor -= 1;
            }
        }
        Ok(due)
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
    /// - The route's segment is -1 ([`Motion`]).
    /// - The posture is -1, the carry selection -1, the carry mode 0, the
    ///   state 3 (standing), the walk flag clear (+0x19c), the next
    ///   foreground clip's raised priority is set again, and the
    ///   appearance is to be read again (+0xd8).
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
        self.motion.segment = -1;
        self.motion.near_end = 0;
        self.motion.finishing = 0;
        self.posture = -1;
        self.carry_selection = -1;
        self.promote = true;
        self.carry_mode = 0;
        self.state = 3;
        self.walking = false;
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
