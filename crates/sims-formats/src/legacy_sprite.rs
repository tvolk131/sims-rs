//! SPR# byte RLE. Original 0x4ba3d4/0x4ba5de decode runs; 0x4ba52a skips rows
//! using their stored lengths. We check both lengths and destination bounds.
use crate::{
    Error, Result,
    reader::Reader,
    sprite::{Frame, Palette},
};

pub(crate) fn decode_frame(data: &[u8], big_endian: bool, palette: &Palette) -> Result<Frame> {
    let mut r = Reader::new(data);
    let word = |r: &mut Reader<'_>| if big_endian { r.be16() } else { r.u16() };
    // Native Rect is top, left, bottom, right; the two following bytes are
    // skipped by the original draw wrapper (0x4c7213).
    let top = word(&mut r)? as i16;
    let left = word(&mut r)? as i16;
    let bottom = word(&mut r)? as i16;
    let right = word(&mut r)? as i16;
    r.skip(2)?;
    let width = usize::try_from(i32::from(right) - i32::from(left))
        .map_err(|_| Error::new(0, "negative SPR# width"))?;
    let height = usize::try_from(i32::from(bottom) - i32::from(top))
        .map_err(|_| Error::new(0, "negative SPR# height"))?;
    let pixels = width
        .checked_mul(height)
        .filter(|&n| n <= 16 * 1024 * 1024)
        .ok_or_else(|| Error::new(0, "SPR# exceeds pixel limit"))?;
    let mut rgba = vec![0; pixels * 4];
    let mut row = 0;
    loop {
        let start = r.pos;
        let op = r.u8()?;
        let count = usize::from(r.u8()?);
        match op {
            5 => {
                if r.remaining() != 0 {
                    return r.fail("trailing SPR# frame bytes");
                }
                break;
            }
            9 => {
                // Native row fetch treats a zero count as one empty row.
                row += count.max(1);
                if row > height {
                    return r.fail("SPR# empty rows exceed image");
                }
            }
            4 => {
                if row >= height {
                    return r.fail("SPR# row exceeds image");
                }
                let length = if count == 2 && r.data.get(r.pos..r.pos + 2) == Some(&[5, 255]) {
                    r.skip(2)?;
                    usize::from(r.be16()?)
                } else {
                    count
                };
                if length < r.pos - start {
                    return r.fail("invalid SPR# row length");
                }
                let bytes = r.take(length - (r.pos - start))?;
                let mut line = Reader::new(bytes);
                let mut col = 0;
                while line.remaining() > 0 {
                    let op = line.u8()?;
                    let n = usize::from(line.u8()?);
                    if n > width - col {
                        return line.fail("SPR# pixel run exceeds row");
                    }
                    match op {
                        1 => {}
                        2 | 3 => {
                            let colors = if op == 2 {
                                line.take(2)?
                            } else {
                                line.take(n)?
                            };
                            for i in 0..n {
                                let index = colors[if op == 2 { 0 } else { i }] as usize;
                                let rgb = palette.0.get(index).ok_or_else(|| {
                                    Error::new(
                                        start,
                                        format!("SPR# palette index {index} out of range"),
                                    )
                                })?;
                                let dst = ((row * width) + col + i) * 4;
                                rgba[dst..dst + 3].copy_from_slice(rgb);
                                rgba[dst + 3] = 255;
                            }
                            if op == 3 && n % 2 != 0 {
                                line.skip(1)?;
                            }
                        }
                        _ => return line.fail(format!("unsupported SPR# pixel command {op}")),
                    }
                    col += n;
                }
                row += 1;
            }
            _ => return r.fail(format!("unsupported SPR# row command {op}")),
        }
    }
    Ok(Frame {
        width: width as u32,
        height: height as u32,
        x: i32::from(left),
        y: i32::from(top),
        flags: 0,
        rgba,
        depth: vec![255; pixels],
    })
}
