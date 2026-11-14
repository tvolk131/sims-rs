//! Windows BMP and Truevision TGA images, the formats of the interface's art.
//!
//! The UI archive (`UIGraphics/UIGraphics.far`) holds 673 BMP files and 8
//! TGA files. The BMP files are 8-bit paletted, stored plain or with RLE8
//! compression, or 24-bit; IFF `BMP_` chunks (a character's faces, for
//! example) are BMP files too. The TGA files are 32-bit with an alpha
//! channel, uncompressed. Those are the kinds read here; any other is an
//! error, never a guess.
//!
//! An image comes out top-down, as straight RGBA. BMP pixels are opaque;
//! the original makes one color transparent per image (usually magenta),
//! which is the caller's choice ([`Image::key`]).

use crate::reader::Reader;
use crate::{Error, Result};

/// The largest image accepted, in pixels. The largest in the install is a
/// 1024 by 768 loading screen.
const MAX_PIXELS: usize = 4096 * 4096;

/// A decoded image: top-down rows of straight RGBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// The pixel at (x, y), or None outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba.get(at..at + 4)?.try_into().ok()
    }

    /// Makes every pixel of color `rgb` fully transparent.
    pub fn key(&mut self, rgb: [u8; 3]) {
        for pixel in self.rgba.as_chunks_mut::<4>().0 {
            if pixel[..3] == rgb {
                pixel[3] = 0;
            }
        }
    }

    /// The rectangle at (x, y) of the given size, or None when it is not
    /// inside the image.
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Option<Image> {
        if x.checked_add(width)? > self.width || y.checked_add(height)? > self.height {
            return None;
        }
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for row in y..y + height {
            let start = (row as usize * self.width as usize + x as usize) * 4;
            rgba.extend_from_slice(self.rgba.get(start..start + width as usize * 4)?);
        }
        Some(Image {
            width,
            height,
            rgba,
        })
    }
}

fn pixels(width: i64, height: i64, at: usize) -> Result<(u32, u32)> {
    let ok = width > 0
        && height > 0
        && width <= i64::from(u32::MAX)
        && height <= i64::from(u32::MAX)
        && (width as u64).saturating_mul(height as u64) <= MAX_PIXELS as u64;
    if !ok {
        return Err(Error::new(
            at,
            format!("bad image size {width} by {height}"),
        ));
    }
    Ok((width as u32, height as u32))
}

