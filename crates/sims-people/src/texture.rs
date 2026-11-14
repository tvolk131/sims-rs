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

/// Decodes an uncompressed bitmap with 8, 24 or 32 bits per pixel, a
/// bit-field one (BI_BITFIELDS) with 16 or 32, or a run-length encoded one
/// with 8 bits (BI_RLE8), bottom-up or top-down.
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
    // The whole header is in the file, so offsets past it cannot
    // overflow, and the pixels do not start inside it.
    let header_end = 14 + header;
    if header_end > data.len() {
        return Err("truncated bitmap header".into());
    }
    if pixels > data.len() {
        return Err("bitmap pixels start past the end of the file".into());
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
        let bytes = count
            .checked_mul(4)
            .and_then(|n| data.get(header_end..header_end.checked_add(n)?))
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
    // BI_BITFIELDS: the red, green and blue masks follow a 40-byte header
    // (and sit at the same place inside a longer one); a header of 56
    // bytes or more also holds an alpha mask. Without one, texels are
    // opaque.
    let masks = if compression == 3 {
        // After a 40-byte header the three masks take 12 more bytes.
        if pixels < header_end.max(66) {
            return Err("bitmap pixels overlap its colour masks".into());
        }
        let alpha = if header >= 56 { u32_at(66)? } else { 0 };
        let masks = [u32_at(54)?, u32_at(58)?, u32_at(62)?, alpha];
        if masks[..3].contains(&0) {
            return Err("a bit-field bitmap with an empty colour mask".into());
        }
        Some(masks.map(Channel::new))
    } else {
        if pixels < header_end {
            return Err("bitmap pixels overlap its header".into());
        }
        None
    };
    // Where the uncompressed pixels of `stride` bytes a row are.
    let body = |stride: usize| {
        stride
            .checked_mul(height as usize)
            .and_then(|n| data.get(pixels..pixels.checked_add(n)?))
            .ok_or("truncated bitmap pixels")
    };
    match (bits, compression) {
        (16 | 32, 3) => {
            let masks = masks.ok_or("bit-field bitmap without masks")?;
            let bytes = usize::from(bits) / 8;
            let stride = (width as usize * bytes).div_ceil(4) * 4;
            let body = body(stride)?;
            for row in 0..height {
                let line = &body[row as usize * stride..];
                for x in 0..width {
                    let p = &line[x as usize * bytes..];
                    let value = if bytes == 2 {
                        u32::from(u16::from_le_bytes([p[0], p[1]]))
                    } else {
                        u32::from_le_bytes([p[0], p[1], p[2], p[3]])
                    };
                    let [r, g, b, a] = masks;
                    let alpha = if a.mask == 0 { 255 } else { a.get(value) };
                    put(x, row, [r.get(value), g.get(value), b.get(value), alpha]);
                }
            }
        }
        (8 | 24 | 32, 0) => {
            let stride = (width as usize * usize::from(bits) / 8).div_ceil(4) * 4;
            let body = body(stride)?;
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

/// One channel of a bit-field bitmap: its mask, and how to scale the bits
/// it selects to 0..=255.
#[derive(Debug, Clone, Copy)]
struct Channel {
    mask: u32,
    shift: u32,
    max: u32,
}

impl Channel {
    fn new(mask: u32) -> Self {
        let shift = if mask == 0 { 0 } else { mask.trailing_zeros() };
        // The bits from the lowest set bit up to the highest; a mask with
        // holes is read as if they were set.
        let max = if mask == 0 {
            0
        } else {
            u32::MAX >> (mask.leading_zeros() + shift)
        };
        Self { mask, shift, max }
    }

    fn get(self, value: u32) -> u8 {
        if self.max == 0 {
            return 0;
        }
        let v = u64::from((value & self.mask) >> self.shift);
        ((v * 255 + u64::from(self.max) / 2) / u64::from(self.max)) as u8
    }
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

    /// A 32-bit bit-field bitmap of one texel with these masks after a
    /// 40-byte header.
    fn bitfields(masks: [u32; 3], texel: u32) -> Vec<u8> {
        let mut body = Vec::new();
        for m in masks {
            body.extend(m.to_le_bytes());
        }
        body.extend(texel.to_le_bytes());
        let mut file = bitmap(1, 1, 32, 3, &[], &body);
        // The pixels start after the masks.
        file[10..14].copy_from_slice(&66u32.to_le_bytes());
        file
    }

    #[test]
    fn a_bit_field_bitmap_reads_its_masks() {
        // Red in the low byte, green in the next, blue in the third.
        let file = bitfields([0xff, 0xff00, 0xff_0000], 0x0003_02ff);
        assert_eq!(read_bmp(&file).unwrap().rgba, [255, 2, 3, 255]);
        // The usual order, blue lowest, reads the same texel differently.
        let file = bitfields([0xff_0000, 0xff00, 0xff], 0x0003_02ff);
        assert_eq!(read_bmp(&file).unwrap().rgba, [3, 2, 255, 255]);
        // A narrow mask is scaled to the full range.
        let file = bitfields([0b1111_1000_0000_0000, 0b111_1110_0000, 0b1_1111], 0xf800);
        assert_eq!(read_bmp(&file).unwrap().rgba, [255, 0, 0, 255]);
        // An empty colour mask is refused.
        assert!(read_bmp(&bitfields([0, 0xff00, 0xff], 0)).is_err());
    }

    #[test]
    fn unsupported_bitmaps_fail() {
        assert!(read_bmp(b"XX").is_err());
        assert!(read_bmp(&bitmap(1, 1, 16, 0, &[], &[0; 4])).is_err());
        assert!(read_bmp(&bitmap(0, 1, 24, 0, &[], &[0; 4])).is_err());
    }

    #[test]
    fn a_bitmap_whose_parts_do_not_fit_fails() {
        let good = bitfields([0xff, 0xff00, 0xff_0000], 0);
        assert!(read_bmp(&good).is_ok());
        // Pixels that start inside the colour masks.
        let mut file = good.clone();
        file[10..14].copy_from_slice(&54u32.to_le_bytes());
        assert!(read_bmp(&file).is_err());
        // A header longer than the file.
        let mut file = good.clone();
        file[14..18].copy_from_slice(&108u32.to_le_bytes());
        assert!(read_bmp(&file).is_err());
        // Pixels that start far past the end, or inside the header.
        for offset in [u32::MAX, 20] {
            let mut file = good.clone();
            file[10..14].copy_from_slice(&offset.to_le_bytes());
            assert!(read_bmp(&file).is_err(), "{offset}");
            let mut file = bitmap(1, 1, 24, 0, &[], &[0; 4]);
            file[10..14].copy_from_slice(&offset.to_le_bytes());
            assert!(read_bmp(&file).is_err(), "{offset}");
        }
        // A palette too large to count.
        let mut file = bitmap(1, 1, 8, 0, &[[0; 4]], &[0; 4]);
        file[46..50].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_bmp(&file).is_err());
    }
}
