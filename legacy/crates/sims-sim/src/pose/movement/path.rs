//! 58dc72 path following and 48313e projection. Path construction/search is
//! deliberately separate; points and all retained native tuning/state are inputs.
use super::*;
mod steering;
mod projection;
use steering::direction;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub sample_step: f32,            // 6d07a8
    pub turn_threshold: f32,         // 6d07b4
    pub turn_step: f32,              // 6d07b8
    pub minimum_brake_time: i32,     // 6d07d4
    pub endpoint_distance: [f32; 2], // 6d07d8/dc (walk/run)
}
impl Settings {
    /// Original globals 6d07a8/b4/b8/d4/d8/dc in the recovered Complete
    /// Collection process. Kept explicit and replaceable for other builds.
    pub const COMPLETE_COLLECTION: Self = Self {
        sample_step: 1.,
        turn_threshold: 2.,
        turn_step: 0.2,
        minimum_brake_time: 100,
        endpoint_distance: [2., 3.],
    };
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Path {
    /// Original word order. The penultimate point is the destination; the last
    /// point supplies the final heading. These are not renderer coordinates.
    pub points: Vec<[i32; 2]>,
    pub settings: Settings,
    pub index: i32,               // +180
    pub progress: f32,            // +1ec
    pub deadline: i32,            // +1c0
    pub phase_window: [f32; 2],   // +174 lower, +170 upper
    pub stop_distance: [f32; 2],  // +178 outside window, +17c inside
    pub start_distance: [f32; 2], // +1f8 walk, +204 run
    pub lookahead: [f32; 2],      // +200 walk, +1fc nonzero walk style
    pub interrupt: i32,           // +284
    pub guard: i32,               // +28c
    pub delta: [f32; 2],          // +294/+298 in animator order
    #[serde(default)]
    pub stop_mode: i32, // +18c, separate from ordinary mode +190
}
impl Path {
    /// Bind the retained constructor words to actual initializer measurements.
    /// The original animator reader does not serialize a path; post-load Reset
    /// also does not recreate one. Route start replaces only the points and
    /// the words written by 589c85, preserving subsequent braking state.
    pub fn initialized(
        settings: Settings,
        phase_window: [f32; 2],
        stop_distance: [f32; 2],
        clip_distances: [f32; 6],
    ) -> Self {
        Self {
            points: vec![],
            settings,
            index: -1,
            progress: 0.,
            deadline: 0,
            phase_window,
            stop_distance,
            start_distance: [clip_distances[0], clip_distances[3]],
            lookahead: [clip_distances[2], clip_distances[1]],
            interrupt: 0,
            guard: 0,
            delta: [0.; 2],
            stop_mode: 0,
        }
    }
}
fn trunc(value: f64) -> Result<i32, String> {
    let value = value.trunc();
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        return Err("native path integer conversion overflow".into());
    }
    Ok(value as i32)
}
pub(super) fn round(value: f64) -> Result<i32, String> {
    trunc(value + 0.5)
}
/// 58e6e5..58e6f3: two multiplies followed by the sum. PC=24 rounds
/// each product, not just the final spill; the prior 037f profile stays wide.
fn square(v: [f32; 2], control: u16) -> f32 {
    if control == 0x003f {
        let y = v[1] * v[1];
        let x = v[0] * v[0];
        y + x
    } else {
        (f64::from(v[1]).powi(2) + f64::from(v[0]).powi(2)) as f32
    }
}
fn squared_integer(a: [i32; 2], b: [i32; 2]) -> i32 {
    let x = a[0].wrapping_sub(b[0]);
    let y = a[1].wrapping_sub(b[1]);
    x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y))
}
/// Native 48313e chooses the nearest integer interpolation on eligible segments.
/// A nonnegative index selects segment max(index-1,0); a negative index scans all.
/// The terminal heading point is excluded. Equal distances retain the earlier point.
pub fn project(points: &[[i32; 2]], point: [i32; 2], index: i32) -> Result<[i32; 2], String> {
    if points.len() < 3 {
        return Err("native projection requires destination and heading points".into());
    }
    let last = points.len() - 3;
    let (first, end) = if index < 0 {
        (0, last)
    } else {
        let n = (index - 1).max(0) as usize;
        (n, n)
    };
    if first > last || end > last {
        return Ok(point);
    }
    let mut result = points[first];
    let mut distance = squared_integer(result, point) as f32;
    for i in first..=end {
        let a = points[i];
        let b = points[i + 1];
        let [x, y] = [
            a[0].wrapping_sub(b[0]) as f64,
            a[1].wrapping_sub(b[1]) as f64,
        ];
        let dx = point[0].wrapping_sub(b[0]) as f64;
        let dy = point[1].wrapping_sub(b[1]) as f64;
        let denominator = y * y + x * x;
        // The original unordered comparison follows the lower-clamp branch.
        let phase = if denominator == 0. {
            0.
        } else {
            ((dy * y + dx * x) / denominator).clamp(0., 1.) as f32
        };
        let projected = [
            round(a[0].wrapping_sub(b[0]) as f64 * f64::from(phase) + b[0] as f64)?,
            round(a[1].wrapping_sub(b[1]) as f64 * f64::from(phase) + b[1] as f64)?,
        ];
        let candidate = squared_integer(projected, point);
        if (candidate as f64) < f64::from(distance) {
            distance = candidate as f32;
            result = projected;
        }
    }
    Ok(result)
}
fn clip<'a>(animator: &Animator, program: &'a Program) -> Result<&'a Animation, String> {
    let layer = animator.layer(foreground(animator)?).unwrap();
    program
        .animations
        .get(&layer.name)
        .map(|a| &a.clip)
        .ok_or_else(|| format!("native walking clip absent: {}", layer.name))
}
/// Original 0x58ede3..0x58ee78: PC=24 rounds each x87 arithmetic result,
/// including sqrt, before later operations. Y spills before visual addition;
/// X retains the historical wide arithmetic until its own publication at 037f.
fn physical_delta(
    distance: f32,
    duration: f32,
    speed: f32,
    step: i32,
    direction: [f32; 2],
    mut square: f32,
    base: [f32; 2],
    control: u16,
) -> ([f32; 2], [f32; 2]) {
    let q = |v: f64| {
        if control == 0x003f {
            f64::from(v as f32)
        } else {
            v
        }
    };
    let period = q(q(f64::from(duration) / f64::from(speed)) / f64::from(step));
    let distance = q(q(q(f64::from(distance) * f64::from(0.33333334_f32)) * 16.) / period);
    if square.abs() <= 0.000001 {
        square = 1.;
    }
    let divisor = q(f64::from(square).sqrt());
    let dx = q(q(distance * f64::from(direction[0])) / divisor);
    let dy = q(q(distance * f64::from(direction[1])) / divisor) as f32;
    let visual = [
        q(dx + f64::from(base[0])) as f32,
        q(f64::from(dy) + f64::from(base[1])) as f32,
    ];
    ([dx as f32, dy], visual)
}

