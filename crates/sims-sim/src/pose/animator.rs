//! Native animator layer ownership (592726/592878) and skin update (5992b3).
//! Resource decoding, animator placement/head state and gameplay callbacks are
//! host services. Configuration is explicit, not inferred from capture defaults.
use super::{
    layer::{Layer, Start},
    sampling::BoneInput,
    update::{Dispatch, Update},
};
use serde::{Deserialize, Serialize};
use sims_formats::{animation::Animation, avatar::Skeleton};
use sims_vm::Program;
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Slot {
    Foreground,
    Carry,
    Base,
}
impl Slot {
    fn index(self) -> usize {
        match self {
            Self::Foreground => 0,
            Self::Carry => 1,
            Self::Base => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub fade_enabled: bool,
    pub fade_duration: i32,
    pub sampling_config: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnedLayer {
    pub id: LayerId,
    pub layer: Layer,
    pub anchor_bone: Option<usize>,
    /// Native layer +0x70, cleared by the anchor property; meaning unresolved.
    pub anchor_state: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Pass {
    now: i32,
    sample_pose: bool,
    callback: bool,
    index: usize,
    /// 5992b3 caches the count but reads the vector storage anew at each index.
    /// Erase shifts entries without clearing the stale tail. Stable identities
    /// retain that defined traversal without dereferencing released storage.
    order: Vec<LayerId>,
    active: Option<LayerId>,
    update: Option<Update>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Animator {
    /// Source-qualified process arithmetic control. None retains the historic
    /// isolated037f probe profile; it is not captured whole-game evidence.
    #[serde(default)]
    fp_control: Option<u16>,
    layers: Vec<OwnedLayer>,
    slots: [Option<LayerId>; 3],
    blend: Option<LayerId>,
    next_id: u64,
    dirty: bool,
    generation: u32,
    pose_generation: u32,
    last_clock: i32,
    /// Native animator +0x1a0, including the discontinuity callback's write.
    promote_next_foreground: bool,
    /// Native foreground selection cache +0x98, invalidated by 586b63.
    selection_cache: i32,
    pub(super) anchor_valid: bool,
    pub(super) anchor_bone: Option<usize>,
    /// Native +18/+1c/+20; retained when the anchor is invalidated.
    pub(super) anchor_point: [f32; 3],
    numbered_events: VecDeque<i32>,
    pending_bone_updates: BTreeMap<usize, u32>,
    /// Same sampler writes also advance bone+150. The asset host owns those
    /// vertex-cache stamps, so retain their ordered aggregate until its next
    /// synchronous geometry transaction, including across save/resume.
    #[serde(default)]
    pending_vertex_updates: BTreeMap<usize, u32>,
    pass: Option<Pass>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub layer: LayerId,
    pub dispatch: Dispatch,
}

impl Animator {
    /// Inputs are explicit because native skeleton construction/load scheduling
    /// establishes the initial clock, generation and dirty state elsewhere.
    pub fn new(
        last_clock: i32,
        generation: u32,
        pose_generation: u32,
        dirty: bool,
        selection_cache: i32,
    ) -> Self {
        Self {
            fp_control: None,
            layers: vec![],
            slots: [None; 3],
            blend: None,
            next_id: 1,
            dirty,
            generation,
            pose_generation,
            last_clock,
            promote_next_foreground: false,
            selection_cache,
            anchor_valid: false,
            anchor_bone: None,
            anchor_point: [0.; 3],
            numbered_events: VecDeque::new(),
            pending_bone_updates: BTreeMap::new(),
            pending_vertex_updates: BTreeMap::new(),
            pass: None,
        }
    }
    pub fn layers(&self) -> &[OwnedLayer] {
        &self.layers
    }
    pub fn destruction_layers(&self) -> [Option<LayerId>; 4] {
        [self.slots[0], self.blend, self.slots[1], self.slots[2]]
    }
    /// Direct 5a0a7b/5991a0 release, without an outgoing fade.
    pub fn detach_for_destruction(&mut self, id: LayerId) -> Result<(), String> {
        self.idle()?;
        if self.layer(id).is_none() {
            return Err("destroyed animator layer absent".into());
        }
        self.release(id);
        for slot in &mut self.slots {
            if *slot == Some(id) {
                *slot = None;
            }
        }
        if self.blend == Some(id) {
            self.blend = None;
        }
        Ok(())
    }
    /// 598c71 removes the first remaining skin layer until the vector is empty.
    /// The resident owns its skin exclusively; prototype mesh caches are separate.
    pub fn release_model_layers(&mut self) -> Result<Vec<LayerId>, String> {
        self.idle()?;
        if self.destruction_layers().iter().any(Option::is_some) {
            return Err("model destruction before explicit layer detachment".into());
        }
        let mut released = vec![];
        while let Some(first) = self.layers.first() {
            let id = first.id;
            self.release(id);
            released.push(id);
        }
        Ok(released)
    }
    pub fn slot(&self, slot: Slot) -> Option<LayerId> {
        self.slots[slot.index()]
    }
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|e| e.id == id).map(|e| &e.layer)
    }
    pub fn busy(&self) -> bool {
        self.pass.is_some()
    }
    pub fn pose_generation(&self) -> u32 {
        self.pose_generation
    }
    pub fn bind_fp_control(&mut self, control: u16) -> Result<(), String> {
        self.idle()?;
        if !matches!(control, 0x003f | 0x037f) {
            return Err("unsupported native animator floating-point control".into());
        }
        self.fp_control = Some(control);
        Ok(())
    }
    pub fn fp_control(&self) -> u16 {
        self.fp_control.unwrap_or(0x037f)
    }
    pub fn take_bone_generation_deltas(&mut self) -> BTreeMap<usize, u32> {
        std::mem::take(&mut self.pending_bone_updates)
    }
    pub fn take_vertex_generation_deltas(&mut self) -> BTreeMap<usize, u32> {
        std::mem::take(&mut self.pending_vertex_updates)
    }
    /// 5994c3 increments both stamps, then 5995ed replaces the pose stamp with
    /// the signed maximum of the recursively updated bone stamps.
    pub(super) fn hierarchy_updated(&mut self, stamp: u32) {
        self.generation = self.generation.wrapping_add(1);
        self.pose_generation = stamp;
    }
    /// 599508 publishes the root bone's geometry generation as the shared
    /// skeleton generation. This does not evaluate or publish a local pose.
    pub(super) fn bounds_updated(&mut self, stamp: u32) {
        self.generation = stamp;
    }
    pub(super) fn geometry_changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
    pub(super) fn generation(&self) -> u32 {
        self.generation
    }
    /// 58831d releases its calibration foreground directly, without creating
    /// the outgoing fade used by ordinary ClearForeground.
    pub(super) fn finish_calibration(&mut self) -> Result<(), String> {
        self.idle()?;
        if let Some(id) = self.slots[Slot::Foreground.index()].take() {
            self.release(id);
        }
        self.selection_cache = -1;
        Ok(())
    }
    pub(super) fn calibration_phase(
        &mut self,
        phase: f32,
        first_sample: bool,
    ) -> Result<(), String> {
        self.idle()?;
        if !phase.is_finite() {
            return Err("invalid calibration phase".into());
        }
        let id = self
            .slot(Slot::Foreground)
            .ok_or("calibration foreground absent")?;
        let layer = self.get_mut(id)?;
        layer.clock.phase = phase;
        // 58831d writes +2c (weight) only before the first sample. The second
        // sample retains changes made by the first skin pass and callbacks.
        if first_sample {
            layer.clock.weight = 1.;
        }
        Ok(())
    }
    /// 5a0eac -> 5995e6 publishes the model matrix without sampling a layer.
    pub(super) fn placement_updated(&mut self) -> Result<(), String> {
        self.idle()?;
        self.pose_generation = self.pose_generation.wrapping_add(1);
        Ok(())
    }
    /// Outer 58da12/58dc72 cleanup, before the skin's own update and retirement.
    pub fn discard_exhausted_blend(&mut self) -> Result<(), String> {
        self.idle()?;
        if let Some(id) = self.blend {
            if self
                .layer(id)
                .ok_or("native blend layer absent")?
                .clock
                .weight
                .abs()
                <= 0.000001_f32
            {
                self.release(id);
                self.blend = None;
            }
        }
        Ok(())
    }
    fn idle(&self) -> Result<(), String> {
        if self.busy() {
            return Err(
                "native layer ownership change during pending update is not implemented".into(),
            );
        }
        Ok(())
    }
    pub(super) fn get_mut(&mut self, id: LayerId) -> Result<&mut Layer, String> {
        self.layers
            .iter_mut()
            .find(|e| e.id == id)
            .map(|e| &mut e.layer)
            .ok_or_else(|| "native animation slot references absent layer".into())
    }
    fn insert(&mut self, layer: Layer) -> Result<LayerId, String> {
        let id = LayerId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("native layer ID exhausted")?;
        // 599128 scans the existing list even if a priority change left it dirty.
        let at = self
            .layers
            .iter()
            .position(|e| layer.priority < e.layer.priority)
            .unwrap_or(self.layers.len());
        self.layers.insert(
            at,
            OwnedLayer {
                id,
                layer,
                anchor_bone: None,
                anchor_state: 0,
            },
        );
        self.dirty = true;
        Ok(id)
    }
    fn release(&mut self, id: LayerId) {
        if let Some(at) = self.layers.iter().position(|e| e.id == id) {
            if let Some(pass) = &mut self.pass {
                for index in at..self.layers.len() - 1 {
                    pass.order[index] = self.layers[index + 1].id;
                }
            }
            self.layers.remove(at);
            self.dirty = true;
        }
    }
    pub fn set_promote_next_foreground(&mut self, value: bool) -> Result<(), String> {
        self.idle()?;
        self.promote_next_foreground = value;
        Ok(())
    }
    pub fn set_priority(&mut self, id: LayerId, priority: i32) -> Result<(), String> {
        self.idle()?;
        self.get_mut(id)?.priority = priority;
        self.dirty = true;
        Ok(())
    }
    pub(super) fn repeat(&mut self, id: LayerId) -> Result<(), String> {
        self.idle()?;
        self.get_mut(id)?.clock.mode = 1;
        Ok(())
    }
    pub(super) fn clear_anchor(&mut self) -> Result<(), String> {
        self.idle()?;
        self.anchor_valid = false;
        self.anchor_bone = None;
        Ok(())
    }
    pub fn selection(&self) -> i32 {
        self.selection_cache
    }
    pub(super) fn set_selection(&mut self, index: i32) -> Result<(), String> {
        self.idle()?;
        self.selection_cache = index;
        Ok(())
    }
    /// 5926d7's lookup gate. Readiness failure still commits the selection and
    /// configures any surviving foreground, so it differs from a plain start.
    pub fn needs_selection(&self, index: i32) -> bool {
        self.slot(Slot::Foreground).is_none() || self.selection_cache != index
    }
    /// Complete a lookup already requested through `needs_selection`. Do not
    /// repeat that gate: resource services may have changed the cache meanwhile.
    pub fn finish_foreground_selection(
        &mut self,
        index: i32,
        mode: i32,
        skeleton: &Skeleton,
        clip: Option<&Animation>,
        now: i32,
        settings: Settings,
    ) -> Result<(), String> {
        self.idle()?;
        self.selection_cache = index;
        self.start(Slot::Foreground, skeleton, clip, false, now, settings)?;
        if let Some(id) = self.slot(Slot::Foreground) {
            self.get_mut(id)?.clock.mode = mode;
        }
        Ok(())
    }
    pub fn finished(&self, slot: Slot) -> Result<bool, String> {
        match self.slot(slot) {
            None => Ok(true),
            Some(id) => self
                .layer(id)
                .map(Layer::finished)
                .ok_or_else(|| "native animation slot references absent layer".into()),
        }
    }
    /// 592878. Foreground becomes the sole outgoing blend; the skin pass retains
    /// it at zero weight. Its cleanup belongs to the enclosing animator update.
    /// Carry fades detach and retire in the skin pass.
    pub fn clear(&mut self, slot: Slot, now: i32, settings: Settings) -> Result<(), String> {
        self.idle()?;
        let Some(id) = self.slot(slot) else {
            return Ok(());
        };
        self.clear_owned(id, Some(slot), now, settings)?;
        self.slots[slot.index()] = None;
        Ok(())
    }
    fn clear_owned(
        &mut self,
        id: LayerId,
        slot: Option<Slot>,
        now: i32,
        settings: Settings,
    ) -> Result<(), String> {
        if !settings.fade_enabled || slot == Some(Slot::Base) || settings.fade_duration == 0 {
            self.release(id);
        } else {
            if settings.fade_duration < 0 {
                return Err("negative native animator fade duration is not supported".into());
            }
            let old = self.get_mut(id)?;
            old.translation_policy = -1;
            old.rotation_policy = -1;
            old.clock.fading = true;
            old.clock.fade_start = now;
            old.clock.fade_duration = settings.fade_duration;
            old.clock.fade_from = old.clock.weight;
            old.clock.fade_to = 0.;
            if slot == Some(Slot::Foreground) {
                if let Some(previous) = self.blend.take() {
                    self.release(previous);
                }
                self.blend = Some(id);
                let priority = self.get_mut(id)?.priority.wrapping_add(1);
                self.set_priority(id, priority)?;
            } else {
                self.get_mut(id)?.remove_when_faded = true;
            }
        }
        Ok(())
    }
    /// Layer/event part of 589220. Its +104 call on the outgoing-blend pointer
    /// follows the non-foreground fade branch: a reset can leave unowned fading
    /// layers in the skin. Removing every layer here changes the next pose.
    pub(super) fn reset(&mut self, now: i32, settings: Settings) -> Result<(), String> {
        self.idle()?;
        self.promote_next_foreground = true;
        self.anchor_bone = None;
        self.anchor_valid = false;
        self.clear_foreground(now, settings)?;
        if let Some(id) = self.blend {
            self.clear_owned(id, None, now, settings)?;
            self.blend = None;
        }
        self.clear(Slot::Carry, now, settings)?;
        self.clear(Slot::Base, now, settings)?;
        self.numbered_events.clear();
        Ok(())
    }
    /// Layer/event part of base Animator Reset (584d76/586c9f). Unlike the
    /// person entry it retains anchors, promotion and the person's carry slot.
    pub(super) fn reset_graphic(&mut self, now: i32, settings: Settings) -> Result<(), String> {
        self.idle()?;
        self.clear_foreground(now, settings)?;
        if let Some(id) = self.blend {
            self.clear_owned(id, None, now, settings)?;
            self.blend = None;
        }
        self.clear(Slot::Base, now, settings)?;
        self.numbered_events.clear();
        Ok(())
    }
    pub fn clear_foreground(&mut self, now: i32, settings: Settings) -> Result<(), String> {
        self.clear(Slot::Foreground, now, settings)?;
        self.selection_cache = -1;
        Ok(())
    }
    /// 592726 after resource readiness. None leaves the old slot untouched.
    /// The host must surface decode errors before calling, not substitute a clip.
    pub fn start(
        &mut self,
        slot: Slot,
        skeleton: &Skeleton,
        clip: Option<&Animation>,
        reverse: bool,
        now: i32,
        settings: Settings,
    ) -> Result<bool, String> {
        self.idle()?;
        let Some(clip) = clip else {
            return Ok(false);
        };
        let mut layer = Layer::bind(
            skeleton,
            clip,
            Start {
                priority: 1,
                speed: if reverse { -1. } else { 1. },
                weight: 1.,
                now,
            },
        )?;
        self.clear(slot, now, settings)?;
        layer.sampling_config = settings.sampling_config;
        layer.clock.phase = if reverse { 1. } else { 0. };
        layer.clock.mode = 0;
        let id = self.insert(layer)?;
        self.slots[slot.index()] = Some(id);
        if slot == Slot::Foreground && self.promote_next_foreground {
            self.promote_next_foreground = false;
            self.set_priority(id, 6)?;
            if let Some(previous) = self.blend.take() {
                self.release(previous);
            }
            let layer = self.get_mut(id)?;
            layer.translation_policy = 0;
            layer.rotation_policy = 0;
        }
        Ok(true)
    }
    /// 591eec installation of an already decoded/bound descriptor. Unlike start,
    /// an empty descriptor clears the slot. The caller handles lookup failure.
    pub fn restore_layer(
        &mut self,
        slot: Slot,
        layer: Option<Layer>,
        now: i32,
        settings: Settings,
    ) -> Result<(), String> {
        self.clear(slot, now, settings)?;
        if let Some(layer) = layer {
            let id = self.insert(layer)?;
            self.slots[slot.index()] = Some(id);
        }
        Ok(())
    }
    /// 5992b3/5991fd. Repeated clocks skip only when clean and not forced.
    /// The pass owns its iteration state, allowing save/resume inside callbacks.
    pub fn begin(
        &mut self,
        now: i32,
        sample_pose: bool,
        callback: bool,
        force: bool,
        bones: &mut [BoneInput],
    ) -> Result<bool, String> {
        self.idle()?;
        if !self.dirty && now == self.last_clock && !force {
            return Ok(false);
        }
        if self
            .layers
            .iter()
            .flat_map(|e| &e.layer.bindings)
            .any(|b| b.bone >= bones.len())
        {
            return Err("native animator bound bone absent".into());
        }
        if self.dirty {
            self.layers.sort_by_key(|e| e.layer.priority);
            for bone in bones.iter_mut() {
                bone.priority = -10000;
            }
            for e in self.layers.iter().filter(|e| e.layer.priority_override) {
                for b in &e.layer.bindings {
                    bones[b.bone].priority = e.layer.priority;
                }
            }
            self.dirty = false;
        }
        self.pass = Some(Pass {
            now,
            sample_pose,
            callback,
            index: 0,
            order: self.layers.iter().map(|e| e.id).collect(),
            active: None,
            update: None,
        });
        Ok(true)
    }
    /// Host applies the returned properties, then acknowledges them. No callback
    /// effects are invented here; original native property dispatch is separate.
    pub fn next(
        &mut self,
        program: &Program,
        bones: &mut [BoneInput],
    ) -> Result<Option<Event>, String> {
        let fp_control = self.fp_control();
        let Some(pass) = &mut self.pass else {
            return Ok(None);
        };
        while pass.index < pass.order.len() {
            let id = *pass.active.get_or_insert(pass.order[pass.index]);
            let owned =
                self.layers.iter_mut().find(|e| e.id == id).ok_or_else(|| {
                    format!("native skin pass references released layer {}", id.0)
                })?;
            // Resolve before advancing, so a missing asset can be loaded/retried.
            let asset = program
                .animations
                .get(&owned.layer.name)
                .ok_or_else(|| format!("native animator missing animation {}", owned.layer.name))?;
            if pass.update.is_none() {
                pass.update = Some(Update::begin_with_control(
                    &mut owned.layer,
                    pass.now,
                    pass.sample_pose,
                    pass.callback,
                    fp_control,
                )?);
            }
            let update = pass.update.as_mut().unwrap();
            let dispatch = update.next(&mut owned.layer, &asset.clip, &asset.samples, bones)?;
            let delta = update.take_generation_delta();
            for (bone, delta) in update.take_bone_generation_deltas() {
                let count = self.pending_bone_updates.entry(bone).or_default();
                *count = count.wrapping_add(delta);
                let count = self.pending_vertex_updates.entry(bone).or_default();
                *count = count.wrapping_add(delta);
            }
            self.generation = self.generation.wrapping_add(delta);
            self.pose_generation = self.pose_generation.wrapping_add(delta);
            if let Some(dispatch) = dispatch {
                return Ok(Some(Event {
                    layer: owned.id,
                    dispatch,
                }));
            }
            pass.update = None;
            pass.active = None;
            pass.index += 1;
        }
        let now = pass.now;
        // Original reverse traversal retires only explicitly marked layers.
        for at in (0..self.layers.len()).rev() {
            let e = &self.layers[at];
            if e.layer.remove_when_faded && e.layer.clock.weight <= 0. {
                let id = e.id;
                self.release(id);
            }
        }
        self.generation = self.generation.wrapping_add(1);
        self.last_clock = now;
        self.pass = None;
        Ok(None)
    }
    pub fn acknowledge(&mut self) -> Result<(), String> {
        self.pass
            .as_mut()
            .and_then(|p| p.update.as_mut())
            .ok_or("no pending native animator pass")?
            .acknowledge()
    }

    pub fn pop_numbered_event(&mut self) -> Option<i32> {
        self.numbered_events.pop_front()
    }
    pub(super) fn clear_numbered_events(&mut self) -> Result<(), String> {
        self.idle()?;
        self.numbered_events.clear();
        Ok(())
    }
    pub(super) fn enqueue_numbered_event(&mut self, value: i32) -> Result<(), String> {
        // Explicit host resource bound, not an inferred native queue capacity.
        if self.numbered_events.len() >= 4096 {
            return Err("native animation event queue is full".into());
        }
        self.numbered_events.push_back(value);
        Ok(())
    }
    pub(super) fn set_interruptible(&mut self, id: LayerId, value: i32) -> Result<(), String> {
        self.get_mut(id)?.interruptible = value;
        Ok(())
    }
    pub(super) fn set_anchor(
        &mut self,
        id: LayerId,
        bone: usize,
        enabled: bool,
    ) -> Result<(), String> {
        let foreground = self.slot(Slot::Foreground) == Some(id);
        let owned = self
            .layers
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or("native anchor layer absent")?;
        owned.anchor_state = 0;
        if foreground {
            owned.anchor_bone = enabled.then_some(bone);
            if enabled {
                if self.anchor_bone != Some(bone) {
                    self.anchor_valid = false;
                    self.anchor_bone = Some(bone);
                }
            } else if self.anchor_bone == Some(bone) {
                self.anchor_valid = false;
                self.anchor_bone = None;
            }
        }
        Ok(())
    }
    pub(super) fn discontinuity(&mut self) -> Result<(), String> {
        if let Some(id) = self.blend {
            // 5912fd mutates the skin's vector immediately, even inside a pass.
            // A later attempt to use a released identity fails in next(). The
            // original synthetic cases dereference freed memory at that point;
            // behavior beyond that access is not a compatibility contract.
            self.release(id);
            self.blend = None;
            self.promote_next_foreground = true;
        }
        Ok(())
    }
}
