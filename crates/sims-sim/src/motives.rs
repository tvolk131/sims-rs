//! A person's motives: how they decay, how the mood is worked out from
//! them, and how an action's motive changes are applied.
//!
//! A person has 16 motives, each a stored float. Motive 3 is the mood, 5
//! energy, 6 comfort, 7 hunger, 8 hygiene, 9 bladder, 11 alertness (below
//! zero the person is asleep), 13 the room, 14 social and 15 fun. The
//! person also keeps the values the last decay left
//! (`previous_motives`).
//!
//! [`Tuning::decay`] is the original's 0x420fb6, which a person's tick
//! runs once every 60 ticks. [`Tuning::mood`] is 0x45d165 with the curve
//! lookup 0x4167de. [`apply_changes`] is the block at 0x45cdb3 of the
//! person's tick (0x45cc8c). All three were read from the original's
//! code, with the old prototype as a guide
//! (legacy/crates/sims-vm/src/motives.rs).
//!
//! The original's FPU runs this arithmetic with single precision: every
//! add, subtract, multiply and divide is rounded to 24 bits. The state of
//! the FPU in a capture of the running game shows that (control word
//! 0x003f at the entry of a lot update), and so does the room arithmetic
//! (`rooms.rs`). `docs/decisions.md` says what was compared with the
//! original bit for bit.

use serde::{Deserialize, Serialize};
use sims_core::{Ext, F32};
use sims_formats::saved_objects::Person;
use std::cmp::Ordering;

/// Motive numbers.
pub const MOOD: usize = 3;
const ENERGY: usize = 5;
const COMFORT: usize = 6;
const HUNGER: usize = 7;
const HYGIENE: usize = 8;
const BLADDER: usize = 9;
const ALERTNESS: usize = 11;
/// The motive a person's tick sets from the score of the room it is in.
pub const ROOM: usize = 13;
const SOCIAL: usize = 14;
const FUN: usize = 15;

/// Person data words the decay reads.
const NEAT: usize = 3;
const OUTGOING: usize = 6;
/// The person's age: 0 or 18 and up is an adult.
const AGE: usize = 58;
/// The kind of person: bit 1 and bit 2 pick the second and third set of
/// rates, and a value above 1 changes how the mood is summed.
const KIND: usize = 65;

/// The decay rates of one kind of person (0x420c36). The original reads
/// them by name from a tuning table of the global file: table 1 for
/// ordinary people, and tables 5 and 6 for the two other kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rates {
    pub bladder_asleep: F32,
    pub bladder_awake: F32,
    pub hygiene_asleep: F32,
    pub hygiene_awake: F32,
    /// Asleep, fun is multiplied by this. Awake, the next is subtracted.
    pub fun_asleep: F32,
    pub fun_awake: F32,
    pub social_multiplier: F32,
    pub social_base: F32,
    pub hunger_ratio: F32,
    pub comfort_slope: F32,
    pub comfort_base: F32,
    pub hunger_to_bladder: F32,
    pub energy_drift: F32,
    /// Asleep between these hours, energy also loses the drift.
    pub wake_hour: i32,
    pub sleep_hour: i32,
    pub energy_asleep: F32,
    pub energy_awake: F32,
}

