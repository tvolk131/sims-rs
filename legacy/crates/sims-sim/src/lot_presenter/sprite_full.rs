//!49f628 full-sprite SPR# stream. Depth runs precede indexed color stores.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Color,
    Depth,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    pub target: Target,
    pub offset: usize,
    pub width: u8,
    pub value: u32,
}
pub struct Input<'a> {
    pub frame: &'a [u8],
    pub palette: &'a [u8],
    pub rows: &'a [i32],
    pub base: i32,
    pub x: i32,
    pub y: i32,
    pub color_pitch: i32,
    pub depth_pitch: i32,
}
fn offset(v: i64) -> Result<usize, String> {
    usize::try_from(v).map_err(|_| "sprite destination precedes buffer".into())
}
pub fn plan(i: Input<'_>) -> Result<Vec<Write>, String> {
    if i.frame.len() < 18 || i.palette.len() != 512 || i.color_pitch <= 0 || i.depth_pitch <= 0 {
        return Err("sprite full inputs differ".into());
    }
    let mut color = (i64::from(i.y) * i64::from(i.color_pitch) + i64::from(i.x)) * 2;
    let mut depth = (i64::from(i.y) * i64::from(i.depth_pitch) + i64::from(i.x)) * 2;
    let mut run_color = None;
    let mut run_depth = None;
    let mut z = None;
    let mut row = 0;
    let mut cursor = 16;
    let mut writes = vec![];
    for _ in 0..65536 {
        let c = i
            .frame
            .get(cursor..cursor + 2)
            .ok_or("sprite command outside source")?;
        let cmd = u16::from_le_bytes(c.try_into().unwrap());
        cursor += 2;
        let n = usize::from(cmd & 0x1fff);
        match cmd & 0xe000 {
            0 => {
                z = Some(
                    i.rows
                        .get(row)
                        .ok_or("sprite depth row absent")?
                        .wrapping_add(i.base) as u16,
                );
                row += 1;
                run_color = Some(color);
                run_depth = Some(depth);
                color += i64::from(i.color_pitch) * 2;
                depth += i64::from(i.depth_pitch) * 2;
            }
            0x6000 => {
                run_color = Some(run_color.ok_or("sprite skip precedes row")? + n as i64 * 2);
                run_depth = Some(run_depth.ok_or("sprite skip depth precedes row")? + n as i64 * 2);
            }
            0x8000 => {
                color += i64::from(i.color_pitch) * n as i64 * 2;
                depth += i64::from(i.depth_pitch) * n as i64 * 2;
                run_color = Some(color - i64::from(i.color_pitch) * 2);
                run_depth = Some(depth - i64::from(i.depth_pitch) * 2);
            }
            0xc000 => {
                let c = run_color.ok_or("sprite color run precedes row")?;
                let d = run_depth.ok_or("sprite depth run precedes row")?;
                let z = u32::from(z.ok_or("sprite depth value unobserved")?);
                for p in 0..n / 2 {
                    writes.push(Write {
                        target: Target::Depth,
                        offset: offset(d + p as i64 * 4)?,
                        width: 4,
                        value: z | (z << 16),
                    });
                }
                if n & 1 != 0 {
                    writes.push(Write {
                        target: Target::Depth,
                        offset: offset(d + (n / 2) as i64 * 4)?,
                        width: 2,
                        value: z,
                    });
                }
                let bytes = i
                    .frame
                    .get(cursor..cursor + n)
                    .ok_or("sprite indices outside source")?;
                for (p, index) in bytes.iter().enumerate() {
                    let index = usize::from(*index) * 2;
                    let value = u32::from(u16::from_le_bytes(
                        i.palette[index..index + 2].try_into().unwrap(),
                    ));
                    writes.push(Write {
                        target: Target::Color,
                        offset: offset(c + p as i64 * 2)?,
                        width: 2,
                        value,
                    });
                }
                cursor += n + (n & 1);
                run_color = Some(c + n as i64 * 2);
                run_depth = Some(d + n as i64 * 2);
            }
            _ => return Ok(writes),
        }
    }
    Err("sprite full command budget exceeded".into())
}
pub fn apply(w: &Write, color: &mut [u8], depth: &mut [u8]) -> Result<(), String> {
    let plane = match w.target {
        Target::Color => color,
        Target::Depth => depth,
    };
    let n = usize::from(w.width);
    if !matches!(n, 2 | 4) {
        return Err("sprite write width differs".into());
    }
    plane
        .get_mut(w.offset..w.offset + n)
        .ok_or("sprite write outside plane")?
        .copy_from_slice(&w.value.to_le_bytes()[..n]);
    Ok(())
}
