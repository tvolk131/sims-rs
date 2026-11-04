//! Native animator placement (592132), before renderer handedness conversion.
//! Cached coordinates and root offsets belong to the animator. The physical
//! object supplies fallback coordinates, facing, floor and person flags.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// Native +80/+84/+88: source position[1], position[0], facing.
    pub base: [f32; 3],
    /// Native +1c4/+1c8/+1cc: movement target. The placement calculator updates
    /// these only for sentinel cache replacements; the movement controller also
    /// writes them. The serialized `previous` name is retained for compatibility.
    pub previous: [f32; 3],
    /// Native +8c/+90/+94, added after cache fallback.
    pub offset: [f32; 3],
    /// Native +d9, marked by placement-cache invalidation. The transform itself
    /// does not clear it; the enclosing animator update owns that decision.
    pub dirty: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Source {
    /// Physical +f4/+f8, in original word order.
    pub position: [i32; 2],
    pub facing: i16,
    pub level: i32,
    /// Low bytes of person fields 84 and 68.
    pub flags: [u8; 2],
}

#[derive(Debug, Clone, Copy)]
pub struct View {
    pub rotation: u8,
    /// Camera data +b8 - zoom*12. Supplied by the active native view profile.
    /// This vector is scaled by level-1, then rotated back by camera direction.
    pub floor_vector: [f32; 3],
}

