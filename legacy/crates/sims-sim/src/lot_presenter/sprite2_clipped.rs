//! Original clipped/unclipped mirrored SPR2 traversal and batched children,
//! alongside4bc606 and its4bbd87/4bc080 forward children.
//! Scratch and destination stores retain original instruction ordering.
use serde::{Deserialize, Serialize};
mod batch;
pub use batch::CopyProfile;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    OldColors,
    SourceColors,
    Depth,
    Color,
    Batch,
    Alphas,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    pub target: Target,
    pub offset: usize,
    pub value: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high: Option<u64>,
}
pub struct Input<'a> {
    pub frame: &'a [u8],
    pub palette: &'a [u8],
    pub z: &'a [u8],
    pub mix: &'a [u8],
    pub color: &'a [u8],
    pub depth: &'a [u8],
    pub pitch: [i32; 2],
    pub origin: [i32; 2],
    pub margins: [i32; 4],
    pub base: i32,
}
///4bca26 reuses its stack frame-pointer argument as a color cursor.
/// Offsets preserve whether a row has replaced the original frame pointer yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArgumentCursor {
    FrameOffset(i64),
    ColorOffset(i64),
}
impl ArgumentCursor {
    fn add(&mut self, bytes: i64) {
        match self {
            Self::FrameOffset(v) | Self::ColorOffset(v) => *v += bytes,
        }
    }
}
fn half(b: &[u8], i: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        b.get(i..i + 2)
            .ok_or("SPR2 halfword outside source")?
            .try_into()
            .unwrap(),
    ))
}
fn pixel(origin: [i32; 2], pitch: i32, x: i32, y: i32) -> Result<usize, String> {
    usize::try_from(
        (i64::from(origin[1]) + i64::from(y)) * i64::from(pitch)
            + (i64::from(origin[0]) + i64::from(x)) * 2,
    )
    .map_err(|_| "SPR2 pixel outside plane".into())
}
pub fn plan(i: Input<'_>) -> Result<Vec<Write>, String> {
    plan_inner(i, None, false).map(|v| v.0)
}
pub fn plan_batched(i: Input<'_>, copy: CopyProfile) -> Result<Vec<Write>, String> {
    plan_inner(i, Some(copy), false).map(|v| v.0)
}
/// Original4bca26/4bd590 reverse destination traversal. Clipping remains in
/// destination coordinates; compressed input is consumed in its original order.
pub fn plan_mirrored(i: Input<'_>) -> Result<Vec<Write>, String> {
    plan_inner(i, None, true).map(|v| v.0)
}
pub fn plan_mirrored_batched(i: Input<'_>, copy: CopyProfile) -> Result<Vec<Write>, String> {
    plan_inner(i, Some(copy), true).map(|v| v.0)
}
/// The returned cursor describes4bca26's stack-argument mutation.4bd590 keeps
/// that argument unchanged; its caller uses only the planned stores.
pub fn plan_mirrored_with_argument(
    i: Input<'_>,
    copy: Option<CopyProfile>,
) -> Result<(Vec<Write>, ArgumentCursor), String> {
    plan_inner(i, copy, true)
}
fn plan_inner(
    i: Input<'_>,
    copy: Option<CopyProfile>,
    mirrored: bool,
) -> Result<(Vec<Write>, ArgumentCursor), String> {
    if i.frame.len() < 18
        || i.palette.len() != 512
        || i.z.len() != 512
        || i.mix.len() != 32768
        || i.pitch.iter().any(|p| *p <= 0)
        || i.margins.iter().any(|v| *v < 0)
    {
        return Err("SPR2 clipped inputs differ".into());
    }
    let width = i32::from(half(i.frame, 0)?);
    let height = i32::from(half(i.frame, 2)?);
    let [left, top, right, bottom] = i.margins;
    let (left, right) = if mirrored {
        (right, width - left)
    } else {
        (left, width - right)
    };
    let bottom = height - bottom;
    if left >= right || top >= bottom {
        return Err("SPR2 fully clipped call belongs to its caller".into());
    }
    let (mut cursor, mut next_y, mut row, mut x) = (16usize, 0i32, None, 0i32);
    let mut writes = Vec::new();
    let mut argument = ArgumentCursor::FrameOffset(0);
    for _ in 0..1048576 {
        let command = cursor;
        let tag = half(i.frame, cursor)?;
        cursor += 2;
        let n = i32::from(tag & 0x1fff);
        match tag & 0xe000 {
            0 => {
                let y = next_y;
                next_y = next_y.checked_add(1).ok_or("SPR2 row overflow")?;
                if y < top {
                    cursor = command
                        .checked_add(n as usize)
                        .ok_or("SPR2 row jump overflow")?;
                    if cursor <= command {
                        return Err("SPR2 invalid row jump".into());
                    }
                    continue;
                }
                if y >= bottom {
                    return Ok((writes, argument));
                }
                row = Some(y);
                x = 0;
                if mirrored {
                    argument =
                        ArgumentCursor::ColorOffset(
                            pixel(i.origin, i.pitch[0], width - 1, y)? as i64
                        );
                }
            }
            0x8000 => {
                next_y = next_y.checked_add(n).ok_or("SPR2 skipped row overflow")?;
                if let Some(y) = row {
                    row = Some(y.checked_add(n - 1).ok_or("SPR2 run row overflow")?);
                }
                argument.add(i64::from(i.pitch[0]) * i64::from(n - 1));
            }
            0x6000 => {
                x = x.checked_add(n).ok_or("SPR2 skip overflow")?;
                argument.add(-2 * i64::from(n));
            }
            0xa000 => return Ok((writes, argument)),
            code @ (0x2000 | 0x4000) => {
                let y = row.ok_or("SPR2 run precedes visible row")?;
                argument.add(-2 * i64::from(n));
                let end = x.checked_add(n).ok_or("SPR2 run overflow")?;
                let lo = x.max(left);
                let hi = end.min(right);
                let count = (hi - lo).max(0) as usize;
                let stride = if code == 0x4000 { 3 } else { 2 };
                let bytes = n as usize * stride;
                let raw = i
                    .frame
                    .get(cursor..cursor + bytes)
                    .ok_or("SPR2 run outside source")?;
                if count >= 16 {
                    let copy = copy
                        .as_ref()
                        .ok_or("SPR2 batched scratch/copy owner is unbound")?;
                    let start = (lo - x).max(0) as usize * stride;
                    batch::run(
                        &i,
                        &raw[start..start + count * stride],
                        if mirrored { width - 1 - lo } else { lo },
                        y,
                        code,
                        copy,
                        &mut writes,
                        mirrored,
                    )?;
                    cursor += bytes;
                    if code == 0x4000 {
                        cursor += cursor & 1;
                    }
                    x = end;
                    continue;
                }
                let start = (lo - x).max(0) as usize * stride;
                if code == 0x4000 {
                    for p in 0..count {
                        let x = lo + p as i32;
                        let at = pixel(
                            i.origin,
                            i.pitch[0],
                            if mirrored { width - 1 - x } else { x },
                            y,
                        )?;
                        writes.push(Write {
                            target: Target::OldColors,
                            offset: p * 4,
                            value: u64::from(half(i.color, at)?),
                            width: None,
                            high: None,
                        });
                    }
                    for p in 0..count {
                        let index = usize::from(raw[start + p * stride + 1]);
                        writes.push(Write {
                            target: Target::SourceColors,
                            offset: p * 4,
                            value: u64::from(half(i.palette, index * 2)?),
                            width: None,
                            high: None,
                        });
                    }
                }
                for p in 0..count {
                    let at = start + p * stride;
                    let z = i
                        .base
                        .wrapping_add(i32::from(half(i.z, usize::from(raw[at]) * 2)? as i16));
                    let x = lo + p as i32;
                    let x = if mirrored { width - 1 - x } else { x };
                    let depth = pixel(i.origin, i.pitch[1], x, y)?;
                    if z > i32::from(half(i.depth, depth)?) {
                        continue;
                    }
                    let index = usize::from(raw[at + 1]);
                    let source = half(i.palette, index * 2)?;
                    let color = pixel(i.origin, i.pitch[0], x, y)?;
                    let value = if code == 0x4000 {
                        let alpha = raw[at + 2];
                        if alpha < 8 {
                            continue;
                        }
                        if alpha > 247 {
                            source
                        } else {
                            let old = half(i.color, color)?;
                            let alpha = usize::from(alpha & 31) * 1024;
                            let channel = |shift: u32| -> u16 {
                                u16::from(
                                    i.mix[alpha
                                        + usize::from((source >> shift) & 31_u16) * 32
                                        + usize::from((old >> shift) & 31_u16)],
                                )
                            };
                            ((channel(11) << 5) | channel(6)) << 6 | channel(0)
                        }
                    } else {
                        source
                    };
                    writes.push(Write {
                        target: Target::Depth,
                        offset: depth,
                        value: u64::from(z as u16),
                        width: None,
                        high: None,
                    });
                    writes.push(Write {
                        target: Target::Color,
                        offset: color,
                        value: u64::from(value),
                        width: None,
                        high: None,
                    });
                }
                cursor += bytes;
                if code == 0x4000 {
                    cursor += (cursor & 1) as usize;
                }
                x = end;
            }
            _ => return Err("SPR2 unsupported command child".into()),
        }
    }
    Err("SPR2 command budget exceeded".into())
}
pub fn apply(
    w: &Write,
    color: &mut [u8],
    depth: &mut [u8],
    old: &mut [u8],
    source: &mut [u8],
) -> Result<(), String> {
    apply_batched(w, color, depth, old, source, &mut [], &mut [])
}
pub fn apply_batched(
    w: &Write,
    color: &mut [u8],
    depth: &mut [u8],
    old: &mut [u8],
    source: &mut [u8],
    batch: &mut [u8],
    alphas: &mut [u8],
) -> Result<(), String> {
    let (b, default) = match w.target {
        Target::Color => (color, 2),
        Target::Depth => (depth, 2),
        Target::OldColors => (old, 4),
        Target::SourceColors => (source, 4),
        Target::Batch => (batch, 4),
        Target::Alphas => (alphas, 4),
    };
    let n = usize::from(w.width.unwrap_or(default));
    if !matches!(n, 1 | 2 | 4 | 8 | 16)
        || (n == 16) != w.high.is_some()
        || (n < 8 && w.value >> (n * 8) != 0)
    {
        return Err("SPR2 store width/value differs".into());
    }
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&w.value.to_le_bytes());
    bytes[8..].copy_from_slice(&w.high.unwrap_or(0).to_le_bytes());
    b.get_mut(w.offset..w.offset + n)
        .ok_or("SPR2 store outside owner")?
        .copy_from_slice(&bytes[..n]);
    Ok(())
}
