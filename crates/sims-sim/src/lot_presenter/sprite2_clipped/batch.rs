use super::{Input, Target, Write, half, pixel};
use serde::{Deserialize, Serialize};

/// Original CRT feature owners and the two backing addresses modulo32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyProfile {
    pub dispatch: [u32; 6],
    pub plane_mod32: [u8; 2],
}
fn emit(w: &mut Vec<Write>, target: Target, offset: usize, width: u8, value: u64) {
    let default = if matches!(target, Target::Depth | Target::Color) {
        2
    } else {
        4
    };
    w.push(Write {
        target,
        offset,
        value,
        width: (width != default).then_some(width),
        high: None,
    });
}
fn blend(i: &Input<'_>, source: u16, old: u16, alpha: u8) -> u16 {
    let a = usize::from(alpha & 31) * 1024;
    let channel = |shift| {
        u16::from(
            i.mix[a
                + usize::from((source >> shift) & 31_u16) * 32
                + usize::from((old >> shift) & 31_u16)],
        )
    };
    ((channel(11) << 5) | channel(6)) << 6 | channel(0)
}
fn copy_stores(
    w: &mut Vec<Write>,
    target: Target,
    offset: usize,
    bytes: &[u8],
    p: &CopyProfile,
) -> Result<(), String> {
    let sse = p.dispatch[1] != 0;
    if !sse && p.dispatch != [0; 6] {
        return Err("SPR2 CRT copy dispatch requires another original child".into());
    }
    let plane = if target == Target::Color { 0 } else { 1 };
    let base = usize::from(p.plane_mod32[plane]);
    if base >= 32 {
        return Err("SPR2 plane alignment differs".into());
    }
    let mut at = 0;
    let mut store = |at: usize, width: usize| {
        let raw = &bytes[at..at + width];
        let mut value = [0; 8];
        value[..width.min(8)].copy_from_slice(&raw[..width.min(8)]);
        emit(
            w,
            target,
            offset + at,
            width as u8,
            u64::from_le_bytes(value),
        );
        if width == 16 {
            w.last_mut().unwrap().high = Some(u64::from_le_bytes(raw[8..].try_into().unwrap()));
        }
    };
    //5a9dd0 aligns to32 with1/2/4/8/16-byte stores;5a98c0 aligns
    //to4 with1/2/4-byte stores. The scalar tail order is identical.
    let alignment = if sse { 32 } else { 4 };
    if (base + offset) % alignment != 0 {
        for width in [1, 2, 4, 8, 16] {
            if !sse && width > 4 {
                break;
            }
            if (base + offset + at) & width != 0 && bytes.len() - at >= width {
                store(at, width);
                at += width;
            }
        }
    }
    if sse {
        while bytes.len() - at >= 32 {
            store(at, 16);
            store(at + 16, 16);
            at += 32;
        }
        while bytes.len() - at >= 8 {
            store(at, 8);
            at += 8;
        }
    } else {
        //The unrolled32/8-byte loops still perform individual DWORD stores.
        while bytes.len() - at >= 4 {
            store(at, 4);
            at += 4;
        }
    }
    for width in [4, 2, 1] {
        if bytes.len() - at >= width {
            store(at, width);
            at += width;
        }
    }
    Ok(())
}
pub(super) fn run(
    i: &Input<'_>,
    raw: &[u8],
    x: i32,
    y: i32,
    code: u16,
    copy: &CopyProfile,
    w: &mut Vec<Write>,
    mirrored: bool,
) -> Result<(), String> {
    if code == 0x2000 {
        opaque(i, raw, x, y, copy, w, mirrored)
    } else {
        alpha(i, raw, x, y, w, mirrored)
    }
}
fn opaque(
    i: &Input<'_>,
    raw: &[u8],
    x: i32,
    y: i32,
    copy: &CopyProfile,
    w: &mut Vec<Write>,
    mirrored: bool,
) -> Result<(), String> {
    let count = raw.len() / 2;
    if count > 160 {
        return Err("SPR2 opaque run exceeds original160-pixel staging owner".into());
    }
    let mut scratch = [0_u8; 0x314];
    for mask in 0..5 {
        emit(w, Target::Batch, mask * 4, 4, 0);
    }
    let start = if mirrored { x - count as i32 + 1 } else { x };
    let depth = pixel(i.origin, i.pitch[1], start, y)?;
    let color = pixel(i.origin, i.pitch[0], start, y)?;
    for p in 0..count {
        let staged = if mirrored { count - 1 - p } else { p };
        let z = i
            .base
            .wrapping_add(i32::from(half(i.z, usize::from(raw[p * 2]) * 2)? as i16));
        let source = half(i.palette, usize::from(raw[p * 2 + 1]) * 2)?;
        for (at, v) in [(0x14 + staged * 2, z as u16), (0x154 + staged * 2, source)] {
            emit(w, Target::Batch, at, 2, u64::from(v));
            scratch[at..at + 2].copy_from_slice(&v.to_le_bytes());
        }
        if z <= i32::from(half(i.depth, depth + staged * 2)?) {
            let at = (staged / 32) * 4;
            let mask =
                u32::from_le_bytes(scratch[at..at + 4].try_into().unwrap()) | (1 << (staged % 32));
            scratch[at..at + 4].copy_from_slice(&mask.to_le_bytes());
            emit(w, Target::Batch, at, 4, u64::from(mask));
        }
    }
    let mask =
        |group: usize| u32::from_le_bytes(scratch[group * 4..group * 4 + 4].try_into().unwrap());
    let groups = count / 32;
    let tail = count % 32;
    //For count160, the original sixth mask aliases the first staged Z DWORD.
    if (0..groups).all(|g| mask(g) == u32::MAX) && mask(groups) == (1_u32 << tail).wrapping_sub(1) {
        copy_stores(
            w,
            Target::Depth,
            depth,
            &scratch[0x14..0x14 + count * 2],
            copy,
        )?;
        copy_stores(
            w,
            Target::Color,
            color,
            &scratch[0x154..0x154 + count * 2],
            copy,
        )?;
    } else {
        for group in 0..=groups {
            let n = if group == groups { tail } else { 32 };
            let start = group * 32;
            if n == 32 && mask(group) == u32::MAX {
                copy_stores(
                    w,
                    Target::Depth,
                    depth + start * 2,
                    &scratch[0x14 + start * 2..0x14 + (start + n) * 2],
                    copy,
                )?;
                copy_stores(
                    w,
                    Target::Color,
                    color + start * 2,
                    &scratch[0x154 + start * 2..0x154 + (start + n) * 2],
                    copy,
                )?;
            } else {
                for p in 0..n {
                    if mask(group) & (1 << p) != 0 {
                        let p = start + p;
                        emit(
                            w,
                            Target::Depth,
                            depth + p * 2,
                            2,
                            u64::from(half(&scratch, 0x14 + p * 2)?),
                        );
                        emit(
                            w,
                            Target::Color,
                            color + p * 2,
                            2,
                            u64::from(half(&scratch, 0x154 + p * 2)?),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
fn alpha(
    i: &Input<'_>,
    raw: &[u8],
    x: i32,
    y: i32,
    w: &mut Vec<Write>,
    mirrored: bool,
) -> Result<(), String> {
    let direction = if mirrored { -1 } else { 1 };
    for (chunk, data) in raw.chunks(16 * 3).enumerate() {
        let count = data.len() / 3;
        let depth_at = |p: usize| {
            pixel(
                i.origin,
                i.pitch[1],
                x + (chunk * 16 + p) as i32 * direction,
                y,
            )
        };
        let color_at = |p: usize| {
            pixel(
                i.origin,
                i.pitch[0],
                x + (chunk * 16 + p) as i32 * direction,
                y,
            )
        };
        let mut zs = [0_i32; 16];
        let mut accepted = 0_u16;
        for p in 0..count {
            emit(
                w,
                Target::Batch,
                0x2d4 + p * 4,
                4,
                u64::from(half(i.depth, depth_at(p)?)?),
            );
        }
        for p in 0..count {
            let z = i
                .base
                .wrapping_add(i32::from(half(i.z, usize::from(data[p * 3]) * 2)? as i16));
            if z <= i32::from(half(i.depth, depth_at(p)?)?) {
                zs[p] = z;
                accepted |= 1 << p;
                emit(w, Target::Batch, 0x294 + p * 4, 4, u64::from(z as u32));
            }
        }
        if accepted == 0 {
            continue;
        }
        for p in 0..count {
            emit(w, Target::Alphas, p * 4, 4, u64::from(data[p * 3 + 2]));
        }
        let mut middle = 0_u16;
        let mut opaque = 0_u16;
        for p in 0..count {
            if accepted & (1 << p) != 0 {
                match data[p * 3 + 2] {
                    0..=7 => {}
                    8..=247 => middle |= 1 << p,
                    _ => opaque |= 1 << p,
                }
            }
        }
        if middle != 0 {
            for p in 0..count {
                emit(
                    w,
                    Target::OldColors,
                    p * 4,
                    4,
                    u64::from(half(i.color, color_at(p)?)?),
                );
            }
            for p in 0..count {
                emit(
                    w,
                    Target::SourceColors,
                    p * 4,
                    4,
                    u64::from(half(i.palette, usize::from(data[p * 3 + 1]) * 2)?),
                );
            }
            if middle == u16::MAX {
                for (p, z) in zs.iter().enumerate() {
                    emit(w, Target::Depth, depth_at(p)?, 2, u64::from(*z as u16));
                }
            }
            for p in 0..count {
                if middle & (1 << p) != 0 {
                    if middle != u16::MAX {
                        emit(w, Target::Depth, depth_at(p)?, 2, u64::from(zs[p] as u16));
                    }
                    let v = blend(
                        i,
                        half(i.palette, usize::from(data[p * 3 + 1]) * 2)?,
                        half(i.color, color_at(p)?)?,
                        data[p * 3 + 2],
                    );
                    emit(w, Target::Color, color_at(p)?, 2, u64::from(v));
                }
            }
        }
        if opaque == u16::MAX {
            for (p, z) in zs.iter().enumerate() {
                emit(w, Target::Depth, depth_at(p)?, 2, u64::from(*z as u16));
            }
        }
        for p in 0..count {
            if opaque & (1 << p) != 0 {
                if opaque != u16::MAX {
                    emit(w, Target::Depth, depth_at(p)?, 2, u64::from(zs[p] as u16));
                }
                emit(
                    w,
                    Target::Color,
                    color_at(p)?,
                    2,
                    u64::from(half(i.palette, usize::from(data[p * 3 + 1]) * 2)?),
                );
            }
        }
    }
    Ok(())
}
