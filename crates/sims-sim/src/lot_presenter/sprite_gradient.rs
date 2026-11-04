//! Original 4b6dad/4b71b5: one shadow bitmap, raw RGB palette, paired depth/color stores.
use super::sprite_full::{Input as Sprite, Target, Write};

/// 4c8450..4c8461: x87 multiply255, add0.5, then __ftol truncation.
/// Limit this interface to the observed normalized brightness domain.
pub fn intensity(brightness: u32, control: u16) -> Result<i32, String> {
    let light = f32::from_bits(brightness);
    if !light.is_finite() || !(0.0..=1.0).contains(&light) {
        return Err("gradient brightness outside observed domain".into());
    }
    let value = match control {
        0x003f => f64::from(light * 255.0 + 0.5),
        // The f32 input, integer multiplier and half offset are exact at
        // integer decision boundaries in this normalized domain.
        0x037f => f64::from(light) * 255.0 + 0.5,
        _ => return Err("gradient brightness floating-point control unsupported".into()),
    };
    Ok(value.trunc() as i32)
}

/// 4a3813's resident-bitmap address, restricted to the classifier's radius.
/// Null bitmap loading is a separate native service, never an empty bitmap.
pub fn kernel_offset(side: i32, pitch: i32, x: i32, y: i32) -> Result<usize, String> {
    if side <= 0 || pitch <= 0 || side % pitch != 0 || pitch % 4 != 0 {
        return Err("gradient kernel dimensions unsupported".into());
    }
    let radius = (side / pitch - 1) / 2;
    if i64::from(x).abs() > i64::from(radius) || i64::from(y).abs() > i64::from(radius) {
        return Err("gradient coordinates outside kernel radius".into());
    }
    let r = i64::from(radius) * 2;
    let (x, y, side, pitch) = (
        i64::from(x),
        i64::from(y),
        i64::from(side),
        i64::from(pitch),
    );
    offset((r - y + x) * (pitch / 2) + (r + x + y) * (pitch / 4) * side)
}