pub struct Terrain<'a> {
    pub size: usize,
    /// Native Arry-0 column storage; corner bytes retain their original order.
    pub corners: &'a [[u8; 4]],
}
impl Terrain<'_> {
    fn validate(&self) -> Result<(), String> {
        if !(2..=64).contains(&self.size) || self.corners.len() != self.size * self.size {
            return Err("invalid native animator terrain array".into());
        }
        Ok(())
    }
    /// Original coordinate conversion narrows shifted coordinates to signed
    /// bytes before bounds checks. Keep that behavior, including wrapped inputs.
    fn cell(&self, point: [i32; 2], inset: bool) -> Option<[usize; 2]> {
        let [a, b] = point.map(|p| (p >> 4) as i8 as i32);
        let edge = i32::from(inset);
        (a >= edge && b >= edge && a < self.size as i32 - edge && b < self.size as i32 - edge)
            .then_some([b as usize, a as usize])
    }
    /// 478dfe uses two triangular planes split along the tile's diagonal. Its
    /// x87 result remains unspilled until the enclosing placement scales it.
    pub fn height(&self, point: [i32; 2]) -> Result<Option<f64>, String> {
        self.height_with_control(point, 0x037f)
    }
    pub fn height_with_control(&self, point: [i32; 2], control: u16) -> Result<Option<f64>, String> {
        if !matches!(control,0x003f|0x037f){return Err("unsupported native terrain x87 control".into());}
        self.validate()?;
        let q = |v| {
            if control == 0x003f {
                retained_single(v)
            } else {
                v
            }
        };
        let Some([column, row]) = self.cell(point, false) else {
            return Ok(None);
        };
        let [c0, c1, c2, c3] = self.corners[column * self.size + row].map(f64::from);
        let a = f64::from(point[1] & 15);
        let b = f64::from(point[0] & 15);
        let value = if a == b {
            q(q(q(q(c3 - c1) * a) * 0.0625) + c1)
        } else if a <= b {
            let along = q(q(q(c0 - c1) * b) * 0.0625);
            let across = q(q(q(c3 - c1) * b) * 0.0625);
            let delta = q(q(q(across - along) * a) / b);
            q(q(delta + along) + c1)
        } else {
            let along = q(q(q(c2 - c1) * a) * 0.0625);
            let across = q(q(q(c3 - c1) * a) * 0.0625);
            let delta = q(q(q(across - along) * b) / a);
            q(q(delta + along) + c1)
        };
        Ok(Some(value))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// Source-handed affine matrix consumed by native bone/attachment sampling.
    pub matrix: [f32; 16],
    pub terrain_point: [i32; 2],
    /// The original pure pool query runs at this byte tile when flags allow it.
    /// Its return value is discarded by 592132; it does not change placement.
    pub pool_query: Option<[i8; 3]>,
}
impl Placement {
    /// Native 5893a9 -> 589361: invalidate only cached XY/heading, preserve
    /// placement compensation and previous coordinates, and mark placement dirty.
    pub fn invalidate(&mut self) {
        self.base = [-999.; 3];
        self.dirty = true;
    }
    pub fn transform(
        &mut self,
        source: Source,
        view: View,
        terrain: Terrain<'_>,
    ) -> Result<Transform, String> {
        self.transform_with_control(source, view, terrain, 0x037f, None)
    }
    /// The retained process owns x87 precision; isolated legacy probes used
    /// 037f. Do not infer that precision from a renderer or the host CPU.
    pub fn transform_with_control(
        &mut self,
        source: Source,
        view: View,
        terrain: Terrain<'_>,
        control: u16,
        terrain_scale: Option<f32>,
    ) -> Result<Transform, String> {
        if !matches!(control, 0x003f | 0x037f) {
            return Err("unsupported native placement x87 control".into());
        }
        let q = |v| {
            if control == 0x003f {
                retained_single(v)
            } else {
                v
            }
        };
        terrain.validate()?;
        if view.rotation > 3
            || self
                .base
                .iter()
                .chain(&self.previous)
                .chain(&self.offset)
                .chain(&view.floor_vector)
                .any(|v| !v.is_finite())
        {
            return Err("invalid native animator placement inputs".into());
        }
        let fallback = [
            source.position[1] as f32,
            source.position[0] as f32,
            f32::from(source.facing),
        ];
        for (i, value) in fallback.into_iter().enumerate() {
            if (f64::from(self.base[i]) + 999.).abs() <= f64::from(0.000001_f32) {
                self.base[i] = value;
                self.previous[i] = value;
            }
        }
        let position: [f32; 3] = std::array::from_fn(|i| self.base[i] + self.offset[i]);
        let round = |v: f32| -> Result<i32, String> {
            let v = (f64::from(v) + 0.5).trunc();
            if !v.is_finite() || v < f64::from(i32::MIN) || v > f64::from(i32::MAX) {
                return Err("native animator point conversion overflow".into());
            }
            Ok(v as i32)
        };
        let point = [round(position[1])?, round(position[0])?];
        let height = if let Some(height) = terrain.height_with_control(point, control)? {
            let scale = match terrain_scale {
                Some(scale) if scale.is_finite() && scale > 0. => scale,
                Some(_) => return Err("invalid retained terrain scale".into()),
                None if control == 0x003f => {
                    return Err(
                        "retained placement needs its initialized terrain scale owner".into(),
                    );
                }
                None => (1. / (2. * 6_f64.sqrt())) as f32,
            };
            q(q(f64::from(scale) * 3.) * height) as f32
        } else {
            0.
        };
        let levels = source.level.wrapping_sub(1) as f32;
        let floor = view.floor_vector.map(|v| levels * v);
        let angle = q(q(-f64::from(view.rotation) * f64::from(std::f32::consts::PI)) * 0.5) as f32;
        let (s, c) = f64::from(angle).sin_cos();
        let (s, c) = (f64::from(s as f32), f64::from(c as f32));
        let floor = [
            q(q(f64::from(floor[0]) * c) + q(f64::from(floor[2]) * s)) as f32,
            floor[1],
            q(q(-f64::from(floor[0]) * s) + q(f64::from(floor[2]) * c)) as f32,
        ];
        // The y-axis source displacement spills before its floor offset; the
        // x-axis product remains extended until the final translation store.
        let z = 0.1875 * position[0];
        let translation = [
            q(f64::from(floor[0]) + q(0.1875 * f64::from(position[1]))) as f32,
            floor[1] + height,
            floor[2] + z,
        ];
        let angle = q(
            q(q(q(f64::from(position[2]) + 6.) * 45.) * f64::from(std::f32::consts::PI))
                * f64::from(f32::from_bits(0x3bb60b61)),
        ) as f32;
        let (s, c) = f64::from(angle).sin_cos();
        let (s, c) = (s as f32, c as f32);
        let matrix = [
            c,
            0.,
            -s,
            0.,
            0.,
            1.,
            0.,
            0.,
            s,
            0.,
            c,
            0.,
            translation[0],
            translation[1],
            translation[2],
            1.,
        ];
        if matrix.iter().any(|v| !v.is_finite()) {
            return Err("native animator matrix overflow".into());
        }
        let pool_query = (source.flags.iter().all(|f| f & 1 == 0)
            && terrain.cell(point, true).is_some())
        .then_some([
            (point[1] >> 4) as i8,
            (point[0] >> 4) as i8,
            source.level as i8,
        ]);
        Ok(Transform {
            matrix,
            terrain_point: point,
            pool_query,
        })
    }
}

