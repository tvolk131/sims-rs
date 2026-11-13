//! Fresh SAnimator model initialization, 58831d. Resource lookup, physical
//! placement, visibility and external property effects are explicit services.
//! The head target remains undefined until a saved read or live target query
//! writes it; disabled tracking does not consume those coordinates.
use super::{
    animator::{Settings, Slot},
    construction, model,
    movement::{self, calibration},
    ordinary, properties, transform,
};
use crate::head_look::rig::Rig;
use serde::{Deserialize, Serialize};
use sims_formats::avatar::Skeleton;
use sims_vm::Program;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    NeedsAppearance {
        name: String,
    },
    /// 592625 clears +d8 and calls 5888d3 before the first resource lookup.
    /// The host must reload the profile and its actual appearance bindings.
    ReloadProfile,
    Animation {
        index: usize,
    },
    ZeroAltitude,
    Placement,
    Geometry {
        force: bool,
    },
    Property(properties::Request),
    Viewport,
    Visibility {
        viewport: [i32; 4],
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Needs,
    Profile,
    Distances(usize),
    Altitude,
    Base,
    Walk,
    BeginSkin(u8),
    Skin(u8),
    Place(u8),
    Geometry(u8),
    Finish,
    FinalPlace,
    Viewport,
    Visibility,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Initialization {
    pub source: Skeleton,
    pub bound: model::Bound,
    pub scalars: construction::State,
    pub ordinary: ordinary::State,
    pub movement: movement::Movement,
    pub rig: Rig,
    pub settings: Settings,
    pub needs_appearance: Option<String>,
    pub anchors: [Option<usize>; 2],
    pub distances: Option<calibration::Distances>,
    pub visible: Option<bool>,
    /// Native +284 becomes one before the final placement/visibility calls.
    pub initialized: bool,
    profile_name: String,
    stage: Stage,
    pending: Option<Request>,
    definition_flags: u8,
    raw_distances: [f32; 5],
    z: [[f32; 2]; 2],
}
impl Initialization {
    /// Caller has already performed 5888d3's profile writes. This fresh-object
    /// entry does not implement rebuilding an existing animated model.
    pub fn new(
        source: Skeleton,
        profile: &[String],
        tick: i32,
        definition_flags: u8,
        settings: Settings,
        crt: &mut u32,
    ) -> Result<Self, String> {
        let rig = Rig::bind(&source, profile)?;
        let profile_name = profile
            .first()
            .ok_or("initializer profile name absent")?
            .clone();
        let mut scalars = construction::State::new(tick);
        let mut bound = model::Bound::new(&source, crt)?;
        // +1ac is first defined by Reset. It is not consumed before this write.
        scalars.carry_selection = Some(-1);
        let mut ordinary = scalars.prefix()?;
        let mut movement = movement::Movement::new(scalars.movement.clone());
        let effects = super::reset::apply_storage(
            &mut bound.skeleton.skin,
            &mut scalars.placement,
            &mut ordinary,
            &mut movement,
            scalars.clock,
            settings,
        )?;
        bound.skeleton.geometry.generation = bound
            .skeleton
            .geometry
            .generation
            .wrapping_add(effects.geometry_generation_delta);
        debug_assert_eq!(effects.route_diagnostic, None);
        Ok(Self {
            source,
            bound,
            scalars,
            ordinary,
            movement,
            rig,
            settings,
            needs_appearance: None,
            anchors: [None; 2],
            distances: None,
            visible: None,
            initialized: false,
            profile_name,
            stage: Stage::Needs,
            pending: None,
            definition_flags,
            raw_distances: [0.; 5],
            z: [[0.; 2]; 2],
        })
    }
    pub fn is_complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn pending_request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn next(
        &mut self,
        program: &Program,
        person: &mut BTreeMap<i16, i16>,
        context: properties::Context,
    ) -> Result<Option<Request>, String> {
        if let Some(request) = &self.pending {
            return Ok(Some(request.clone()));
        }
        loop {
            let request = match self.stage {
                Stage::Needs => Request::NeedsAppearance {
                    name: format!("{}-needs", self.profile_name),
                },
                Stage::Profile => Request::ReloadProfile,
                Stage::Distances(i) => Request::Animation {
                    index: [19, 21, 20, 22, 23][i],
                },
                Stage::Altitude => Request::ZeroAltitude,
                Stage::Base => Request::Animation { index: 3 },
                Stage::Walk => Request::Animation { index: 20 },
                Stage::BeginSkin(sample) => {
                    if sample < 2 {
                        self.bound.skeleton.skin.animator.calibration_phase(
                            self.scalars.phase_window[usize::from(sample)],
                            sample == 0,
                        )?;
                    }
                    self.bound
                        .skeleton
                        .skin
                        .begin(self.scalars.clock, true, true, true)?;
                    self.stage = Stage::Skin(sample);
                    continue;
                }
                Stage::Skin(sample) => {
                    let request = self.bound.skeleton.skin.next(program, person, context)?;
                    for (bone, delta) in self
                        .bound
                        .skeleton
                        .skin
                        .animator
                        .take_bone_generation_deltas()
                    {
                        let stamp = self
                            .bound
                            .skeleton
                            .hierarchy
                            .updates
                            .get_mut(bone)
                            .ok_or("initialization bone stamp absent")?;
                        *stamp = stamp.wrapping_add(delta);
                    }
                    if let Some(request) = request {
                        Request::Property(request)
                    } else {
                        self.stage = if sample == 2 {
                            Stage::Finish
                        } else {
                            Stage::Place(sample)
                        };
                        continue;
                    }
                }
                Stage::Place(_) | Stage::FinalPlace => Request::Placement,
                Stage::Geometry(_) => Request::Geometry { force: false },
                Stage::Finish => {
                    self.bound.skeleton.skin.animator.finish_calibration()?;
                    self.scalars.selection = -1;
                    self.scalars.placement.dirty = true;
                    self.initialized = true;
                    self.stage = Stage::FinalPlace;
                    continue;
                }
                Stage::Viewport => Request::Viewport,
                Stage::Visibility => return Err("initializer visibility response absent".into()),
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request.clone());
            return Ok(Some(request));
        }
    }
    pub fn needs_appearance(&mut self, name: &str) -> Result<(), String> {
        if self.pending.as_ref()
            != Some(&Request::NeedsAppearance {
                name: name.to_owned(),
            })
        {
            return Err("initializer needs lookup does not match".into());
        }
        self.needs_appearance = Some(name.to_owned());
        // 597785 retains the appearance resource; it is not a mesh binding.
        // 588d6a visits the fresh empty accessory vector, then invalidates it.
        self.bound.refresh_empty_accessories();
        self.pending = None;
        self.stage = Stage::Profile;
        Ok(())
    }
    /// Acknowledges the host's 5888d3 service, including appearance rebuilding.
    /// Reloading only person words is insufficient for a live household.
    pub fn profile_reloaded(&mut self) -> Result<(), String> {
        if self.pending != Some(Request::ReloadProfile) {
            return Err("initializer profile reload not pending".into());
        }
        self.pending = None;
        self.stage = Stage::Distances(0);
        Ok(())
    }
    pub fn animation(&mut self, name: Option<&str>, program: &Program) -> Result<(), String> {
        if !matches!(self.pending, Some(Request::Animation { .. })) {
            return Err("initializer animation lookup not pending".into());
        }
        let clip = name
            .map(|name| {
                program
                    .animations
                    .get(name)
                    .map(|a| &a.clip)
                    .ok_or("initialization animation is not loaded")
            })
            .transpose()?;
        match self.stage {
            Stage::Distances(i) => {
                self.raw_distances[i] = clip.map_or(0., |clip| clip.distance);
                if i == 4 {
                    let d = calibration::Distances::new(self.raw_distances)?;
                    self.scalars.clip_distances = [
                        d.start_distance[0],
                        d.lookahead[1],
                        d.lookahead[0],
                        d.start_distance[1],
                        d.special_lookahead,
                        d.short_lookahead,
                    ];
                    self.distances = Some(d);
                    self.stage = Stage::Altitude;
                } else {
                    self.stage = Stage::Distances(i + 1);
                }
            }
            Stage::Base => {
                self.bound.skeleton.skin.animator.start(
                    Slot::Base,
                    &self.source,
                    clip,
                    false,
                    self.scalars.clock,
                    self.settings,
                )?;
                self.stage = if self.definition_flags & 2 == 0 {
                    Stage::Walk
                } else {
                    Stage::BeginSkin(2)
                };
            }
            Stage::Walk => {
                let clip = clip.ok_or("initializer walking clip absent")?;
                let a = [
                    calibration::property(clip, "anchor", "1")?
                        .ok_or("initialization anchor 1 absent")?,
                    calibration::property(clip, "anchor", "2")?
                        .ok_or("initialization anchor 2 absent")?,
                ];
                let anchors = a.map(|(motion, _)| {
                    self.source
                        .bones
                        .iter()
                        .position(|b| b.name == clip.motions[motion].bone)
                });
                if anchors.iter().any(Option::is_none) {
                    return Err("initialization anchor bone absent".into());
                }
                let phases = calibration::phases(a.map(|(_, time)| time), clip.duration_ms)?;
                self.bound
                    .skeleton
                    .skin
                    .animator
                    .finish_foreground_selection(
                        20,
                        0,
                        &self.source,
                        Some(clip),
                        self.scalars.clock,
                        self.settings,
                    )?;
                self.scalars.selection = 20;
                self.anchors = anchors;
                self.scalars.phase_window = phases;
                self.stage = Stage::BeginSkin(0);
            }
            _ => return Err("initializer animation stage mismatch".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn altitude_written(&mut self) -> Result<(), String> {
        if self.pending != Some(Request::ZeroAltitude) {
            return Err("initializer altitude not pending".into());
        }
        self.pending = None;
        self.stage = Stage::Base;
        Ok(())
    }
    pub fn property_done(&mut self, surface: Option<i32>) -> Result<(), String> {
        if !matches!(self.pending, Some(Request::Property(_))) {
            return Err("initializer property not pending".into());
        }
        if matches!(
            self.pending,
            Some(Request::Property(
                properties::Request::Dress { .. } | properties::Request::Undress { .. }
            ))
        ) {
            return Err("initializer appearance callback needs nonempty model geometry".into());
        }
        self.bound.skeleton.skin.acknowledge(surface)?;
        self.pending = None;
        Ok(())
    }
    pub fn place(
        &mut self,
        source: transform::Source,
        view: transform::View,
        terrain: transform::Terrain<'_>,
    ) -> Result<transform::Transform, String> {
        if self.pending != Some(Request::Placement) {
            return Err("initializer placement not pending".into());
        }
        let transform = self.scalars.placement.transform(source, view, terrain)?;
        self.bound.publish(model::Matrix {
            values: transform.matrix,
            kind: 0,
        })?;
        match self.stage {
            Stage::Place(sample) => {
                let s = &mut self.bound.skeleton;
                s.hierarchy.update(&mut s.skin, false)?;
                self.stage = Stage::Geometry(sample);
            }
            Stage::FinalPlace => self.stage = Stage::Viewport,
            _ => return Err("initializer placement stage mismatch".into()),
        }
        self.pending = None;
        Ok(transform)
    }
    /// The host has executed 5a15bc for the model's actual appearance bindings.
    pub fn geometry_done(&mut self) -> Result<(), String> {
        if self.pending != Some(Request::Geometry { force: false }) {
            return Err("initializer geometry not pending".into());
        }
        let Stage::Geometry(sample) = self.stage else {
            return Err("initializer geometry stage mismatch".into());
        };
        self.z[usize::from(sample)] = self
            .anchors
            .map(|bone| self.bound.skeleton.hierarchy.absolute[bone.unwrap()].translation[2]);
        self.stage = if sample == 0 {
            Stage::BeginSkin(1)
        } else {
            self.scalars.stop_distance = calibration::stop_distances(self.z)?;
            Stage::Finish
        };
        self.pending = None;
        Ok(())
    }
    pub fn viewport(&mut self, viewport: [i32; 4]) -> Result<(), String> {
        if self.pending != Some(Request::Viewport) {
            return Err("initializer viewport not pending".into());
        }
        self.stage = Stage::Visibility;
        self.pending = Some(Request::Visibility { viewport });
        Ok(())
    }
    pub fn visibility(&mut self, visible: bool) -> Result<(), String> {
        if !matches!(self.pending, Some(Request::Visibility { .. })) {
            return Err("initializer visibility not pending".into());
        }
        self.visible = Some(visible);
        self.stage = Stage::Done;
        self.pending = None;
        Ok(())
    }
}
