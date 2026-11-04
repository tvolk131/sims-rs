//! Native layer construction and saved descriptors (5978ef, 597a88, 591eec).
//! Resources/bone-name interning are explicit inputs. pose::update supplies the
//! resumable event/pose pass; animator scheduling and property effects are separate.
use super::{clock::Clock, sampling};
use serde::{Deserialize, Serialize};
use sims_formats::{
    animation::{Animation, Samples},
    avatar::Skeleton,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub bone: usize,
    pub motion: usize,
    /// Per-motion native cursor starts at zero even for a nonzero saved phase.
    pub event_cursor: Option<i32>,
}
#[derive(Debug, Clone, Copy)]
pub struct MotionInput {
    pub bone_id: u32,
    pub frames: u32,
    pub events: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub original_duration: f32,
    pub clock: Clock,
    pub frames: u32,
    pub bindings: Vec<Binding>,
    pub priority: i32,
    pub priority_override: bool,
    /// Native +0x5c, copied from animator configuration; no inferred meaning.
    pub sampling_config: u32,
    pub frame_skip: u32,
    pub remove_when_faded: bool,
    pub interruptible: i32,
    pub translation_policy: i32,
    pub rotation_policy: i32,
}
#[derive(Debug, Clone, Copy)]
pub struct Start {
    pub priority: i32,
    pub speed: f32,
    pub weight: f32,
    pub now: i32,
}
impl Layer {
    /// The bone-slot table represents native global bone IDs. Missing/out-of-range
    /// IDs are skipped; duplicate motions and their source order are preserved.
    pub fn new(
        name: String,
        original_duration: f32,
        motions: &[MotionInput],
        bone_slots: &[Option<usize>],
        start: Start,
    ) -> Result<Self, String> {
        if !original_duration.is_finite() || original_duration < 0. || !start.weight.is_finite() {
            return Err("invalid native layer input".into());
        }
        let mut layer = Self {
            name,
            original_duration,
            clock: Clock {
                current: start.now,
                previous: start.now,
                duration: 0.,
                speed: 1.,
                phase: 0.,
                weight: start.weight,
                event_time: 0,
                previous_event_time: 0,
                ended: false,
                mode: 1,
                fading: false,
                fade_start: 0,
                fade_duration: 500,
                fade_from: 1.,
                fade_to: 0.,
            },
            // This is the first asset motion, even if that motion cannot bind.
            frames: motions.first().map_or(0, |m| m.frames),
            bindings: motions
                .iter()
                .enumerate()
                .filter_map(|(motion, m)| {
                    bone_slots
                        .get(m.bone_id as usize)
                        .copied()
                        .flatten()
                        .map(|bone| Binding {
                            bone,
                            motion,
                            event_cursor: (m.events != 0).then_some(0),
                        })
                })
                .collect(),
            priority: start.priority,
            priority_override: false,
            sampling_config: 1,
            frame_skip: 0,
            remove_when_faded: false,
            interruptible: -1,
            translation_policy: 1,
            rotation_policy: 1,
        };
        layer.set_speed(start.speed)?;
        Ok(layer)
    }
    /// Resource adapter: exact bone labels stand in for a shared native name ID.
    /// No assumption is made that skeleton array indices equal native bone IDs.
    pub fn bind(skeleton: &Skeleton, clip: &Animation, start: Start) -> Result<Self, String> {
        let slots: Vec<_> = skeleton
            .bones
            .iter()
            .enumerate()
            .map(|(i, _)| Some(i))
            .collect();
        let motions = clip
            .motions
            .iter()
            .map(|m| MotionInput {
                bone_id: skeleton
                    .bones
                    .iter()
                    .position(|b| b.name == m.bone)
                    .map_or(u32::MAX, |i| i as u32),
                frames: m.frames,
                events: m.event_lists.iter().map(Vec::len).sum(),
            })
            .collect::<Vec<_>>();
        Self::new(clip.name.clone(), clip.duration_ms, &motions, &slots, start)
    }
    /// Original 598636: near-zero speed has zero duration, including signed zero.
    /// Preserve a negative-zero quotient, which the native comparison does not flip.
    pub fn set_speed(&mut self, speed: f32) -> Result<(), String> {
        if !speed.is_finite() {
            return Err("invalid native layer speed".into());
        }
        let mut duration = if speed.abs() > 0.000001 {
            self.original_duration / speed
        } else {
            0.
        };
        if duration < 0. {
            duration = -duration;
        }
        if !duration.is_finite() {
            return Err("native layer duration overflow".into());
        }
        self.clock.speed = speed;
        self.clock.duration = duration;
        Ok(())
    }
    pub fn sample_parameters(&self) -> sampling::Layer {
        sampling::Layer {
            weight: self.clock.weight,
            priority: self.priority,
            translation_policy: self.translation_policy,
            rotation_policy: self.rotation_policy,
        }
    }
    /// Original 586d41. A caller treats an absent layer as finished as well.
    pub fn finished(&self) -> bool {
        self.clock.ended || self.clock.speed == 0.
    }

