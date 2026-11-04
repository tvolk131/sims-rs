//! Original 4c4907 ordinary dual-SPR# blitter and 4ba3d4/4ba5de row writes.
//! The depth stream gates coverage; its byte values are not per-pixel depths.
//! Scratch rows survive skipped color indices. Depth stores precede color.
use super::sprite_full::{Target, Write};

pub struct Input<'a> {
    pub mask: &'a [u8],
    pub color: &'a [u8],
    pub palette: &'a [u8],
    pub screen: [i32; 2],
    /// Native clipping margins, left/top/right/bottom.
    pub margins: [i32; 4],
    pub color_pitch: i32,
    pub depth_pitch: i32,
    /// 49a5d3's signed depth result plus the caller's additional offset.
    pub rows: &'a [i32],
    pub columns: &'a [u16],
    pub mask_scratch: &'a [u8],
    pub color_scratch: &'a [u8],
    pub depth: &'a [u8],
}
///49f2a3 uses two index iterators, its own scratch rows and one depth per row.
pub struct ConstantInput<'a> {
    pub frame: &'a [u8],
    pub palette: &'a [u8],
    pub screen: [i32; 2],
    pub margins: [i32; 4],
    pub color_pitch: i32,
    pub depth_pitch: i32,
    pub rows: &'a [i32],
    pub mask_scratch: &'a [u8],
    pub index_scratch: &'a [u8],
    pub depth: &'a [u8],
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConstantPlan {
    pub writes: Vec<Write>,
    pub mask_scratch: Vec<u8>,
    pub index_scratch: Vec<u8>,
}
///49ecbc maps mask indices into eight depth-column/offset combinations.
///Both streams remain indexed; unlike4c4907, the color scratch holds indices.
pub struct MappedInput<'a> {
    pub mask: &'a [u8],
    pub color: &'a [u8],
    pub palette: &'a [u8],
    pub map: &'a [u8],
    pub columns: [&'a [u16]; 8],
    pub offsets: [i32; 8],
    pub rows: &'a [i32],
    pub screen: [i32; 2],
    pub margins: [i32; 4],
    pub color_pitch: i32,
    pub depth_pitch: i32,
    pub mask_scratch: &'a [u8],
    pub index_scratch: &'a [u8],
    pub depth: &'a [u8],
}
pub fn plan_mapped(i: MappedInput<'_>) -> Result<ConstantPlan, String> {
    let (width, mask, _) = decode(i.mask)?;
    let (color_width, color, _) = decode(i.color)?;
    if i.mask[..8] != i.color[..8]
        || width != color_width
        || mask.len() != color.len()
        || i.palette.len() != 512
        || i.map.len() != 256
        || i.columns.iter().any(|c| c.len() < width)
        || i.rows.len() != mask.len()
        || i.mask_scratch.len() != width
        || i.index_scratch.len() != width
        || i.color_pitch < width as i32
        || i.depth_pitch < width as i32
        || i.margins.iter().any(|v| *v < 0)
    {
        return Err("mapped sprite inputs differ".into());
    }
    let [l, t, r, b] = i.margins.map(|v| v as usize);
    if l + r > width || t + b > mask.len() {
        return Err("mapped sprite clip exceeds frame".into());
    }
    let mut p = ConstantPlan {
        writes: vec![],
        mask_scratch: i.mask_scratch.to_vec(),
        index_scratch: i.index_scratch.to_vec(),
    };
    for y in t..mask.len() - b {
        for (dst, value) in p.mask_scratch.iter_mut().zip(&mask[y].pixels) {
            if let Some(value) = value {
                *dst = *value;
            }
        }
        if !mask[y].fetched {
            continue;
        }
        for (dst, value) in p.index_scratch.iter_mut().zip(&color[y].pixels) {
            if let Some(value) = value {
                *dst = *value;
            }
        }
        for x in l..width - r {
            let mask = p.mask_scratch[x];
            if mask == 255 {
                continue;
            }
            let group = usize::from(i.map[usize::from(mask)]);
            if group >= 8 {
                return Err("mapped sprite depth group exceeds table".into());
            }
            p.mask_scratch[x] = 255;
            let z = i32::from(i.columns[group][x])
                .wrapping_add(i.offsets[group])
                .wrapping_add(i.rows[y]);
            let offset = |pitch: i32| {
                usize::try_from(
                    ((i64::from(i.screen[1]) + y as i64) * i64::from(pitch)
                        + i64::from(i.screen[0])
                        + x as i64)
                        * 2,
                )
                .map_err(|_| "mapped sprite negative destination")
            };
            let at = offset(i.depth_pitch)?;
            let old = u16::from_le_bytes(
                i.depth
                    .get(at..at + 2)
                    .ok_or("mapped sprite depth outside plane")?
                    .try_into()
                    .unwrap(),
            );
            if z <= i32::from(old) + 1 {
                p.writes.push(Write {
                    target: Target::Depth,
                    offset: at,
                    width: 2,
                    value: u32::from(z as u16),
                });
                let index = usize::from(p.index_scratch[x]) * 2;
                p.writes.push(Write {
                    target: Target::Color,
                    offset: offset(i.color_pitch)?,
                    width: 2,
                    value: u32::from(u16::from_le_bytes(
                        i.palette[index..index + 2].try_into().unwrap(),
                    )),
                });
            }
        }
        p.mask_scratch[..l].fill(255);
        p.mask_scratch[width - r..].fill(255);
    }
    Ok(p)
}
pub fn plan_constant(i: ConstantInput<'_>) -> Result<ConstantPlan, String> {
    let (width, rows, _) = decode(i.frame)?;
    if i.palette.len() != 512
        || i.rows.len() != rows.len()
        || i.mask_scratch.len() != width
        || i.index_scratch.len() != width
        || i.color_pitch < width as i32
        || i.depth_pitch < width as i32
        || i.margins.iter().any(|v| *v < 0)
    {
        return Err("constant sprite inputs differ".into());
    }
    let [l, t, r, b] = i.margins.map(|v| v as usize);
    if l + r > width || t + b > rows.len() {
        return Err("constant sprite clip exceeds frame".into());
    }
    let mut p = ConstantPlan {
        writes: vec![],
        mask_scratch: i.mask_scratch.to_vec(),
        index_scratch: i.index_scratch.to_vec(),
    };
    for y in t..rows.len() - b {
        for (dst, v) in p.mask_scratch.iter_mut().zip(&rows[y].pixels) {
            if let Some(v) = v {
                *dst = *v
            }
        }
        if !rows[y].fetched {
            continue;
        }
        for (dst, v) in p.index_scratch.iter_mut().zip(&rows[y].pixels) {
            if let Some(v) = v {
                *dst = *v
            }
        }
        for x in l..width - r {
            if p.mask_scratch[x] == 255 {
                continue;
            }
            p.mask_scratch[x] = 255;
            let offset = |pitch: i32| {
                usize::try_from(
                    ((i64::from(i.screen[1]) + y as i64) * i64::from(pitch)
                        + i64::from(i.screen[0])
                        + x as i64)
                        * 2,
                )
                .map_err(|_| "constant sprite negative destination")
            };
            let at = offset(i.depth_pitch)?;
            let old = u16::from_le_bytes(
                i.depth
                    .get(at..at + 2)
                    .ok_or("constant sprite depth outside plane")?
                    .try_into()
                    .unwrap(),
            );
            let z = i.rows[y];
            if z <= i32::from(old) {
                let n = usize::from(p.index_scratch[x]) * 2;
                p.writes.push(Write {
                    target: Target::Color,
                    offset: offset(i.color_pitch)?,
                    width: 2,
                    value: u32::from(u16::from_le_bytes(i.palette[n..n + 2].try_into().unwrap())),
                });
                p.writes.push(Write {
                    target: Target::Depth,
                    offset: at,
                    width: 2,
                    value: u32::from(z as u16),
                });
            }
        }
        p.mask_scratch[..l].fill(255);
        p.mask_scratch[width - r..].fill(255);
    }
    Ok(p)
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Plan {
    pub writes: Vec<Write>,
    pub mask_scratch: Vec<u8>,
    pub color_scratch: Vec<u8>,
}
struct Row {
    pixels: Vec<Option<u8>>,
    fetched: bool,
}
fn take<'a>(b: &'a [u8], at: &mut usize, count: usize) -> Result<&'a [u8], String> {
    let end = at.checked_add(count).ok_or("dual sprite source overflow")?;
    let r = b
        .get(*at..end)
        .ok_or("dual sprite source outside retained frame")?;
    *at = end;
    Ok(r)
}
/// Preserve writes, including a literal 255 and transparent skips, separately.
fn decode(b: &[u8]) -> Result<(usize, Vec<Row>, usize), String> {
    if b.len() < 12 {
        return Err("dual sprite short frame".into());
    }
    let h: [i16; 4] =
        std::array::from_fn(|i| i16::from_le_bytes(b[2 * i..2 * i + 2].try_into().unwrap()));
    let width = usize::try_from(i32::from(h[3]) - i32::from(h[1]))
        .map_err(|_| "dual sprite negative width")?;
    let height = usize::try_from(i32::from(h[2]) - i32::from(h[0]))
        .map_err(|_| "dual sprite negative height")?;
    if width == 0 || width > 4096 || height > 4096 {
        return Err("dual sprite dimensions exceed bound".into());
    }
    let mut cursor = 10;
    let mut rows = vec![];
    for _ in 0..65536 {
        let start = cursor;
        let cmd = take(b, &mut cursor, 2)?;
        let count = usize::from(cmd[1]);
        match cmd[0] {
            5 => {
                if rows.len() != height {
                    return Err("dual sprite row extent differs".into());
                }
                return Ok((width, rows, cursor));
            }
            9 => {
                for _ in 0..count.max(1) {
                    rows.push(Row {
                        pixels: vec![None; width],
                        fetched: false,
                    });
                }
            }
            4 => {
                let length = if count == 2 && b.get(cursor..cursor + 2) == Some(&[5, 255]) {
                    take(b, &mut cursor, 2)?;
                    usize::from(u16::from_be_bytes(
                        take(b, &mut cursor, 2)?.try_into().unwrap(),
                    ))
                } else {
                    count
                };
                let end = start
                    .checked_add(length)
                    .ok_or("dual sprite row overflow")?;
                if end < cursor || end > b.len() {
                    return Err("dual sprite row outside frame".into());
                }
                let mut row = Row {
                    pixels: vec![None; width],
                    fetched: false,
                };
                let mut x = 0;
                while cursor < end {
                    let cmd = take(b, &mut cursor, 2)?;
                    let n = usize::from(cmd[1]);
                    if x + n > width {
                        return Err("dual sprite run exceeds row".into());
                    }
                    match cmd[0] {
                        1 => (),
                        2 => {
                            let value = take(b, &mut cursor, 2)?[0];
                            row.pixels[x..x + n].fill(Some(value));
                            row.fetched = true;
                        }
                        3 => {
                            let data = take(b, &mut cursor, n)?;
                            for (dst, &v) in row.pixels[x..x + n].iter_mut().zip(data) {
                                *dst = Some(v);
                            }
                            take(b, &mut cursor, n & 1)?;
                            row.fetched = true;
                        }
                        _ => return Err("unsupported dual sprite pixel command".into()),
                    }
                    x += n;
                }
                if cursor != end {
                    return Err("dual sprite row length differs".into());
                }
                rows.push(row);
            }
            _ => return Err("unsupported dual sprite row command".into()),
        }
        if rows.len() > height {
            return Err("dual sprite rows exceed bounds".into());
        }
    }
    Err("dual sprite command budget exceeded".into())
}
/// Size of the validated first SPR# frame, excluding an unowned suffix.
pub fn frame_bytes(encoded: &[u8]) -> Result<usize, String> {
    decode(encoded).map(|(_, _, used)| used)
}