impl Rates {
    /// Reads the rates of kind `profile` (0, 1 or 2). `lookup` gives the
    /// table's value for a name, or None for a name the table lacks; the
    /// original has a default for each.
    ///
    /// Kind 0 loses the energy span over its waking hours and gains it
    /// back over the rest of the day. The two other kinds lose the span
    /// in one hour's worth of decays and gain the waking hours.
    ///
    /// These are worked out when the game starts. Which precision the
    /// FPU has then is not known; the 64-bit register is used here, and
    /// the results equal the tables in a memory image of the running
    /// game for the Complete Collection's values. A waking-hours value of
    /// 0 or 24 divides by zero in the original and gives an infinite
    /// rate here too.
    pub fn from_lookup(profile: usize, lookup: impl Fn(&str) -> Option<f32>) -> Self {
        let get = |name: &str, fallback: f32| F32::from_f32(lookup(name).unwrap_or(fallback));
        let span = get("energy span", 180.0).ext();
        let hours = get("wake hours", 16.0).ext().to_i32_truncate();
        let wake_hour = get("wake hour", 7.0).ext().to_i32_truncate();
        let energy_drift = get("energy drift", 0.01);
        let hunger_to_bladder = get("hunger to bladder multiplier", 0.25);
        let active = get("comfort decrement active", 0.4);
        let lazy = get("comfort decrement lazy", 0.6);
        let hunger_ratio = get("hunger decrement ratio", f32::from_bits(0x3abb_3ee7));
        let social_base = get("social decrement base", 0.1);
        let social_multiplier = get("social decrement multiplier", 0.0004);
        let fun_awake = get("ent decrement, awake", 0.25);
        let fun_asleep = get("ent multiplier, asleep", 0.5);
        let hygiene_awake = get("hyg decrement, awake", 0.15);
        let hygiene_asleep = get("hyg decrement, asleep", 0.1);
        let bladder_awake = get("bladder decrement, awake", 0.3);
        let bladder_asleep = get("bladder decrement, asleep", 0.2);
        let thirty = Ext::from_i32(30);
        let (energy_awake, energy_asleep) = if profile == 0 {
            (
                -(span / Ext::from_i32(hours) / thirty),
                span / Ext::from_i32(24i32.wrapping_sub(hours)) / thirty,
            )
        } else {
            (-(span / thirty), Ext::from_i32(hours) / thirty)
        };
        Self {
            bladder_asleep,
            bladder_awake,
            hygiene_asleep,
            hygiene_awake,
            fun_asleep,
            fun_awake,
            social_multiplier,
            social_base,
            hunger_ratio,
            comfort_slope: ((active.ext() - lazy.ext()) * F32::from_bits(THOUSANDTH).ext())
                .to_f32(),
            comfort_base: lazy,
            hunger_to_bladder,
            energy_drift,
            wake_hour,
            sleep_hour: wake_hour.wrapping_add(hours),
            energy_asleep: energy_asleep.to_f32(),
            energy_awake: energy_awake.to_f32(),
        }
    }
}

/// The float 0.001 as the original holds it (0x666a24).
const THOUSANDTH: u32 = 0x3a83_126f;

/// One curve that turns a motive's level into its weight in the mood
/// (0x461423): points with rising x, and between two points a straight
/// line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Curve {
    /// The motive the curve weighs.
    pub motive: usize,
    /// The points, as x and y, by rising x.
    pub points: Vec<[F32; 2]>,
    /// For each point but the last, one over the distance in x to the
    /// next point, stored as a float as the original stores it.
    pub inverses: Vec<F32>,
}

impl Curve {
    /// Reads a curve from its text: pairs of whole numbers written as
    /// `(x;y)`. A point goes in by its x, a second point with the same x
    /// is dropped, and so is every point beyond `capacity`.
    ///
    /// Ported from the old prototype's `Curve::parse`
    /// (legacy/crates/sims-vm/src/motives.rs), which compared it with
    /// the original's 0x461423 and 0x46137b run alone. **Unverified**
    /// against the running game for text that is not well formed.
    pub fn parse(motive: usize, text: &str, capacity: usize) -> Self {
        let mut points: Vec<[F32; 2]> = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('(') {
            rest = &rest[start + 1..];
            let Some(end) = rest.find(')') else { break };
            if let Some((x, y)) = rest[..end].split_once(';')
                && let (Ok(x), Ok(y)) = (x.trim().parse::<i32>(), y.trim().parse::<i32>())
            {
                let x = Ext::from_i32(x).to_f32();
                if points.len() < capacity && !points.iter().any(|p| p[0] == x) {
                    let place = points.partition_point(|p| below(p[0].ext(), x.ext()));
                    points.insert(place, [x, Ext::from_i32(y).to_f32()]);
                }
            }
            rest = &rest[end + 1..];
        }
        let inverses = points
            .windows(2)
            .map(|pair| (Ext::from_i32(1) / (pair[1][0].ext() - pair[0][0].ext())).to_f32())
            .collect();
        Self {
            motive,
            points,
            inverses,
        }
    }