/// Optional, explicitly retained terrain owners. An absent extent or directory
/// is unknown, not an empty/flat lot. Native45ee2e can skip both before any read.
/// In-bounds height evaluation still needs the separate703f28/703f2c cache owner
/// and is deliberately unsupported even when a corner directory is supplied.
#[derive(Clone, Copy)]
pub struct RetainedTerrain<'a> {
    pub extent: Option<usize>,
    pub corners: Option<&'a [[u8; 4]]>,
}
impl RetainedTerrain<'_> {
    fn require_outside(&self, point: [i32; 2]) -> Result<(), String> {
        //45ee21..45ee30: point[1] is tested first, after narrowing the shifted
        // value to signed AL. Even a negative point[0] does not skip extent.
        let first = (point[1] >> 4) as i8 as i32;
        if first < 0 {
            return Ok(());
        }
        let extent = self
            .extent
            .ok_or("retained placement terrain extent is unknown")?;
        let extent_i32 = i32::try_from(extent)
            .map_err(|_| "retained placement terrain extent exceeds signed native domain")?;
        if first >= extent_i32 {
            return Ok(());
        }
        let second = (point[0] >> 4) as i8 as i32;
        if second < 0 || second >= extent_i32 {
            return Ok(());
        }
        let corners = self
            .corners
            .ok_or("retained placement corner directory is unknown")?;
        if extent.checked_mul(extent) != Some(corners.len()) {
            return Err("invalid retained placement corner directory extent".into());
        }
        Err("retained in-bounds placement needs the703f28/703f2c scale-cache owner".into())
    }
}