impl Movement {
    pub(super) fn follow_path(
        &mut self,
        animator: &mut Animator,
        placement: &mut Placement,
        ordinary: &mut ordinary::State,
        facing: &mut i16,
        source: Source,
        layers: LayerSettings,
        program: &Program,
    ) -> Result<(), String> {
        let mut path = self.path.take().ok_or("native path absent")?;
        let result = self.follow(
            &mut path, animator, placement, ordinary, facing, source, layers, program,
        );
        self.path = Some(path);
        result
    }
    fn follow(
        &mut self,
        path: &mut Path,
        animator: &mut Animator,
        placement: &mut Placement,
        ordinary: &mut ordinary::State,
        facing: &mut i16,
        source: Source,
        layers: LayerSettings,
        program: &Program,
    ) -> Result<(), String> {
        if path.points.len() < 3 || path.index < 0 || path.index as usize > path.points.len() - 2 {
            return Err("invalid native walking path/index".into());
        }
        let pass = self.pass.as_ref().ok_or("native movement absent")?;
        let now = pass.now;
        let input = pass.entry.movement_input;
        let style = pass.entry.walk_style;
        let mut delta = [0.; 2];
        let mut distance = 0.;
        let mut length_squared = 0.;
        let end = path.points.len() - 2;
        let mut found = false;
        for _ in 0..65536 {
            let index = path.index as usize;
            let [y, x] = path.points[index].map(|v| v as f64);
            let [py, px] = path.points[index.saturating_sub(1)].map(|v| v as f64);
            let segment = ((y - py).powi(2) + (x - px).powi(2)).sqrt() as f32;
            delta = [
                (x - f64::from(placement.base[0])) as f32,
                (y - f64::from(placement.base[1])) as f32,
            ];
            distance = ((y - f64::from(placement.base[1])).powi(2)
                + (x - f64::from(placement.base[0])).powi(2))
            .sqrt() as f32;
            if index == end {
                found = true;
                break;
            }
            if animator
                .slot(Slot::Foreground)
                .is_none_or(|id| animator.layer(id).unwrap().interruptible != 0)
            {
                ordinary.mode = if style == 1 { 10 } else { 9 };
            }
            let increment = if segment.abs() > 0.000001 {
                path.settings.sample_step / segment
            } else {
                1.
            };
            if !increment.is_finite() || increment <= 0. {
                return Err("invalid native path sampling increment".into());
            }
            let back = [px - x, py - y];
            let relative = [
                (f64::from(placement.base[0]) - x) as f32,
                (f64::from(placement.base[1]) - y) as f32,
            ];
            path.progress = projection::progress(back, relative, animator.fp_control());
            path.progress = path.progress.clamp(0., 1.);
            while segment > 0. && path.progress < 1. {
                let phase = f64::from(path.progress);
                let q = [
                    round(x * phase + px * (1. - phase))?,
                    round(y * phase + py * (1. - phase))?,
                ];
                delta = [
                    (q[0] as f64 - f64::from(placement.base[0])) as f32,
                    (q[1] as f64 - f64::from(placement.base[1])) as f32,
                ];
                length_squared = square(delta, animator.fp_control());
                let look = f64::from(path.lookahead[usize::from(style != 0)]) * 0.5;
                if f64::from(length_squared) >= look * look {
                    found = true;
                    break;
                }
                let next = path.progress + increment;
                if next == path.progress {
                    return Err("native path sampling stopped advancing".into());
                }
                path.progress = next;
            }
            if found {
                break;
            }
            path.progress = 0.;
            path.index += 1;
        }
        if !found {
            return Err("native path sampling budget exhausted".into());
        }
        if path.index as usize == end {
            let [y, x] = path.points[end].map(|v| v as f32);
            let next = path.points[end + 1];
            let end_heading = direction([
                (next[1] as f64 - f64::from(x)) as f32,
                (next[0] as f64 - f64::from(y)) as f32,
            ], animator.fp_control())?;
            if source.object9 != 0 {
                let threshold =
                    f64::from(path.settings.endpoint_distance[usize::from(style == 1)]) * 1.5;
                if distance <= path.lookahead[0] {
                    self.pass.as_mut().unwrap().root_motion = 0;
                }
                if self.state.reason == 0 && f64::from(distance) >= threshold {
                    ordinary.mode = if style == 1 { 10 } else { 9 };
                    self.pass.as_mut().unwrap().root_motion =
                        self.pass.as_ref().unwrap().entry.root_motion;
                } else if input != 0 {
                    self.state.reason = 1;
                    self.prepare_stop(
                        path,
                        animator,
                        placement,
                        ordinary,
                        [x, y, end_heading as f32],
                    )?;
                    return Ok(());
                } else {
                    placement.previous[..2].copy_from_slice(&placement.base[..2]);
                    self.state.reason = 1;
                    self.state.status = -1;
                    ordinary.mode = 3;
                    path.guard = 1;
                    path.progress = 1.;
                    if animator.slot(Slot::Foreground).is_none() {
                        return Ok(());
                    }
                    self.pass.as_mut().unwrap().stage = Stage::Physical {
                        direction: path.delta,
                        square: square(path.delta, animator.fp_control()),
                        after: true,
                    };
                    return Ok(());
                }
            } else if path.deadline == 0 {
                let foreground = animator
                    .slot(Slot::Foreground)
                    .and_then(|id| animator.layer(id));
                let phase = foreground.map_or(0., |l| l.clock.phase);
                let outside = phase < path.phase_window[0] || path.phase_window[1] <= phase;
                let mut stop = path.stop_distance[usize::from(!outside)];
                if input == 0 && style == 1 {
                    stop += stop;
                }
                if distance <= stop {
                    self.state.reason = 1;
                    if input != 0 {
                        self.prepare_stop(
                            path,
                            animator,
                            placement,
                            ordinary,
                            [x, y, end_heading as f32],
                        )?;
                        return Ok(());
                    }
                    if foreground.is_none_or(|l| l.interruptible != 0 || l.finished()) {
                        let weight =
                            (f64::from(distance) / f64::from(stop)).max(f64::from(0.2_f32));
                        let mut gap =
                            f64::from(path.phase_window[usize::from(!outside)]) - f64::from(phase);
                        if gap < 1. {
                            gap += 1.;
                        }
                        let duration = foreground.map_or(0., |l| {
                            f64::from(l.original_duration) / f64::from(ordinary.rate)
                        });
                        let duration = trunc(duration * gap)?.max(path.settings.minimum_brake_time);
                        path.deadline = now.wrapping_add(duration);
                        if let Some(id) = animator.slot(Slot::Foreground) {
                            let clock = &mut animator.get_mut(id)?.clock;
                            clock.fading = true;
                            clock.fade_start = now;
                            clock.fade_duration = duration;
                            clock.fade_from = clock.weight;
                            clock.fade_to = weight as f32;
                        }
                    }
                }
            } else {
                if path.deadline > 0
                    && (style == 1
                        || distance < path.settings.endpoint_distance[0]
                        || path.deadline <= now)
                {
                    path.deadline = -1;
                    path.guard = 0;
                    self.state.flags[1] = 0;
                }
                if path.deadline < 0 {
                    self.state.status = -1;
                    placement.previous = [x, y, end_heading as f32];
                    placement.base[0] = x;
                    placement.base[1] = y;
                    path.deadline = 0;
                    path.guard = 0;
                    ordinary.mode = 3;
                    self.state.flags[1] = 0;
                    path.progress = 1.;
                    animator.clear_foreground(
                        ordinary::clock(self.pass.as_ref().unwrap().tick, ordinary.clock_mode),
                        layers,
                    )?;
                    return Ok(());
                }
            }
            length_squared = square(delta, animator.fp_control());
        }
        if length_squared.abs() <= 0.000001 {
            self.tail();
            return Ok(());
        }
        let desired = direction(delta, animator.fp_control())?;
        let (value, bending) = match steering::adjust(
            desired, placement.base[2], ordinary.rate, source.species,
            path.settings, animator.fp_control(),
        )? {
            steering::Adjustment::Turn => {
                placement.previous = [placement.base[0], placement.base[1], desired as f32];
                ordinary.mode = 3;
                self.state.flags[0] = 0;
                return Ok(());
            }
            steering::Adjustment::Step { heading, bending } => (heading, bending),
        };
        heading(placement, facing, value)?;
        let start = f64::from(path.start_distance[usize::from(style == 1)]);
        if path.guard != 0
            || self.state.flags[0] != 0
            || [20, 21].contains(&animator.selection())
            || (input == 0 && (bending || f64::from(length_squared) <= start * start))
        {
            self.state.flags[0] = 1;
            if ordinary.mode == 10 && style == 0 {
                ordinary.mode = 9;
            }
            let index = match ordinary.mode {
                9 => 20,
                10 => 21,
                _ => 0,
            };
            let phase = if let Some(id) = animator.slot(Slot::Foreground) {
                if clip(animator, program)?.moving {
                    animator.layer(id).unwrap().clock.phase
                } else {
                    0.
                }
            } else {
                0.
            };
            self.pass.as_mut().unwrap().stage = Stage::Select {
                index,
                kind: Selection::Walk {
                    phase,
                    direction: delta,
                    square: length_squared,
                },
            };
        } else {
            ordinary.mode = if style == 1 { 11 } else { 6 };
            animator.clear_anchor()?;
        }
        Ok(())
    }
    fn prepare_stop(
        &mut self,
        path: &mut Path,
        animator: &mut Animator,
        placement: &mut Placement,
        ordinary: &mut ordinary::State,
        end: [f32; 3],
    ) -> Result<(), String> {
        self.state.start = placement.base;
        self.state.end = end;
        placement.previous = end;
        if self.state.flags[1] == 0 {
            ordinary.mode = 3;
            match self.state.reason {
                1 => self.state.status = -1,
                2 => self.state.status = 2,
                3 => self.state.status = 3,
                _ => (),
            }
            self.state.start = placement.base;
            self.state.end = placement.base;
            placement.previous = placement.base;
            self.pass.as_mut().unwrap().stage = Stage::Dispatch;
        } else {
            ordinary.mode = 7;
            path.progress = 1.;
            animator.set_selection(19)?;
            self.pass.as_mut().unwrap().stage = Stage::StopSelect;
        }
        Ok(())
    }
    pub(super) fn physical_step(
        &mut self,
        animator: &Animator,
        placement: &Placement,
        level: i32,
        program: &Program,
        direction: [f32; 2],
        length_squared: f32,
        after: bool,
    ) -> Result<(), String> {
        self.state.flags[1] = 1;
        let layer = animator.layer(foreground(animator)?).unwrap();
        let asset = clip(animator, program)?;
        let step = self.pass.as_ref().unwrap().step as i32;
        let (delta, visual) = physical_delta(
            asset.distance,
            asset.duration_ms,
            layer.clock.speed,
            step,
            direction,
            length_squared,
            [placement.base[0], placement.base[1]],
            animator.fp_control(),
        );
        let path = self.path.as_mut().ok_or("native step path absent")?;
        path.delta = delta;
        let position = project(
            &path.points,
            [round(f64::from(visual[1]))?, round(f64::from(visual[0]))?],
            path.index,
        )?;
        self.pass.as_mut().unwrap().stage = Stage::Accept {
            position,
            level,
            visual,
            after,
        };
        Ok(())
    }
    pub fn accept_step(&mut self, accepted: bool) -> Result<(), String> {
        let pass = self.pass.as_mut().ok_or("native movement absent")?;
        let Stage::Accept {
            position,
            level,
            visual,
            after,
        } = pass.stage
        else {
            return Err("native step acceptance not pending".into());
        };
        if pass.pending != Some(Request::AcceptStep { position, level }) {
            return Err("native step acceptance not pending".into());
        }
        pass.pending = None;
        if accepted {
            pass.stage = Stage::Move {
                position,
                level,
                visual,
                after,
            };
        } else {
            self.state.status = 3;
            self.tail();
        }
        Ok(())
    }
    /// Acknowledge the physical move only after its placement/callback services
    /// complete. Those services can change interruption/animation state, which
    /// this continuation deliberately re-reads before committing the next stage.
    pub fn moved(
        &mut self,
        animator: &mut Animator,
        placement: &mut Placement,
        ordinary: &mut ordinary::State,
    ) -> Result<(), String> {
        let pass = self.pass.as_mut().ok_or("native movement absent")?;
        let Stage::Move {
            position,
            level,
            visual,
            after,
        } = pass.stage
        else {
            return Err("native physical move not pending".into());
        };
        if pass.pending != Some(Request::MoveStep { position, level }) {
            return Err("native physical move not pending".into());
        }
        pass.pending = None;
        placement.base[..2].copy_from_slice(&visual);
        let input = pass.entry.movement_input;
        let mut path = self.path.take().ok_or("native path absent")?;
        let interruptible = animator
            .slot(Slot::Foreground)
            .is_none_or(|id| animator.layer(id).unwrap().interruptible != 0);
        let result = if path.interrupt != 0 && self.state.status == 1 && interruptible {
            if input != 0 {
                self.state.reason = 3;
                self.prepare_stop(&mut path, animator, placement, ordinary, placement.base)
            } else {
                self.state.status = 3;
                self.tail();
                Ok(())
            }
        } else if after {
            placement.previous = placement.base;
            self.pass.as_mut().unwrap().stage = Stage::Dispatch;
            Ok(())
        } else {
            self.tail();
            Ok(())
        };
        self.path = Some(path);
        result
    }
    pub fn stop_selected(
        &mut self,
        animator: &mut Animator,
        ordinary: &ordinary::State,
        skeleton: &Skeleton,
        clip: Option<&Animation>,
        layers: LayerSettings,
    ) -> Result<(), String> {
        let pass = self.pass.as_mut().ok_or("native movement absent")?;
        if pass.pending != Some(Request::StopForeground { index: 19 }) {
            return Err("native stop lookup not pending".into());
        }
        if clip.is_some() {
            animator.start(
                Slot::Foreground,
                skeleton,
                clip,
                true,
                ordinary::clock(pass.tick, ordinary.clock_mode),
                layers,
            )?;
        }
        pass.pending = None;
        pass.stage = Stage::Dispatch;
        Ok(())
    }
}