    /// The curve's value at `x` (0x4167de), left in an FPU register.
    ///
    /// The original looks from the last point down for the first one
    /// that `x` lies above. At or below the first point the value is the
    /// first point's, above the last point the last point's. Between two
    /// points it is the distance from the lower one, times the stored
    /// inverse, times the rise, plus the lower point's value, each step
    /// rounded to single precision.
    pub fn weight(&self, x: F32) -> Ext {
        let Some(last) = self.points.len().checked_sub(1) else {
            return Ext::ZERO;
        };
        let mut found = None;
        for (i, point) in self.points.iter().enumerate().rev() {
            let distance = sub(x.ext(), point[0].ext()).to_f32();
            if above(distance.ext(), Ext::ZERO) {
                found = Some((i, distance));
                break;
            }
        }
        match found {
            None => self.points[0][1].ext(),
            Some((i, _)) if i == last => self.points[i][1].ext(),
            Some((i, distance)) => {
                let (low, high) = (self.points[i][1].ext(), self.points[i + 1][1].ext());
                let along = mul(distance.ext(), self.inverses[i].ext());
                add(mul(along, sub(high, low)), low)
            }
        }
    }
}

/// The constants a person's motives are updated with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tuning {
    /// The rates of the three kinds of person.
    pub rates: [Rates; 3],
    /// The curves that weigh an adult's motives in its mood, and a
    /// child's (the original's tables at 0x6e51f8 and 0x6e5160).
    pub adult_mood: Vec<Curve>,
    pub child_mood: Vec<Curve>,
}

/// The motives the mood curves weigh, in the order of the curves' string
/// table. From the old prototype (`Tuning::from_program` in
/// legacy/crates/sims-vm/src/motives.rs).
pub const MOOD_MOTIVES: [usize; 7] = [7, 6, 8, 9, 15, 13, 14];

/// How many points a mood curve holds at most.
pub const MOOD_POINTS: usize = 8;

impl Tuning {
    /// One decay of a person's motives (0x420fb6), at the lot's `hour`
    /// (global 0):
    ///
    /// 1. Energy gains the asleep rate or loses the awake one. Asleep
    ///    between the wake hour and the sleep hour it also loses the
    ///    drift.
    /// 2. If hunger rose since the last decay, bladder loses a share of
    ///    the rise.
    /// 3. Comfort loses a rate that depends on person data word 3.
    /// 4. Hunger loses a share of its distance from -100.
    /// 5. Awake, social loses a rate that depends on person data word 6.
    /// 6. Fun, hygiene and bladder lose their awake or asleep rates;
    ///    asleep, fun is multiplied instead.
    /// 7. Every motive is kept within -100 and 100 and copied to the
    ///    previous values.
    /// 8. The mood is worked out ([`Tuning::mood`]). The previous values
    ///    keep the mood from before.
    pub fn decay(&self, person: &mut Person, hour: i16) {
        let word = |index: usize| i32::from(person.data.get(index).copied().unwrap_or(0));
        let kind = word(KIND);
        let rates = &self.rates[if kind & 2 != 0 {
            1
        } else if kind & 4 != 0 {
            2
        } else {
            0
        }];
        let (neat, outgoing) = (word(NEAT), word(OUTGOING));
        let mut m = person.motives.map(F32::from_f32);
        let previous_hunger = F32::from_f32(person.previous_motives[HUNGER]);
        let asleep = above(Ext::ZERO, m[ALERTNESS].ext());
        if asleep {
            let energy = add(rates.energy_asleep.ext(), m[ENERGY].ext());
            let hour = i32::from(hour);
            m[ENERGY] = if hour > rates.wake_hour && hour < rates.sleep_hour {
                sub(energy, rates.energy_drift.ext()).to_f32()
            } else {
                energy.to_f32()
            };
        } else {
            m[ENERGY] = add(rates.energy_awake.ext(), m[ENERGY].ext()).to_f32();
        }
        let eaten = sub(m[HUNGER].ext(), previous_hunger.ext());
        if above(eaten, Ext::ZERO) {
            m[BLADDER] = sub(m[BLADDER].ext(), mul(eaten, rates.hunger_to_bladder.ext())).to_f32();
        }
        m[COMFORT] = sub(
            m[COMFORT].ext(),
            add(
                mul(Ext::from_i32(neat), rates.comfort_slope.ext()),
                rates.comfort_base.ext(),
            ),
        )
        .to_f32();
        m[HUNGER] = sub(
            m[HUNGER].ext(),
            mul(
                add(m[HUNGER].ext(), Ext::from_i32(100)),
                rates.hunger_ratio.ext(),
            ),
        )
        .to_f32();
        if !asleep {
            m[SOCIAL] = sub(
                m[SOCIAL].ext(),
                add(
                    mul(Ext::from_i32(outgoing), rates.social_multiplier.ext()),
                    rates.social_base.ext(),
                ),
            )
            .to_f32();
        }
        if asleep {
            m[FUN] = mul(rates.fun_asleep.ext(), m[FUN].ext()).to_f32();
            m[HYGIENE] = sub(m[HYGIENE].ext(), rates.hygiene_asleep.ext()).to_f32();
            m[BLADDER] = sub(m[BLADDER].ext(), rates.bladder_asleep.ext()).to_f32();
        } else {
            m[FUN] = sub(m[FUN].ext(), rates.fun_awake.ext()).to_f32();
            m[HYGIENE] = sub(m[HYGIENE].ext(), rates.hygiene_awake.ext()).to_f32();
            m[BLADDER] = sub(m[BLADDER].ext(), rates.bladder_awake.ext()).to_f32();
        }
        let (least, most) = (Ext::from_i32(-100), Ext::from_i32(100));
        for value in &mut m {
            // The original writes 100 unless 100 is at or above the
            // value, so a value that is not a number becomes 100 too.
            if !at_or_above(most, value.ext()) {
                *value = most.to_f32();
            }
            if !at_or_above(value.ext(), least) {
                *value = least.to_f32();
            }
        }
        person.previous_motives = m.map(F32::to_f32);
        m[MOOD] = self.mood(&m, word(AGE), kind);
        person.motives = m.map(F32::to_f32);
    }