/// Decodes a BMP file.
pub fn bmp(data: &[u8]) -> Result<Image> {
    let mut r = Reader::new(data);
    if r.take(2)? != b"BM" {
        return r.fail("not a BMP file");
    }
    r.skip(8)?;
    let offset = r.u32()? as usize;
    let header = r.u32()?;
    if header < 40 {
        return r.fail(format!("unsupported BMP header of {header} bytes"));
    }
    let width = r.i32()?;
    let raw_height = r.i32()?;
    let planes = r.u16()?;
    let depth = r.u16()?;
    let compression = r.u32()?;
    r.skip(12)?;
    let used = r.u32()?;
    if planes != 1 {
        return r.fail("BMP with more than one plane");
    }
    let top_down = raw_height < 0;
    let (width, height) = pixels(i64::from(width), i64::from(raw_height).abs(), 18)?;
    let palette = if depth == 8 {
        let count = if used == 0 { 256 } else { used as usize };
        if count > 256 {
            return r.fail(format!("BMP palette of {count} colors"));
        }
        let at = 14 + header as usize;
        let table = data
            .get(at..)
            .and_then(|rest| rest.get(..count * 4))
            .ok_or_else(|| Error::new(at, "BMP palette outside file"))?;
        let mut palette = [[0u8; 3]; 256];
        for (entry, bgr) in palette.iter_mut().zip(table.as_chunks::<4>().0) {
            *entry = [bgr[2], bgr[1], bgr[0]];
        }
        palette
    } else {
        [[0; 3]; 256]
    };
    let body = data
        .get(offset..)
        .ok_or_else(|| Error::new(10, "BMP pixel data outside file"))?;
    let (w, h) = (width as usize, height as usize);
    // Rows as stored: bottom-up unless the height is negative.
    let mut rows = vec![0u8; w * h * 3];
    let mut put = |x: usize, stored_row: usize, rgb: [u8; 3]| {
        if x < w && stored_row < h {
            let y = if top_down {
                stored_row
            } else {
                h - 1 - stored_row
            };
            rows[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&rgb);
        }
    };
    match (depth, compression) {
        (8, 0) | (24, 0) => {
            let bytes = usize::from(depth / 8);
            let stride = (w * bytes).div_ceil(4) * 4;
            if body.len() < stride * h {
                return Err(Error::new(offset, "BMP pixel data is short"));
            }
            for stored_row in 0..h {
                let line = &body[stored_row * stride..stored_row * stride + w * bytes];
                for x in 0..w {
                    let rgb = if depth == 8 {
                        palette[usize::from(line[x])]
                    } else {
                        let p = &line[x * 3..x * 3 + 3];
                        [p[2], p[1], p[0]]
                    };
                    put(x, stored_row, rgb);
                }
            }
        }
        (8, 1) => {
            if top_down {
                return Err(Error::new(22, "top-down RLE8 BMP"));
            }
            // Pixels a run never reaches keep palette color 0.
            for stored_row in 0..h {
                for x in 0..w {
                    put(x, stored_row, palette[0]);
                }
            }
            let mut r = Reader::new(body);
            let (mut x, mut y) = (0usize, 0usize);
            loop {
                let count = r.u8()?;
                let value = r.u8()?;
                if count > 0 {
                    for _ in 0..count {
                        put(x, y, palette[usize::from(value)]);
                        x += 1;
                    }
                    continue;
                }
                match value {
                    0 => {
                        x = 0;
                        y += 1;
                    }
                    1 => break,
                    2 => {
                        x += usize::from(r.u8()?);
                        y += usize::from(r.u8()?);
                    }
                    n => {
                        let run = r.take(usize::from(n))?;
                        for &index in run {
                            put(x, y, palette[usize::from(index)]);
                            x += 1;
                        }
                        if n % 2 == 1 {
                            r.skip(1)?;
                        }
                    }
                }
                if y > h {
                    return r.fail("RLE8 data runs past the image");
                }
            }
        }
        _ => {
            return Err(Error::new(
                28,
                format!("unsupported BMP: {depth} bits per pixel, compression {compression}"),
            ));
        }
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for rgb in rows.as_chunks::<3>().0 {
        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

/// Decodes an uncompressed 32-bit TGA file (image type 2), the only kind
/// the install holds.
pub fn tga(data: &[u8]) -> Result<Image> {
    let mut r = Reader::new(data);
    let id_length = r.u8()?;
    let color_map = r.u8()?;
    let kind = r.u8()?;
    r.skip(9)?;
    let width = r.u16()?;
    let height = r.u16()?;
    let depth = r.u8()?;
    let descriptor = r.u8()?;
    if color_map != 0 || kind != 2 || depth != 32 {
        return r.fail(format!(
            "unsupported TGA: type {kind}, {depth} bits per pixel, color map {color_map}"
        ));
    }
    if descriptor & 0x10 != 0 {
        return r.fail("right-to-left TGA");
    }
    let (width, height) = pixels(i64::from(width), i64::from(height), 12)?;
    r.skip(usize::from(id_length))?;
    let (w, h) = (width as usize, height as usize);
    let body = r.take(w * h * 4)?;
    // Bit 5 of the descriptor: rows run top-down; otherwise bottom-up.
    let top_down = descriptor & 0x20 != 0;
    let mut rgba = vec![0u8; w * h * 4];
    for stored_row in 0..h {
        let y = if top_down {
            stored_row
        } else {
            h - 1 - stored_row
        };
        for x in 0..w {
            let p = &body[(stored_row * w + x) * 4..(stored_row * w + x) * 4 + 4];
            rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(
        width: i32,
        height: i32,
        depth: u16,
        compression: u32,
        palette: &[[u8; 3]],
    ) -> Vec<u8> {
        let offset = 54 + palette.len() * 4;
        let mut b = b"BM".to_vec();
        b.extend(0u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend((offset as u32).to_le_bytes());
        b.extend(40u32.to_le_bytes());
        b.extend(width.to_le_bytes());
        b.extend(height.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(depth.to_le_bytes());
        b.extend(compression.to_le_bytes());
        b.extend([0; 12]);
        b.extend((palette.len() as u32).to_le_bytes());
        b.extend(0u32.to_le_bytes());
        for c in palette {
            b.extend([c[2], c[1], c[0], 0]);
        }
        b
    }

    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    #[test]
    fn plain_8_bit_rows_are_bottom_up_and_padded() {
        let mut b = header(3, 2, 8, 0, &[RED, BLUE]);
        b.extend([0, 1, 0, 0xee]); // bottom row, padded to 4
        b.extend([1, 1, 1, 0xee]); // top row
        let image = bmp(&b).unwrap();
        assert_eq!((image.width, image.height), (3, 2));
        assert_eq!(image.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(image.pixel(0, 1), Some([255, 0, 0, 255]));
        assert_eq!(image.pixel(1, 1), Some([0, 0, 255, 255]));
    }

    #[test]
    fn rle8_runs_escapes_and_absolute_mode() {
        let mut b = header(4, 2, 8, 1, &[RED, BLUE]);
        // Bottom row: two BLUE, then an absolute run of 3 (padded) clipped
        // at the edge; end of line. Top row: delta right 2, one BLUE, end.
        b.extend([2, 1, 0, 3, 0, 1, 0, 0, 0, 0, 0, 2, 2, 0, 1, 1, 0, 1]);
        let image = bmp(&b).unwrap();
        let row = |y| {
            (0..4)
                .map(|x| image.pixel(x, y).unwrap()[2])
                .collect::<Vec<_>>()
        };
        assert_eq!(row(1), [255, 255, 0, 255]);
        assert_eq!(row(0), [0, 0, 255, 0]);
    }

    #[test]
    fn rgb_24_bit() {
        let mut b = header(1, 1, 24, 0, &[]);
        b.extend([1, 2, 3, 0]);
        assert_eq!(bmp(&b).unwrap().pixel(0, 0), Some([3, 2, 1, 255]));
    }

    #[test]
    fn malformed_bmp_never_panics() {
        let mut b = header(4, 2, 8, 1, &[RED, BLUE]);
        b.extend([2, 1, 0, 3, 0, 1, 0, 0, 0, 0, 0, 2, 2, 0, 1, 1, 0, 1]);
        for end in 0..b.len() {
            let _ = bmp(&b[..end]);
        }
        let mut huge = header(1 << 20, 1 << 20, 24, 0, &[]);
        huge.extend([0; 16]);
        assert!(bmp(&huge).is_err());
        assert!(bmp(&header(2, 2, 4, 0, &[])).is_err());
    }

    #[test]
    fn tga_bottom_up_bgra() {
        let mut b = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        b.extend(1u16.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend([32, 8]);
        b.extend([1, 2, 3, 4, 5, 6, 7, 8]);
        let image = tga(&b).unwrap();
        assert_eq!(image.pixel(0, 1), Some([3, 2, 1, 4]));
        assert_eq!(image.pixel(0, 0), Some([7, 6, 5, 8]));
        for end in 0..b.len() {
            assert!(tga(&b[..end]).is_err());
        }
    }

    #[test]
    fn crop_and_key() {
        let mut image = Image {
            width: 2,
            height: 2,
            rgba: vec![
                255, 0, 255, 255, 1, 1, 1, 255, //
                2, 2, 2, 255, 255, 0, 255, 255,
            ],
        };
        image.key([255, 0, 255]);
        assert_eq!(image.pixel(0, 0), Some([255, 0, 255, 0]));
        let cell = image.crop(1, 0, 1, 2).unwrap();
        assert_eq!(cell.rgba, [1, 1, 1, 255, 255, 0, 255, 0]);
        assert!(image.crop(1, 1, 2, 1).is_none());
        assert!(image.crop(u32::MAX, 0, 2, 1).is_none());
    }
}
