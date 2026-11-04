//! Original4b10c4: shade the retained color plane, testing but never storing depth.
use super::{
    sprite_full::{Target, Write},
    sprite_gradient::Tables,
};
pub struct Input<'a> {
    pub origin: [i32; 2],
    pub size: [i32; 2],
    pub mask: &'a [u8],
    pub mask_pitch: i32,
    pub bias: i32,
    pub rows: &'a [i32],
    pub depth_offset: i32,
    pub clip: [i32; 4],
    pub color: &'a [u8],
    pub depth: &'a [u8],
    pub pitches: [i32; 2],
    pub rgb555: bool,
    pub tables: Tables<'a>,
}
fn offset(v: i64) -> Result<usize, String> {
    usize::try_from(v).map_err(|_| "shadow pixel precedes plane".into())
}
fn half(b: &[u8], at: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2)
            .ok_or("shadow pixel outside plane")?
            .try_into()
            .unwrap(),
    ))
}
pub fn plan(i: Input<'_>) -> Result<Vec<Write>, String> {
    let [x, y] = i.origin;
    let [width, height] = i.size;
    let [left, top, right, bottom] = i.clip;
    if width < 0
        || height < 0
        || i.mask_pitch < width
        || i.mask_pitch <= 0
        || i.pitches.iter().any(|p| *p <= 0)
    {
        return Err("shadow rectangle/pitch unsupported".into());
    }
    if x.wrapping_add(width) <= left || x >= right || y.wrapping_add(height) <= top || y >= bottom {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for row in 0..height {
        let sy = y.wrapping_add(row);
        if sy >= bottom {
            return Ok(out);
        }
        let z = i
            .rows
            .get(row as usize)
            .ok_or("shadow depth row absent")?
            .wrapping_add(i.depth_offset);
        if sy < top {
            continue;
        }
        for col in 0..width {
            let sx = x.wrapping_add(col);
            if sx >= right {
                break;
            }
            if sx < left {
                continue;
            }
            let alpha = i32::from(
                *i.mask
                    .get(offset(
                        i64::from(row) * i64::from(i.mask_pitch) + i64::from(col),
                    )?)
                    .ok_or("shadow mask pixel absent")?,
            )
            .wrapping_add(i.bias);
            if alpha >= 248 {
                continue;
            }
            let depth = offset(i64::from(sy) * i64::from(i.pitches[1]) + i64::from(sx) * 2)?;
            if z > i32::from(half(i.depth, depth)?) {
                continue;
            }
            let color = offset(i64::from(sy) * i64::from(i.pitches[0]) + i64::from(sx) * 2)?;
            let value = u32::from(half(i.color, color)?);
            let rgb = if i.rgb555 {
                (((value & 0x7c00) << 3 | value & 0x3e0) << 3 | value & 31) << 3
            } else {
                (((value & 0xf800) << 3 | value & 0x7e0) << 2 | value & 31) << 3
            };
            out.push(Write {
                target: Target::Color,
                offset: color,
                width: 2,
                value: i.tables.color(rgb, alpha.max(0))?,
            });
        }
    }
    Ok(out)
}
