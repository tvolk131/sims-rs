//! Shared 5a0a8a model visibility: eight projected local-bound corners and a
//! quarter-size screen margin. This uses the supplied native camera matrix;
//! choosing/updating the live camera is a separate service.
use super::model::{Bounds, Matrix};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Projection {
    pub matrix: Matrix,
    pub screen: Option<[f32; 4]>,
    pub rectangle: Option<[i32; 4]>,
    pub visible: bool,
}

fn integer(value: f64) -> Result<i32, String> {
    // __ftol returns i64; this routine uses its low word. Reject native
    // indefinite integer results, rather than Rust's saturating float casts.
    let value = value.trunc();
    if !value.is_finite() || value < i64::MIN as f64 || value >= 9223372036854775808. {
        return Err("model visibility integer conversion overflow".into());
    }
    Ok(value as i64 as i32)
}

pub fn project(
    bounds: Bounds,
    scale: Matrix,
    mut placement: Matrix,
    mut camera: Matrix,
    viewport: [i32; 4],
) -> Result<Projection, String> {
    // The first call is raw 462af3, followed by the 5c37a0 constructor;
    // neither input's cached kind participates. Only the later 49b87c scale
    // multiplication uses the tagged matrix shortcuts.
    placement.kind = 0;
    camera.kind = 0;
    let matrix = scale.multiply(placement.multiply(camera)?)?;
    let mut result = Projection {
        matrix,
        screen: None,
        rectangle: None,
        visible: false,
    };
    if bounds
        .minimum
        .into_iter()
        .chain(bounds.maximum)
        .any(|v| !v.is_finite())
    {
        return Err("non-finite model visibility bounds".into());
    }
    if bounds
        .minimum
        .into_iter()
        .zip(bounds.maximum)
        .any(|(a, b)| a > b)
    {
        return Ok(result);
    }
    // Native uses these finite sentinels, not infinities or the first corner.
    let mut screen = [1_000_000., 1_000_000., -1_000_000., -1_000_000.];
    for corner in 0..8 {
        let point = std::array::from_fn(|axis| {
            if corner & (1 << axis) == 0 {
                bounds.minimum[axis]
            } else {
                bounds.maximum[axis]
            }
        });
        let [x, y, _] = matrix.point(point)?;
        if x < screen[0] {
            screen[0] = x;
        }
        if y < screen[1] {
            screen[1] = y;
        }
        if x > screen[2] {
            screen[2] = x;
        }
        if y > screen[3] {
            screen[3] = y;
        }
    }
    let [left, top, right, bottom] = [
        integer(f64::from(screen[0]))?,
        integer(f64::from(screen[1]))?,
        integer(f64::from(screen[2]) + 0.9999)?,
        integer(f64::from(screen[3]) + 0.9999)?,
    ];
    result.screen = Some(screen);
    result.rectangle = Some([left, top, right, bottom]);
    if left < right && top < bottom {
        let dx = right.wrapping_sub(left) >> 2;
        let dy = bottom.wrapping_sub(top) >> 2;
        result.visible = top.wrapping_sub(dy) <= viewport[3]
            && left.wrapping_sub(dx) <= viewport[2]
            && bottom.wrapping_add(dy) >= viewport[1]
            && right.wrapping_add(dx) >= viewport[0];
    }
    Ok(result)
}
