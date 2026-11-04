//! Native need decay and score arithmetic. Stored values are single precision;
//! motive updates retain the observed gameplay or historical probe precision.
use serde::{Deserialize, Serialize};
pub mod score_owner;

/// Precision of the native motive update's arithmetic, distinct from its f32
/// storage. Live Wine gameplay uses x87 PC=00; the historical isolated probes
/// used PC=11. Keep old managed snapshots/probes explicit rather than silently
/// treating their widened intermediates as evidence for gameplay precision.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Precision {
    #[default]
    LegacyExtended,
    Single,
}
impl Precision {
    fn round(self, value: f64) -> f64 {
        match self {
            Self::LegacyExtended => value,
            Self::Single => f64::from(value as f32),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    pub motive: usize,
    pub points: Vec<[f32; 2]>,
}
impl Curve {
    /// 0x461423 / 0x46137b: integer coordinate pairs, sorted insertion, first
    /// value retained at duplicate x, and a fixed native allocation capacity.
    pub fn parse(motive: usize, text: &str, capacity: usize) -> Result<Self, String> {
        let mut points: Vec<[f32; 2]> = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('(') {
            rest = &rest[start + 1..];
            let Some(end) = rest.find(')') else { break };
            if let Some((x, y)) = rest[..end].split_once(';') {
                if let (Ok(x), Ok(y)) = (x.trim().parse::<i32>(), y.trim().parse::<i32>()) {
                    let x = x as f32;
                    if points.len() < capacity && !points.iter().any(|p| p[0] == x) {
                        let i = points.partition_point(|p| p[0] < x);
                        points.insert(i, [x, y as f32]);
                    }
                }
            }
            rest = &rest[end + 1..];
        }
        let curve = Self { motive, points };
        curve.validate()?;
        Ok(curve)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.motive >= 16
            || self.points.len() > 256
            || self.points.iter().flatten().any(|x| !x.is_finite())
            || self.points.windows(2).any(|p| p[0][0] >= p[1][0])
        {
            return Err("invalid native motive curve".into());
        }
        Ok(())
    }
    /// Original 0x4167de returns an extended value; do not round the weight to
    /// f32 before its caller multiplies it. The x distance and reciprocal are
    /// individually stored as f32 in the native implementation.
    pub fn evaluate(&self, x: f32) -> f64 {
        self.evaluate_with_precision(x, Precision::LegacyExtended)
    }
    fn evaluate_with_precision(&self, x: f32, precision: Precision) -> f64 {
        let Some(first) = self.points.first() else {
            return 0.0;
        };
        let Some(i) = self.points.iter().rposition(|p| x - p[0] > 0.0) else {
            return f64::from(first[1]);
        };
        let a = self.points[i];
        let Some(b) = self.points.get(i + 1) else {
            return f64::from(a[1]);
        };
        let distance = x - a[0];
        let inverse = (1.0 / (f64::from(b[0]) - f64::from(a[0]))) as f32;
        //41683c..41684a computes distance*inverse first. Reassociating the
        //three multiplications changes single-precision gameplay results.
        let fraction = precision.round(f64::from(distance) * f64::from(inverse));
        let delta = precision.round(f64::from(b[1]) - f64::from(a[1]));
        precision.round(precision.round(fraction * delta) + f64::from(a[1]))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rates {
    pub bladder_asleep: f32,
    pub bladder_awake: f32,
    pub hygiene_asleep: f32,
    pub hygiene_awake: f32,
    pub fun_asleep: f32,
    pub fun_awake: f32,
    pub social_multiplier: f32,
    pub social_base: f32,
    pub hunger_ratio: f32,
    pub comfort_slope: f32,
    pub comfort_base: f32,
    pub hunger_bladder: f32,
    pub energy_drift: f32,
    pub sleep_hour: i32,
    pub wake_hour: i32,
    pub energy_asleep: f32,
    pub energy_awake: f32,
}

impl Rates {
    /// FCNS 1/5/6 lookup and normalization from 0x420c36. Missing records use
    /// native defaults; the caller supplies only named resource lookup.
    pub fn from_lookup(
        profile: usize,
        lookup: impl Fn(&str) -> Option<f32>,
    ) -> Result<Self, String> {
        if profile > 2 {
            return Err("invalid motive rate profile".into());
        }
        let get = |name, default| lookup(name).unwrap_or(default);
        let span = get("energy span", 180.0);
        let hours = get("wake hours", 16.0) as i32;
        let wake_hour = get("wake hour", 7.0) as i32;
        if profile == 0 && !(1..24).contains(&hours) {
            return Err("unsupported degenerate energy schedule".into());
        }
        let active = get("comfort decrement active", 0.4);
        let lazy = get("comfort decrement lazy", 0.6);
        let rates = Self {
            bladder_asleep: get("bladder decrement, asleep", 0.2),
            bladder_awake: get("bladder decrement, awake", 0.3),
            hygiene_asleep: get("hyg decrement, asleep", 0.1),
            hygiene_awake: get("hyg decrement, awake", 0.15),
            fun_asleep: get("ent multiplier, asleep", 0.5),
            fun_awake: get("ent decrement, awake", 0.25),
            social_multiplier: get("social decrement multiplier", 0.0004),
            social_base: get("social decrement base", 0.1),
            hunger_ratio: get("hunger decrement ratio", f32::from_bits(0x3abb3ee7)),
            comfort_slope: ((f64::from(active) - f64::from(lazy)) * f64::from(0.001f32)) as f32,
            comfort_base: lazy,
            hunger_bladder: get("hunger to bladder multiplier", 0.25),
            energy_drift: get("energy drift", 0.01),
            sleep_hour: wake_hour.wrapping_add(hours),
            wake_hour,
            energy_asleep: if profile == 0 {
                (f64::from(span) / f64::from(24 - hours) / 30.0) as f32
            } else {
                hours as f32 / 30.0
            },
            energy_awake: if profile == 0 {
                -(f64::from(span) / f64::from(hours) / 30.0) as f32
            } else {
                -(f64::from(span) / 30.0) as f32
            },
        };
        if serde_json::to_value(&rates)
            .map_err(|e| e.to_string())?
            .as_object()
            .unwrap()
            .values()
            .any(|v| v.is_null())
        {
            return Err("nonfinite motive rate".into());
        }
        Ok(rates)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tuning {
    #[serde(default)]
    pub update_precision: Precision,
    pub rates: [Rates; 3],
    pub adult_mood: Vec<Curve>,
    pub child_mood: Vec<Curve>,
    pub adult_scores: Vec<Curve>,
    pub child_scores: Vec<Curve>,
}
impl Tuning {
    /// Validate the current coefficient/curve caches without reconstructing
    /// their initialization inputs from FCNS/STR resources. Native globals
    /// and personal caches are live state, not resource-identity bindings.
    pub fn validate_cached(&self) -> Result<(), String> {
        for r in &self.rates {
            if [
                r.bladder_asleep,
                r.bladder_awake,
                r.hygiene_asleep,
                r.hygiene_awake,
                r.fun_asleep,
                r.fun_awake,
                r.social_multiplier,
                r.social_base,
                r.hunger_ratio,
                r.comfort_slope,
                r.comfort_base,
                r.hunger_bladder,
                r.energy_drift,
                r.energy_asleep,
                r.energy_awake,
            ]
            .iter()
            .any(|v| !v.is_finite())
            {
                return Err("nonfinite retained motive coefficient".into());
            }
        }
        for curves in [
            &self.adult_mood,
            &self.child_mood,
            &self.adult_scores,
            &self.child_scores,
        ] {
            for curve in curves {
                curve.validate()?;
            }
        }
        Ok(())
    }
    pub fn from_program(program: &crate::Program) -> Result<Self, String> {
        let global = program
            .modules
            .get(&program.globals)
            .ok_or("missing global tuning module")?;
        let mut rates = Vec::new();
        for (profile, id) in [1, 5, 6].into_iter().enumerate() {
            let constants = global
                .named_constants
                .get(&id)
                .ok_or_else(|| format!("missing motive FCNS {id}"))?;
            rates.push(Rates::from_lookup(profile, |name| constants.get(name))?);
        }
        let mood_indices = [7, 6, 8, 9, 15, 13, 14];
        let score_indices = [5, 6, 7, 8, 9, 3, 13, 14, 15];
        let load = |id, indices: &[usize], capacity| -> Result<Vec<Curve>, String> {
            let strings = global
                .strings
                .get(&id)
                .ok_or_else(|| format!("missing motive curve STR# {id}"))?;
            indices
                .iter()
                .enumerate()
                .map(|(i, &motive)| {
                    let text = strings.get(i).ok_or("missing motive curve entry")?;
                    Curve::parse(motive, text, capacity)
                })
                .collect()
        };
        Ok(Self {
            update_precision: Precision::Single,
            rates: rates.try_into().unwrap(),
            adult_mood: load(502, &mood_indices, 8)?,
            child_mood: load(504, &mood_indices, 8)?,
            adult_scores: load(501, &score_indices, 4)?,
            child_scores: load(503, &score_indices, 4)?,
        })
    }
    pub fn mood_curves(&self, age: i16) -> &[Curve] {
        if age == 0 || age > 17 {
            &self.adult_mood
        } else {
            &self.child_mood
        }
    }
}

/// State owned by the native motive wrapper, outside the 16-word motive array.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateState {
    pub previous: [u32; 16],
    pub last_decay: i32,
}
impl UpdateState {
    /// 0x420f6a + person constructor 0x459327. Previous hunger starts at zero
    /// while current hunger starts at -40; copying current motives loses this.
    pub fn constructed() -> ([u32; 16], Self) {
        let mut previous = [0; 16];
        previous[5] = 70f32.to_bits();
        let mut values = previous;
        values[7] = (-40f32).to_bits();
        (
            values,
            Self {
                previous,
                last_decay: 0,
            },
        )
    }

    /// Native 0x45cc8c after VM completion. Room membership and environment are
    /// supplied by the host; the native room -5 path preserves the old value.
    pub fn finish(
        &mut self,
        values: &mut [u32; 16],
        person: &mut [i16; 256],
        hidden: bool,
        tick: i32,
        hour: i16,
        tuning: &Tuning,
        environment: Option<f32>,
        changes: &[Change],
    ) {
        if tick >= self.last_decay.wrapping_add(60) {
            self.last_decay = tick;
            self.periodic(values, person, hidden, hour, tuning);
        }
        if let Some(environment) = environment {
            values[13] = environment.to_bits();
        }
        person[27] = person[27].wrapping_add(1);
        apply_changes(values, changes);
    }

    /// Native 0x45d145, called after its owner writes the periodic-decay clock.
    /// Keeping this separate from the per-tick tail lets the ordinary animator
    /// run between decay and environment/counter/script-change application.
    pub fn periodic(
        &mut self,
        values: &mut [u32; 16],
        person: &[i16; 256],
        hidden: bool,
        hour: i16,
        tuning: &Tuning,
    ) {
        if person[29] == 0 && !hidden {
            decay_with_precision(
                values,
                &mut self.previous,
                person,
                hour,
                &tuning.rates,
                tuning.mood_curves(person[58]),
                tuning.update_precision,
            );
        }
    }
}

/// 0x420fb6. Previous values are copied before Mood is recomputed.
pub fn decay(
    values: &mut [u32; 16],
    previous: &mut [u32; 16],
    person: &[i16; 256],
    hour: i16,
    rates: &[Rates; 3],
    mood_curves: &[Curve],
) {
    decay_with_precision(
        values,
        previous,
        person,
        hour,
        rates,
        mood_curves,
        Precision::LegacyExtended,
    );
}

pub fn decay_with_precision(
    values: &mut [u32; 16],
    previous: &mut [u32; 16],
    person: &[i16; 256],
    hour: i16,
    rates: &[Rates; 3],
    mood_curves: &[Curve],
    precision: Precision,
) {
    let round = |v| precision.round(v);
    let profile = if person[65] & 2 != 0 {
        1
    } else if person[65] & 4 != 0 {
        2
    } else {
        0
    };
    let r = &rates[profile];
    let mut v = values.map(f32::from_bits);
    let asleep = v[11] < 0.0;
    let mut energy = round(
        f64::from(v[5])
            + f64::from(if asleep {
                r.energy_asleep
            } else {
                r.energy_awake
            }),
    );
    if asleep && i32::from(hour) > r.wake_hour && i32::from(hour) < r.sleep_hour {
        energy = round(energy - f64::from(r.energy_drift));
    }
    v[5] = energy as f32;
    let eaten = round(f64::from(v[7]) - f64::from(f32::from_bits(previous[7])));
    if eaten > 0.0 {
        v[9] = (f64::from(v[9]) - round(eaten * f64::from(r.hunger_bladder))) as f32;
    }
    v[6] = (f64::from(v[6])
        - round(
            round(f64::from(person[3]) * f64::from(r.comfort_slope)) + f64::from(r.comfort_base),
        )) as f32;
    v[7] = (f64::from(v[7]) - round(round(f64::from(v[7]) + 100.0) * f64::from(r.hunger_ratio)))
        as f32;
    if !asleep {
        v[14] = (f64::from(v[14])
            - round(
                round(f64::from(person[6]) * f64::from(r.social_multiplier))
                    + f64::from(r.social_base),
            )) as f32;
    }
    v[15] = if asleep {
        (f64::from(v[15]) * f64::from(r.fun_asleep)) as f32
    } else {
        v[15] - r.fun_awake
    };
    v[8] -= if asleep {
        r.hygiene_asleep
    } else {
        r.hygiene_awake
    };
    v[9] -= if asleep {
        r.bladder_asleep
    } else {
        r.bladder_awake
    };
    for value in &mut v {
        *value = value.clamp(-100.0, 100.0);
    }
    *previous = v.map(f32::to_bits);
    v[3] = mood_with_precision(&v, person[65], mood_curves, precision);
    *values = v.map(f32::to_bits);
}

pub fn mood(values: &[f32; 16], species: i16, curves: &[Curve]) -> f32 {
    mood_with_precision(values, species, curves, Precision::LegacyExtended)
}
pub fn mood_with_precision(
    values: &[f32; 16],
    species: i16,
    curves: &[Curve],
    precision: Precision,
) -> f32 {
    let mut total = 0f32;
    let mut weights = 0f32;
    for curve in curves {
        let value = values[curve.motive];
        let weight = curve.evaluate_with_precision(value, precision);
        if species < 2 || value != 13.0 {
            total = (f64::from(total) + precision.round(weight * f64::from(value))) as f32;
            weights = (f64::from(weights) + weight) as f32;
        }
    }
    total / weights
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Advertisement {
    pub personality: i16,
    pub minimum: i16,
    pub delta: i16,
}

/// Native 0x42144e / 0x4214a1; selection, distance attenuation and eligibility
/// are separate services. The final result is rounded here for comparison/storage.
pub fn score(
    values: &[u32; 16],
    person: &[i16; 256],
    curves: &[Curve],
    advertisements: Option<&[Advertisement; 16]>,
) -> Result<f32, String> {
    Ok(score_extended(values, person, curves, advertisements)? as f32)
}

/// Prospective score stays in the x87 register through improvement/attenuation.
/// Current score is separately rounded by its caller before candidate scanning.
pub fn score_extended(
    values: &[u32; 16],
    person: &[i16; 256],
    curves: &[Curve],
    advertisements: Option<&[Advertisement; 16]>,
) -> Result<f64, String> {
    if curves.is_empty() {
        return Err("autonomy needs a nonempty score curve table".into());
    }
    let mut sum = 0f32;
    for curve in curves {
        let mut value = f32::from_bits(values[curve.motive]);
        if let Some(ads) = advertisements {
            let ad = &ads[curve.motive];
            let improvement = if ad.personality == 0 {
                f64::from(i32::from(ad.minimum) + i32::from(ad.delta))
            } else {
                // 0x666a68: after the six paired personality modifiers the
                // original table also selects skills and other person fields.
                let (field, inverse) = match ad.personality {
                    1..=12 => (
                        2 + (ad.personality as usize - 1) / 2,
                        ad.personality % 2 == 0,
                    ),
                    13..=16 => (ad.personality as usize - 4, false),
                    17 | 18 | 20 => (55, false),
                    19 => (15, false),
                    21 => (17, false),
                    22 => (18, false),
                    _ => return Err("advertisement modifier outside native table".into()),
                };
                let mut p = i32::from(person[field]);
                if inverse {
                    p = 1000 - p;
                }
                f64::from(ad.delta) * f64::from(p) * f64::from(0.001f32) + f64::from(ad.minimum)
            };
            value = (improvement * f64::from(0.001f32) + f64::from(value)) as f32;
        }
        sum = (f64::from(sum) + curve.evaluate(value)) as f32;
    }
    Ok(f64::from(sum) / curves.len() as f64)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Change {
    pub motive: u8,
    pub delta: u32,
    pub target: u32,
}
impl Change {
    pub fn new(motive: u8, hourly: i16, target: i16) -> Result<Self, String> {
        if !matches!(motive, 5..=9 | 14..=15) {
            return Err("invalid motive delta index (native error 0x1a)".into());
        }
        Ok(Self {
            motive,
            delta: (f32::from(hourly) * f32::from_bits(0x3a11a2b4)).to_bits(),
            target: f32::from(target).to_bits(),
        })
    }
}

/// The ordered 0x45cdac..0x45ce14 person-update block. Multiple entries for the
/// same motive are retained and run in insertion order. No universal clamp.
pub fn apply_changes(values: &mut [u32; 16], changes: &[Change]) {
    for change in changes {
        let value = f32::from_bits(values[usize::from(change.motive)]);
        let delta = f32::from_bits(change.delta);
        let target = f32::from_bits(change.target);
        if (delta >= 0.0 && value < target) || (delta < 0.0 && value > target) {
            let next = f64::from(value) + f64::from(delta);
            let next = if delta >= 0.0 {
                next.min(f64::from(target))
            } else {
                next.max(f64::from(target))
            };
            values[usize::from(change.motive)] = (next as f32).to_bits();
        }
    }
}

impl crate::Vm {
    pub(crate) fn set_motive_changes(
        &mut self,
        program: &crate::Program,
        thread: &mut crate::Thread,
        operands: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let actor = thread.stack.last().ok_or("no motive-change frame")?.caller;
        if self.world.objects[&actor].person.is_none() {
            return Err("motive change needs a person".into());
        }
        if operands[3] & 1 != 0 {
            self.world
                .objects
                .get_mut(&actor)
                .unwrap()
                .motive_changes
                .clear();
            return Ok((true, "cleared native motive changes".into()));
        }
        // Native validates the index before resolving either operand.
        Change::new(operands[2], 0, 0)?;
        let delta = self.read(
            program,
            thread,
            operands[0],
            i16::from_le_bytes([operands[4], operands[5]]),
        )?;
        let target = self.read(
            program,
            thread,
            operands[1],
            i16::from_le_bytes([operands[6], operands[7]]),
        )?;
        let changes = &mut self.world.objects.get_mut(&actor).unwrap().motive_changes;
        if changes.len() >= 4096 {
            return Err("motive-change diagnostic allocation limit".into());
        }
        changes.push(Change::new(operands[2], delta, target)?);
        Ok((
            true,
            format!(
                "motive {} changes {delta}/hour toward {target}",
                operands[2]
            ),
        ))
    }
}
