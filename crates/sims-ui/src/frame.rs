//! A CPU pixel buffer the interface draws into, so drawing needs no window
//! and `cargo test` can check it.

use sims_formats::bitmap::Image;

/// A rectangle in pixels. Width and height are never negative.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width: if width < 0 { 0 } else { width },
            height: if height < 0 { 0 } else { height },
        }
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }

    /// The same rectangle moved by `dx`, `dy`.
    pub fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(
            self.x.saturating_add(dx),
            self.y.saturating_add(dy),
            self.width,
            self.height,
        )
    }
}

/// An RGB image, row by row from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<[u8; 3]>,
}

impl Frame {
    /// A frame of `width` by `height` filled with `background`. A size
    /// above 16384 on a side is clamped to it.
    pub fn new(width: u32, height: u32, background: [u8; 3]) -> Self {
        let (width, height) = (width.min(16384), height.min(16384));
        Self {
            width,
            height,
            rgb: vec![background; width as usize * height as usize],
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (u32::try_from(x).ok()?, u32::try_from(y).ok()?);
        (x < self.width && y < self.height).then(|| y as usize * self.width as usize + x as usize)
    }

    pub fn pixel(&self, x: i32, y: i32) -> Option<[u8; 3]> {
        self.index(x, y).map(|i| self.rgb[i])
    }

    /// Blends `rgb` over one pixel with coverage `alpha` (255 opaque).
    pub fn blend(&mut self, x: i32, y: i32, rgb: [u8; 3], alpha: u8) {
        let Some(i) = self.index(x, y) else {
            return;
        };
        let pixel = &mut self.rgb[i];
        if alpha == 255 {
            *pixel = rgb;
        } else if alpha > 0 {
            let a = u32::from(alpha);
            for (c, s) in pixel.iter_mut().zip(rgb) {
                *c = ((u32::from(s) * a + u32::from(*c) * (255 - a) + 127) / 255) as u8;
            }
        }
    }

    /// Fills `rect` (clipped to the frame) with `rgb`.
    pub fn fill(&mut self, rect: Rect, rgb: [u8; 3]) {
        let clip = |start: i32, length: i32, size: u32| {
            let end = i64::from(start) + i64::from(length);
            let size = i64::from(size);
            let (start, end) = (i64::from(start).clamp(0, size), end.min(size));
            start as usize..end.max(start) as usize
        };
        let xs = clip(rect.x, rect.width, self.width);
        for y in clip(rect.y, rect.height, self.height) {
            let row = y * self.width as usize;
            self.rgb[row + xs.start..row + xs.end].fill(rgb);
        }
    }

    /// Draws the part `source` of `image` with its top-left corner at
    /// `at`, blending by the image's alpha. Parts of `source` outside the
    /// image, and of the target outside the frame, are skipped.
    pub fn draw(&mut self, image: &Image, source: Rect, at: [i32; 2]) {
        // Offsets into `source` that land inside both the image and the
        // frame, worked out in i64 so nothing overflows.
        let range = |start: i32, length: i32, size: u32, target: i32, frame: u32| {
            let (start, target) = (i64::from(start), i64::from(target));
            let low = 0.max(-start).max(-target);
            let high = i64::from(length)
                .min(i64::from(size) - start)
                .min(i64::from(frame) - target);
            low..high.max(low)
        };
        let xs = range(source.x, source.width, image.width, at[0], self.width);
        for sy in range(source.y, source.height, image.height, at[1], self.height) {
            for sx in xs.clone() {
                let (ix, iy) = (i64::from(source.x) + sx, i64::from(source.y) + sy);
                if let Some([r, g, b, a]) = image.pixel(ix as u32, iy as u32) {
                    let (x, y) = (i64::from(at[0]) + sx, i64::from(at[1]) + sy);
                    self.blend(x as i32, y as i32, [r, g, b], a);
                }
            }
        }
    }

    /// Draws all of `image` at `at`.
    pub fn draw_all(&mut self, image: &Image, at: [i32; 2]) {
        let size = |n: u32| i32::try_from(n).unwrap_or(i32::MAX);
        self.draw(
            image,
            Rect::new(0, 0, size(image.width), size(image.height)),
            at,
        );
    }

    /// The frame with each pixel repeated `scale` times across and down,
    /// for a high-DPI display. A scale of 0 is taken as 1.
    pub fn upscale(&self, scale: u32) -> Self {
        let scale = scale.max(1);
        let mut out = Self::new(
            self.width.saturating_mul(scale),
            self.height.saturating_mul(scale),
            [0; 3],
        );
        for y in 0..out.height {
            for x in 0..out.width {
                out.rgb[(y * out.width + x) as usize] =
                    self.rgb[((y / scale) * self.width + x / scale) as usize];
            }
        }
        out
    }

    /// Each color through the original's 16-bit display (5 bits red, 6
    /// green, 5 blue), widened back by repeating the high bits, as
    /// `sims_render::Canvas::rgb565` does. Comparisons with screenshots of
    /// the original are made on this.
    pub fn quantized(&self) -> Self {
        let q = |[r, g, b]: [u8; 3]| {
            let (r, g, b) = (r >> 3, g >> 2, b >> 3);
            [
                (r << 3) | (r >> 2),
                (g << 2) | (g >> 4),
                (b << 3) | (b >> 2),
            ]
        };
        Self {
            width: self.width,
            height: self.height,
            rgb: self.rgb.iter().map(|&c| q(c)).collect(),
        }
    }

    /// The frame as a PNG file (8-bit RGB, stored without compression).
    /// The same writer as `crates/sims-render/src/png.rs`.
    pub fn png(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity((self.width as usize * 3 + 1) * self.height as usize);
        for row in self.rgb.chunks(self.width.max(1) as usize) {
            raw.push(0);
            raw.extend(row.iter().flatten());
        }
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = Vec::new();
        header.extend(self.width.to_be_bytes());
        header.extend(self.height.to_be_bytes());
        header.extend([8, 2, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &header);
        chunk(&mut out, b"IDAT", &zlib_stored(&raw));
        chunk(&mut out, b"IEND", &[]);
        out
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = data.chunks(65535).collect();
    if blocks.is_empty() {
        out.extend([1, 0, 0, 0xff, 0xff]);
    }
    for (i, block) in blocks.iter().enumerate() {
        out.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend(*block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend(((b << 16) | a).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, rgba: &[[u8; 4]]) -> Image {
        Image {
            width,
            height,
            rgba: rgba.iter().flatten().copied().collect(),
        }
    }

    #[test]
    fn draw_clips_and_blends() {
        let mut f = Frame::new(3, 2, [0, 0, 0]);
        let im = image(
            2,
            2,
            &[
                [255, 0, 0, 255],
                [0, 0, 0, 0],
                [0, 255, 0, 128],
                [9, 9, 9, 255],
            ],
        );
        f.draw_all(&im, [2, 0]);
        assert_eq!(f.pixel(2, 0), Some([255, 0, 0]));
        assert_eq!(f.pixel(2, 1), Some([0, 128, 0]));
        // Outside the image or the frame: nothing happens, nothing panics.
        f.draw(&im, Rect::new(-5, -5, 50, 50), [-100, 1000]);
        f.draw(
            &im,
            Rect::new(i32::MAX, i32::MAX, i32::MAX, 2),
            [i32::MAX, i32::MIN],
        );
        f.fill(Rect::new(i32::MAX - 1, 0, i32::MAX, 5), [1, 1, 1]);
        assert_eq!(f.pixel(0, 0), Some([0, 0, 0]));
    }

    #[test]
    fn upscale_quantize_and_png() {
        let mut f = Frame::new(2, 1, [0, 0, 0]);
        f.fill(Rect::new(1, 0, 1, 1), [255, 255, 7]);
        let big = f.upscale(2);
        assert_eq!((big.width, big.height), (4, 2));
        assert_eq!(big.pixel(3, 1), Some([255, 255, 7]));
        assert_eq!(f.quantized().pixel(1, 0), Some([255, 255, 0]));
        let png = f.png();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
    }

    #[test]
    fn rect_contains_without_overflow() {
        let r = Rect::new(i32::MAX - 1, 0, 10, 1);
        assert!(r.contains(i32::MAX, 0));
        assert!(!r.contains(0, 0));
        assert_eq!(Rect::new(0, 0, -3, 2).width, 0);
    }
}