#[cfg(test)]
mod precision_tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[test]
    #[ignore = "private unmodified original x87 movement arithmetic observations"]
    fn original_walk_delta_precision_controls_match_exact_bits() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../fixtures/private/goth-pose-compare-20261030-first/walk-delta-native-first.json",
        );
        let raw = std::fs::read(path).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&raw)),
            "d68d563e881b219752c064fd110b7694cff0e8bbc894495da14b131a6c26e398"
        );
        let input: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(
            input["code"]["sha256"],
            "f66858131d28bb13b8b8760d8f1108c370e779ec1ebea675c220f62e2929bfc5"
        );
        let rows = input["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 192);
        for (index, row) in rows.iter().enumerate() {
            let c = &row["input"];
            let number = |key: &str| c[key].as_f64().unwrap() as f32;
            let pair = |key: &str| std::array::from_fn(|i| c[key][i].as_f64().unwrap() as f32);
            let (delta, visual) = physical_delta(
                number("distance"),
                number("duration"),
                number("speed"),
                c["step"].as_i64().unwrap() as i32,
                pair("direction"),
                number("square"),
                pair("base"),
                row["control"].as_u64().unwrap() as u16,
            );
            assert_eq!(
                serde_json::json!(delta.map(f32::to_bits)),
                row["delta_bits"],
                "native delta case {index}"
            );
            assert_eq!(
                serde_json::json!(visual.map(f32::to_bits)),
                row["visual_bits"],
                "native visual case {index}"
            );
        }
    }
}
