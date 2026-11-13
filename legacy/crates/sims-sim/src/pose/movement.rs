//! Ordinary 58dc72 movement transitions, sharing the real animator and placement.
//! Path-following states require an explicit native path and runtime state.
//! Resource, base-posture and placement services retain resumable call boundaries.
use super::{
    animator::{Animator, LayerId, Settings as LayerSettings, Slot},
    ordinary::{self, Entry, RateSettings},
    transform::Placement,
};
use serde::{Deserialize, Serialize};
use sims_formats::{animation::Animation, avatar::Skeleton};
use sims_vm::Program;
pub mod calibration;
pub mod path;
pub mod route;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// +188, +19c/+290, +1d0..+1d8, +1dc..+1e4, +1f0/+1f4, +288, +29c.
    pub status: i32,
    pub flags: [i32; 2],
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub turn_delta: f32,
    pub nominal_turn: f32,
    pub reason: i32,
    pub apply_rate: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Explicit native configuration: +6d07b0, +bc, +c4, +c8, +cc, +e0, +e4.
    pub short_distance: f32,
    pub turn_rate: f32,
    pub small_turn: f32,
    pub medium_turn: f32,
    pub large_turn: f32,
    pub minimum_weight: f32,
    pub snap_distance: f32,
    pub rate: RateSettings,
}
#[derive(Debug, Clone, Copy)]
pub struct Source {
    pub species: i16,
    pub person78: i16,
    pub walk_style: i16,
    pub object9: i16,
    pub level: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Foreground { index: i32 },
    RootDisplacement { index: i32 },
    BasePosture,
    Placement,
    StopForeground { index: i32 },
    AcceptStep { position: [i32; 2], level: i32 },
    MoveStep { position: [i32; 2], level: i32 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Continuation {
    Tail { now: i32, entry: Entry },
    Path { now: i32, step: u32, entry: Entry },
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
enum Selection {
    Turn,
    Short {
        weight: f32,
    },
    Start,
    Walk {
        phase: f32,
        direction: [f32; 2],
        square: f32,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
enum Stage {
    Dispatch,
    Displacement {
        index: i32,
        angle: f32,
        distance: f32,
    },
    Select {
        index: i32,
        kind: Selection,
    },
    Selected(Selection),
    Base,
    Placement,
    StopSelect,
    Physical {
        direction: [f32; 2],
        square: f32,
        after: bool,
    },
    Accept {
        position: [i32; 2],
        level: i32,
        visual: [f32; 2],
        after: bool,
    },
    Move {
        position: [i32; 2],
        level: i32,
        visual: [f32; 2],
        after: bool,
    },
    Done,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Pass {
    tick: i32,
    now: i32,
    step: u32,
    entry: Entry,
    root_motion: u8,
    stage: Stage,
    pending: Option<Request>,
    continuation: Option<Continuation>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Movement {
    pub state: State,
    pub path: Option<path::Path>,
    /// Animator +7c; 59209f also publishes this scalar * 256 to object data 22.
    #[serde(default = "route::initial_scalar_7c")]
    pub scalar_7c: f32,
    pass: Option<Pass>,
}

fn foreground(animator: &Animator) -> Result<LayerId, String> {
    animator
        .slot(Slot::Foreground)
        .ok_or("native movement requires foreground layer".into())
}
fn speed(animator: &mut Animator, value: f32) -> Result<(), String> {
    let id = foreground(animator)?;
    animator.get_mut(id)?.set_speed(value)
}
fn normalized(mut value: f64, minimum: f64) -> Result<f64, String> {
    if !value.is_finite() || value.abs() > 1_000_000. {
        return Err("native heading normalization exceeds diagnostic budget".into());
    }
    while value < minimum {
        value += 8.;
    }
    while value >= minimum + 8. {
        value -= 8.;
    }
    Ok(value)
}
fn heading(placement: &mut Placement, facing: &mut i16, value: f32) -> Result<(), String> {
    let rounded = (f64::from(value) + 0.5).trunc();
    if !rounded.is_finite() || rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
        return Err("native heading integer conversion overflow".into());
    }
    // 58488b and 5920e8: the visual heading remains fractional/unwrapped.
    placement.base[2] = value;
    *facing = (rounded as i32).rem_euclid(8) as i16;
    Ok(())
}
fn interpolate(phase: f64, other: f64, start: f32, target: f32) -> f32 {
    (other * f64::from(start) + phase * f64::from(target)) as f32
}
impl Movement {
    pub fn pending_request(&self) -> Option<Request> {
        self.pass.as_ref().and_then(|pass| pass.pending)
    }
    pub fn busy(&self) -> bool {
        self.pass.is_some()
    }
    pub fn new(state: State) -> Self {
        Self {
            state,
            path: None,
            scalar_7c: route::initial_scalar_7c(),
            pass: None,
        }
    }
    pub fn begin(&mut self, tick: i32, continuation: ordinary::Continuation) -> Result<(), String> {
        if self.pass.is_some() {
            return Err("native movement already pending".into());
        }
        let ordinary::Continuation::Movement { now, step, entry } = continuation else {
            return Err("ordinary prefix did not request movement".into());
        };
        self.pass = Some(Pass {
            tick,
            now,
            step,
            entry,
            root_motion: entry.root_motion,
            stage: Stage::Dispatch,
            pending: None,
            continuation: None,
        });
        Ok(())
    }
    fn settle(&mut self, animator: &mut Animator) -> Result<(), String> {
        self.state.flags = [0; 2];
        animator.clear_anchor()
    }
    fn tail(&mut self) {
        let pass = self.pass.as_mut().unwrap();
        pass.stage = Stage::Done;
        pass.continuation = Some(Continuation::Tail {
            now: pass.now,
            entry: Entry {
                root_motion: pass.root_motion,
                ..pass.entry
            },
        });
    }
    fn finish_transition(
        &mut self,
        animator: &mut Animator,
        placement: &mut Placement,
        ordinary: &mut ordinary::State,
        facing: &mut i16,
        layers: LayerSettings,
    ) -> Result<(), String> {
        let pass = self.pass.as_ref().unwrap();
        animator.clear_foreground(ordinary::clock(pass.tick, ordinary.clock_mode), layers)?;
        ordinary.mode = 3;
        heading(placement, facing, self.state.end[2])?;
        placement.previous[2] = placement.base[2];
        for i in 0..2 {
            placement.base[i] = self.state.end[i];
            placement.previous[i] = self.state.end[i];
        }
        Ok(())
    }
    pub fn next(
        &mut self,
        animator: &mut Animator,
        placement: &mut Placement,
        ordinary: &mut ordinary::State,
        facing: &mut i16,
        source: Source,
        settings: Settings,
        layers: LayerSettings,
        program: &Program,
    ) -> Result<Option<Request>, String> {
        if animator.busy() {
            return Err("native movement during skin traversal".into());
        }
        // Each iteration is a native same-tick state transition, not a game tick.
        for _ in 0..1024 {
            let pass = self.pass.as_ref().ok_or("native movement absent")?;
            if let Some(request) = pass.pending {
                return Ok(Some(request));
            }
            let request = match pass.stage {
                Stage::Done => return Ok(None),
                Stage::Displacement { index, .. } => Request::RootDisplacement { index },
                Stage::Placement => Request::Placement,
                Stage::Base => Request::BasePosture,
                Stage::StopSelect => Request::StopForeground { index: 19 },
                Stage::Accept {
                    position, level, ..
                } => Request::AcceptStep { position, level },
                Stage::Move {
                    position, level, ..
                } => Request::MoveStep { position, level },
                Stage::Physical {
                    direction,
                    square,
                    after,
                } => {
                    self.physical_step(
                        animator,
                        placement,
                        source.level,
                        program,
                        direction,
                        square,
                        after,
                    )?;
                    continue;
                }
                Stage::Select { index, kind } => {
                    if animator.needs_selection(index) {
                        Request::Foreground { index }
                    } else {
                        self.pass.as_mut().unwrap().stage = Stage::Selected(kind);
                        continue;
                    }
                }
                Stage::Selected(kind) => {
                    if let Selection::Walk {
                        phase,
                        direction,
                        square,
                    } = kind
                    {
                        speed(animator, ordinary.rate)?;
                        let id = foreground(animator)?;
                        animator.get_mut(id)?.clock.phase = phase;
                        self.pass.as_mut().unwrap().stage = Stage::Physical {
                            direction,
                            square,
                            after: false,
                        };
                        continue;
                    }
                    if kind == Selection::Start {
                        let rate = settings.rate.target(
                            source.walk_style,
                            source.species,
                            ordinary.mode,
                        )? as f32;
                        ordinary.rate = rate;
                        ordinary.target_rate = rate;
                        speed(animator, rate)?;
                        let layer = animator.layer(foreground(animator)?).unwrap();
                        let clip = &program
                            .animations
                            .get(&layer.name)
                            .ok_or_else(|| format!("missing native movement clip {}", layer.name))?
                            .clip;
                        let distance = f64::from(clip.distance) * f64::from(0.33333334_f32) * 16.;
                        let angle = (f64::from(placement.base[2]) + 6.)
                            * 45.
                            * f64::from(std::f32::consts::PI)
                            * f64::from(0.0055555557_f32);
                        // Native spills cosine, while sine and distance stay extended.
                        let cosine = angle.cos() as f32;
                        self.state.start[0] = placement.base[0];
                        self.state.start[1] = placement.base[1];
                        placement.previous[0] =
                            (f64::from(cosine) * distance + f64::from(placement.base[0])) as f32;
                        placement.previous[1] =
                            (angle.sin() * distance + f64::from(placement.base[1])) as f32;
                        self.pass.as_mut().unwrap().stage = Stage::Dispatch;
                        continue;
                    }
                    speed(animator, ordinary.rate)?;
                    let id = foreground(animator)?;
                    match kind {
                        Selection::Turn => {
                            animator.set_priority(id, 5)?;
                            ordinary.mode = 4;
                        }
                        Selection::Short { weight } => {
                            animator.get_mut(id)?.clock.weight = weight;
                            ordinary.mode = 5;
                        }
                        Selection::Start => unreachable!(),
                        Selection::Walk { .. } => unreachable!(),
                    }
                    self.tail();
                    continue;
                }
                Stage::Dispatch => {
                    match ordinary.mode {
                        0 => {
                            if self.state.apply_rate != 0 {
                                let old =
                                    animator.layer(foreground(animator)?).unwrap().clock.speed;
                                speed(
                                    animator,
                                    if old < 0. {
                                        -ordinary.rate
                                    } else {
                                        ordinary.rate
                                    },
                                )?;
                            }
                            self.tail();
                            continue;
                        }
                        1 | 2 => {
                            if animator.slot(Slot::Foreground).is_some()
                                && (ordinary.mode == 2 || source.person78 == 0)
                            {
                                animator.clear_foreground(
                                    ordinary::clock(pass.tick, ordinary.clock_mode),
                                    layers,
                                )?;
                                ordinary.base_posture = -1;
                                self.pass.as_mut().unwrap().stage = Stage::Base;
                            } else {
                                self.settle(animator)?;
                                self.tail();
                            }
                            continue;
                        }
                        3 => {
                            if placement.base.contains(&-999.) {
                                self.pass.as_mut().unwrap().stage = Stage::Placement;
                                continue;
                            }
                            placement.previous[2] =
                                normalized(f64::from(placement.previous[2]), 0.)? as f32;
                            placement.base[2] =
                                normalized(f64::from(placement.base[2]), 0.)? as f32;
                            let mut delta =
                                f64::from(placement.previous[2]) - f64::from(placement.base[2]);
                            let dx = (f64::from(placement.previous[0])
                                - f64::from(placement.base[0]))
                                as f32;
                            let dy = (f64::from(placement.previous[1])
                                - f64::from(placement.base[1]))
                                as f32;
                            let epsilon = f64::from(0.000001_f32);
                            let same =
                                f64::from(dx).abs() <= epsilon && f64::from(dy).abs() <= epsilon;
                            if delta.abs() <= epsilon && same {
                                match self.state.status {
                                    0 => (),
                                    -1 | 2 | 3 => self.state.status = 2,
                                    _ => {
                                        ordinary.mode = 9;
                                        continue;
                                    }
                                }
                                self.settle(animator)?;
                                self.tail();
                                continue;
                            }
                            let distance = if same {
                                0.
                            } else {
                                (f64::from(dx).powi(2) + f64::from(dy).powi(2)).sqrt() as f32
                            };
                            delta = if delta.abs() <= epsilon {
                                0.
                            } else {
                                normalized(delta, -4.)?
                            };
                            let pet = source.species > 1;
                            let turn = f64::from(settings.turn_rate)
                                * if pet { 0.0625 } else { 0.25 }
                                / f64::from(ordinary.rate);
                            let snap = if pet {
                                settings.snap_distance * 0.2
                            } else {
                                settings.snap_distance
                            };
                            let short = if pet {
                                settings.short_distance * 0.083333336
                            } else {
                                settings.short_distance
                            };
                            if -turn < delta && delta < turn {
                                if distance < snap {
                                    placement.base[0] = placement.previous[0];
                                    placement.base[1] = placement.previous[1];
                                    heading(placement, facing, placement.previous[2])?;
                                    continue;
                                }
                                if distance < short {
                                    let angle = f64::from(dy).atan2(f64::from(dx));
                                    let turns =
                                        angle / (f64::from(std::f32::consts::PI) * 2.) + 0.0625;
                                    let sector = ((f64::from(turns as f32) - turns.floor()) * 8.)
                                        .floor()
                                        as i32;
                                    let index = [15, 13, 11, 12, 14, 16, 18, 17]
                                        .get(sector as usize)
                                        .copied()
                                        .unwrap_or(-1);
                                    self.pass.as_mut().unwrap().stage = Stage::Displacement {
                                        index,
                                        angle: angle as f32,
                                        distance,
                                    };
                                    continue;
                                }
                            }
                            let (index, nominal) = if delta < -f64::from(settings.large_turn) {
                                (5, -4.)
                            } else if delta > f64::from(settings.large_turn) {
                                (4, 4.)
                            } else if delta < -f64::from(settings.medium_turn) {
                                (7, -2.)
                            } else if delta < -f64::from(settings.small_turn) {
                                (9, -1.)
                            } else if delta > f64::from(settings.medium_turn) {
                                (6, 2.)
                            } else if delta > f64::from(settings.small_turn) {
                                (8, 1.)
                            } else {
                                (10, 0.)
                            };
                            self.state.turn_delta = delta as f32;
                            self.state.nominal_turn = nominal;
                            self.state.end = placement.previous;
                            self.state.start = placement.base;
                            placement.previous[2] = (f64::from(self.state.turn_delta)
                                - f64::from(nominal)
                                + f64::from(placement.base[2]))
                                as f32;
                            self.pass.as_mut().unwrap().stage = Stage::Select {
                                index,
                                kind: Selection::Turn,
                            };
                            continue;
                        }
                        4 | 5 => {
                            self.settle(animator)?;
                            let id = foreground(animator)?;
                            let phase = animator.layer(id).unwrap().clock.phase;
                            if phase < 1. {
                                let other = (1. - f64::from(phase)) as f32;
                                // Heading uses the still-extended complement; XY reloads its spill.
                                let value = interpolate(
                                    f64::from(phase),
                                    1. - f64::from(phase),
                                    self.state.start[2],
                                    placement.previous[2],
                                );
                                heading(placement, facing, value)?;
                                for i in 0..2 {
                                    placement.base[i] = interpolate(
                                        f64::from(phase),
                                        f64::from(other),
                                        self.state.start[i],
                                        placement.previous[i],
                                    );
                                }
                                speed(animator, ordinary.rate)?;
                                self.tail();
                                continue;
                            }
                            if ordinary.mode == 4 {
                                let layer = animator.get_mut(id)?;
                                layer.translation_policy = -1;
                                layer.rotation_policy = -1;
                                animator.set_promote_next_foreground(true)?;
                            } else {
                                animator.get_mut(id)?.clock.weight = 0.;
                            }
                            self.finish_transition(animator, placement, ordinary, facing, layers)?;
                            continue;
                        }
                        6 | 11 => {
                            let index = if ordinary.mode == 6 { 19 } else { 22 };
                            if animator.selection() != index {
                                self.pass.as_mut().unwrap().stage = Stage::Select {
                                    index,
                                    kind: Selection::Start,
                                };
                                continue;
                            }
                            self.state.flags = [0; 2];
                            speed(animator, ordinary.rate)?;
                            let phase = animator.layer(foreground(animator)?).unwrap().clock.phase;
                            if phase >= 0.95_f32 {
                                animator.clear_foreground(
                                    ordinary::clock(pass.tick, ordinary.clock_mode),
                                    layers,
                                )?;
                                placement.base[..2].copy_from_slice(&placement.previous[..2]);
                                placement.previous[2] = placement.base[2];
                                ordinary.mode = if ordinary.mode == 6 { 9 } else { 10 };
                                self.state.flags = [1; 2];
                                continue;
                            }
                            for i in 0..2 {
                                placement.base[i] = interpolate(
                                    f64::from(phase),
                                    1. - f64::from(phase),
                                    self.state.start[i],
                                    placement.previous[i],
                                );
                            }
                            self.tail();
                            continue;
                        }
                        7 => {
                            self.settle(animator)?;
                            let id = foreground(animator)?;
                            let layer = animator.layer(id).unwrap();
                            let phase = if layer.clock.ended {
                                1.
                            } else if layer.clock.speed < 0. {
                                1. - f64::from(layer.clock.phase)
                            } else {
                                f64::from(layer.clock.phase)
                            };
                            if phase >= 1. || layer.clock.speed.abs() <= 0.000001 {
                                let layer = animator.get_mut(id)?;
                                layer.translation_policy = -1;
                                layer.rotation_policy = -1;
                                animator.set_promote_next_foreground(true)?;
                                self.finish_transition(
                                    animator, placement, ordinary, facing, layers,
                                )?;
                                match self.state.reason {
                                    1 => self.state.status = -1,
                                    2 => self.state.status = 2,
                                    3 => self.state.status = 3,
                                    _ => (),
                                }
                                continue;
                            }
                            let delta = normalized(
                                f64::from(placement.previous[2]) - f64::from(self.state.start[2]),
                                -4.,
                            )?;
                            heading(
                                placement,
                                facing,
                                (delta * phase + f64::from(self.state.start[2])) as f32,
                            )?;
                            let other = (1. - phase) as f32;
                            let spilled_phase = phase as f32;
                            for i in 0..2 {
                                placement.base[i] = interpolate(
                                    f64::from(spilled_phase),
                                    f64::from(other),
                                    self.state.start[i],
                                    placement.previous[i],
                                );
                            }
                            self.tail();
                            continue;
                        }
                        8 => {
                            self.state.flags = [1; 2];
                            self.tail();
                            continue;
                        }
                        9 | 10 => {
                            if self.path.is_some() {
                                self.follow_path(
                                    animator, placement, ordinary, facing, source, layers, program,
                                )?;
                                continue;
                            }
                            let pass = self.pass.as_mut().unwrap();
                            pass.continuation = Some(Continuation::Path {
                                now: pass.now,
                                step: pass.step,
                                entry: pass.entry,
                            });
                            pass.stage = Stage::Done;
                            continue;
                        }
                        12 => {
                            self.settle(animator)?;
                            if let Some(id) = animator.slot(Slot::Foreground) {
                                if animator.layer(id).unwrap().interruptible == 0 {
                                    speed(animator, ordinary.rate)?;
                                    self.tail();
                                    continue;
                                }
                            }
                            ordinary.mode = 3;
                            continue;
                        }
                        _ => {
                            self.tail();
                            continue;
                        }
                    }
                }
            };
            self.pass.as_mut().unwrap().pending = Some(request);
            return Ok(Some(request));
        }
        Err("native movement transition budget exhausted".into())
    }
    pub fn selected(
        &mut self,
        animator: &mut Animator,
        ordinary: &ordinary::State,
        skeleton: &Skeleton,
        clip: Option<&Animation>,
        layers: LayerSettings,
    ) -> Result<(), String> {
        let pass = self.pass.as_mut().ok_or("native movement absent")?;
        let Stage::Select { index, kind } = pass.stage else {
            return Err("native foreground selection not pending".into());
        };
        if pass.pending != Some(Request::Foreground { index }) {
            return Err("native foreground selection not pending".into());
        }
        animator.finish_foreground_selection(
            index,
            if matches!(kind, Selection::Walk { .. }) {
                1
            } else {
                0
            },
            skeleton,
            clip,
            ordinary::clock(pass.tick, ordinary.clock_mode),
            layers,
        )?;
        pass.pending = None;
        pass.stage = Stage::Selected(kind);
        Ok(())
    }
    pub fn displacement(
        &mut self,
        placement: &mut Placement,
        facing: &mut i16,
        displacement: [f32; 3],
        settings: Settings,
    ) -> Result<(), String> {
        let pass = self.pass.as_mut().ok_or("native movement absent")?;
        let Stage::Displacement {
            index,
            angle,
            distance,
        } = pass.stage
        else {
            return Err("native displacement not pending".into());
        };
        if pass.pending != Some(Request::RootDisplacement { index })
            || displacement.iter().any(|v| !v.is_finite())
        {
            return Err("invalid native displacement response".into());
        }
        heading(placement, facing, placement.previous[2])?;
        self.state.end = placement.previous;
        self.state.start = placement.base;
        let (sine, cosine) = f64::from(angle).sin_cos();
        let [x, y, _] = displacement.map(f64::from);
        placement.previous[0] = (f64::from(placement.previous[0]) - (cosine * x + sine * y)) as f32;
        placement.previous[1] = (f64::from(placement.previous[1]) - (sine * x + cosine * y)) as f32;
        let mut weight = if f64::from(distance) < 0.1 {
            f64::from(settings.minimum_weight)
        } else if distance >= 1. {
            1.
        } else {
            f64::from(distance) / (x * x + y * y).sqrt()
        };
        if weight < f64::from(settings.minimum_weight) {
            weight = f64::from(settings.minimum_weight);
        }
        if !weight.is_finite() {
            return Err("native short-step weight is non-finite".into());
        }
        pass.pending = None;
        pass.stage = Stage::Select {
            index,
            kind: Selection::Short {
                weight: weight as f32,
            },
        };
        Ok(())
    }
    /// The caller must execute the requested 590430 base update or 592132
    /// placement publication before acknowledging; this is not a substitute.
    pub fn serviced(&mut self, request: Request, animator: &mut Animator) -> Result<(), String> {
        let pass = self.pass.as_mut().ok_or("native movement absent")?;
        if pass.pending != Some(request)
            || !matches!(request, Request::BasePosture | Request::Placement)
        {
            return Err("native movement service mismatch".into());
        }
        pass.pending = None;
        if request == Request::BasePosture {
            self.settle(animator)?;
        }
        self.tail();
        Ok(())
    }
    /// Execute movement's sentinel-placement call using the same 592132
    /// calculator and model publication used by the shared refresh tail.
    pub fn place(
        &mut self,
        character: &mut super::refresh::Character,
        source: super::transform::Source,
        view: super::transform::View,
        terrain: super::transform::Terrain<'_>,
    ) -> Result<super::transform::Transform, String> {
        self.place_with_scale(character, source, view, terrain, None)
    }
    pub fn place_with_scale(
        &mut self,
        character: &mut super::refresh::Character,
        source: super::transform::Source,
        view: super::transform::View,
        terrain: super::transform::Terrain<'_>,
        terrain_scale: Option<f32>,
    ) -> Result<super::transform::Transform, String> {
        if self.pass.as_ref().and_then(|p| p.pending) != Some(Request::Placement)
            || character.busy()
        {
            return Err("native movement placement is not ready".into());
        }
        let mut placement = character.placement.clone();
        let transform = placement.transform_with_control(
            source,
            view,
            terrain,
            character.skin.animator.fp_control(),
            terrain_scale,
        )?;
        let mut model = character.model.clone();
        let matrix = model.publish(super::model::Matrix {
            values: transform.matrix,
            kind: 0,
        })?;
        character.skin.animator.placement_updated()?;
        character.placement = placement;
        character.model = model;
        character.matrix = matrix.values;
        self.serviced(Request::Placement, &mut character.skin.animator)?;
        Ok(transform)
    }
    pub fn finish(&mut self) -> Result<Continuation, String> {
        let pass = self.pass.as_ref().ok_or("native movement absent")?;
        let result = pass.continuation.ok_or("native movement unfinished")?;
        self.pass = None;
        Ok(result)
    }
}