    /// Pose-only evaluation of the already-established phase. This never advances
    /// the clock, consumes events or modifies cursor state. Binding order matters
    /// when several motions target the same bone. Markers come from the rig owner.
    pub fn sample(
        &self,
        clip: &Animation,
        samples: &Samples,
        bones: &mut [sampling::BoneInput],
        elapsed: f32,
    ) -> Result<(), String> {
        if clip.name != self.name {
            return Err("native layer clip mismatch".into());
        }
        if self.frames == 0 || self.bindings.is_empty() {
            return Ok(());
        }
        let selection = sampling::Selection::new(
            self.clock.phase,
            self.frames,
            self.clock.mode,
            elapsed,
            self.clock.duration,
            self.frame_skip,
        )?;
        for binding in &self.bindings {
            let bone = bones
                .get_mut(binding.bone)
                .ok_or("native layer bound bone absent")?;
            let motion = clip
                .motions
                .get(binding.motion)
                .ok_or("native layer bound motion absent")?;
            bone.local =
                sampling::motion(*bone, self.sample_parameters(), selection, motion, samples)?
                    .local;
        }
        Ok(())
    }
}

/// Native 5991fd/597c1d. Equal priorities retain list order. Only layers with
/// the override flag assign per-bone priority; their weight does not gate this.
/// Caller owns the native dirty flag and invokes this when that flag is set.
pub fn prepare(layers: &mut [Layer], bone_priorities: &mut [i32]) -> Result<(), String> {
    if layers
        .iter()
        .flat_map(|l| &l.bindings)
        .any(|b| b.bone >= bone_priorities.len())
    {
        return Err("native layer bound bone absent".into());
    }
    layers.sort_by_key(|l| l.priority);
    bone_priorities.fill(-10000);
    for layer in layers.iter().filter(|l| l.priority_override) {
        for binding in &layer.bindings {
            bone_priorities[binding.bone] = layer.priority;
        }
    }
    Ok(())
}

/// Original saved layer text has eight semicolon-separated fields. It does not
/// save current clock, event cursor, fade state or the layer's native pointers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Descriptor {
    pub name: String,
    pub priority: i32,
    pub speed: f32,
    pub phase: f32,
    pub weight: f32,
    pub mode: i32,
    pub translation_policy: i32,
    pub rotation_policy: i32,
}
impl Descriptor {
    /// Mirrors native parsing, including ASCII atoi prefixes and wrapped i32
    /// decimal accumulation. Empty/incomplete input clears the destination layer.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.split('\0').next()?;
        let fields: Vec<_> = text.splitn(8, ';').collect();
        if fields.len() != 8 {
            return None;
        }
        let scaled = |s| (f64::from(atoi(s)) * f64::from(0.001_f32)) as f32;
        Some(Self {
            name: fields[0].into(),
            priority: atoi(fields[1]),
            speed: scaled(fields[2]),
            phase: scaled(fields[3]),
            weight: scaled(fields[4]),
            mode: atoi(fields[5]),
            translation_policy: atoi(fields[6]),
            rotation_policy: atoi(fields[7]),
        })
    }
    /// Construction boundary of 591eec. Caller must clear its slot before parsing
    /// or resolving the asset, and insert successful layers in native priority order.
    /// Unlike Rust snapshot restoration this resets event cursors and fade history.
    pub fn bind(
        &self,
        skeleton: &Skeleton,
        clip: &Animation,
        now: i32,
        sampling_config: u32,
    ) -> Result<Layer, String> {
        if clip.name != self.name {
            return Err("saved layer resolved to a different clip".into());
        }
        let mut layer = Layer::bind(
            skeleton,
            clip,
            Start {
                priority: self.priority,
                speed: self.speed,
                weight: self.weight,
                now,
            },
        )?;
        layer.clock.phase = self.phase;
        layer.clock.mode = self.mode;
        layer.translation_policy = self.translation_policy;
        layer.rotation_policy = self.rotation_policy;
        layer.sampling_config = sampling_config;
        Ok(layer)
    }
}
pub(super) fn atoi(text: &str) -> i32 {
    let mut bytes = text
        .bytes()
        .skip_while(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 11 | 12))
        .peekable();
    let negative = bytes.peek() == Some(&b'-');
    if matches!(bytes.peek(), Some(b'-' | b'+')) {
        bytes.next();
    }
    let mut value = 0_u32;
    for b in bytes.take_while(u8::is_ascii_digit) {
        value = value.wrapping_mul(10).wrapping_add(u32::from(b - b'0'));
    }
    if negative {
        value.wrapping_neg() as i32
    } else {
        value as i32
    }
}
