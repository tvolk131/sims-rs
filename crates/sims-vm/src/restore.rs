//! A person's animator as the load restores it (0x5918ba, the animator's
//! read, virtual function of vtable 0x671ee4), as far as the simulation
//! reads it after the person's reset in load step 13 (0x589220).
//!
//! The read, for the save versions the port reads (from 0x3d):
//!
//! 1. +0xa4, the events Animate Sim has seen ([`Animator::seen`]), then
//!    +0xa0, whether a clip plays. The reset clears +0xa0 (0x58932c) and
//!    keeps +0xa4.
//! 2. The three layer descriptors, each through 0x591eec into the
//!    foreground slot +0xc, the carry slot +0x13c and the base slot +0x14,
//!    in that order. Each makes a layer in the skin ([`descriptor`]).
//! 3. The routing state +0x190, which the reset sets to 3.
//!
//! 4. The head: +0x210 the heading, +0x214 the target's heading, +0x218
//!    the turn and +0x21c the target point; then the placement's
//!    compensation (+0x8c, three floats).
//!
//! The person's reset in load step 13 runs only when the person's type's
//! scripts changed since the save ([`sims_sim::World::changed_types`]).
//! On the Goth house and the Newbie house every person's type has; on the
//! Pleasants' none has. Most of what follows is for a person the load
//! resets; one it does not reset keeps a live animator (see "A person the
//! load does not reset" below).
//!
//! Then (0x591c2e on): the clock is read (0x434101 with 0), the skin is
//! updated with the layers at once (0x5992b3, forced), the animator is
//! placed on the person's tile (virtual function 0x60, 0x592132), the
//! bones placed (0x5994c3), and the head takes its step (0x590768),
//! measures (0x5904d2) and is restored (0x590b27). The person's data words
//! are read after the animator, so that step sees the constructor's zero
//! words: bit 0 of word 42 is clear, so the measurement does not run. The
//! constructor zeroed the head clock (+0x164, 0x587d80) and set the
//! animator's clock (+0xb4) to the clock at the load's tick count
//! (0x585483), so unless that is 0 the step runs (0x590788): word 45 is
//! 0, so nothing counts down, and the turn relaxes once (x0.8); then it
//! settles. The animator's refresh
//! (virtual function 0x54 with 0, 0x58da12) reads the same clock again
//! and changes nothing the simulation keeps.
//!
//! The reset then takes the three layers out of the skin (virtual function
//! 0x104, 0x592878) but leaves the bones they posed and the skin's model
//! matrix ([`sims_sim::animator::HeldPose`]). It leaves the head's fields.
//!
//! The target point (+0x21c) is kept: a measurement whose target object
//! is gone reads it (0x5906bc). Not kept: a layer's +0x5c, +0x78 and +0x7c, which the
//! restore sets and nothing ported reads. A compensation other than 0
//! fails, since the placement adds it and that is not ported.
//!
//! **A person the load does not reset** keeps what the read and the
//! constructor left ([`Interpreter::restore_live`]):
//!
//! - The constructor's fields (0x587d..): [`LOADED`]'s, but the reset's
//!   own writes are not made, so [`Animator::promote`] stays 0.
//! - Its clock (+0xb4): the clock at the load's tick count (0x585483,
//!   from 0x587f5a).
//! - +0xa4 and +0xa0 as saved: the events seen and whether Animate Sim's
//!   clip plays ([`Animator::playing`]).
//! - The three layers, live in their slots: the foreground, the carry
//!   and the base ([`sims_sim::animator::Base::Loaded`]). Each is made at
//!   the clock, with the saved phase and end mode.
//! - The routing state as saved if it is 0 to 3; any other becomes 3
//!   (0x591d44 to 0x591d64, after the read of +0x190).
//! - The skin updated with the layers at the clock (0x5992b3, forced): no
//!   time passes, so each layer's phase stays and its event time becomes
//!   the phase's. A timed event that update reaches would be handled
//!   (0x5912fd) before the person's data words are read; that fails.
//! - Placed on the person's tile (0x592132), and the head's step taken,
//!   which sets the head's clock to the animator's.
//!
//! The capture `house06-pleasants-ticks-20261115` shows this at entry 0:
//! persons 117 and 240, saved in states 4 and 9, are in state 3, with
//! their destinations at their positions; persons 241 to 243 are in state
//! 0 with Animate Sim's clip playing, the foreground layer's clocks both
//! at the load's clock and its event times at its phase.

use crate::fault::Trap;
use crate::interpreter::Interpreter;
use sims_core::{Ext, F32};
use sims_sim::Object;
use sims_sim::animator::{Advance, Animator, Base, HeldPose, LOADED, Layer};