#[cfg(test)]
mod tests {
    #[test]
    fn mapped_depth_uses_all_eight_groups_plus_one_and_depth_before_color() {
        let frame = |p: [u8; 3]| {
            let mut f = vec![0, 0, 0, 0, 1, 0, 3, 0, 0, 0, 4, 8, 3, 3];
            f.extend(p);
            f.extend([0, 5, 0]);
            f
        };
        let mask = frame([0, 1, 2]);
        let color = frame([5, 6, 7]);
        let mut map = vec![0; 256];
        map[0] = 4;
        map[1] = 1;
        map[2] = 7;
        let columns: Vec<Vec<u16>> = (0..8)
            .map(|g| (0..3).map(|x| g * 10 + x).collect())
            .collect();
        let mut palette = vec![0; 512];
        for index in 0..256u16 {
            palette[usize::from(index) * 2..usize::from(index) * 2 + 2]
                .copy_from_slice(&(0x100 + index).to_le_bytes());
        }
        let depth: Vec<u8> = [44u16, 10, 80]
            .iter()
            .flat_map(|d| d.to_le_bytes())
            .collect();
        let p = super::plan_mapped(super::MappedInput {
            mask: &mask,
            color: &color,
            palette: &palette,
            map: &map,
            columns: std::array::from_fn(|n| columns[n].as_slice()),
            offsets: [0, 1, 2, 3, 4, 5, 6, 7],
            rows: &[1],
            screen: [0, 0],
            margins: [0; 4],
            color_pitch: 3,
            depth_pitch: 3,
            mask_scratch: &[255; 3],
            index_scratch: &[0; 3],
            depth: &depth,
        })
        .unwrap();
        assert_eq!(p.mask_scratch, [255; 3]);
        assert_eq!(p.index_scratch, [5, 6, 7]);
        assert_eq!(
            p.writes
                .iter()
                .map(|w| (w.target, w.offset, w.value))
                .collect::<Vec<_>>(),
            [
                (super::Target::Depth, 0, 45),
                (super::Target::Color, 0, 0x105),
                (super::Target::Depth, 4, 80),
                (super::Target::Color, 4, 0x107)
            ]
        );
    }
    #[test]
    fn constant_depth_has_no_plus_one_tolerance_and_stores_color_first() {
        let frame = [0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 4, 6, 2, 1, 7, 0, 5, 0];
        let mut palette = vec![0; 512];
        palette[14..16].copy_from_slice(&0x1234u16.to_le_bytes());
        let depth = 5u16.to_le_bytes();
        let mask = [255];
        let indices = [0];
        let run = |z| {
            super::plan_constant(super::ConstantInput {
                frame: &frame,
                palette: &palette,
                screen: [0, 0],
                margins: [0; 4],
                color_pitch: 1,
                depth_pitch: 1,
                rows: &[z],
                mask_scratch: &mask,
                index_scratch: &indices,
                depth: &depth,
            })
            .unwrap()
        };
        assert!(run(6).writes.is_empty());
        let p = run(5);
        assert_eq!(p.writes.len(), 2);
        assert_eq!(
            (p.writes[0].target, p.writes[0].value),
            (super::Target::Color, 0x1234)
        );
        assert_eq!(
            (p.writes[1].target, p.writes[1].value),
            (super::Target::Depth, 5)
        );
    }
    #[test]
    fn frame_extent_excludes_unowned_suffix_and_rejects_a_missing_terminator() {
        let mut frame = vec![0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 9, 1, 5, 0];
        assert_eq!(super::frame_bytes(&frame).unwrap(), 14);
        frame.extend_from_slice(&[255; 8]);
        assert_eq!(super::frame_bytes(&frame).unwrap(), 14);
        assert!(super::frame_bytes(&frame[..13]).is_err());
        frame[4] = 2;
        assert!(super::frame_bytes(&frame).is_err());
    }
}
pub fn plan(i: Input<'_>) -> Result<Plan, String> {
    let (width, mask, _) = decode(i.mask)?;
    let (color_width, color, _) = decode(i.color)?;
    if i.mask[..8] != i.color[..8]
        || width != color_width
        || mask.len() != color.len()
        || i.palette.len() != 512
        || i.color_pitch < width as i32
        || i.depth_pitch < width as i32
        || i.margins.iter().any(|&v| v < 0)
        || i.columns.len() < width
        || i.rows.len() < mask.len()
        || i.mask_scratch.len() != width
        || i.color_scratch.len() != width * 2
    {
        return Err("dual sprite inputs differ".into());
    }
    let [left, top, right, bottom] = i.margins.map(|v| v as usize);
    if left + right > width || top + bottom > mask.len() {
        return Err("dual sprite clipping exceeds bounds".into());
    }
    let mut result = Plan {
        writes: vec![],
        mask_scratch: i.mask_scratch.to_vec(),
        color_scratch: i.color_scratch.to_vec(),
    };
    for y in top..mask.len() - bottom {
        for (dst, value) in result.mask_scratch.iter_mut().zip(&mask[y].pixels) {
            if let Some(value) = value {
                *dst = *value;
            }
        }
        if !mask[y].fetched {
            continue;
        }
        for (x, value) in color[y].pixels.iter().enumerate() {
            if let Some(value) = value {
                result.color_scratch[x * 2..x * 2 + 2].copy_from_slice(
                    &i.palette[usize::from(*value) * 2..usize::from(*value) * 2 + 2],
                );
            }
        }
        for x in left..width - right {
            if result.mask_scratch[x] == 255 {
                continue;
            }
            result.mask_scratch[x] = 255;
            let z = i.rows[y].wrapping_add(i32::from(i.columns[x]));
            let offset = |pitch: i32| -> Result<usize, String> {
                let pos = (i64::from(i.screen[1]) + y as i64) * i64::from(pitch)
                    + i64::from(i.screen[0])
                    + x as i64;
                usize::try_from(pos * 2)
                    .map_err(|_| "dual sprite destination precedes plane".into())
            };
            let d = offset(i.depth_pitch)?;
            let previous = u16::from_le_bytes(
                i.depth
                    .get(d..d + 2)
                    .ok_or("dual sprite depth outside plane")?
                    .try_into()
                    .unwrap(),
            );
            if z <= i32::from(previous) + 1 {
                result.writes.push(Write {
                    target: Target::Depth,
                    offset: d,
                    width: 2,
                    value: u32::from(z as u16),
                });
                result.writes.push(Write {
                    target: Target::Color,
                    offset: offset(i.color_pitch)?,
                    width: 2,
                    value: u32::from(u16::from_le_bytes(
                        result.color_scratch[x * 2..x * 2 + 2].try_into().unwrap(),
                    )),
                });
            }
        }
        result.mask_scratch[..left].fill(255);
        result.mask_scratch[width - right..].fill(255);
    }
    Ok(result)
}