    /// The mood (0x45d165): the motives' values weighted by their curves,
    /// over the sum of the weights. An adult (age 0, or 18 and up) and a
    /// child have their own curves.
    ///
    /// For a person whose kind is above 1 the original leaves out a
    /// motive whose value is exactly 13.0, or not a number. That reads
    /// like a slip for "motive 13", and is ported as it is.
    pub fn mood(&self, motives: &[F32; 16], age: i32, kind: i32) -> F32 {
        let curves = if age == 0 || age >= 18 {
            &self.adult_mood
        } else {
            &self.child_mood
        };
        let (mut total, mut weights) = (F32::ZERO, F32::ZERO);
        for curve in curves {
            let Some(&value) = motives.get(curve.motive) else {
                continue;
            };
            let weight = curve.weight(value);
            let skipped = matches!(
                value.ext().partial_cmp(&Ext::from_i32(13)),
                None | Some(Ordering::Equal)
            );
            if kind > 1 && skipped {
                continue;
            }
            total = add(mul(weight, value.ext()), total.ext()).to_f32();
            weights = add(weight, weights.ext()).to_f32();
        }
        div(total.ext(), weights.ext()).to_f32()
    }
}

/// Applies the motive changes an action left running on a person, once
/// (0x45cdb3 in the person's tick). Each change names a motive, an amount
/// to add each tick and a value to stop at. A change that adds stops at
/// its value from below, one that takes away from above; a motive already
/// at or past the value is left alone. Changes run in the order they
/// were set, and two may name the same motive.
pub fn apply_changes(person: &mut Person) {
    let changes = std::mem::take(&mut person.motive_deltas);
    for change in &changes {
        let Some(motive) = usize::try_from(change.motive)
            .ok()
            .and_then(|index| person.motives.get_mut(index))
        else {
            continue;
        };
        let value = F32::from_f32(*motive).ext();
        let delta = F32::from_f32(change.delta).ext();
        let stop = F32::from_f32(change.stop_at).ext();
        if at_or_above(delta, Ext::ZERO) {
            if above(stop, value) {
                let next = add(delta, value);
                *motive = if above(next, stop) { stop } else { next }
                    .to_f32()
                    .to_f32();
            }
        } else if !at_or_above(stop, value) {
            let next = add(delta, value);
            *motive = if at_or_above(next, stop) { next } else { stop }
                .to_f32()
                .to_f32();
        }
    }
    person.motive_deltas = changes;
}

fn above(a: Ext, b: Ext) -> bool {
    a.partial_cmp(&b) == Some(Ordering::Greater)
}

fn below(a: Ext, b: Ext) -> bool {
    a.partial_cmp(&b) == Some(Ordering::Less)
}

fn at_or_above(a: Ext, b: Ext) -> bool {
    matches!(a.partial_cmp(&b), Some(Ordering::Greater | Ordering::Equal))
}

fn add(a: Ext, b: Ext) -> Ext {
    (a + b).single()
}

fn sub(a: Ext, b: Ext) -> Ext {
    (a - b).single()
}

fn mul(a: Ext, b: Ext) -> Ext {
    (a * b).single()
}

fn div(a: Ext, b: Ext) -> Ext {
    (a / b).single()
}
