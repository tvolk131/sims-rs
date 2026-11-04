//! Vitaboy bone-local samples, source handedness conversion, and two-bone skinning.
use anyhow::{Result, bail};
use glam::{Mat4, Quat, Vec3};
use sims_formats::avatar::{Mesh, Skeleton};
use sims_vm::program::AnimationAsset;
fn translation(v: [f32; 3]) -> Vec3 {
    Vec3::new(-v[0], v[1], v[2])
}
fn rotation(v: [f32; 4]) -> Result<Quat> {
    let q = Quat::from_xyzw(v[0], -v[1], -v[2], -v[3]);
    if !q.is_finite() || q.length_squared() < 0.0001 {
        bail!("invalid animation quaternion");
    }
    Ok(q.normalize())
}
pub fn evaluate(
    skeleton: &Skeleton,
    animation: &AnimationAsset,
    frame: f32,
    looping: bool,
) -> Result<Vec<Mat4>> {
    let local = sims_sim::pose::sampling::at_frame(
        skeleton,
        &animation.clip,
        &animation.samples,
        frame,
        looping,
    )
    .map_err(anyhow::Error::msg)?;
    from_local_bones(skeleton, &local)
}
/// Present a completed simulation pose without resampling or ticking animation.
pub fn from_local_bones(
    skeleton: &Skeleton,
    local: &[sims_sim::pose::LocalBone],
) -> Result<Vec<Mat4>> {
    let parents = skeleton.bones.iter().map(|b| b.parent).collect::<Vec<_>>();
    from_absolute_bones(
        &sims_sim::pose::hierarchy(&parents, local, native_identity())
            .map_err(anyhow::Error::msg)?,
    )
}
/// Present an already evaluated native hierarchy, including temporary head pose.
/// This performs only handedness conversion, without resampling or ticking it.
pub fn from_absolute_bones(bones: &[sims_sim::head_look::rig::AbsoluteBone]) -> Result<Vec<Mat4>> {
    bones.iter().copied().map(native_matrix).collect()
}
/// Apply the published native model transform before presentation conversion.
/// The caller supplies the effective matrix (including body scale); no clock,
/// animation sampling or separate laboratory actor placement runs here.
pub fn from_placed_absolute_bones(
    bones: &[sims_sim::head_look::rig::AbsoluteBone],
    model: sims_sim::pose::model::Matrix,
) -> Result<Vec<Mat4>> {
    bones
        .iter()
        .map(|bone| {
            let native = sims_sim::pose::model::Matrix {
                values: bone.matrix().map_err(anyhow::Error::msg)?,
                kind: 0,
            }
            .multiply(model)
            .map_err(anyhow::Error::msg)?;
            presentation_matrix(native.values)
        })
        .collect()
}
fn native_identity() -> sims_sim::head_look::rig::AbsoluteBone {
    sims_sim::head_look::rig::AbsoluteBone {
        translation: [0.; 3],
        rotation: [0., 0., 0., 1.],
    }
}
/// Evaluate imported layer phases without ticking clocks or delivering events.
/// Useful for frozen source-pose inspection; it does not run the animator load tail.
pub fn evaluate_layers(
    skeleton: &Skeleton,
    layers: &[sims_sim::pose::layer::Layer],
    program: &sims_vm::Program,
) -> Result<Vec<Mat4>> {
    use sims_sim::pose::{LocalBone, layer, sampling::BoneInput};
    let mut layers = layers.to_vec();
    let mut priorities = vec![0; skeleton.bones.len()];
    layer::prepare(&mut layers, &mut priorities).map_err(anyhow::Error::msg)?;
    let mut bones = skeleton
        .bones
        .iter()
        .zip(priorities)
        .map(|(bone, priority)| BoneInput {
            local: LocalBone {
                translation: bone.translation,
                rotation: bone.rotation,
            },
            priority,
            blend_marker: bone.flags[2],
        })
        .collect::<Vec<_>>();
    for layer in &layers {
        let asset = program
            .animations
            .get(&layer.name)
            .ok_or_else(|| anyhow::anyhow!("saved layer {} absent", layer.name))?;
        layer
            .sample(&asset.clip, &asset.samples, &mut bones, 0.)
            .map_err(anyhow::Error::msg)?;
    }
    let local = bones.into_iter().map(|b| b.local).collect::<Vec<_>>();
    from_local_bones(skeleton, &local)
}
/// Convert a completed native pose only at the presentation boundary. Preserve
/// its source quaternion magnitude; animation sampling/hierarchy own the math.
fn native_matrix(bone: sims_sim::head_look::rig::AbsoluteBone) -> Result<Mat4> {
    presentation_matrix(bone.matrix().map_err(anyhow::Error::msg)?)
}
fn presentation_matrix(native: [f32; 16]) -> Result<Mat4> {
    let m = Mat4::from_cols_array(&native);
    let mirror = Mat4::from_scale(Vec3::new(-1., 1., 1.));
    let m = mirror * m * mirror;
    if !m.is_finite() {
        bail!("invalid native presentation matrix");
    }
    Ok(m)
}
/// Apply the original temporary head rotation to a presentation pose. Only the
/// HEAD subtree changes; the caller retains the unmodified animation pose for
/// the next update, matching native 0x590b27 restoration. This does not tick the
/// head-look controller or infer a target from the camera.
pub fn head_overlay(skeleton: &Skeleton, world: &[Mat4], turn: f32) -> Result<Vec<Mat4>> {
    if world.len() != skeleton.bones.len()
        || world.iter().any(|m| !m.is_finite())
        || !turn.is_finite()
    {
        bail!("invalid head overlay pose");
    }
    let mut out = world.to_vec();
    if turn.abs() < 0.01 {
        return Ok(out);
    }
    let head = skeleton
        .bones
        .iter()
        .position(|b| b.name == sims_sim::head_look::rig::HEAD_BONE)
        .ok_or_else(|| anyhow::anyhow!("native head-look bone is absent"))?;
    let parent = skeleton.bones[head]
        .parent
        .map_or(Mat4::IDENTITY, |p| world[p]);
    let (_, q, t) = (parent.inverse() * world[head]).to_scale_rotation_translation();
    // Invert the source-handedness conversion used by rotation().
    let base = [q.x, -q.y, -q.z, -q.w];
    let posed = sims_sim::head_look::compose(turn, base)
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| anyhow::anyhow!("nonzero head overlay produced no rotation"))?;
    out[head] = parent * Mat4::from_rotation_translation(rotation(posed)?, t);
    let delta = out[head] * world[head].inverse();
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let mut ancestor = bone.parent;
        while let Some(p) = ancestor {
            if p == head {
                out[i] = delta * world[i];
                break;
            }
            ancestor = skeleton.bones[p].parent;
        }
    }
    Ok(out)
}
/// Apply the automatic or script-selected priority-10 carry clip to authored channels.
/// Unauthored bones and channels keep the foreground/locomotion pose. Native
/// general-purpose layer fades are not modeled.
pub fn carry_overlay(
    skeleton: &Skeleton,
    world: &[Mat4],
    animation: &AnimationAsset,
    frame: f32,
) -> Result<Vec<Mat4>> {
    if world.len() != skeleton.bones.len() {
        bail!("carry pose does not match skeleton");
    }
    let sample = locals(skeleton, animation, frame, true)?;
    let local: Vec<_> = skeleton
        .bones
        .iter()
        .enumerate()
        .map(|(i, bone)| {
            let base = bone
                .parent
                .map_or(world[i], |p| world[p].inverse() * world[i]);
            let Some(motion) = animation.clip.motions.iter().find(|m| m.bone == bone.name) else {
                return base;
            };
            let (_, br, bt) = base.to_scale_rotation_translation();
            let (_, sr, st) = sample[i].to_scale_rotation_translation();
            Mat4::from_rotation_translation(
                if motion.rotation.is_some() { sr } else { br },
                if motion.translation.is_some() { st } else { bt },
            )
        })
        .collect();
    hierarchy(skeleton, &local)
}
/// Native background clips loop at priority -10. Only channels absent from
/// the foreground clip take the background sample; carry overlays apply later.
pub fn background_underlay(
    skeleton: &Skeleton,
    world: &[Mat4],
    background: &AnimationAsset,
    frame: f32,
    foreground: Option<&AnimationAsset>,
) -> Result<Vec<Mat4>> {
    let sample = locals(skeleton, background, frame, true)?;
    let local = skeleton
        .bones
        .iter()
        .enumerate()
        .map(|(i, bone)| {
            let base = bone
                .parent
                .map_or(world[i], |p| world[p].inverse() * world[i]);
            let Some(bg) = background.clip.motions.iter().find(|m| m.bone == bone.name) else {
                return base;
            };
            let fg = foreground.and_then(|a| a.clip.motions.iter().find(|m| m.bone == bone.name));
            let (_, br, bt) = base.to_scale_rotation_translation();
            let (_, sr, st) = sample[i].to_scale_rotation_translation();
            Mat4::from_rotation_translation(
                if bg.rotation.is_some() && fg.is_none_or(|m| m.rotation.is_none()) {
                    sr
                } else {
                    br
                },
                if bg.translation.is_some() && fg.is_none_or(|m| m.translation.is_none()) {
                    st
                } else {
                    bt
                },
            )
        })
        .collect::<Vec<_>>();
    hierarchy(skeleton, &local)
}
fn locals(
    skeleton: &Skeleton,
    animation: &AnimationAsset,
    frame: f32,
    looping: bool,
) -> Result<Vec<Mat4>> {
    sims_sim::pose::sampling::at_frame(
        skeleton,
        &animation.clip,
        &animation.samples,
        frame,
        looping,
    )
    .map_err(anyhow::Error::msg)?
    .into_iter()
    .map(|local| {
        native_matrix(
            sims_sim::pose::compose_bone(local, native_identity()).map_err(anyhow::Error::msg)?,
        )
    })
    .collect()
}