// Round finite intermediates to the003f precision-control's24 significand bits,
// retaining a wider exponent until the actual f32 store. Narrowing every basic
// operation to f32 would introduce premature subnormal/overflow stores that the
// x87 instruction sequence does not perform. Products/additions in this bounded
// path start from <=32-bit integers or single values and fit f64's exponent.
pub(super) fn retained_single(value: f64) -> f64 {
    if value == 0. || !value.is_finite() {
        return value;
    }
    let bits = value.to_bits();
    let mask = (1u64 << 29) - 1;
    let tail = bits & mask;
    let half = 1u64 << 28;
    let up = tail > half || tail == half && (bits >> 29) & 1 != 0;
    f64::from_bits((bits & !mask) + if up { 1 << 29 } else { 0 })
}
fn retained_sin_cos(angle: f32) -> Result<(f32, f32), String> {
    // Native5c3a36..5c3a48 uses FCOS/FSIN followed by single stores. Their range
    // failure semantics are not supplied by the portable transcendental calls.
    if !angle.is_finite() || f64::from(angle).abs() >= 2f64.powi(63) {
        return Err("unsupported retained placement transcendental range".into());
    }
    let (s, c) = f64::from(angle).sin_cos();
    Ok((s as f32, c as f32))
}
impl Placement {
    /// Source-bound ordinary placement math with an actual precision-control
    /// input. Supports003f and proven outside-lot exits, not an invented terrain
    /// or scale cache. Unsupported/missing inputs leave every cache unchanged.
    /// Existing transform(Terrain) keeps its original strict validation/math.
    /// The two retained Goth matrices are bit-exact tests; this is not a general
    /// x87 transcendental emulator or a whole-frame/native parity contract.
    pub fn transform_retained(
        &mut self,
        source: Source,
        view: View,
        terrain: RetainedTerrain<'_>,
        actual_fp_control: u16,
    ) -> Result<Transform, String> {
        if actual_fp_control != 0x003f {
            return Err("unsupported retained placement x87 control".into());
        }
        if view.rotation > 3
            || self
                .base
                .iter()
                .chain(&self.previous)
                .chain(&self.offset)
                .chain(&view.floor_vector)
                .any(|v| !v.is_finite())
        {
            return Err("invalid native animator placement inputs".into());
        }
        let mut candidate = self.clone();
        let fallback = [
            source.position[1] as f32,
            source.position[0] as f32,
            f32::from(source.facing),
        ];
        for (i, value) in fallback.into_iter().enumerate() {
            if retained_single(f64::from(candidate.base[i]) + 999.).abs() <= f64::from(0.000001_f32)
            {
                candidate.base[i] = value;
                candidate.previous[i] = value;
            }
        }
        let position: [f32; 3] = std::array::from_fn(|i| {
            retained_single(f64::from(candidate.base[i]) + f64::from(candidate.offset[i])) as f32
        });
        let integer = |value: f32| -> Result<i32, String> {
            let value = retained_single(f64::from(value) + 0.5).trunc();
            if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
                return Err("native animator point conversion overflow".into());
            }
            Ok(value as i32)
        };
        let point = [integer(position[1])?, integer(position[0])?];
        terrain.require_outside(point)?;
        //5868d9 initializes height to +0 before45ee1d; the supported outside
        // branches skip interpolation and the mutable cached scale entirely.
        let height = 0.;
        let levels = source.level.wrapping_sub(1) as f32;
        let floor = view
            .floor_vector
            .map(|v| retained_single(f64::from(levels) * f64::from(v)) as f32);
        let angle = retained_single(
            retained_single(f64::from(-i32::from(view.rotation)) * f64::from(std::f32::consts::PI))
                * 0.5,
        ) as f32;
        let floor = if angle == 0. {
            //5c39aa identity-kind fast path copies all three source words.
            floor
        } else {
            let (s, c) = retained_sin_cos(angle)?;
            let q = retained_single;
            let [x, y, z] = floor.map(f64::from);
            let [s, c] = [f64::from(s), f64::from(c)];
            //5c4532..5c4584 multiplication/addition order, including zero terms.
            [
                q(q(q(q(s * z) + q(0. * y)) + q(x * c)) + 0.) as f32,
                q(q(q(q(0. * x) + q(0. * z)) + q(y * 1.)) + 0.) as f32,
                q(q(q(q(-s * x) + q(c * z)) + q(0. * y)) + 0.) as f32,
            ]
        };
        let q = retained_single;
        let factor = q(3. * f64::from(0.0625_f32));
        let z = q(factor * f64::from(position[0])) as f32;
        let translation = [
            q(f64::from(floor[0]) + q(factor * f64::from(position[1]))) as f32,
            q(f64::from(floor[1]) + height) as f32,
            q(f64::from(floor[2]) + f64::from(z)) as f32,
        ];
        //586a35/3b/47/49 all round under actual PC24 before the angle store.
        let angle = q(
            q(q(q(f64::from(position[2]) + 6.) * 45.) * f64::from(std::f32::consts::PI))
                * f64::from(f32::from_bits(0x3bb60b61)),
        ) as f32;
        let (s, c) = retained_sin_cos(angle)?;
        let matrix = [
            c,
            0.,
            -s,
            0.,
            0.,
            1.,
            0.,
            0.,
            s,
            0.,
            c,
            0.,
            translation[0],
            translation[1],
            translation[2],
            1.,
        ];
        if matrix.iter().any(|v| !v.is_finite()) {
            return Err("native animator matrix overflow".into());
        }
        //Every supported terrain branch is outside the full extent; it is also
        // outside the inset pool domain. The ordinary graphic uses flags[1;2].
        let result = Transform {
            matrix,
            terrain_point: point,
            pool_query: None,
        };
        *self = candidate;
        Ok(result)
    }
}

#[cfg(test)]
mod retained_tests;