/// The descriptors' scale (0x666a24): 0.001.
const MILLI: F32 = F32::from_bits(0x3a83_126f);

/// What the load leaves of one person's animator: the animator itself
/// when it differs from [`LOADED`], the pose its skin holds, and the
/// layer in its base slot ([`sims_sim::World::base_layers`]).
#[derive(Debug, Default)]
pub(crate) struct Restored {
    pub animator: Option<Animator>,
    pub held: Option<HeldPose>,
    pub base: Option<Layer>,
    /// The bones whose translation the restore's skin update could write
    /// (0x5992b3, 0x598226), for [`sims_sim::World::translated_bones`]:
    /// for a person the load does not reset. A reset person's are noted
    /// when its held pose ends.
    pub moved: Vec<String>,
}

/// One layer descriptor as the restore reads it (0x591eec; the save
/// writes it with "%s;%d;%d;%d;%d;%d;%d;%d", 0x5864ff).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Descriptor {
    /// The clip's name, up to the first ';'.
    pub name: String,
    pub priority: i32,
    /// The speed, phase and weight, each read in thousandths.
    pub speed: F32,
    pub phase: F32,
    pub weight: F32,
    /// The end mode (+0x40).
    pub mode: i32,
}

/// The C runtime's `atoi` (0x636029): leading white space, a sign, then
/// decimal digits up to the first other character. It accumulates in 32
/// bits with no overflow check, so a longer number wraps.
fn atoi(text: &[u8]) -> i32 {
    let mut at = 0;
    while at < text.len() && matches!(text[at], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        at += 1;
    }
    let negative = text.get(at) == Some(&b'-');
    if matches!(text.get(at), Some(b'-' | b'+')) {
        at += 1;
    }
    let mut total: i32 = 0;
    while let Some(&digit) = text.get(at).filter(|d| d.is_ascii_digit()) {
        total = total.wrapping_mul(10).wrapping_add(i32::from(digit - b'0'));
        at += 1;
    }
    if negative {
        total.wrapping_neg()
    } else {
        total
    }
}

/// Thousandths read as a float (`fild`, `fmul` by [`MILLI`], stored).
fn thousandths(value: i32) -> F32 {
    (Ext::from_i32(value) * MILLI.ext()).single().to_f32()
}

/// Reads a layer descriptor as 0x591eec does: the name up to the first
/// ';', then seven numbers, each after the next ';'. An empty descriptor,
/// or one that runs out of ';' before the seventh number, makes no layer.
pub(crate) fn descriptor(text: &[u8]) -> Option<Descriptor> {
    let name_end = text.iter().position(|&b| b == b';')?;
    let name = String::from_utf8_lossy(&text[..name_end]).into_owned();
    let mut numbers = [0i32; 7];
    let mut at = name_end;
    for (i, number) in numbers.iter_mut().enumerate() {
        if i > 0 {
            // 0x636520: the next ';' after the field just read.
            at += 1 + text[at + 1..].iter().position(|&b| b == b';')?;
        }
        *number = atoi(&text[at + 1..]);
    }
    let [priority, speed, phase, weight, mode, _, _] = numbers;
    Some(Descriptor {
        name,
        priority,
        speed: thousandths(speed),
        phase: thousandths(phase),
        weight: thousandths(weight),
        mode,
    })
}