pub struct Tables<'a> {
    /// Native contiguous 6edf48..6ef028 conversion and packing tables.
    pub bytes: &'a [u8],
    pub green_shift: u32,
}
impl Tables<'_> {
    /// Original438e6d packing and light lookup, shared with object shadows.
    pub fn color(&self, rgb: u32, light: i32) -> Result<u32, String> {
        if self.bytes.len() != 0x10e0 || !matches!(self.green_shift, 2 | 3) {
            return Err("gradient RGB table layout unsupported".into());
        }
        let level = (light.clamp(0, 255) >> 3) as usize;
        let blue = usize::from(self.bytes[level * 32 + ((rgb >> 3) & 31) as usize]);
        let green = usize::from(
            self.bytes[0x400 + level * 64 + (((rgb >> 8) & 255) >> self.green_shift) as usize],
        );
        let red = usize::from(self.bytes[0xc00 + level * 32 + ((rgb >> 19) & 31) as usize]);
        let word = |p: usize| -> Result<u32, String> {
            Ok(u32::from(u16::from_le_bytes(
                self.bytes
                    .get(p..p + 2)
                    .ok_or("gradient packing lookup outside tables")?
                    .try_into()
                    .unwrap(),
            )))
        };
        Ok(u32::from(
            *self
                .bytes
                .get(0x1000 + blue)
                .ok_or("gradient blue packing absent")?,
        ) | word(0x1020 + green * 2)?
            | word(0x10a0 + red * 2)?)
    }
}
pub struct Input<'a> {
    /// Palette is 256 raw little-endian RGB words here, rather than 16-bit colors.
    pub sprite: Sprite<'a>,
    pub mask: &'a [u8],
    pub mask_pitch: i32,
    pub light: i32,
    pub tables: Tables<'a>,
}
fn offset(v: i64) -> Result<usize, String> {
    usize::try_from(v).map_err(|_| "gradient destination precedes plane".into())
}
pub fn plan(i: Input<'_>, clip: Option<[i32; 4]>) -> Result<Vec<Write>, String> {
    let s = &i.sprite;
    if s.frame.len() < 18
        || s.palette.len() != 1024
        || s.color_pitch <= 0
        || s.depth_pitch <= 0
        || i.mask_pitch <= 0
        || clip.is_some_and(|v| v.iter().any(|&n| n < 0))
    {
        return Err("gradient sprite inputs differ".into());
    }
    let margins = clip.unwrap_or([0; 4]);
    let width = i64::from(u16::from_le_bytes(s.frame[..2].try_into().unwrap()));
    let height = i64::from(u16::from_le_bytes(s.frame[2..4].try_into().unwrap()));
    let cp = i64::from(s.color_pitch) * 2;
    let dp = i64::from(s.depth_pitch) * 2;
    let mp = i64::from(i.mask_pitch);
    let start = (i64::from(s.y) * i64::from(s.color_pitch) + i64::from(s.x)) * 2;
    let top = start + i64::from(margins[1]) * cp;
    let bottom = start + (height - i64::from(margins[3])) * cp;
    let mut color = start;
    let mut depth = (i64::from(s.y) * i64::from(s.depth_pitch) + i64::from(s.x)) * 2;
    let mut mask = 0;
    let mut run = None;
    let mut bounds = None;
    let mut z = None;
    let mut row = 0;
    let mut cursor = 16;
    let mut writes = vec![];
    for _ in 0..65536 {
        let at = cursor;
        let cmd = u16::from_le_bytes(
            s.frame
                .get(cursor..cursor + 2)
                .ok_or("gradient command outside stream")?
                .try_into()
                .unwrap(),
        );
        cursor += 2;
        let n = usize::from(cmd & 0x1fff);
        match cmd & 0xe000 {
            0 => {
                run = Some((color, depth, mask));
                let current = color;
                color += cp;
                depth += dp;
                mask += mp;
                if clip.is_some() {
                    if current < top {
                        cursor = at + n;
                    } else {
                        if current >= bottom {
                            return Ok(writes);
                        }
                        bounds = Some((
                            current + i64::from(margins[0]) * 2,
                            current + (width - i64::from(margins[2])) * 2,
                        ));
                    }
                }
                z = Some(
                    s.rows
                        .get(row)
                        .ok_or("gradient depth row absent")?
                        .wrapping_add(s.base) as u16,
                );
                row += 1;
            }
            0x6000 => {
                let (c, d, m) = run.ok_or("gradient skip precedes row")?;
                run = Some((c + n as i64 * 2, d + n as i64 * 2, m + n as i64));
            }
            0x8000 => {
                color += cp * n as i64;
                depth += dp * n as i64;
                mask += mp * n as i64;
                run = Some((color - cp, depth - dp, mask - mp));
            }
            0xa000 => return Ok(writes),
            0xc000 => {
                let (c, d, m) = run.ok_or("gradient run precedes row")?;
                let end = c + n as i64 * 2;
                let (head, tail) = if clip.is_some() {
                    let (left, right) = bounds.ok_or("gradient clip row bounds absent")?;
                    if !(left < end && c < right) {
                        cursor += n;
                        cursor += cursor & 1;
                        run = Some((end, d + n as i64 * 2, m + n as i64));
                        continue;
                    }
                    (
                        ((left - c).max(0) / 2) as usize,
                        ((end - right).max(0) / 2) as usize,
                    )
                } else {
                    (0, 0)
                };
                if head + tail <= n {
                    let z = u32::from(z.ok_or("gradient depth unobserved")?);
                    for p in head..n - tail {
                        let index = usize::from(
                            *s.frame
                                .get(cursor + p)
                                .ok_or("gradient palette index absent")?,
                        ) * 4;
                        let rgb =
                            u32::from_le_bytes(s.palette[index..index + 4].try_into().unwrap());
                        let light = i.light.wrapping_add(i32::from(
                            *i.mask
                                .get(offset(m + p as i64)?)
                                .ok_or("gradient mask pixel absent")?,
                        ));
                        writes.push(Write {
                            target: Target::Depth,
                            offset: offset(d + p as i64 * 2)?,
                            width: 2,
                            value: z,
                        });
                        writes.push(Write {
                            target: Target::Color,
                            offset: offset(c + p as i64 * 2)?,
                            width: 2,
                            value: i.tables.color(rgb, light)?,
                        });
                    }
                }
                cursor += n;
                cursor += cursor & 1;
                run = Some((end, d + n as i64 * 2, m + n as i64));
            }
            _ if clip.is_none() => return Ok(writes),
            _ => (),
        }
    }
    Err("gradient sprite command budget exceeded".into())
}
