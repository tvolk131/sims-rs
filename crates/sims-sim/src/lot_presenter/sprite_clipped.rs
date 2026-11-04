//!49f949: clipped SPR# indexed color and depth writes. Margins are L/T/R/B.
//! Row headers jump by their encoded byte length when clipped at the top.
use super::sprite_full::{Input, Target, Write};

pub fn plan(i: Input<'_>, clip: [i32; 4]) -> Result<Vec<Write>, String> {
    if i.frame.len() < 18
        || i.palette.len() != 512
        || i.color_pitch <= 0
        || i.depth_pitch <= 0
        || clip.iter().any(|&v| v < 0)
    {
        return Err("sprite clipped inputs differ".into());
    }
    let width = i64::from(u16::from_le_bytes(i.frame[..2].try_into().unwrap()));
    let height = i64::from(u16::from_le_bytes(i.frame[2..4].try_into().unwrap()));
    let cp = i64::from(i.color_pitch) * 2;
    let dp = i64::from(i.depth_pitch) * 2;
    let start = (i64::from(i.y) * i64::from(i.color_pitch) + i64::from(i.x)) * 2;
    let top = start + i64::from(clip[1]) * cp;
    let bottom = start + (height - i64::from(clip[3])) * cp;
    let mut color = start;
    let mut depth = (i64::from(i.y) * i64::from(i.depth_pitch) + i64::from(i.x)) * 2;
    let mut run = None;
    let mut bounds = None;
    let mut z = None;
    let mut row = 0;
    let mut cursor = 16;
    let mut writes = vec![];
    for _ in 0..65536 {
        let at = cursor;
        let raw = i
            .frame
            .get(cursor..cursor + 2)
            .ok_or("clipped sprite command outside source")?;
        let cmd = u16::from_le_bytes(raw.try_into().unwrap());
        cursor += 2;
        let n = usize::from(cmd & 0x1fff);
        match cmd & 0xe000 {
            0 => {
                run = Some((color, depth));
                color += cp;
                depth += dp;
                let current = color - cp;
                if current < top {
                    // Native LEA uses the command's own start, not its payload.
                    cursor = at + n;
                } else {
                    if current >= bottom {
                        return Ok(writes);
                    }
                    bounds = Some((
                        current + i64::from(clip[0]) * 2,
                        current + (width - i64::from(clip[2])) * 2,
                    ));
                }
                z = Some(
                    i.rows
                        .get(row)
                        .ok_or("clipped sprite depth row absent")?
                        .wrapping_add(i.base) as u16,
                );
                row += 1;
            }
            0x6000 => {
                let (c, d) = run.ok_or("clipped sprite skip precedes row")?;
                run = Some((c + n as i64 * 2, d + n as i64 * 2));
            }
            0x8000 => {
                if n != 0 {
                    color += cp * n as i64;
                    depth += dp * n as i64;
                    run = Some((color - cp, depth - dp));
                }
            }
            0xa000 => return Ok(writes),
            0xc000 => {
                let (mut c, mut d) = run.ok_or("clipped sprite run precedes row")?;
                let (left, right) = bounds.ok_or("clipped sprite row limits unobserved")?;
                let end = c + n as i64 * 2;
                if left < end && c < right {
                    let tail = ((end - right).max(0) / 2) as usize;
                    let head = ((left - c).max(0) / 2) as usize;
                    // Opposing margins can cross. Native signed ESI then
                    // suppresses both store loops and still consumes the run.
                    if head + tail > n {
                        cursor += n;
                        cursor += cursor & 1;
                        run = Some((end, d + n as i64 * 2));
                        continue;
                    }
                    let count = n - head - tail;
                    cursor += head;
                    c += head as i64 * 2;
                    d += head as i64 * 2;
                    let z = u32::from(z.ok_or("clipped sprite depth unobserved")?);
                    for p in 0..count / 2 {
                        writes.push(Write {
                            target: Target::Depth,
                            offset: offset(d + p as i64 * 4)?,
                            width: 4,
                            value: z | z << 16,
                        });
                    }
                    if count & 1 != 0 {
                        writes.push(Write {
                            target: Target::Depth,
                            offset: offset(d + (count / 2) as i64 * 4)?,
                            width: 2,
                            value: z,
                        });
                    }
                    let bytes = i
                        .frame
                        .get(cursor..cursor + count)
                        .ok_or("clipped sprite indices outside source")?;
                    for (p, &index) in bytes.iter().enumerate() {
                        let index = usize::from(index) * 2;
                        writes.push(Write {
                            target: Target::Color,
                            offset: offset(c + p as i64 * 2)?,
                            width: 2,
                            value: u32::from(u16::from_le_bytes(
                                i.palette[index..index + 2].try_into().unwrap(),
                            )),
                        });
                    }
                    cursor += count + tail;
                    run = Some((c + (count + tail) as i64 * 2, d + (count + tail) as i64 * 2));
                } else {
                    cursor += n;
                    run = Some((end, d + n as i64 * 2));
                }
                cursor += cursor & 1;
            }
            _ => (), // Original ignores unrecognized command tags.
        }
    }
    Err("clipped sprite command bound exceeded".into())
}
fn offset(v: i64) -> Result<usize, String> {
    usize::try_from(v).map_err(|_| "clipped sprite destination precedes plane".into())
}