impl Interpreter<'_> {
    /// Person `id`'s animator and held pose as the load leaves them
    /// (the module documentation). `object` is the person as read; the
    /// placement takes its position and its data word 1, the direction.
    /// `reset` says whether load step 13 resets the person: it does when
    /// its type's scripts changed since the save
    /// ([`sims_sim::World::changed_types`], `crate::visits`).
    ///
    /// A person the load does not reset keeps a live animator
    /// ([`Interpreter::restore_live`]).
    pub(crate) fn restore_animator(
        &self,
        id: u16,
        object: &Object,
        ticks: u32,
        reset: bool,
    ) -> Result<Restored, Trap> {
        let Some(person) = object.person.as_ref() else {
            return Ok(Restored::default());
        };
        let floats = person.floats;
        if floats[6..].iter().any(|&v| v != 0.0) {
            return Err(Trap::Unsupported(format!(
                "person {id}'s saved animator has a placement compensation of {:?} (+0x8c, \
                 0x5918ba): the placement adds it (0x592132), which is not ported",
                &floats[6..]
            )));
        }
        if !reset {
            return self.restore_live(id, object, person, ticks);
        }
        // The layers, each read (0x591eec) and inserted into the skin
        // before the first layer of a higher priority (0x599128): by
        // priority, and in the order read for the same priority.
        let mut layers: Vec<Layer> = Vec::new();
        for text in &person.animations {
            let Some(d) = descriptor(text.as_bytes()) else {
                continue;
            };
            // Virtual function 0xf0 (0x592613): the clip by name in the
            // global table (0x5a03e4). The original makes no layer for a
            // name it does not hold; that our content holds no clip
            // the original does is not ruled out, so it fails.
            let clip = self.program.animations.clip(&d.name).map_err(|e| {
                Trap::Unsupported(format!(
                    "person {id}'s saved layer names a clip {:?} that is not found (0x592613, \
                     0x5a03e4): {e}",
                    d.name
                ))
            })?;
            // 0x5a0a5b, 0x597874: made at the load's clock, which is
            // 0 here (LOADED); then the restore sets the phase (+0x28) and
            // the end mode (+0x40).
            let mut layer = Layer::make(
                &d.name,
                F32::from_bits(clip.duration),
                clip.frames,
                clip.motions > 0,
                clip.tracks.len(),
                d.priority,
                d.speed,
                d.weight,
                LOADED.clock,
            );
            layer.phase = d.phase;
            layer.mode = d.mode;
            let at = layers
                .iter()
                .position(|l| l.priority > layer.priority)
                .unwrap_or(layers.len());
            layers.insert(at, layer);
        }
        let held = (!layers.is_empty()).then(|| {
            let mut placed = LOADED;
            let direction = object.data.get(1).copied().unwrap_or(0);
            crate::animator::place(&mut placed, object.position, direction);
            HeldPose {
                layers,
                position: placed.position,
            }
        });

        let mut a = LOADED;
        a.seen = person.animation_events;
        restore_head(&mut a, &floats, ticks);
        Ok(Restored {
            animator: (a != LOADED).then_some(a),
            held,
            base: None,
            moved: Vec::new(),
        })
    }

    /// The animator of person `id`, which the load does not reset (the
    /// module documentation, "A person the load does not reset").
    ///
    /// Fails for what is not ported: a timed event the restore's skin
    /// update reaches, and a base clip with timed events, which the base
    /// slot's layers are kept from having ([`Base`]).
    fn restore_live(
        &self,
        id: u16,
        object: &Object,
        person: &sims_formats::saved_objects::Person,
        ticks: u32,
    ) -> Result<Restored, Trap> {
        let clock = LOADED.clock_at(ticks as i32);
        // The foreground (+0xc), carry (+0x13c) and base (+0x14) slots,
        // in the order read.
        let mut slots: [Option<Layer>; 3] = Default::default();
        for (slot, text) in slots.iter_mut().zip(&person.animations) {
            let Some(d) = descriptor(text.as_bytes()) else {
                continue;
            };
            let clip = self.program.animations.clip(&d.name).map_err(|e| {
                Trap::Unsupported(format!(
                    "person {id}'s saved layer names a clip {:?} that is not found (0x592613, \
                     0x5a03e4): {e}",
                    d.name
                ))
            })?;
            let mut layer = Layer::make(
                &d.name,
                F32::from_bits(clip.duration),
                clip.frames,
                clip.motions > 0,
                clip.tracks.len(),
                d.priority,
                d.speed,
                d.weight,
                clock,
            );
            layer.phase = d.phase;
            layer.mode = d.mode;
            *slot = Some(layer);
        }
        let [foreground, carry, base] = slots;
        if let Some(base) = &base
            && !base.cursors.is_empty()
        {
            return Err(Trap::Unsupported(format!(
                "person {id}'s saved base layer plays {:?}, whose clip has timed events, at the \
                 load's skin update (0x5992b3): the base slot's layer is not ported to handle \
                 them",
                base.clip
            )));
        }

        let mut a = LOADED;
        // The reset sets +0x1a0 (0x589220); the constructor clears it.
        a.promote = false;
        a.clock = clock;
        a.seen = person.animation_events;
        a.playing = person.engaged != 0;
        a.state = if (0..=3).contains(&person.routing_state) {
            person.routing_state
        } else {
            3
        };
        a.foreground = foreground;
        a.carry = carry;
        if base.is_some() {
            a.base = Base::Loaded;
        }
        let mut base = base;
        // 0x5992b3, forced: the layers by priority, the carry before a
        // foreground of higher priority (the order read decides a tie:
        // the foreground was read first).
        let carry_first = crate::animator::carry_first(&a);
        let mut order: Vec<&mut Layer> = Vec::new();
        order.extend(base.as_mut());
        let (first, second) = if carry_first {
            (a.carry.as_mut(), a.foreground.as_mut())
        } else {
            (a.foreground.as_mut(), a.carry.as_mut())
        };
        order.extend(first);
        order.extend(second);
        for layer in order {
            self.update_restored(id, layer, clock)?;
        }
        // The forced update poses the skin with every layer (0x5992b3), so
        // the bones they move count as moved since the skin was made, as
        // at any skin update.
        let moved = self.translated_by(
            id,
            [base.as_ref(), a.foreground.as_ref(), a.carry.as_ref()]
                .into_iter()
                .flatten(),
        )?;
        a.skin_clock = clock;
        a.skin_dirty = false;
        let direction = object.data.get(1).copied().unwrap_or(0);
        crate::animator::place(&mut a, object.position, direction);
        restore_head(&mut a, &person.floats, ticks);
        a.head_clock = a.clock;
        Ok(Restored {
            animator: Some(a),
            held: None,
            base,
            moved,
        })
    }

    /// One layer's update at the restore's skin update (0x597c4c at clock
    /// `now`, the clock it was made at). A timed event it reaches fails:
    /// the animator would handle it (0x5912fd) before the person's data
    /// words are read, which is not ported.
    fn update_restored(&self, id: u16, layer: &mut Layer, now: i32) -> Result<(), Trap> {
        match layer.update(now).map_err(Trap::Unsupported)? {
            Advance::Unbound => return Ok(()),
            Advance::Silent => {}
            Advance::Events => {
                let clip = self
                    .program
                    .animations
                    .clip(&layer.clip)
                    .map_err(|e| Trap::Unsupported(format!("a layer's clip: {e}")))?;
                for (track, motion) in clip.tracks.iter().enumerate() {
                    let times: Vec<(i32, bool)> = motion
                        .events
                        .iter()
                        .map(|e| (e.time, !e.properties.is_empty()))
                        .collect();
                    if !layer
                        .due(track, &times)
                        .map_err(Trap::Unsupported)?
                        .is_empty()
                    {
                        return Err(Trap::Unsupported(format!(
                            "person {id}'s restored layer {:?} reaches a timed event at the \
                             load's skin update (0x5992b3), before the person's data words \
                             are read: not ported",
                            layer.clip
                        )));
                    }
                }
            }
        }
        layer.commit();
        Ok(())
    }
}