fn hierarchy(skeleton: &Skeleton, locals: &[Mat4]) -> Result<Vec<Mat4>> {
    let mut world = vec![None; skeleton.bones.len()];
    fn resolve(i: usize, s: &Skeleton, local: &[Mat4], out: &mut [Option<Mat4>]) -> Mat4 {
        if let Some(m) = out[i] {
            return m;
        }
        let m = if let Some(p) = s.bones[i].parent {
            resolve(p, s, local, out) * local[i]
        } else {
            local[i]
        };
        out[i] = Some(m);
        m
    }
    for i in 0..world.len() {
        resolve(i, skeleton, locals, &mut world);
    }
    Ok(world.into_iter().map(Option::unwrap).collect())
}
/// Original locomotion poses retargeted to the collision-checked path. Remove horizontal
/// root displacement and yaw once, keeping vertical bob/pitch/roll and limb animation.
fn in_place(
    skeleton: &Skeleton,
    animation: &AnimationAsset,
    frame: f32,
    looping: bool,
) -> Result<Vec<Mat4>> {
    let mut local = locals(skeleton, animation, frame, looping)?;
    let root = skeleton
        .bones
        .iter()
        .position(|b| b.parent.is_none())
        .ok_or_else(|| anyhow::anyhow!("skeleton has no root"))?;
    let (_, rotation, mut translation) = local[root].to_scale_rotation_translation();
    let forward = rotation * Vec3::Z;
    let yaw = forward.x.atan2(forward.z);
    let correction = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2 - yaw);
    translation.x = 0.;
    translation.z = 0.;
    local[root] = Mat4::from_rotation_translation(correction * rotation, translation);
    Ok(local)
}
fn blend(a: &[Mat4], b: &[Mat4], weight: f32) -> Vec<Mat4> {
    a.iter()
        .zip(b)
        .map(|(a, b)| {
            let (_, ar, at) = a.to_scale_rotation_translation();
            let (_, br, bt) = b.to_scale_rotation_translation();
            Mat4::from_rotation_translation(ar.slerp(br, weight), at.lerp(bt, weight))
        })
        .collect()
}
const MODEL_UNITS_PER_DISTANCE: f32 = 3. / 16000.;
fn start_frame(a: &AnimationAsset, length: f32, traveled: f32) -> f32 {
    let last = (a.clip.frames - 1) as f32;
    let release = a
        .clip
        .events
        .iter()
        .find(|e| e.pairs.iter().any(|(k, v)| k == "anchor" && v == "0"))
        .map_or(0., |e| e.time_ms as f32 * 0.03)
        .min(last);
    let proportion = (length * MODEL_UNITS_PER_DISTANCE / a.clip.distance.max(0.001)).min(1.);
    release + (last - release) * proportion * traveled
}
/// Synchronize the first swing using the same toe's release/contact in both clips.
/// This is derived from immutable assets and the saved start distance, never render history.
struct StartStride {
    phase: usize,
    source_release: f32,
    walk_release: f32,
    source_per_walk: f32,
    frames_per_distance: f32,
    offset: f32,
}
impl StartStride {
    fn new(
        program: &sims_vm::Program,
        movement: &sims_sim::locomotion::Locomotion,
    ) -> Option<Self> {
        let phase = movement
            .phases
            .iter()
            .position(|p| p.kind == sims_sim::locomotion::Kind::Start)?;
        let p = &movement.phases[phase];
        let start = program.animations.get(&p.clip)?;
        let walk = program.animations.get(sims_sim::locomotion::WALK)?;
        if walk.clip.distance <= 0. || walk.clip.frames == 0 {
            return None;
        }
        // Some source clips annotate both toe joints; use a track present in both.
        let (release, walk_release) = start
            .clip
            .events
            .iter()
            .filter(|e| e.pairs.iter().any(|(k, v)| k == "anchor" && v == "0"))
            .find_map(|e| {
                walk.clip
                    .events
                    .iter()
                    .find(|w| {
                        w.bone == e.bone && w.pairs.iter().any(|(k, v)| k == "anchor" && v == "0")
                    })
                    .map(|w| (e, w))
            })?;
        let contact = start.clip.events.iter().find(|e| {
            e.bone == release.bone
                && e.time_ms > release.time_ms
                && e.pairs.iter().any(|(k, v)| k == "anchor" && v != "0")
        })?;
        let source_release = release.time_ms as f32 * 0.03;
        let source_span = (contact.time_ms - release.time_ms) as f32 * 0.03;
        let walk_release = walk_release.time_ms as f32 * 0.03;
        let walk_span = walk
            .clip
            .events
            .iter()
            .filter(|e| {
                e.bone == release.bone && e.pairs.iter().any(|(k, v)| k == "anchor" && v != "0")
            })
            .map(|e| (e.time_ms as f32 * 0.03 - walk_release).rem_euclid(walk.clip.frames as f32))
            .filter(|span| *span > 0.)
            .min_by(f32::total_cmp)?;
        let source_per_walk = source_span / walk_span;
        let frames_per_distance =
            MODEL_UNITS_PER_DISTANCE * walk.clip.frames as f32 / walk.clip.distance;
        let end = start_frame(start, (p.to_distance - p.from_distance) as f32, 1.);
        let entry_frame = walk_release + (end - source_release) / source_per_walk;
        Some(Self {
            phase,
            source_release,
            walk_release,
            source_per_walk,
            frames_per_distance,
            offset: entry_frame - p.to_distance as f32 * frames_per_distance,
        })
    }
    fn walk_frame(&self, distance: f32) -> f32 {
        distance * self.frames_per_distance + self.offset
    }
    fn source_frame(&self, distance: f32) -> f32 {
        self.source_release + (self.walk_frame(distance) - self.walk_release) * self.source_per_walk
    }
}
fn phase_pose(
    skeleton: &Skeleton,
    program: &sims_vm::Program,
    phase: &sims_sim::locomotion::Phase,
    progress: f32,
    stride: Option<&StartStride>,
) -> Result<Vec<Mat4>> {
    use sims_sim::locomotion::Kind;
    let a = program
        .animations
        .get(&phase.clip)
        .ok_or_else(|| anyhow::anyhow!("missing locomotion clip {}", phase.clip))?;
    let traveled = match phase.kind {
        Kind::Start => progress * progress,
        Kind::Stop => progress * (2. - progress),
        _ => progress,
    };
    let length = (phase.to_distance - phase.from_distance) as f32;
    let distance = phase.from_distance as f32 + length * traveled;
    let moving_turn = phase.kind == Kind::Turn && length > 0.;
    if phase.kind == Kind::Walk || moving_turn {
        let walk = program
            .animations
            .get(sims_sim::locomotion::WALK)
            .ok_or_else(|| anyhow::anyhow!("missing walking cycle"))?;
        if walk.clip.distance <= 0. {
            bail!("walking cycle requires positive source Distance");
        }
        let frame = stride.map_or_else(
            || distance * MODEL_UNITS_PER_DISTANCE / walk.clip.distance * walk.clip.frames as f32,
            |s| s.walk_frame(distance),
        );
        let mut local = in_place(skeleton, walk, frame, true)?;
        if moving_turn {
            // Walking-turn assets contain several steps. Compressing those legs into a
            // short bend would race the feet, so layer torso anticipation over the stride.
            let turn = in_place(skeleton, a, progress * (a.clip.frames - 1) as f32, false)?;
            let weight = (std::f32::consts::PI * progress).sin().max(0.) * 0.65;
            let combined = blend(&local, &turn, weight);
            for (i, bone) in skeleton.bones.iter().enumerate() {
                if matches!(
                    bone.name.as_str(),
                    "SPINE" | "SPINE1" | "SPINE2" | "NECK" | "HEAD"
                ) {
                    local[i] = combined[i];
                }
            }
        }
        return Ok(local);
    }
    let last = (a.clip.frames - 1) as f32;
    let frame = match phase.kind {
        Kind::Start => start_frame(a, length, traveled),
        Kind::Stop => {
            let proportion = (length * (3. / 16000.) / a.clip.distance.max(0.001)).min(1.);
            last * (1. - proportion + proportion * traveled)
        }
        _ => progress * last,
    };
    in_place(skeleton, a, frame, false)
}
fn gait(phase: &sims_sim::locomotion::Phase) -> bool {
    use sims_sim::locomotion::Kind;
    phase.kind == Kind::Walk
        || (phase.kind == Kind::Turn && phase.to_distance > phase.from_distance)
}
// Transition time belongs to an animation change, not to a grid/path segment.
// A short straight followed by a walking turn must carry the same blend through.
const BLEND_TICKS: f32 = 6.;
fn blended_locomotion(
    skeleton: &Skeleton,
    program: &sims_vm::Program,
    movement: &sims_sim::locomotion::Locomotion,
    phase_index: usize,
    progress: f32,
    stride: Option<&StartStride>,
) -> Result<Vec<Mat4>> {
    let stand = program
        .animations
        .get(sims_sim::locomotion::STAND)
        .ok_or_else(|| anyhow::anyhow!("missing stand pose"))?;
    let phase = &movement.phases[phase_index];
    let mut local = phase_pose(skeleton, program, phase, progress, stride)?;
    let time = phase.start_tick as f32 + progress * phase.ticks as f32;
    let mut entry = phase_index;
    if gait(phase) {
        while entry > 0 && gait(&movement.phases[entry - 1]) {
            entry -= 1;
        }
    }
    let elapsed = time - movement.phases[entry].start_tick as f32;
    if elapsed < BLEND_TICKS {
        let previous = if entry == 0 {
            in_place(skeleton, stand, 0., true)?
        } else if let Some(s) = stride.filter(|s| s.phase + 1 == entry && gait(phase)) {
            // Keep the outgoing foot moving toward its contact while crossfading.
            // Holding the boundary pose for six ticks would still shorten the step.
            let start = &movement.phases[s.phase];
            let distance = phase.from_distance as f32
                + (phase.to_distance - phase.from_distance) as f32 * progress;
            let mut advancing = in_place(
                skeleton,
                &program.animations[&start.clip],
                s.source_frame(distance),
                false,
            )?;
            // A very short start may still be blending out of standing at the boundary.
            let age = start.ticks as f32 + elapsed;
            if age < BLEND_TICKS {
                let origin = if s.phase == 0 {
                    in_place(skeleton, stand, 0., true)?
                } else {
                    blended_locomotion(skeleton, program, movement, s.phase - 1, 1., stride)?
                };
                let t = age / BLEND_TICKS;
                advancing = blend(&origin, &advancing, t * t * (3. - 2. * t));
            }
            advancing
        } else {
            blended_locomotion(skeleton, program, movement, entry - 1, 1., stride)?
        };
        let t = elapsed / BLEND_TICKS;
        local = blend(&previous, &local, t * t * (3. - 2. * t));
    }
    // Reach a neutral pose before handing ownership back to the original interaction.
    if phase_index + 1 == movement.phases.len() {
        let blend_ticks = BLEND_TICKS.min(phase.ticks as f32 / 2.);
        let remaining = (1. - progress) * phase.ticks as f32;
        if remaining < blend_ticks {
            let t = 1. - remaining / blend_ticks;
            local = blend(
                &local,
                &in_place(skeleton, stand, 0., true)?,
                t * t * (3. - 2. * t),
            );
        }
    }
    Ok(local)
}
pub fn locomotion(
    skeleton: &Skeleton,
    program: &sims_vm::Program,
    movement: &sims_sim::locomotion::Locomotion,
    sample: sims_sim::locomotion::Sample,
) -> Result<Vec<Mat4>> {
    if movement.phases.is_empty() {
        let stand = program
            .animations
            .get(sims_sim::locomotion::STAND)
            .ok_or_else(|| anyhow::anyhow!("missing stand pose"))?;
        return evaluate(skeleton, stand, 0., true);
    }
    let stride = StartStride::new(program, movement);
    hierarchy(
        skeleton,
        &blended_locomotion(
            skeleton,
            program,
            movement,
            sample.phase,
            sample.progress,
            stride.as_ref(),
        )?,
    )
}
pub fn skin(mesh: &Mesh, skeleton: &Skeleton, bones: &[Mat4]) -> Result<Vec<(Vec3, Vec3)>> {
    let mapping: Vec<_> = mesh
        .bones
        .iter()
        .map(|n| {
            skeleton
                .bones
                .iter()
                .position(|b| b.name.eq_ignore_ascii_case(n))
                .ok_or_else(|| anyhow::anyhow!("missing mesh bone {n}"))
        })
        .collect::<Result<_>>()?;
    let mut vertices = Vec::new();
    for v in &mesh.vertices {
        let m = bones[mapping[v.bone]];
        vertices.push((
            m.transform_point3(translation(v.position)),
            m.transform_vector3(translation(v.normal)),
        ));
    }
    for b in &mesh.blends {
        let m = bones[mapping[b.bone]];
        let v = &mut vertices[b.vertex];
        v.0 =
            v.0.lerp(m.transform_point3(translation(b.position)), b.weight);
        v.1 =
            v.1.lerp(m.transform_vector3(translation(b.normal)), b.weight);
    }
    Ok(vertices)
}
