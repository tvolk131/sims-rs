//! Skin textures: Windows bitmaps (`.bmp`), as the game's skins are stored.

/// A decoded texture, rows top to bottom, four bytes (red, green, blue,
/// alpha) per texel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Texture {
    /// A texture of one texel.
    pub fn solid(rgba: [u8; 4]) -> Self {
        Self {
            width: 1,
            height: 1,
            rgba: rgba.to_vec(),
        }
    }

    /// The texel at `u`, `v` (0 to 1, `v` from the top), nearest, with
    /// the coordinates wrapped.
    pub fn sample(&self, u: f32, v: f32) -> [u8; 4] {
        let wrap = |t: f32, n: u32| {
            let t = t - t.floor();
            ((t * n as f32) as u32).min(n - 1)
        };
        let x = wrap(u, self.width);
        let y = wrap(v, self.height);
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}

/// The largest side accepted, in texels.
const MAX_SIDE: u32 = 4096;

/// Decodes an uncompressed bitmap with 8, 24 or 32 bits per pixel, or a
/// run-length encoded one with 8 bits (BI_RLE8), bottom-up or top-down.
/// Every texel is opaque, but for texels a run-length bitmap skips,
/// which stay transparent black.
pub fn read_bmp(data: &[u8]) -> Result<Texture, String> {
    let u16_at = |at: usize| {
        data.get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or("truncated bitmap header")
    };
    let u32_at = |at: usize| {
        data.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or("truncated bitmap header")
    };
    if data.get(..2) != Some(b"BM") {
        return Err("not a bitmap".into());
    }
    let pixels = u32_at(10)? as usize;
    let header = u32_at(14)? as usize;
    if header < 40 {
        return Err(format!("unsupported bitmap header of {header} bytes"));
    }
    let width = u32_at(18)? as i32;
    let height = u32_at(22)? as i32;
    let bits = u16_at(28)?;
    let compression = u32_at(30)?;
    let used = u32_at(46)?;
    let (top_down, height) = if height < 0 {
        (true, height.unsigned_abs())
    } else {
        (false, height as u32)
    };
    if width <= 0 || height == 0 || width as u32 > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("unsupported bitmap size {width}x{height}"));
    }
    let width = width as u32;
    let palette = if bits <= 8 {
        let count = if used == 0 { 1 << bits } else { used as usize };
        let start = 14 + header;
        let bytes = data
            .get(start..start + count * 4)
            .ok_or("truncated bitmap palette")?;
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| [c[2], c[1], c[0], 255])
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let lookup = |index: u8| -> Result<[u8; 4], String> {
        palette
            .get(usize::from(index))
            .copied()
            .ok_or_else(|| "bitmap index outside its palette".to_owned())
    };
    let mut rgba = vec![0; (width * height * 4) as usize];
    let mut put = |x: u32, row: u32, texel: [u8; 4]| {
        let y = if top_down { row } else { height - 1 - row };
        let i = ((y * width + x) * 4) as usize;
        rgba[i..i + 4].copy_from_slice(&texel);
    };
    match (bits, compression) {
        (8 | 24 | 32, 0) | (32, 3) => {
            let stride = (width as usize * usize::from(bits) / 8).div_ceil(4) * 4;
            let body = data
                .get(pixels..pixels + stride * height as usize)
                .ok_or("truncated bitmap pixels")?;
            for row in 0..height {
                let line = &body[row as usize * stride..];
                for x in 0..width {
                    let texel = match bits {
                        8 => lookup(line[x as usize])?,
                        24 => {
                            let p = &line[x as usize * 3..];
                            [p[2], p[1], p[0], 255]
                        }
                        _ => {
                            let p = &line[x as usize * 4..];
                            [p[2], p[1], p[0], 255]
                        }
                    };
                    put(x, row, texel);
                }
            }
        }
        (8, 1) => {
            let mut at = pixels;
            let (mut x, mut row) = (0u32, 0u32);
            let byte = |at: usize| data.get(at).copied().ok_or("truncated bitmap run");
            loop {
                let (count, value) = (byte(at)?, byte(at + 1)?);
                at += 2;
                if count > 0 {
                    let texel = lookup(value)?;
                    for _ in 0..count {
                        if x < width && row < height {
                            put(x, row, texel);
                        }
                        x += 1;
                    }
                    continue;
                }
                match value {
                    0 => {
                        x = 0;
                        row += 1;
                    }
                    1 => break,
                    2 => {
                        x += u32::from(byte(at)?);
                        row += u32::from(byte(at + 1)?);
                        at += 2;
                    }
                    n => {
                        for i in 0..usize::from(n) {
                            let texel = lookup(byte(at + i)?)?;
                            if x < width && row < height {
                                put(x, row, texel);
                            }
                            x += 1;
                        }
                        at += usize::from(n).div_ceil(2) * 2;
                    }
                }
                if row >= height {
                    break;
                }
            }
        }
        _ => {
            return Err(format!(
                "unsupported bitmap: {bits} bits per pixel, compression {compression}"
            ));
        }
    }
    Ok(Texture {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bitmap file around `body`, with `palette` colors (blue, green,
    /// red, zero).
    fn bitmap(
        width: i32,
        height: i32,
        bits: u16,
        compression: u32,
        palette: &[[u8; 4]],
        body: &[u8],
    ) -> Vec<u8> {
        let offset = 54 + palette.len() * 4;
        let mut out = b"BM".to_vec();
        out.extend(((offset + body.len()) as u32).to_le_bytes());
        out.extend([0; 4]);
        out.extend((offset as u32).to_le_bytes());
        out.extend(40u32.to_le_bytes());
        out.extend(width.to_le_bytes());
        out.extend(height.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(bits.to_le_bytes());
        out.extend(compression.to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        out.extend([0; 8]);
        out.extend((palette.len() as u32).to_le_bytes());
        out.extend([0; 4]);
        for c in palette {
            out.extend(c);
        }
        out.extend(body);
        out
    }

    #[test]
    fn a_paletted_bitmap_reads_bottom_up() {
        let palette = [[0, 0, 255, 0], [255, 0, 0, 0]];
        // Two rows of three texels, each padded to four bytes. The file's
        // first row is the image's bottom row.
        let body = [0, 1, 0, 0, 1, 1, 1, 0];
        let t = read_bmp(&bitmap(3, 2, 8, 0, &palette, &body)).unwrap();
        assert_eq!((t.width, t.height), (3, 2));
        assert_eq!(t.sample(0.0, 0.0), [0, 0, 255, 255]);
        assert_eq!(t.sample(0.0, 0.9), [255, 0, 0, 255]);
        assert_eq!(t.sample(0.5, 0.9), [0, 0, 255, 255]);
    }

    #[test]
    fn a_true_colour_bitmap_reads_top_down() {
        // One row of two texels, then padding, in a top-down file.
        let body = [1, 2, 3, 4, 5, 6, 0, 0, 7, 8, 9, 10, 11, 12, 0, 0];
        let t = read_bmp(&bitmap(2, -2, 24, 0, &[], &body)).unwrap();
        assert_eq!(&t.rgba[..8], &[3, 2, 1, 255, 6, 5, 4, 255]);
        assert_eq!(&t.rgba[8..], &[9, 8, 7, 255, 12, 11, 10, 255]);
    }

    #[test]
    fn a_run_length_bitmap_reads_runs_and_literals() {
        let palette = [[0, 0, 0, 0], [0, 0, 255, 0], [0, 255, 0, 0]];
        // Bottom row: a run of two of colour 1, then end of line. Top
        // row: three literal texels 2, 1, 2 (padded), then end of bitmap.
        let body = [2, 1, 0, 0, 0, 3, 2, 1, 2, 0, 0, 1];
        let t = read_bmp(&bitmap(3, 2, 8, 1, &palette, &body)).unwrap();
        let (r, g, k) = ([255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 0, 0]);
        let texels = t
            .rgba
            .chunks(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect::<Vec<_>>();
        // Texels a run does not reach stay transparent black.
        assert_eq!(texels, [g, r, g, r, r, k]);
    }

    #[test]
    fn unsupported_bitmaps_fail() {
        assert!(read_bmp(b"XX").is_err());
        assert!(read_bmp(&bitmap(1, 1, 16, 0, &[], &[0; 4])).is_err());
        assert!(read_bmp(&bitmap(0, 1, 24, 0, &[], &[0; 4])).is_err());
    }
}
