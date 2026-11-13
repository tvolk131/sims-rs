//! Native model transforms (585515/5a0e32/5a0ffd) and body scale (5a0932).
//! These publish placement to the skeleton and its bounds; they do not sample
//! animation or update mesh bounds. Matrix kind is original cached metadata.
use serde::{Deserialize, Serialize};

/// Native empty box from 66745c/667458; an absent mesh is not a point at zero.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub minimum: [f32; 3],
    pub maximum: [f32; 3],
}
impl Bounds {
    pub const EMPTY: Self = Self {
        minimum: [1e38; 3],
        maximum: [-1e38; 3],
    };
}

/// Model/skeleton owner after 5a0646/5a06d4's normal named-skeleton branch,
/// with no optional appearance or animation. The pose is still unpublished.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bound {
    pub skeleton: super::skeleton::Instance,
    pub model: Model,
    pub matrix: Matrix,
    pub local_bounds: Bounds,
    pub render_bounds: Bounds,
    pub bone_bounds: Vec<Bounds>,
    /// Native +134. Distinct from pose stamps (+130) and vertex stamps (+150).
    pub bone_bounds_generations: Vec<u32>,
}
impl Bound {
    pub fn new(source: &sims_formats::avatar::Skeleton, crt: &mut u32) -> Result<Self, String> {
        Self::from_skeleton(super::skeleton::Instance::clone_prototype(source, crt)?)
    }
    /// The built-in `root` branch of 5a06d4 constructs one bone directly; it
    /// neither looks up a prototype nor executes the clone setters/RNG draws.
    pub fn root(crt: &mut u32) -> Result<Self, String> {
        Self::from_skeleton(super::skeleton::Instance::root(crt))
    }
    fn from_skeleton(skeleton: super::skeleton::Instance) -> Result<Self, String> {
        let bones = skeleton.skin.bones.len();
        let mut bound = Self {
            skeleton,
            model: Model {
                placement: Matrix::IDENTITY,
                scale: Matrix::IDENTITY,
                bounds_transform: Matrix::IDENTITY,
                // 5cf799 defines the low three bits as 1. Reserved allocator
                // bits retained by native code are normalized to zero.
                bounds_flags: 7,
            },
            matrix: Matrix::IDENTITY,
            local_bounds: Bounds::EMPTY,
            render_bounds: Bounds::EMPTY,
            bone_bounds: vec![Bounds::EMPTY; bones],
            bone_bounds_generations: vec![u32::MAX; bones],
        };
        bound.refresh_geometry(false)?;
        Ok(bound)
    }
    /// 588d6a with its fresh, empty accessory vector. This owner is still bare;
    /// attaching appearances requires a host that retains their mesh geometry.
    pub fn refresh_empty_accessories(&mut self) {
        self.skeleton.geometry.generation = self.skeleton.geometry.generation.wrapping_add(1);
        self.skeleton.skin.animator.geometry_changed();
        self.skeleton.geometry.dirty = true;
    }
    pub fn publish(&mut self, placement: Matrix) -> Result<(), String> {
        self.matrix = self
            .model
            .publish_with_control(placement, self.skeleton.skin.animator.fp_control())?;
        self.skeleton.skin.animator.placement_updated()
    }
    /// 5a15bc / 59934b / 5a1103, restricted to this model's empty appearance
    /// vector. It does not evaluate a pose or invent vertex bounds.
    pub fn refresh_geometry(&mut self, force: bool) -> Result<(), String> {
        if self.skeleton.skin.busy() {
            return Err("model geometry during skin callback".into());
        }
        let geometry = &mut self.skeleton.geometry;
        if force || geometry.dirty {
            self.local_bounds = Bounds::EMPTY;
            for stamp in &mut geometry.bone_generations {
                *stamp = stamp.wrapping_add(1);
            }
            self.skeleton.skin.animator.geometry_changed();
            geometry.dirty = false;
        }
        if force || geometry.bounds_generation != self.skeleton.skin.animator.generation() {
            if geometry.evaluated_generation != geometry.generation {
                geometry.generation = self.bone_bounds_generations[0];
                geometry.evaluated_generation = geometry.generation;
            }
            if geometry.bounds_generation != self.skeleton.skin.animator.generation() {
                geometry.bounds_generation = geometry.bone_generations[0];
                self.skeleton
                    .skin
                    .animator
                    .bounds_updated(geometry.bounds_generation);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Matrix {
    pub values: [f32; 16],
    pub kind: u32,
}
impl Matrix {
    pub const IDENTITY: Self = Self {
        values: [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ],
        kind: 1,
    };
    fn validate(&self) -> Result<(), String> {
        if self.values.iter().any(|v| !v.is_finite()) {
            return Err("invalid native model matrix".into());
        }
        Ok(())
    }
    /// 5c39a0 / 5c4510 point transformation, preserving native kind shortcuts.
    pub fn point(self, point: [f32; 3]) -> Result<[f32; 3], String> {
        self.validate()?;
        if point.iter().any(|v| !v.is_finite()) {
            return Err("invalid native model point".into());
        }
        let result = match self.kind {
            1 => point,
            2 => std::array::from_fn(|i| point[i] + self.values[12 + i]),
            _ => {
                // 5c4510 retains the reciprocal in x87 until all three output
                // stores. This is a projective transform, including when a
                // caller supplied a non-affine matrix with the general tag.
                let term = |row: usize, column: usize| {
                    f64::from(point[row]) * f64::from(self.values[row * 4 + column])
                };
                let reciprocal =
                    1. / (term(0, 3) + term(2, 3) + term(1, 3) + f64::from(self.values[15]));
                std::array::from_fn(|i| {
                    let sum = if i == 0 {
                        term(2, i) + term(1, i) + term(0, i)
                    } else {
                        term(0, i) + term(2, i) + term(1, i)
                    };
                    ((sum + f64::from(self.values[12 + i])) * reciprocal) as f32
                })
            }
        };
        if result.iter().any(|v| !v.is_finite()) {
            return Err("native model point overflow".into());
        }
        Ok(result)
    }
    /// 49b87c / 462af3. Kind 1 bypasses multiplication even if the cached values
    /// differ; recomputing the kind from the array would change native behavior.
    pub fn multiply(self, other: Self) -> Result<Self, String> {
        self.multiply_with_control(other, 0x037f)
    }
    pub fn multiply_with_control(self, other: Self, control: u16) -> Result<Self, String> {
        if !matches!(control, 0x003f | 0x037f) {
            return Err("unsupported native matrix arithmetic control".into());
        }
        let q = |v: f64| {
            if control == 0x003f {
                super::transform::retained_single(v)
            } else {
                v
            }
        };
        self.validate()?;
        other.validate()?;
        if self.kind == 1 {
            return Ok(other);
        }
        if other.kind == 1 {
            return Ok(self);
        }
        let values = std::array::from_fn(|i| {
            let (row, column) = (i / 4, i % 4);
            // Original x87 addition order differs in the translation row.
            let order = match (row == 3, column == 0) {
                (false, true) => [2, 1, 0, 3],
                (false, false) => [0, 3, 2, 1],
                (true, true) => [3, 2, 1, 0],
                (true, false) => [3, 1, 2, 0],
            };
            let term = |k| {
                q(f64::from(self.values[row * 4 + k]) * f64::from(other.values[k * 4 + column]))
            };
            order[1..]
                .iter()
                .fold(term(order[0]), |sum, &k| q(sum + term(k))) as f32
        });
        let result = Self { values, kind: 0 };
        result.validate()?;
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    /// Native model +24, before applying its +68 body scale.
    pub placement: Matrix,
    pub scale: Matrix,
    /// Render/bounds object +18/+58 and its flags +14.
    pub bounds_transform: Matrix,
    pub bounds_flags: u32,
}
impl Model {
    /// 5a0932 changes scale without publishing placement or sampling a pose.
    pub fn set_scale(&mut self, scale: [f32; 3]) -> Result<(), String> {
        if scale.iter().any(|v| !v.is_finite()) {
            return Err("invalid native body scale".into());
        }
        let mut matrix = Matrix::IDENTITY;
        if scale
            .iter()
            .any(|v| (f64::from(*v) - 1.).abs() > f64::from(0.000001_f32))
        {
            matrix.values[0] = scale[0];
            matrix.values[5] = scale[1];
            matrix.values[10] = scale[2];
            matrix.kind = 0;
        }
        self.scale = matrix;
        Ok(())
    }
    /// 5a0e32 publishes a tagged matrix. The returned transform must be copied
    /// to skeleton +1c/+5c and its pose stamp incremented by 5995e6.
    pub fn publish(&mut self, placement: Matrix) -> Result<Matrix, String> {
        self.publish_with_control(placement, 0x037f)
    }
    pub fn publish_with_control(
        &mut self,
        placement: Matrix,
        control: u16,
    ) -> Result<Matrix, String> {
        let transform = self.prepare_with_control(placement, control)?;
        self.finish(transform);
        Ok(transform)
    }
    /// Visible model update computes this before property callbacks, then keeps
    /// the result on its stack until geometry/bounds have been updated.
    pub(super) fn prepare_with_control(
        &mut self,
        placement: Matrix,
        control: u16,
    ) -> Result<Matrix, String> {
        let transform = self.scale.multiply_with_control(placement, control)?;
        self.placement = placement;
        Ok(transform)
    }
    pub(super) fn finish(&mut self, transform: Matrix) {
        self.bounds_transform = transform;
        self.bounds_flags |= 2;
    }
    /// 5a0ffd first copy-constructs an untagged matrix (5c37a0). Its clock
    /// argument is unused; invisible publication does not advance layers.
    pub fn hidden(&mut self, values: [f32; 16]) -> Result<Matrix, String> {
        self.publish(Matrix { values, kind: 0 })
    }
}
