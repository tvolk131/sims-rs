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
//! 4. The head: +0x210 the heading, +0x214 the target's heading, +0x218
//!    the turn and +0x21c the target point; then the placement's
//!    compensation (+0x8c, three floats).
//!
//! Then (0x591c2e on): the clock is read (0x434101 with 0), the skin is
//! updated with the layers at once (0x5992b3, forced), the animator is
//! placed on the person's tile (virtual function 0x60, 0x592132), the
//! bones placed (0x5994c3), and the head takes its step (0x590768),
//! measures (0x5904d2) and is restored (0x590b27). The person's data words
//! are read after the animator, so that step sees the constructor's zero
//! words: word 45 is 0 and bit 0 of word 42 is clear, so the turn relaxes
//! and settles, and the measurement does not run. The animator's refresh
//! (virtual function 0x54 with 0, 0x58da12) reads the same clock again
//! and changes nothing the simulation keeps.
//!
//! The reset then takes the three layers out of the skin (virtual function
//! 0x104, 0x592878) but leaves the bones they posed and the skin's model
//! matrix ([`sims_sim::animator::HeldPose`]). It leaves the head's fields.
//!
//! Not kept: the target point (+0x21c), which nothing reads before the
//! measurement writes it; a layer's +0x5c, +0x78 and +0x7c, which the
//! restore sets and nothing ported reads. A compensation other than 0
//! fails, since the placement adds it and that is not ported.

use crate::fault::Trap;
use crate::interpreter::Interpreter;
use sims_core::{Ext, F32};
use sims_sim::Object;
use sims_sim::animator::{Animator, HeldPose, LOADED, Layer};

/// The descriptors' scale (0x666a24): 0.001.
const MILLI: F32 = F32::from_bits(0x3a83_126f);

/// What the load leaves of one person's animator: the animator itself
/// when it differs from [`LOADED`], and the pose its skin holds.
#[derive(Debug, Default)]
pub(crate) struct Restored {
    pub animator: Option<Animator>,
    pub held: Option<HeldPose>,
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
    pub(crate) fn restore_animator(&self, id: u16, object: &Object) -> Result<Restored, Trap> {
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
        a.head.heading = F32::from_f32(floats[0]);
        a.head.target_heading = F32::from_f32(floats[1]);
        a.head.turn = F32::from_f32(floats[2]);
        // 0x590768 with the constructor's zero data words.
        a.head.relax();
        a.head.settle();
        Ok(Restored {
            animator: (a != LOADED).then_some(a),
            held,
        })
    }
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
