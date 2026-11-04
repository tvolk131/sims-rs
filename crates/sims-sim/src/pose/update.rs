//! Resumable native 597c4c layer update: clock, then each bound motion's events
//! and pose channels, then event-time commit. The host owns property callbacks.
use super::{
    events::{self, Entry, Scan},
    layer::Layer,
    sampling::{self, BoneInput, Selection},
};
use serde::{Deserialize, Serialize};
use sims_formats::animation::{Animation, Samples};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dispatch {
    pub binding: usize,
    pub bone: usize,
    pub motion: usize,
    pub event: usize,
    pub phase: f32,
    pub entry: Entry,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Update {
    #[serde(default = "extended_control")]
    fp_control: u16,
    selection: Option<Selection>,
    binding: usize,
    entered: bool,
    scan: Option<Scan>,
    pending: Option<Dispatch>,
    sample_pose: bool,
    callback: bool,
    complete: bool,
    generation_delta: u32,
    bone_generation_deltas: Vec<(usize, u32)>,
}
fn extended_control() -> u16 {
    0x037f
}
impl Update {
    pub fn begin(
        layer: &mut Layer,
        now: i32,
        sample_pose: bool,
        callback: bool,
    ) -> Result<Self, String> {
        Self::begin_with_control(layer, now, sample_pose, callback, 0x037f)
    }
    pub fn begin_with_control(
        layer: &mut Layer,
        now: i32,
        sample_pose: bool,
        callback: bool,
        fp_control: u16,
    ) -> Result<Self, String> {
        let has_frames = layer.frames != 0 && !layer.bindings.is_empty();
        let elapsed = layer.clock.advance_with_control(
            now,
            layer.original_duration,
            has_frames,
            fp_control,
        )?;
        let selection = if has_frames && layer.clock.weight > 0. {
            Some(Selection::new_with_control(
                layer.clock.phase,
                layer.frames,
                layer.clock.mode,
                elapsed,
                layer.clock.duration,
                layer.frame_skip,
                fp_control,
            )?)
        } else {
            None
        };
        let complete = selection.is_none();
        if complete && has_frames {
            layer.clock.finish();
        }
        Ok(Self {
            fp_control,
            selection,
            binding: 0,
            entered: false,
            scan: None,
            pending: None,
            sample_pose,
            callback,
            complete,
            generation_delta: 0,
            bone_generation_deltas: vec![],
        })
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
    /// Repeated calls return the same pending event until the host acknowledges
    /// its callback. Save/restore during missing-asset handling cannot skip it.
    pub fn next(
        &mut self,
        layer: &mut Layer,
        clip: &Animation,
        samples: &Samples,
        bones: &mut [BoneInput],
    ) -> Result<Option<Dispatch>, String> {
        if let Some(pending) = &self.pending {
            return Ok(Some(pending.clone()));
        }
        if self.complete {
            return Ok(None);
        }
        if layer.name != clip.name {
            return Err("native layer update clip mismatch".into());
        }
        while self.binding < layer.bindings.len() {
            let binding = layer.bindings[self.binding].clone();
            let bone = bones
                .get_mut(binding.bone)
                .ok_or("native layer bound bone absent")?;
            let motion = clip
                .motions
                .get(binding.motion)
                .ok_or("native layer bound motion absent")?;
            if !self.entered {
                if bone.priority > layer.priority {
                    self.binding += 1;
                    continue;
                }
                self.entered = true;
                if binding.event_cursor.is_some()
                    && motion.event_lists.iter().any(|list| !list.is_empty())
                {
                    self.scan = Some(Scan::new(
                        layer.clock.speed,
                        layer.clock.event_time,
                        layer.clock.previous_event_time,
                        layer.original_duration,
                    )?);
                }
            }
            if let Some(scan) = &mut self.scan {
                let entries = events::stream(motion)?;
                let cursor = layer.bindings[self.binding]
                    .event_cursor
                    .as_mut()
                    .ok_or("pending native event cursor removed")?;
                if let Some(event) =
                    scan.next(cursor, &entries, layer.clock.event_time, self.callback)
                {
                    let dispatch = Dispatch {
                        binding: self.binding,
                        bone: binding.bone,
                        motion: binding.motion,
                        event,
                        phase: layer.clock.phase,
                        entry: entries[event].clone(),
                    };
                    self.pending = Some(dispatch.clone());
                    return Ok(Some(dispatch));
                }
                self.scan = None;
            }
            if self.sample_pose {
                // Native priority is checked once, before this binding's events.
                let input = BoneInput {
                    priority: layer.priority,
                    ..*bone
                };
                let sampled = sampling::motion_with_control(
                    input,
                    layer.sample_parameters(),
                    self.selection.unwrap(),
                    motion,
                    samples,
                    self.fp_control,
                )?;
                bone.local = sampled.local;
                // 598601/598609: both skeleton stamps increment once for a
                // binding that writes either channel, even if values coincide.
                if sampled.translation_changed || sampled.rotation_changed {
                    self.generation_delta = self.generation_delta.wrapping_add(1);
                    self.bone_generation_deltas.push((
                        binding.bone,
                        u32::from(sampled.translation_changed)
                            + u32::from(sampled.rotation_changed),
                    ));
                }
            }
            self.binding += 1;
            self.entered = false;
        }
        layer.clock.finish();
        self.complete = true;
        Ok(None)
    }
    /// Called only after the property callback has completed. The subsequent
    /// next() performs the original post-callback cursor movement.
    pub fn acknowledge(&mut self) -> Result<(), String> {
        self.pending
            .take()
            .ok_or("no pending native animation event")?;
        Ok(())
    }
    /// The enclosing skin applies these increments before invoking the next
    /// callback, so an in-callback snapshot sees all prior binding writes.
    pub fn take_generation_delta(&mut self) -> u32 {
        std::mem::take(&mut self.generation_delta)
    }
    /// Native translation and rotation writes each increment bone +0x130.
    /// This differs from the once-per-binding skeleton stamp increment.
    pub fn take_bone_generation_deltas(&mut self) -> Vec<(usize, u32)> {
        std::mem::take(&mut self.bone_generation_deltas)
    }
}
