//! 49b0de's initialized household view, including original device matrices.
//! Viewport and scrolling are caller inputs; this does not own gameplay state.
use super::{Matrix, rotation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// Viewport origin and dimensions, before 5cf206's half-size calculation.
    pub viewport: [u32; 4],
    pub center: [i32; 2],
    pub zoom: u32,
    pub rotation: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub screen: Matrix,
    pub inverse: Matrix,
    pub rotation_matrix: Matrix,
    pub view: Matrix,
    pub projection: Matrix,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub far: f32,
    pub distance: f32,
    pub angles: [f32; 2],
}

fn length(v: [f32; 3]) -> f64 {
    let [x, y, z] = v.map(f64::from);
    (z * z + y * y + x * x).sqrt()
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let inverse = 1. / length(v);
    v.map(|n| (f64::from(n) * inverse) as f32)
}

/// 49b853 / 5c4050: pivoted Gauss-Jordan, retaining the float stores between
/// products and subtraction. Singular native matrices throw; report an error.
pub fn inverse(matrix: Matrix) -> Result<Matrix, String> {
    if matrix.values.iter().any(|n| !n.is_finite()) {
        return Err("invalid camera matrix".into());
    }
    let mut a: [[f32; 4]; 4] =
        std::array::from_fn(|r| std::array::from_fn(|c| matrix.values[r * 4 + c]));
    let mut b: [[f32; 4]; 4] =
        std::array::from_fn(|r| std::array::from_fn(|c| if r == c { 1. } else { 0. }));
    for col in 0..4 {
        let mut pivot = col;
        for r in col + 1..4 {
            if a[pivot][col].abs() < a[r][col].abs() {
                pivot = r;
            }
        }
        a.swap(pivot, col);
        b.swap(pivot, col);
        if a[col][col] == 0. {
            return Err("singular camera matrix".into());
        }
        let reciprocal = 1. / f64::from(a[col][col]);
        for c in 0..4 {
            a[col][c] = (f64::from(a[col][c]) * reciprocal) as f32;
            b[col][c] = (f64::from(b[col][c]) * reciprocal) as f32;
        }
        for r in 0..4 {
            if r == col {
                continue;
            }
            let factor = a[r][col];
            for c in 0..4 {
                let product = b[col][c] * factor;
                b[r][c] -= product;
                if c == 0 {
                    a[r][c] =
                        (f64::from(a[r][c]) - f64::from(a[col][c]) * f64::from(factor)) as f32;
                } else {
                    let product = a[col][c] * factor;
                    a[r][c] -= product;
                }
            }
        }
    }
    Ok(Matrix {
        values: std::array::from_fn(|i| b[i / 4][i % 4]),
        kind: u32::from(matrix.kind == 1),
    })
}
fn transpose(m: Matrix) -> Matrix {
    Matrix {
        values: std::array::from_fn(|i| m.values[(i % 4) * 4 + i / 4]),
        kind: u32::from(m.kind == 1),
    }
}

impl Input {
    pub fn build(self) -> Result<View, String> {
        let [x, y, width, height] = self.viewport;
        if !(1..=3).contains(&self.zoom)
            || self.rotation > 3
            || width == 0
            || height == 0
            || self.viewport.iter().any(|&n| n > i32::MAX as u32)
            || self.center.contains(&i32::MIN)
        {
            return Err("unsupported household camera inputs".into());
        }
        let pi = f64::from(std::f32::consts::PI);
        let scalar = (1. / (6_f64.sqrt() + 6_f64.sqrt())) as f32;
        let base = (f64::from(scalar) * 255. * 3. + 68.) as f32;
        let diagonal = (2_f64.sqrt() * 3. * 64.) as f32;
        let far = (f64::from(base) * 0.5 + 3_f64.sqrt() * f64::from(diagonal) * 0.5) as f32;
        let distance = far - 12.5;
        let width_f = width as f32;
        let extent = 2_f64.sqrt() * f64::from(width_f) * 3. / f64::from(16 << self.zoom);
        let step = -((extent / f64::from(width_f)) as f32);
        let yaw =
            (pi * (45. - f64::from(self.rotation) * 90.) * f64::from(0.0055555557_f32)) as f32;
        let pitch = (pi * f64::from(-0.16666667_f32)) as f32;
        let rotation_basis = rotation(0, pitch).multiply(rotation(1, yaw))?;
        let mut position = rotation_basis.point([0., 0., distance])?;
        let direction = normalize(position.map(|n| -n));
        let basis = transpose(inverse(rotation_basis)?);
        let horizontal = basis.point([step, 0., 0.])?;
        let vertical = basis.point([0., step, 0.])?;
        if matches!(self.rotation, 1 | 2) {
            position[0] += 192.;
        }
        if matches!(self.rotation, 2 | 3) {
            position[2] += 192.;
        }
        for (amount, vector) in [
            ((-self.center[0]) as f32, horizontal),
            ((-self.center[1]) as f32, vertical),
            ((width >> 1) as f32, horizontal),
            ((height >> 1) as f32, vertical),
        ] {
            for axis in 0..3 {
                let product = amount * vector[axis];
                position[axis] += product;
            }
        }
        let pitch = (-f64::from(direction[1]) / length(direction)).asin() as f32;
        let flat = normalize([direction[0], 0., direction[2]]);
        let yaw = (f64::from(flat[2]).acos() * if flat[0] < 0. { -1. } else { 1. }) as f32;
        let mut translation = Matrix::IDENTITY;
        translation.kind = 2;
        translation.values[12..15].copy_from_slice(&position);
        let view = inverse(
            rotation(0, pitch)
                .multiply(rotation(1, yaw))?
                .multiply(translation)?,
        )?;
        let half = [(width as f32) * 0.5, (height as f32) * 0.5];
        let aspect = half[0] / half[1];
        let sy = f64::from(extent as f32) * 0.5;
        let sx = sy / f64::from(aspect);
        let product = sx * (sy as f32) as f64;
        let mut projection = Matrix {
            values: [0.; 16],
            kind: 0,
        };
        projection.values[0] = sx as f32;
        projection.values[5] = sy as f32;
        projection.values[10] = (product / f64::from(far)) as f32;
        projection.values[14] = -0.;
        projection.values[15] = product as f32;
        let mut viewport = Matrix::IDENTITY;
        viewport.kind = 0;
        viewport.values[0] = half[0];
        viewport.values[5] = -half[1];
        viewport.values[10] = 65535.;
        viewport.values[12] = x as f32 + half[0];
        viewport.values[13] = y as f32 + half[1];
        let screen = view.multiply(projection)?.multiply(viewport)?;
        Ok(View {
            inverse: inverse(screen)?,
            screen,
            view,
            projection,
            position,
            direction,
            far,
            distance,
            angles: [yaw, pitch],
            rotation_matrix: rotation(
                1,
                (f64::from((0u32.wrapping_sub(self.rotation)) & 3) * pi * 0.5) as f32,
            ),
        })
    }
}

impl View {
    /// 4e7b24..4e7bb0 and 5d0beb: turn the original view direction by pi/9
    /// and normalize it for the temporary portrait light. The caller separately
    /// saves ambient state and sets all four ambient channels to one.
    pub fn portrait_light(self) -> Result<[f32; 3], String> {
        let angle = (f64::from(std::f32::consts::PI) * f64::from(0.11111111_f32)) as f32;
        Ok(normalize(rotation(1, angle).point(self.direction)?))
    }
}
