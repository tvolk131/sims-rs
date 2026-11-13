//! Retained ordinary 58598c Refresh, before its later destructor. Raw caches
//! remain raw until the original service writes them. This adapter currently
//! supports an empty-layer, empty-appearance, single identity-root skin.
use super::{Id, Matrix, Retained};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::{graphic_frame as frame, graphic_transform, model, transform};
use sims_vm::{Definition, ObjectId, Vm, refresh::DrawState};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoneFields {
    pub first_child: Option<Id>,
    pub priority: i32,
    pub attachment: Matrix,
    pub rotation_bits: [u32; 9],
    pub bounds_generation: u32,
    pub bounds_bits: [u32; 6],
    pub vertex_bounds_bits: [u32; 6],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fields {
    pub clock_mode: u8,
    pub now: i32,
    pub explicit: u8,
    pub visible: u8,
    pub bones: BTreeMap<Id, BoneFields>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub retained: Retained,
    pub fields: Fields,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Physical {
    pub position: [i32; 2],
    pub facing: i16,
    pub level: i32,
}
/// Each value is an explicit input from the owning process/view, including the
/// x87 control and hierarchy origin. Missing terrain stays unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub tick: i32,
    pub physical: Physical,
    pub camera_rotation: u8,
    pub floor_vector_bits: [u32; 3],
    pub terrain_extent: Option<usize>,
    pub terrain_corners: Option<Vec<[u8; 4]>>,
    pub fp_control: u16,
    pub origin_pose_bits: [u32; 7],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub object: ObjectId,
    pub definition: Definition,
    pub registry_lifetime: u32,
    pub native_actor: Id,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invalidation {
    before: DrawState,
    render_clock: u32,
    after: DrawState,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Action {
    Begin { only_visible: bool },
    Advance(Environment),
    Invalidate(Invalidation),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Core {
    input: Input,
    update: Option<frame::Update>,
    external: Option<frame::Request>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    initial: Input,
    binding: Binding,
    core: Core,
    history: Vec<Action>,
}
const IDENTITY_POSE: [u32; 7] = [0, 0, 0, 0, 0, 0, 0x3f800000];
const IDENTITY_ROTATION: [u32; 9] = [0x3f800000, 0, 0, 0, 0x3f800000, 0, 0, 0, 0x3f800000];
fn empty_bounds() -> [u32; 6] {
    let b = model::Bounds::EMPTY;
    std::array::from_fn(|i| {
        if i < 3 {
            b.minimum[i]
        } else {
            b.maximum[i - 3]
        }
        .to_bits()
    })
}
fn matrix(raw: &Matrix) -> model::Matrix {
    model::Matrix {
        values: raw.bits.map(f32::from_bits),
        kind: raw.flags,
    }
}
#[cfg(test)]
fn raw(value: model::Matrix) -> Matrix {
    Matrix {
        bits: value.values.map(f32::to_bits),
        flags: value.kind,
    }
}
impl Input {
    fn validate(&self) -> Result<()> {
        // Reuse the retained allocation/identity validator. No fresh graphics,
        // hierarchy, RNG or constructor defaults are introduced.
        super::Owner::from_retained(self.retained.clone())?;
        let r = &self.retained;
        ensure!(
            r.global_focus.is_none(),
            "retained Refresh shared focus owner unsupported"
        );
        ensure!(
            r.skin.bones.len() == 1 && self.fields.bones.len() == 1,
            "retained Refresh requires its single root owner"
        );
        let b = &r.skin.bones[0];
        let extra = self
            .fields
            .bones
            .get(&b.id)
            .context("retained Refresh bone fields absent")?;
        ensure!(
            r.skin.root == b.id
                && b.parent.is_none()
                && b.next_sibling.is_none()
                && extra.first_child.is_none(),
            "retained Refresh root topology unsupported"
        );
        ensure!(
            b.flags == [1; 3] && b.wiggle_amplitude_bits == 0,
            "retained Refresh root flags/wiggle unsupported"
        );
        Ok(())
    }
}
impl Owner {
    pub fn new(input: Input, binding: Binding) -> Result<Self> {
        input.validate()?;
        ensure!(
            binding.object > 0
                && binding.registry_lifetime != 0
                && binding.native_actor == input.retained.actor,
            "retained Refresh object binding differs"
        );
        Ok(Self {
            initial: input.clone(),
            binding,
            core: Core {
                input,
                update: None,
                external: None,
            },
            history: vec![],
        })
    }
    pub fn input(&self) -> &Input {
        &self.core.input
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn pending(&self) -> Option<&frame::Request> {
        self.core.external.as_ref()
    }
    pub fn complete(&self) -> bool {
        self.core
            .update
            .as_ref()
            .is_some_and(frame::Update::complete)
    }
    pub fn update_pending(&self) -> bool {
        self.core.update.as_ref().is_some_and(|u| !u.complete())
    }
    pub fn validate(&self) -> Result<()> {
        let mut replay = Self::new(self.initial.clone(), self.binding.clone())?;
        ensure!(
            self.history.len() <= 4096,
            "retained Refresh history exceeds bound"
        );
        for action in &self.history {
            replay.core.apply(action)?;
        }
        ensure!(
            replay.core == self.core,
            "retained Refresh differs from its owned history"
        );
        self.core.input.validate()?;
        Ok(())
    }
    pub fn validate_vm(&self, vm: &Vm) -> Result<()> {
        self.validate()?;
        let b = &self.binding;
        let object = vm
            .world
            .objects
            .get(&b.object)
            .context("retained Refresh object absent")?;
        ensure!(
            object.person.is_none()
                && object.definition.as_ref() == Some(&b.definition)
                && vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .and_then(|s| s.handle(b.object))
                    == Some(b.registry_lifetime),
            "retained Refresh registry binding differs"
        );
        Ok(())
    }
    fn commit(&mut self, action: Action) -> Result<Option<frame::Request>> {
        self.validate()?;
        ensure!(
            self.history.len() < 4096,
            "retained Refresh history exceeds bound"
        );
        let mut candidate = self.core.clone();
        let request = candidate.apply(&action)?;
        self.core = candidate;
        self.history.push(action);
        Ok(request)
    }
    /// The actual Refresh argument controls the visible gate. It is distinct
    /// from the Animator's raw explicit-position byte.
    pub fn begin_refresh(&mut self, only_visible: bool) -> Result<()> {
        self.commit(Action::Begin { only_visible })?;
        Ok(())
    }
    /// Execute one owned service. Invalidate remains pending until the real VM
    /// service runs; polling it does not reread clocks or mutate history.
    pub fn advance(&mut self, environment: &Environment) -> Result<Option<frame::Request>> {
        self.validate()?;
        if self.core.external.is_some() {
            return Ok(self.core.external.clone());
        }
        if self.complete() {
            return Ok(None);
        }
        self.commit(Action::Advance(environment.clone()))
    }
    pub fn perform_invalidation(&mut self, vm: &mut Vm) -> Result<()> {
        self.validate_vm(vm)?;
        ensure!(
            self.pending() == Some(&frame::Request::Invalidate),
            "retained Refresh invalidation is not pending"
        );
        let id = self.binding.object;
        let object = &vm.world.objects[&id];
        ensure!(
            object.container.is_none() && object.slots.iter().all(Option::is_none),
            "retained Refresh containment service unsupported"
        );
        let refresh = vm
            .world
            .refresh
            .as_ref()
            .context("retained Refresh draw owner absent")?;
        let before = refresh
            .draw
            .get(&id)
            .context("retained Refresh object draw absent")?
            .clone();
        ensure!(
            before.dynamic,
            "retained Refresh visibility producer unsupported"
        );
        let mut after = before.clone();
        after.expiry = i32::MAX;
        after.refreshed_at = refresh.render_clock;
        let action = Action::Invalidate(Invalidation {
            before,
            render_clock: refresh.render_clock,
            after,
        });
        // Finish every fallible owner-side check before touching VM state. The
        // supported VM branch preflights containment/draw state before writes.
        let mut candidate = self.clone();
        candidate.commit(action)?;
        candidate.validate()?;
        vm.refresh_object(id, 0, 0, 0).map_err(anyhow::Error::msg)?;
        *self = candidate;
        Ok(())
    }
    /// Keep the verified prelude rather than rebasing teardown history onto a
    /// newly constructed, apparently pristine retained graph.
    pub fn into_retirement(self) -> Result<Retirement> {
        self.validate()?;
        ensure!(
            self.complete() && self.pending().is_none(),
            "retained Refresh is unfinished"
        );
        let owner = super::Owner::from_retained(self.core.input.retained.clone())?;
        Ok(Retirement {
            prelude: self,
            owner,
        })
    }
}
impl Core {
    fn state(&self) -> frame::State {
        let r = &self.input.retained;
        frame::State {
            clock_mode: self.input.fields.clock_mode,
            now: self.input.fields.now,
            visible: self.input.fields.visible,
            blend: None,
            placement: graphic_transform::Placement {
                base: r.reset.scale_bits.map(f32::from_bits),
                offset: r.reset.offset_bits.map(f32::from_bits),
                dirty: r.reset.dirty[1] != 0,
                explicit: self.input.fields.explicit != 0,
            },
            matrix: matrix(&r.reset.embedded_matrix),
        }
    }
    fn acknowledge(&mut self, request: &frame::Request) -> Result<()> {
        let mut state = self.state();
        self.update
            .as_mut()
            .context("retained Refresh continuation absent")?
            .acknowledge(&mut state, request, frame::Reply::Done)
            .map_err(anyhow::Error::msg)?;
        // This entry never writes the raw visibility/explicit/dirty bytes.
        self.input.fields.now = state.now;
        Ok(())
    }
    fn apply(&mut self, action: &Action) -> Result<Option<frame::Request>> {
        match action {
            Action::Begin { only_visible } => {
                ensure!(
                    self.external.is_none()
                        && self.update.as_ref().is_none_or(frame::Update::complete),
                    "retained Refresh is already active"
                );
                self.update = Some(frame::Update::new(frame::Entry::Refresh, *only_visible));
                Ok(None)
            }
            Action::Invalidate(receipt) => {
                ensure!(
                    self.external == Some(frame::Request::Invalidate),
                    "retained Refresh invalidation is not pending"
                );
                let mut expected = receipt.before.clone();
                ensure!(
                    expected.dynamic,
                    "retained Refresh visibility producer unsupported"
                );
                expected.expiry = i32::MAX;
                expected.refreshed_at = receipt.render_clock;
                ensure!(
                    receipt.after == expected,
                    "retained Refresh invalidation receipt differs"
                );
                self.acknowledge(&frame::Request::Invalidate)?;
                self.external = None;
                Ok(Some(frame::Request::Invalidate))
            }
            Action::Advance(environment) => {
                ensure!(
                    self.external.is_none(),
                    "retained Refresh external service is pending"
                );
                let mut state = self.state();
                let r = &self.input.retained;
                let request = self
                    .update
                    .as_mut()
                    .context("retained Refresh continuation absent")?
                    .advance(
                        &mut state,
                        environment.tick,
                        Some(sims_sim::pose::graphic_appearance::Model {
                            model: r.model.id,
                            skin: r.skin.id,
                        }),
                    )
                    .map_err(anyhow::Error::msg)?;
                self.input.fields.now = state.now;
                match &request {
                    Some(frame::Request::Invalidate) => self.external = request.clone(),
                    Some(request) => {
                        self.service(request, environment)?;
                        self.acknowledge(request)?;
                    }
                    None => {}
                }
                Ok(request)
            }
        }
    }
    fn service(&mut self, request: &frame::Request, env: &Environment) -> Result<()> {
        let r = &mut self.input.retained;
        match request {
            frame::Request::Skin { skin, now, .. } => {
                ensure!(*skin == r.skin.id, "retained Refresh skin identity differs");
                if r.skin.hierarchy_dirty == 0 && r.skin.last_clock == *now as u32 {
                    return Ok(());
                }
                if r.skin.hierarchy_dirty != 0 {
                    self.input
                        .fields
                        .bones
                        .get_mut(&r.skin.root)
                        .unwrap()
                        .priority = -10000;
                    r.skin.hierarchy_dirty = 0;
                }
                r.skin.pose_generation = r.skin.pose_generation.wrapping_add(1);
                r.skin.last_clock = *now as u32;
            }
            frame::Request::Placement => {
                ensure!(
                    self.input.fields.explicit == 0,
                    "retained Refresh explicit-position callback unsupported"
                );
                // This local helper scratch is not a saved person movement cache.
                let mut placement = transform::Placement {
                    base: r.reset.scale_bits.map(f32::from_bits),
                    previous: [0.; 3],
                    offset: r.reset.offset_bits.map(f32::from_bits),
                    dirty: r.reset.dirty[1] != 0,
                };
                let computed = placement
                    .transform_retained(
                        transform::Source {
                            position: env.physical.position,
                            facing: env.physical.facing,
                            level: env.physical.level,
                            flags: [1; 2],
                        },
                        transform::View {
                            rotation: env.camera_rotation,
                            floor_vector: env.floor_vector_bits.map(f32::from_bits),
                        },
                        transform::RetainedTerrain {
                            extent: env.terrain_extent,
                            corners: env.terrain_corners.as_deref(),
                        },
                        env.fp_control,
                    )
                    .map_err(anyhow::Error::msg)?;
                // The retained scale shortcut is proven; general matrix products
                // under003f need their own precision-bound implementation.
                ensure!(
                    r.model.scale.flags == 1,
                    "retained Refresh nonidentity scale unsupported"
                );
                //49b87c checks kind1 before reading the cached scale words.
                //Those raw words can be stale NaNs/garbage and stay untouched.
                let published = Matrix {
                    bits: computed.matrix.map(f32::to_bits),
                    flags: 0,
                };
                r.reset.scale_bits = placement.base.map(f32::to_bits);
                r.reset.offset_bits = placement.offset.map(f32::to_bits);
                r.reset.embedded_matrix = published.clone();
                r.model.placement = published.clone();
                r.skin.pose = published.clone();
                r.notification.transform = published;
                r.notification.packed |= 2;
                r.skin.hierarchy_generation = r.skin.hierarchy_generation.wrapping_add(1);
            }
            frame::Request::Hierarchy { skin } => {
                ensure!(
                    *skin == r.skin.id,
                    "retained Refresh hierarchy skin differs"
                );
                if r.skin.hierarchy_evaluated == r.skin.hierarchy_generation {
                    return Ok(());
                }
                let b = &mut r.skin.bones[0];
                ensure!(
                    env.fp_control == 0x003f,
                    "retained Refresh hierarchy x87 control unsupported"
                );
                ensure!(
                    env.origin_pose_bits == IDENTITY_POSE && b.local_pose_bits == IDENTITY_POSE,
                    "retained Refresh nonidentity hierarchy math unsupported"
                );
                // 59615b/596489 overwrite these caches only now. Arbitrary stale
                // words and NaNs in the old absolute/matrix cache remain valid
                // retained input, never constructor inputs.
                b.absolute_pose_bits = IDENTITY_POSE;
                self.input
                    .fields
                    .bones
                    .get_mut(&b.id)
                    .unwrap()
                    .rotation_bits = IDENTITY_ROTATION;
                b.update = b.update.wrapping_add(1);
                r.skin.pose_generation = r.skin.pose_generation.wrapping_add(1);
                r.skin.hierarchy_generation = r.skin.hierarchy_generation.wrapping_add(1);
                r.skin.hierarchy_evaluated = b.update;
                r.skin.hierarchy_generation = b.update;
            }
            frame::Request::Geometry { model } => {
                ensure!(
                    *model == r.model.id,
                    "retained Refresh geometry model differs"
                );
                // No appearances: original local bounds stay empty and the
                // bounds-notification consumer returns without publication.
                ensure!(
                    r.skin.bounds_bits == empty_bounds(),
                    "retained Refresh nonempty cached bounds unsupported"
                );
                if r.skin.geometry_dirty != 0 {
                    r.skin.bounds_bits = empty_bounds();
                    for b in &mut r.skin.bones {
                        b.geometry_stamp = b.geometry_stamp.wrapping_add(1);
                        self.input
                            .fields
                            .bones
                            .get_mut(&b.id)
                            .unwrap()
                            .vertex_bounds_bits = empty_bounds();
                    }
                    r.skin.pose_generation = r.skin.pose_generation.wrapping_add(1);
                    r.skin.geometry_dirty = 0;
                }
                if r.skin.vertex_generation != r.skin.pose_generation {
                    let b = &r.skin.bones[0];
                    if r.skin.bounds_generation != r.skin.generation {
                        let stamp = self.input.fields.bones[&b.id].bounds_generation;
                        r.skin.bounds_generation = stamp;
                        r.skin.generation = stamp;
                    }
                    if r.skin.vertex_generation != r.skin.pose_generation {
                        r.skin.vertex_generation = b.geometry_stamp;
                        r.skin.pose_generation = b.geometry_stamp;
                    }
                }
            }
            _ => bail!("unsupported retained ordinary Refresh service"),
        }
        Ok(())
    }
}

/// The active raw graph is the destructor's graph; the frame is a historical
/// prelude retained solely to validate its initial state and exact handoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retirement {
    prelude: Owner,
    owner: super::Owner,
}
impl Retirement {
    pub fn prelude(&self) -> &Owner {
        &self.prelude
    }
    pub fn validate(&self) -> Result<()> {
        self.prelude.validate()?;
        ensure!(
            self.prelude.complete()
                && self.prelude.pending().is_none()
                && self.owner.initial == self.prelude.input().retained,
            "retained Refresh/destructor handoff differs"
        );
        self.owner.validate()
    }
    pub fn retained(&self) -> &Retained {
        self.owner.retained()
    }
    pub fn destructor(&self) -> &super::Owner {
        &self.owner
    }
    pub fn advance(&mut self) -> Result<bool> {
        self.validate()?;
        self.owner.advance()
    }
    pub fn release_animator(&mut self) -> Result<()> {
        self.validate()?;
        self.owner.release_animator()
    }
}

#[cfg(test)]
mod tests;
