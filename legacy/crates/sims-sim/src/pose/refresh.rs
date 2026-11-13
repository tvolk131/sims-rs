//! Resumable 58da12 refresh, 5918ba load tail and shared 58dc72 ordinary tail.
//! The ordinary caller must first complete carry/base selection and movement.
//! Placement, geometry, appearance binding, viewport and carried-object work are explicit
//! host requests. Completing a request means its service actually ran.
use super::{LocalBone, hierarchy, live::Live, model, properties, transform};
use crate::head_look::{
    self, Fields, State, Target,
    rig::{AbsoluteBone, Rig},
};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hierarchy {
    pub parents: Vec<Option<usize>>,
    pub absolute: Vec<AbsoluteBone>,
    /// Native bone +0x130, compared as signed words by 599634.
    pub updates: Vec<u32>,
    /// Skeleton +0xa0. The layer owner's pose_generation is +0xa4.
    pub evaluated_generation: u32,
}
impl Hierarchy {
    fn validate(&self, skin: &Live) -> Result<(), String> {
        if self.absolute.len() != skin.bones.len() || self.updates.len() != skin.bones.len() {
            return Err("native hierarchy storage length mismatch".into());
        }
        // Also validates root, indices, cycles and local finite coordinates.
        hierarchy(
            &self.parents,
            &skin.bones.iter().map(|b| b.local).collect::<Vec<_>>(),
            origin(),
        )?;
        Ok(())
    }
    pub fn update(&mut self, skin: &mut Live, force: bool) -> Result<(), String> {
        if !force && self.evaluated_generation == skin.animator.pose_generation() {
            return Ok(());
        }
        self.validate(skin)?;
        self.absolute = super::hierarchy_with_control(
            &self.parents,
            &skin.bones.iter().map(|b| b.local).collect::<Vec<_>>(),
            origin(),
            skin.animator.fp_control(),
        )?;
        for stamp in &mut self.updates {
            *stamp = stamp.wrapping_add(1);
        }
        let stamp = *self.updates.iter().max_by_key(|s| **s as i32).unwrap();
        self.updates.fill(stamp);
        self.evaluated_generation = stamp;
        skin.animator.hierarchy_updated(stamp);
        Ok(())
    }
    fn update_branch(&mut self, skin: &Live, bone: usize) -> Result<(), String> {
        let parent = self
            .parents
            .get(bone)
            .copied()
            .flatten()
            .ok_or("native head bone has no parent")?;
        self.branch(skin, bone, self.absolute[parent])
    }
    fn branch(&mut self, skin: &Live, bone: usize, parent: AbsoluteBone) -> Result<(), String> {
        self.absolute[bone] = super::compose_bone_with_control(
            skin.bones[bone].local,
            parent,
            skin.animator.fp_control(),
        )?;
        self.updates[bone] = self.updates[bone].wrapping_add(1);
        for child in 0..self.parents.len() {
            if self.parents[child] == Some(bone) {
                self.branch(skin, child, self.absolute[bone])?;
            }
        }
        Ok(())
    }
}
fn origin() -> AbsoluteBone {
    AbsoluteBone {
        translation: [0.; 3],
        rotation: [0., 0., 0., 1.],
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Censor {
    /// Stable host binding identity, not the original process pointer.
    pub binding: Option<u64>,
    pub mask: i32,
}
impl Censor {
    /// 58faf6 sign-extends person word 30, then expands bit 7 to all seven
    /// censor parts. Other high bits are retained when bit 7 is clear.
    pub fn normalized_mask(raw: i16) -> i32 {
        if raw & 0x80 != 0 {
            0x7f
        } else {
            i32::from(raw)
        }
    }

    /// One native binding service at a time. Replacement detaches the old
    /// instance before binding the new mask, even if no mesh matches it.
    pub fn next_request(&self, mask: i32) -> Option<Request> {
        if (mask == 0 || mask != self.mask) && self.binding.is_some() {
            Some(Request::UnbindCensor {
                binding: self.binding.unwrap(),
            })
        } else if mask != 0 && self.binding.is_none() {
            Some(Request::BindCensor { mask })
        } else {
            None
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    Viewport,
    Visibility {
        viewport: [i32; 4],
    },
    /// 59934b / 5a1103 propagation in the first visible model pass. The host
    /// owns appearance geometry, mesh stamps and local bounds.
    ModelGeometry,
    /// Copy the skeleton's local bounds to the model's bounds object (5a0fcf).
    ModelBounds,
    /// 5a1092 refreshes each appearance binding after saved head observation,
    /// before restoring the head and entering the ordinary forced refresh.
    LoadedAppearance,
    /// The final 58da12 call in 5918ba obtains a fresh clock. Earlier load-tail
    /// sampling uses the retained animator clock, not this new value.
    RefreshClock,
    /// 58f6eb..58f86a root-anchor correction, with the ordinary caller's saved
    /// entry flags. It may request another placement before completing.
    RootMotionCorrection,
    Invalidate,
    UnbindCensor {
        binding: u64,
    },
    BindCensor {
        mask: i32,
    },
    Property(properties::Request),
    Placement,
    /// Propagate the current absolute pose to appearance geometry/attachments.
    /// None is the full skeleton; a branch is the temporary HEAD subtree.
    /// Native mesh/bounds cache stamps are owned by that host, not this state.
    Geometry {
        branch: Option<usize>,
        force: bool,
    },
    HeadTarget {
        object: ObjectId,
    },
    /// Resolve the owner's current carried-object slot/anchor and execute its
    /// native dependent refresh if present. An empty slot completes directly.
    CarriedObject,
    /// Ordinary dependent update (virtual 50), unlike the special refresh.
    OrdinaryCarriedObject,
}
#[derive(Debug, Clone)]
pub enum Response {
    Done,
    Viewport([i32; 4]),
    Visible(bool),
    CensorBinding(Option<u64>),
    PropertySurface(i32),
    Placement(model::Matrix),
    HeadTarget(Option<Target>),
    Clock(i32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Viewport,
    Visibility,
    BeginModel,
    ModelSkin,
    ModelHierarchy,
    ModelGeometry,
    ModelBounds,
    ModelPublish,
    RootMotion,
    RootPlacement,
    Invalidate,
    Censor,
    BindCensor,
    BeginSkin,
    Skin,
    Placement,
    Hierarchy,
    Geometry,
    Head,
    HeadGeometry,
    Observe,
    LoadedAppearance,
    Restore,
    RestoreGeometry,
    Carried,
    FinalVisibility,
    RestartRefresh,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Pass {
    stage: Stage,
    callbacks: bool,
    censor_mask: Option<i32>,
    saved_head: Option<(usize, [f32; 4])>,
    target: Option<Target>,
    // Separate from target: a completed missing-object lookup must survive JSON
    // restoration without turning back into an unperformed lookup.
    target_resolved: bool,
    pending: Option<Request>,
    ordinary: bool,
    loaded: bool,
    only_visible: bool,
    viewport: [i32; 4],
    model_visible: bool,
    model_input: model::Matrix,
    model_output: Option<model::Matrix>,
    sample_pose: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Character {
    pub skin: Live,
    pub hierarchy: Hierarchy,
    pub head: State,
    pub rig: Rig,
    pub matrix: [f32; 16],
    pub placement: transform::Placement,
    pub model: model::Model,
    pub censor: Censor,
    pub clock: i32,
    pub visible: bool,
    pass: Option<Pass>,
}
impl Character {
    /// Animator +2c / 58933b assigns root compensation and calls 592132
    /// synchronously. Snap's preceding +20 reset is a separate operation.
    /// This publishes placement without sampling animation or advancing time.
    pub fn offset_placement(
        &mut self,
        offsets: [f32; 3],
        source: transform::Source,
        view: transform::View,
        terrain: transform::Terrain<'_>,
    ) -> Result<transform::Transform, String> {
        self.offset_placement_with_scale(offsets, source, view, terrain, None)
    }
    pub fn offset_placement_with_scale(
        &mut self,
        offsets: [f32; 3],
        source: transform::Source,
        view: transform::View,
        terrain: transform::Terrain<'_>,
        terrain_scale: Option<f32>,
    ) -> Result<transform::Transform, String> {
        if self.busy() {
            return Err("native animator offsets during a pending refresh".into());
        }
        let mut placement = self.placement.clone();
        placement.offset = offsets;
        let transform = placement.transform_with_control(
            source,
            view,
            terrain,
            self.skin.animator.fp_control(),
            terrain_scale,
        )?;
        let mut model = self.model.clone();
        let matrix = model.publish_with_control(
            model::Matrix {
                values: transform.matrix,
                kind: 0,
            },
            self.skin.animator.fp_control(),
        )?;
        self.skin.animator.placement_updated()?;
        self.placement = placement;
        self.model = model;
        self.matrix = matrix.values;
        Ok(transform)
    }

    /// The external service is part of a saved native continuation. Observing it
    /// never advances the layer clocks or repeats a property callback.
    pub fn pending_request(&self) -> Option<&Request> {
        self.pass.as_ref().and_then(|p| p.pending.as_ref())
    }

    pub fn validate(&self, program: &Program) -> Result<(), String> {
        self.hierarchy.validate(&self.skin)?;
        self.head.validate()?;
        for anchor in [
            self.rig.head,
            self.rig.hand,
            self.rig.pelvis,
            self.rig.forward,
        ]
        .into_iter()
        .flatten()
        .map(|a| a.bone)
        .chain(self.rig.turn_bone)
        {
            if anchor >= self.skin.bones.len() {
                return Err("native character rig bone absent".into());
            }
        }
        if self
            .matrix
            .iter()
            .chain(self.placement.base.iter())
            .chain(self.placement.previous.iter())
            .chain(self.placement.offset.iter())
            .chain(self.model.placement.values.iter())
            .chain(self.model.scale.values.iter())
            .chain(self.model.bounds_transform.values.iter())
            .any(|v| !v.is_finite())
        {
            return Err("invalid saved native character transform".into());
        }
        for owned in self.skin.animator.layers() {
            let layer = &owned.layer;
            layer.clock.validate()?;
            let asset = program
                .animations
                .get(&layer.name)
                .ok_or("saved native character clip absent")?;
            if layer
                .bindings
                .iter()
                .any(|b| b.bone >= self.skin.bones.len() || b.motion >= asset.clip.motions.len())
            {
                return Err("saved native character layer binding absent".into());
            }
        }
        Ok(())
    }
    pub fn new(
        skin: Live,
        hierarchy: Hierarchy,
        head: State,
        rig: Rig,
        matrix: [f32; 16],
        placement: transform::Placement,
        model: model::Model,
        censor: Censor,
        clock: i32,
    ) -> Result<Self, String> {
        hierarchy.validate(&skin)?;
        head.validate()?;
        if matrix.iter().any(|v| !v.is_finite()) {
            return Err("invalid native placement matrix".into());
        }
        for bone in [rig.head, rig.hand, rig.pelvis, rig.forward]
            .into_iter()
            .flatten()
            .map(|a| a.bone)
            .chain(rig.turn_bone)
        {
            if bone >= skin.bones.len() {
                return Err("native character rig bone absent".into());
            }
        }
        Ok(Self {
            skin,
            hierarchy,
            head,
            rig,
            matrix,
            placement,
            model,
            censor,
            clock,
            visible: false,
            pass: None,
        })
    }
    pub fn busy(&self) -> bool {
        self.pass.is_some() || self.skin.busy()
    }
    /// Consume the ordinary root service with its actual entry/live inputs.
    /// A correction requests placement before invalidation and the second skin
    /// pass; it must not accidentally enter that pass's hierarchy/head tail.
    pub fn correct_root(&mut self, source: super::root_motion::Source) -> Result<(), String> {
        if self.pass.as_ref().and_then(|p| p.pending.as_ref())
            != Some(&Request::RootMotionCorrection)
        {
            return Err("native root correction is not pending".into());
        }
        let mut matrix = self.model.placement;
        matrix.kind = 0;
        let corrected = self.skin.animator.correct_root(
            &self.hierarchy.absolute,
            &mut self.placement,
            matrix,
            source,
        )?;
        let pass = self.pass.as_mut().unwrap();
        pass.pending = None;
        pass.stage = if corrected {
            Stage::RootPlacement
        } else {
            Stage::Invalidate
        };
        Ok(())
    }
    /// Complete placement using current physical storage and view/terrain data.
    /// The original cached position survives refreshes and Rust snapshots.
    pub fn place(
        &mut self,
        source: transform::Source,
        view: transform::View,
        terrain: transform::Terrain<'_>,
    ) -> Result<transform::Transform, String> {
        self.place_with_scale(source, view, terrain, None)
    }
    pub fn place_with_scale(
        &mut self,
        source: transform::Source,
        view: transform::View,
        terrain: transform::Terrain<'_>,
        terrain_scale: Option<f32>,
    ) -> Result<transform::Transform, String> {
        if self.pass.as_ref().and_then(|p| p.pending.as_ref()) != Some(&Request::Placement) {
            return Err("native placement service is not pending".into());
        }
        let transform = self.placement.transform_with_control(
            source,
            view,
            terrain,
            self.skin.animator.fp_control(),
            terrain_scale,
        )?;
        self.acknowledge(Response::Placement(model::Matrix {
            values: transform.matrix,
            kind: 0,
        }))?;
        Ok(transform)
    }
    /// Native +0x109 is the visibility state. A false visibility state suppresses
    /// the entire only-visible refresh; an unconditional call always samples.
    pub fn begin(
        &mut self,
        now: i32,
        only_visible: bool,
        visible: bool,
        callbacks: bool,
    ) -> Result<bool, String> {
        if self.busy() {
            return Err("native character refresh already pending".into());
        }
        self.visible = visible;
        if only_visible && !visible {
            return Ok(false);
        }
        self.clock = now;
        self.skin.animator.discard_exhausted_blend()?;
        self.pass = Some(Pass {
            stage: Stage::Invalidate,
            callbacks,
            censor_mask: None,
            saved_head: None,
            target: None,
            target_resolved: false,
            pending: None,
            ordinary: false,
            loaded: false,
            only_visible,
            viewport: [0; 4],
            model_visible: false,
            model_input: model::Matrix::IDENTITY,
            model_output: None,
            sample_pose: true,
        });
        Ok(true)
    }
    /// Run 5918ba's read-side tail for save versions >60, after restoring
    /// appearance, descriptors, movement mode, head values and placement offset.
    /// The initial skin/head pass uses the existing clock without blend cleanup;
    /// the nested forced refresh obtains its own clock only after that pass.
    pub fn begin_loaded_tail(&mut self, callbacks: bool) -> Result<(), String> {
        if self.busy() {
            return Err("native character update already pending".into());
        }
        self.pass = Some(Pass {
            stage: Stage::Invalidate,
            callbacks,
            censor_mask: None,
            saved_head: None,
            target: None,
            target_resolved: false,
            pending: None,
            ordinary: false,
            loaded: true,
            only_visible: false,
            viewport: [0; 4],
            model_visible: false,
            model_input: model::Matrix::IDENTITY,
            model_output: None,
            sample_pose: true,
        });
        Ok(())
    }
    /// Start at 58f688 after the ordinary caller has obtained its clock,
    /// retired an exhausted blend, selected carry/base layers and advanced
    /// movement. `matrix` is the actor's current +38 transform at that boundary.
    /// No prefix service is silently supplied by this entry point.
    pub fn begin_ordinary_tail(
        &mut self,
        now: i32,
        only_visible: bool,
        callbacks: bool,
        matrix: model::Matrix,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("native character update already pending".into());
        }
        self.clock = now;
        self.pass = Some(Pass {
            stage: Stage::Viewport,
            callbacks,
            censor_mask: None,
            saved_head: None,
            target: None,
            target_resolved: false,
            pending: None,
            ordinary: true,
            loaded: false,
            only_visible,
            viewport: [0; 4],
            model_visible: !only_visible,
            model_input: model::Matrix { kind: 0, ..matrix },
            model_output: None,
            sample_pose: true,
        });
        Ok(())
    }
    pub fn next(
        &mut self,
        program: &Program,
        person: &mut BTreeMap<i16, i16>,
        context: properties::Context,
        level: i32,
    ) -> Result<Option<Request>, String> {
        loop {
            let Some(pass) = &mut self.pass else {
                return Ok(None);
            };
            if let Some(request) = &pass.pending {
                return Ok(Some(request.clone()));
            }
            let request = match pass.stage {
                Stage::Viewport => Some(Request::Viewport),
                Stage::Visibility | Stage::FinalVisibility => Some(Request::Visibility {
                    viewport: pass.viewport,
                }),
                Stage::BeginModel => {
                    pass.model_output =
                        Some(self.model.prepare_with_control(
                            pass.model_input,
                            self.skin.animator.fp_control(),
                        )?);
                    if pass.model_visible {
                        self.skin.begin(self.clock, true, pass.callbacks, false)?;
                        pass.stage = Stage::ModelSkin;
                    } else {
                        pass.stage = Stage::ModelPublish;
                    }
                    continue;
                }
                Stage::ModelHierarchy => {
                    self.hierarchy.update(&mut self.skin, false)?;
                    pass.stage = Stage::ModelGeometry;
                    continue;
                }
                Stage::ModelGeometry => Some(Request::ModelGeometry),
                Stage::ModelBounds => Some(Request::ModelBounds),
                Stage::LoadedAppearance => Some(Request::LoadedAppearance),
                Stage::RestartRefresh => Some(Request::RefreshClock),
                Stage::ModelPublish => {
                    self.skin.animator.placement_updated()?;
                    let transform = pass
                        .model_output
                        .take()
                        .ok_or("native model output absent")?;
                    self.model.finish(transform);
                    self.matrix = transform.values;
                    self.visible = pass.model_visible;
                    pass.stage = Stage::RootMotion;
                    continue;
                }
                Stage::RootMotion => Some(Request::RootMotionCorrection),
                Stage::RootPlacement => Some(Request::Placement),
                Stage::Invalidate => Some(Request::Invalidate),
                Stage::Censor => {
                    let mask = match pass.censor_mask {
                        Some(mask) => mask,
                        None => {
                            let raw =
                                i32::from(*person.get(&30).ok_or("native censor field absent")?);
                            if pass.ordinary {
                                pass.sample_pose =
                                    self.visible || !pass.only_visible || self.censor.mask != raw;
                            }
                            let mask = Censor::normalized_mask(raw as i16);
                            pass.censor_mask = Some(mask);
                            mask
                        }
                    };
                    match self.censor.next_request(mask) {
                        Some(request @ Request::UnbindCensor { .. }) => Some(request),
                        Some(Request::BindCensor { .. }) => {
                            pass.stage = Stage::BindCensor;
                            continue;
                        }
                        None => {
                            pass.stage = Stage::BeginSkin;
                            continue;
                        }
                        _ => unreachable!(),
                    }
                }
                Stage::BindCensor => Some(Request::BindCensor {
                    mask: pass.censor_mask.unwrap(),
                }),
                Stage::BeginSkin => {
                    self.skin.begin(
                        self.clock,
                        pass.sample_pose,
                        pass.callbacks,
                        !pass.ordinary,
                    )?;
                    pass.stage = Stage::Skin;
                    continue;
                }
                Stage::Skin | Stage::ModelSkin => {
                    let request = self.skin.next(program, person, context)?;
                    for (bone, delta) in self.skin.animator.take_bone_generation_deltas() {
                        let stamp = self
                            .hierarchy
                            .updates
                            .get_mut(bone)
                            .ok_or("native bone update index absent")?;
                        *stamp = stamp.wrapping_add(delta);
                    }
                    if let Some(request) = request {
                        Some(Request::Property(request))
                    } else {
                        pass.stage = if pass.stage == Stage::ModelSkin {
                            Stage::ModelHierarchy
                        } else {
                            Stage::Placement
                        };
                        continue;
                    }
                }
                Stage::Placement => Some(Request::Placement),
                Stage::Hierarchy => {
                    self.hierarchy.update(&mut self.skin, !pass.ordinary)?;
                    pass.stage = Stage::Geometry;
                    continue;
                }
                Stage::Geometry => Some(Request::Geometry {
                    branch: None,
                    force: !pass.ordinary,
                }),
                Stage::Head => {
                    let mut fields = fields(person)?;
                    if self.head.advance(self.clock as u32, &mut fields)? {
                        let bone = self.rig.turn_bone.ok_or("native HEAD bone absent")?;
                        let saved = self.skin.bones[bone].local.rotation;
                        let rotation = head_look::compose(self.head.turn, saved)?
                            .ok_or("native head composition absent")?;
                        self.skin.bones[bone].local.rotation = rotation;
                        self.hierarchy.update_branch(&self.skin, bone)?;
                        pass.saved_head = Some((bone, saved));
                    }
                    write_fields(person, fields);
                    pass.stage = if pass.saved_head.is_some() {
                        Stage::HeadGeometry
                    } else {
                        Stage::Observe
                    };
                    continue;
                }
                Stage::HeadGeometry | Stage::RestoreGeometry => Some(Request::Geometry {
                    branch: Some(pass.saved_head.unwrap().0),
                    force: false,
                }),
                Stage::Observe => {
                    let mut fields = fields(person)?;
                    let has_basis = self.rig.head.is_some() && self.rig.forward.is_some();
                    if has_basis
                        && fields.flags & 1 != 0
                        && fields.target != 0
                        && !pass.target_resolved
                    {
                        Some(Request::HeadTarget {
                            object: fields.target,
                        })
                    } else {
                        let target = pass.target;
                        self.head.observe(
                            &mut fields,
                            level,
                            has_basis,
                            |_| Ok(target),
                            || {
                                let point = |anchor: head_look::rig::Anchor| {
                                    self.hierarchy.absolute[anchor.bone]
                                        .anchor_point(anchor.kind, self.matrix)
                                };
                                Ok([
                                    point(self.rig.head.unwrap())?,
                                    point(self.rig.forward.unwrap())?,
                                ])
                            },
                        )?;
                        write_fields(person, fields);
                        pass.stage = if pass.loaded {
                            Stage::LoadedAppearance
                        } else {
                            Stage::Restore
                        };
                        continue;
                    }
                }
                Stage::Restore => {
                    if let Some((bone, rotation)) = pass.saved_head {
                        self.skin.bones[bone].local.rotation = rotation;
                        self.hierarchy.update_branch(&self.skin, bone)?;
                        pass.stage = Stage::RestoreGeometry;
                    } else {
                        pass.stage = if pass.loaded {
                            Stage::RestartRefresh
                        } else if pass.ordinary {
                            Stage::FinalVisibility
                        } else {
                            Stage::Carried
                        };
                    }
                    continue;
                }
                Stage::Carried => Some(if pass.ordinary {
                    Request::OrdinaryCarriedObject
                } else {
                    Request::CarriedObject
                }),
                Stage::Complete => {
                    self.pass = None;
                    return Ok(None);
                }
            };
            pass.pending = request.clone();
            return Ok(request);
        }
    }
    pub fn acknowledge(&mut self, response: Response) -> Result<(), String> {
        let pass = self
            .pass
            .as_mut()
            .ok_or("native character refresh absent")?;
        let request = pass
            .pending
            .as_ref()
            .ok_or("native character request absent")?;
        match (request, response) {
            (Request::Viewport, Response::Viewport(viewport)) => {
                pass.viewport = viewport;
                pass.stage = if pass.only_visible {
                    Stage::Visibility
                } else {
                    Stage::BeginModel
                };
            }
            (Request::Visibility { .. }, Response::Visible(visible)) => {
                self.visible = visible;
                if pass.stage == Stage::Visibility {
                    pass.model_visible = visible;
                    pass.stage = Stage::BeginModel;
                } else {
                    pass.stage = Stage::Carried;
                }
            }
            (Request::ModelGeometry, Response::Done) => pass.stage = Stage::ModelBounds,
            (Request::ModelBounds, Response::Done) => {
                self.model.bounds_flags &= !1;
                pass.stage = Stage::ModelPublish;
            }
            (Request::LoadedAppearance, Response::Done) => pass.stage = Stage::Restore,
            (Request::RefreshClock, Response::Clock(now)) => {
                self.skin.animator.discard_exhausted_blend()?;
                self.clock = now;
                pass.loaded = false;
                pass.censor_mask = None;
                pass.target = None;
                pass.target_resolved = false;
                pass.stage = Stage::Invalidate;
            }
            (Request::Invalidate, Response::Done) => pass.stage = Stage::Censor,
            (Request::UnbindCensor { .. }, Response::Done) => {
                self.censor.binding = None;
                self.censor.mask = 0;
                pass.stage = if pass.censor_mask == Some(0) {
                    Stage::BeginSkin
                } else {
                    Stage::BindCensor
                };
            }
            (Request::BindCensor { mask }, Response::CensorBinding(binding)) => {
                self.censor = Censor {
                    binding,
                    mask: *mask,
                };
                pass.stage = Stage::BeginSkin;
            }
            (
                Request::Property(properties::Request::FootstepSurface { .. }),
                Response::PropertySurface(surface),
            ) => self.skin.acknowledge(Some(surface))?,
            (Request::Property(request), Response::Done)
                if !matches!(request, properties::Request::FootstepSurface { .. }) =>
            {
                self.skin.acknowledge(None)?
            }
            (Request::Placement, Response::Placement(matrix)) => {
                let mut model = self.model.clone();
                let published =
                    model.publish_with_control(matrix, self.skin.animator.fp_control())?;
                self.skin.animator.placement_updated()?;
                self.model = model;
                self.matrix = published.values;
                pass.stage = if pass.stage == Stage::RootPlacement {
                    Stage::Invalidate
                } else {
                    Stage::Hierarchy
                };
            }
            (Request::Geometry { .. }, Response::Done) => {
                pass.stage = match pass.stage {
                    Stage::Geometry => Stage::Head,
                    Stage::HeadGeometry => Stage::Observe,
                    Stage::RestoreGeometry => {
                        pass.saved_head = None;
                        if pass.loaded {
                            Stage::RestartRefresh
                        } else if pass.ordinary {
                            Stage::FinalVisibility
                        } else {
                            Stage::Carried
                        }
                    }
                    _ => return Err("native geometry request has invalid stage".into()),
                };
            }
            (Request::HeadTarget { .. }, Response::HeadTarget(target)) => {
                pass.target = target;
                pass.target_resolved = true;
            }
            (Request::CarriedObject | Request::OrdinaryCarriedObject, Response::Done) => {
                pass.stage = Stage::Complete
            }
            _ => return Err("native character response does not match pending service".into()),
        }
        pass.pending = None;
        Ok(())
    }
    /// Presentation/attachment sampling can request the temporary head pose
    /// without advancing its clock or accumulating it into the base animation.
    pub fn displayed_pose(&self) -> Result<Vec<AbsoluteBone>, String> {
        if self.busy() {
            return Err("native character pose is mid-refresh".into());
        }
        let mut locals: Vec<LocalBone> = self.skin.bones.iter().map(|b| b.local).collect();
        if self.head.turn.abs() >= 0.01_f32 {
            let bone = self.rig.turn_bone.ok_or("native HEAD bone absent")?;
            if let Some(rotation) = head_look::compose(self.head.turn, locals[bone].rotation)? {
                locals[bone].rotation = rotation;
            }
        }
        hierarchy(&self.hierarchy.parents, &locals, origin())
    }
}
fn fields(person: &BTreeMap<i16, i16>) -> Result<Fields, String> {
    let get = |field| {
        person
            .get(&field)
            .copied()
            .ok_or_else(|| format!("native head field {field} absent"))
    };
    Ok(Fields {
        target: get(41)?,
        flags: get(42)?,
        mode: get(44)?,
        countdown: get(45)?,
    })
}
fn write_fields(person: &mut BTreeMap<i16, i16>, fields: Fields) {
    for (field, value) in [
        (41, fields.target),
        (42, fields.flags),
        (44, fields.mode),
        (45, fields.countdown),
    ] {
        person.insert(field, value);
    }
}
