//! Populated 5a15bc / 59934b / 5a1103 geometry publication. Immutable mesh data
//! and shared mesh caches live outside each model; bindings retain the model's
//! bone mapping, bounds and drawable. Vertex skinning is a separate native pass.
use super::{
    mesh::{MeshBounds, Pose},
    model::{Bound, Bounds},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drawable {
    pub bounds: Bounds,
    pub flags: u32,
}
impl Drawable {
    fn publish(&mut self, bounds: Bounds) {
        self.bounds = bounds;
        self.flags &= !1;
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    /// Stable index in the shared mesh cache pool, independent of bone IDs.
    pub mesh: usize,
    pub bones: Vec<usize>,
    pub bounds: Bounds,
    pub drawable: Drawable,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub bindings: Vec<Binding>,
    /// Skin +88: appearance instances, each with its ordered binding vector.
    /// An existing empty appearance is distinct from an empty appearance list.
    pub appearances: Vec<Vec<usize>>,
    /// Bone +154. Independent of the attachment bounds at +138.
    pub bone_vertex_bounds: Vec<Bounds>,
}
impl State {
    pub fn empty(bones: usize) -> Self {
        Self {
            bindings: vec![],
            appearances: vec![],
            bone_vertex_bounds: vec![Bounds::EMPTY; bones],
        }
    }
    /// Successful 599427 attachment with already constructed drawable bindings
    /// and fully resolved bone names. Missing-bone and allocator failure paths
    /// must be handled before calling this successful-attachment entry point.
    pub fn attach(
        &mut self,
        bound: &mut Bound,
        bindings: Vec<Binding>,
        meshes: &[MeshBounds],
    ) -> Result<usize, String> {
        for binding in &bindings {
            let mesh = meshes.get(binding.mesh).ok_or("new mesh cache absent")?;
            if binding.bones.len() != mesh.local.len()
                || binding
                    .bones
                    .iter()
                    .any(|&i| i >= bound.bone_bounds_generations.len())
            {
                return Err("new mesh bone mapping invalid".into());
            }
        }
        let mut appearance = Vec::with_capacity(bindings.len());
        for binding in bindings {
            // 599e87 invalidates each resolved bone; 59e1b6 and 595629 each
            // invalidate the owner once after constructing the mesh binding.
            for &bone in &binding.bones {
                bound.bone_bounds_generations[bone] =
                    bound.bone_bounds_generations[bone].wrapping_add(1);
                bound.skeleton.geometry.generation =
                    bound.skeleton.geometry.generation.wrapping_add(1);
            }
            bound.skeleton.geometry.generation = bound.skeleton.geometry.generation.wrapping_add(2);
            bound.skeleton.geometry.dirty = true;
            appearance.push(self.bindings.len());
            self.bindings.push(binding);
        }
        let id = self.appearances.len();
        self.appearances.push(appearance);
        // 599427 invalidates even when the appearance has no mesh bindings.
        bound.skeleton.geometry.generation = bound.skeleton.geometry.generation.wrapping_add(1);
        bound.skeleton.geometry.dirty = true;
        Ok(id)
    }
    /// 599469 with the last reference to a live appearance instance. Instances
    /// created by `attach` are exclusively owned by this model. The outer list
    /// is erased before reverse binding/bone destruction (5956f8 / 599c8c).
    /// Mesh prototype caches outlive their model bindings. Dead bindings retain
    /// stable indices in snapshots, but are no longer visited by `refresh`.
    pub fn detach(&mut self, bound: &mut Bound, appearance: usize) -> Result<(), String> {
        let indices = self
            .appearances
            .get(appearance)
            .ok_or("appearance geometry instance absent")?;
        for &index in indices {
            let binding = self
                .bindings
                .get(index)
                .ok_or("removed mesh binding absent")?;
            if binding
                .bones
                .iter()
                .any(|&b| b >= bound.bone_bounds_generations.len())
            {
                return Err("removed mesh bone mapping invalid".into());
            }
        }
        let indices = self.appearances.remove(appearance);
        for index in indices.into_iter().rev() {
            for &bone in self.bindings[index].bones.iter().rev() {
                bound.bone_bounds_generations[bone] =
                    bound.bone_bounds_generations[bone].wrapping_add(1);
                bound.skeleton.geometry.generation =
                    bound.skeleton.geometry.generation.wrapping_add(1);
            }
            // 59e23c invalidates after releasing the drawable, then 5956f8
            // invalidates after removing the instance's binding record.
            bound.skeleton.geometry.generation = bound.skeleton.geometry.generation.wrapping_add(2);
            bound.skeleton.geometry.dirty = true;
        }
        bound.skeleton.geometry.generation = bound.skeleton.geometry.generation.wrapping_add(1);
        bound.skeleton.geometry.dirty = true;
        Ok(())
    }
    fn reset_vertices(&mut self, bound: &mut Bound) {
        for stamp in &mut bound.skeleton.geometry.bone_generations {
            *stamp = stamp.wrapping_add(1);
        }
        self.bone_vertex_bounds.fill(Bounds::EMPTY);
        bound.skeleton.skin.animator.geometry_changed();
    }
    fn publish_generation(bound: &mut Bound) {
        let stamp = bound.skeleton.geometry.bone_generations[0];
        bound.skeleton.geometry.bounds_generation = stamp;
        bound.skeleton.skin.animator.bounds_updated(stamp);
    }
    fn poses(&self, bound: &Bound, meshes: &[MeshBounds]) -> Result<Vec<Pose>, String> {
        if bound.skeleton.skin.busy() {
            return Err("model geometry during skin callback".into());
        }
        let bones = bound.skeleton.hierarchy.absolute.len();
        if bones == 0
            || self.bone_vertex_bounds.len() != bones
            || bound.skeleton.geometry.bone_generations.len() != bones
            || bound.skeleton.hierarchy.updates.len() != bones
            || bound.bone_bounds_generations.len() != bones
        {
            return Err("model geometry bone storage mismatch".into());
        }
        for &index in self.appearances.iter().flatten() {
            let binding = self
                .bindings
                .get(index)
                .ok_or("appearance geometry binding absent")?;
            let mesh = meshes.get(binding.mesh).ok_or("bound mesh cache absent")?;
            if binding.bones.len() != mesh.local.len()
                || mesh.stamps.len() != mesh.local.len()
                || binding.bones.iter().any(|&b| b >= bones)
            {
                return Err("bound mesh bone mapping invalid".into());
            }
        }
        bound
            .skeleton
            .hierarchy
            .absolute
            .iter()
            .zip(&bound.skeleton.hierarchy.updates)
            .map(|(&pose, &stamp)| {
                Pose::from_absolute_with_control(
                    pose,
                    stamp,
                    bound.skeleton.skin.animator.fp_control(),
                )
            })
            .collect()
    }
    fn refresh_meshes(
        &mut self,
        bound: &mut Bound,
        meshes: &mut [MeshBounds],
        force: bool,
        poses: &[Pose],
    ) -> Result<(), String> {
        // 59934b clears skin/vertex bounds first; it does not force the mesh's
        // pose-stamp gate. It can therefore reuse a previously posed mesh box.
        if force || bound.skeleton.geometry.dirty {
            bound.local_bounds = Bounds::EMPTY;
            self.reset_vertices(bound);
            for &index in self.appearances.iter().flatten() {
                let binding = &self.bindings[index];
                let inputs: Vec<_> = binding.bones.iter().map(|&i| poses[i]).collect();
                meshes[binding.mesh].update(&inputs, false)?;
            }
            bound.skeleton.geometry.dirty = false;
        }
        Ok(())
    }
    /// 5a15de(31, force) / 5a1439 after HEAD hierarchy propagation. The
    /// selector 31 visits every appearance entry. Only bindings containing a
    /// classified +12c bone update their boxes, but the resulting combined box
    /// is published to every visited drawable. Model/skin bounds are retained.
    pub fn refresh_head(
        &mut self,
        bound: &mut Bound,
        meshes: &mut [MeshBounds],
        force: bool,
    ) -> Result<(), String> {
        let poses = self.poses(bound, meshes)?;
        if bound.skeleton.named_bone_flags.len() != poses.len() {
            return Err("head geometry bone classification absent".into());
        }
        self.refresh_meshes(bound, meshes, force, &poses)?;
        let mut combined = Bounds::EMPTY;
        for &index in self.appearances.iter().flatten() {
            let binding = &mut self.bindings[index];
            if !binding
                .bones
                .iter()
                .any(|&bone| bound.skeleton.named_bone_flags[bone])
            {
                continue;
            }
            let inputs: Vec<_> = binding.bones.iter().map(|&i| poses[i]).collect();
            meshes[binding.mesh].update(&inputs, false)?;
            binding.bounds = meshes[binding.mesh].bounds;
            binding.drawable.publish(binding.bounds);
            if valid(binding.bounds) {
                for axis in 0..3 {
                    combined.minimum[axis] =
                        combined.minimum[axis].min(binding.bounds.minimum[axis]);
                    combined.maximum[axis] =
                        combined.maximum[axis].max(binding.bounds.maximum[axis]);
                }
            }
            bound.skeleton.skin.animator.geometry_changed();
        }
        if valid(combined) {
            for &index in self.appearances.iter().flatten() {
                self.bindings[index].drawable.publish(combined);
            }
        }
        Ok(())
    }
    pub fn refresh(
        &mut self,
        bound: &mut Bound,
        meshes: &mut [MeshBounds],
        force: bool,
    ) -> Result<(), String> {
        let poses = self.poses(bound, meshes)?;
        self.refresh_meshes(bound, meshes, force, &poses)?;
        let update = |binding: &Binding, meshes: &mut [MeshBounds]| -> Result<(), String> {
            let inputs: Vec<_> = binding.bones.iter().map(|&i| poses[i]).collect();
            meshes[binding.mesh].update(&inputs, false)?;
            Ok(())
        };
        // 5a1103's independent gate and empty-appearance branch.
        if !force
            && bound.skeleton.geometry.bounds_generation
                == bound.skeleton.skin.animator.generation()
        {
            return Ok(());
        }
        let previous = bound.local_bounds;
        if self.appearances.is_empty() {
            if valid(previous) {
                bound.local_bounds = Bounds::EMPTY;
                self.reset_vertices(bound);
                bound.render_bounds = Bounds::EMPTY;
                bound.model.bounds_flags &= !1;
            }
            if bound.skeleton.geometry.evaluated_generation != bound.skeleton.geometry.generation {
                bound.skeleton.geometry.generation = bound.bone_bounds_generations[0];
                bound.skeleton.geometry.evaluated_generation = bound.skeleton.geometry.generation;
            }
            if bound.skeleton.geometry.bounds_generation
                != bound.skeleton.skin.animator.generation()
            {
                Self::publish_generation(bound);
            }
            return Ok(());
        }
        let mut combined = Bounds::EMPTY;
        for &index in self.appearances.iter().flatten() {
            let binding = &mut self.bindings[index];
            binding.bounds = Bounds::EMPTY;
            update(binding, meshes)?;
            binding.bounds = meshes[binding.mesh].bounds;
            binding.drawable.publish(binding.bounds);
            if valid(binding.bounds) {
                for axis in 0..3 {
                    combined.minimum[axis] =
                        combined.minimum[axis].min(binding.bounds.minimum[axis]);
                    combined.maximum[axis] =
                        combined.maximum[axis].max(binding.bounds.maximum[axis]);
                }
            }
        }
        Self::publish_generation(bound);
        let published = if valid(combined) { combined } else { previous };
        bound.skeleton.geometry.generation = bound.skeleton.geometry.generation.wrapping_add(1);
        bound.local_bounds = published;
        bound.render_bounds = published;
        bound.model.bounds_flags &= !1;
        for &index in self.appearances.iter().flatten() {
            self.bindings[index].drawable.publish(published);
        }
        Ok(())
    }
}
fn valid(b: Bounds) -> bool {
    (0..3).all(|i| b.maximum[i] >= b.minimum[i])
}