/// The head's saved fields, and the step the restore then takes
/// (0x590768): the constructor zeroed the head clock (+0x164, 0x587d80)
/// and set the animator's clock (+0xb4) to the clock at the load's tick
/// count (0x585483, from 0x587f5a). The step runs when the two differ
/// (0x590788): word 45 is 0, so nothing counts down, and bit 0 of word 42
/// is clear, so the turn relaxes. At tick 0 they are the same and it only
/// settles.
fn restore_head(a: &mut Animator, floats: &[f32; 9], ticks: u32) {
    a.head.heading = F32::from_f32(floats[0]);
    a.head.target_heading = F32::from_f32(floats[1]);
    a.head.turn = F32::from_f32(floats[2]);
    a.head.target = Some([3, 4, 5].map(|i| F32::from_f32(floats[i])));
    if LOADED.clock_at(ticks as i32) != LOADED.head_clock {
        a.head.relax();
    }
    a.head.settle();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoi_reads_as_the_c_runtime_does() {
        assert_eq!(atoi(b"1000;"), 1000);
        assert_eq!(atoi(b"  -10;"), -10);
        assert_eq!(atoi(b"+7x"), 7);
        assert_eq!(atoi(b";1"), 0);
        assert_eq!(atoi(b""), 0);
        assert_eq!(atoi(b"4294967297"), 1);
    }

    #[test]
    fn a_descriptor_reads_seven_numbers_after_the_name() {
        let d = descriptor(b"c2o-standing-loop;-10;1000;228;1000;1;1;1").unwrap();
        assert_eq!(d.name, "c2o-standing-loop");
        assert_eq!((d.priority, d.mode), (-10, 1));
        assert_eq!(d.speed, F32::from_bits(0x3f80_0000));
        assert_eq!(d.weight, F32::from_bits(0x3f80_0000));
        // 228 * 0.001f, rounded to single.
        assert!((d.phase.to_f32() - 0.228).abs() < 1e-6);
        // The last number needs no ';' after it; the sixth does.
        assert!(descriptor(b"a;1;1000;0;1000;0;1;").is_some());
        assert!(descriptor(b"a;1;1000;0;1000;0;1").is_none());
        assert!(descriptor(b"a;1;1000;0;1000;0").is_none());
        assert!(descriptor(b"").is_none());
        assert!(descriptor(b"no separator").is_none());
    }
}
