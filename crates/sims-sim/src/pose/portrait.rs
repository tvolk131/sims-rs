//! Native portrait model selection and shared camera-matrix math. The caller
//! must run forced 58a178 appearance selection before collecting models.
//! View acquisition, GPU drawing, lighting and crop are separate services.
use super::model::Matrix;
use serde::{Deserialize, Serialize};

pub mod border;
pub mod draw;
pub mod view;

/// 4e810d clears the destination, then visits appearances and bindings in order.
/// 431726 uses exact length/byte equality; duplicates and null model values stay.
pub fn collect_models<'a, T: Copy, I>(appearances: I, output: &mut Vec<T>)
where
    I: IntoIterator,
    I::Item: IntoIterator<Item = (&'a [u8], T)>,
{
    output.clear();
    for appearance in appearances {
        for (name, model) in appearance {
            if name == b"HEAD" {
                output.push(model);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// Actual camera's +a8 inverse screen matrix, including its cached kind.
    pub inverse: Matrix,
    pub rect: [i32; 4],
    pub zoom: u32,
    pub rotation: i32,
    pub angles: [f32; 2],
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub matrix: Matrix,
    pub projected: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CropInput {
    pub target: [i32; 2],
    pub fixed: Option<[i32; 4]>,
    /// 4c357c's successful [left, top, right, bottom] border counts. None is
    /// scan failure: the native caller retains the entire scratch surface.
    pub margins: Option<[i32; 4]>,
    pub padding: [i32; 4],
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    pub rect: [i32; 4],
    pub scale: [f32; 2],
}
impl CropInput {
    /// 4e7cba..4e7e63. Return the source rectangle and scaling arguments, not
    /// pixels. The caller supplies a real border scan or the generator's fixed
    /// crop. Keep native asymmetric expansion at the scratch surface edges.
    pub fn crop(self) -> Result<Crop, String> {
        if self.target.iter().any(|&n| n <= 0) {
            return Err("invalid portrait target dimensions".into());
        }
        let mut rect = self.fixed.unwrap_or_else(|| {
            let margins = self.margins.map_or([0; 4], |mut margins| {
                for (margin, padding) in margins.iter_mut().zip(self.padding) {
                    if *margin >= padding {
                        *margin = margin.wrapping_sub(padding);
                    }
                }
                margins
            });
            [
                margins[0],
                margins[1],
                200_i32.wrapping_sub(margins[2]),
                200_i32.wrapping_sub(margins[3]),
            ]
        });
        if rect.iter().any(|&n| !(0..=200).contains(&n)) || rect[0] >= rect[2] || rect[1] >= rect[3]
        {
            return Err("invalid portrait source rectangle".into());
        }
        let aspect = f64::from(self.target[0]) / f64::from(self.target[1]);
        let width = rect[2] - rect[0];
        let height = rect[3] - rect[1];
        let source = f64::from(width) / f64::from(height);
        let expansion = if source > aspect {
            Some((
                1,
                ((f64::from(width) / aspect + 0.5).trunc().min(200.)) as i32 - height,
            ))
        } else if source < aspect {
            Some((
                0,
                ((f64::from(height) * aspect + 0.5).trunc().min(200.)) as i32 - width,
            ))
        } else {
            None
        };
        if let Some((axis, mut extra)) = expansion {
            while extra > 1 && rect[axis] > 0 && rect[axis + 2] < 200 {
                rect[axis] -= 1;
                rect[axis + 2] += 1;
                extra -= 2;
            }
            while extra > 0 && rect[axis + 2] < 200 {
                rect[axis + 2] += 1;
                extra -= 1;
            }
        }
        Ok(Crop {
            rect,
            scale: std::array::from_fn(|axis| {
                (f64::from(self.target[axis]) / f64::from(rect[axis + 2] - rect[axis])) as f32
            }),
        })
    }
}
fn rotation(axis: usize, angle: f32) -> Matrix {
    let (s, c) = f64::from(angle).sin_cos();
    let (s, c) = (s as f32, c as f32);
    let mut m = Matrix::IDENTITY;
    m.kind = 0;
    if axis == 0 {
        m.values[5] = c;
        m.values[6] = s;
        m.values[9] = -s;
        m.values[10] = c;
    } else if axis == 1 {
        m.values[0] = c;
        m.values[2] = -s;
        m.values[8] = s;
        m.values[10] = c;
    } else {
        m.values[0] = c;
        m.values[1] = s;
        m.values[4] = -s;
        m.values[5] = c;
    }
    m
}
impl Camera {
    /// 4e7905..4e7ad1: screen-center projection and row-matrix composition.
    /// Preserve explicit f32 stores between x87 calculations and native tags.
    pub fn transform(self) -> Result<Transform, String> {
        if self.angles.iter().any(|a| !a.is_finite()) {
            return Err("invalid portrait angles".into());
        }
        let center = [
            (f64::from(self.rect[2].wrapping_sub(self.rect[0])) * 0.5 + f64::from(self.rect[0]))
                as f32,
            (f64::from(self.rect[3].wrapping_sub(self.rect[1])) * 0.5 + f64::from(self.rect[1]))
                as f32,
            32767.,
        ];
        let projected = self.inverse.point(center)?;
        let scale = (30. / f64::from(1_i32.wrapping_shl(self.zoom & 31))) as f32;
        let mut matrix = Matrix::IDENTITY;
        matrix.kind = 0;
        for i in [0, 5, 10] {
            matrix.values[i] = scale;
        }
        let pi = f64::from(std::f32::consts::PI);
        matrix = matrix.multiply(rotation(
            1,
            (pi * f64::from(0.05_f32) + f64::from(self.angles[0])) as f32,
        ))?;
        matrix = matrix.multiply(rotation(2, (pi * 0.5) as f32))?;
        matrix = matrix.multiply(rotation(
            1,
            ((f64::from(self.rotation as f32) - 0.5) * pi * -0.5 + f64::from(self.angles[1]))
                as f32,
        ))?;
        let mut translation = Matrix::IDENTITY;
        translation.kind = 2;
        translation.values[12..15].copy_from_slice(&projected);
        matrix = matrix.multiply(translation)?;
        Ok(Transform { matrix, projected })
    }
}
