//! SPR2 decoding and an isolated-object DGRP preview, retaining straight alpha and depth.
use crate::iff::Iff;
use crate::reader::Reader;
use crate::{Error, Result};
use std::ops::Range;
const MAX_PIXELS: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Palette(pub Vec<[u8; 3]>);
impl Palette {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        r.u32()?;
        let count = r.u32()? as usize;
        r.skip(8)?;
        if count > 256 {
            return r.fail("palette exceeds 256 colors");
        }
        r.count(count, 3)?;
        let mut colors = Vec::with_capacity(count);
        for _ in 0..count {
            colors.push(r.take(3)?.try_into().unwrap());
        }
        Ok(Self(colors))
    }
}
#[derive(Debug, Clone)]
pub struct SpriteSet {
    pub version: u32,
    pub default_palette: u32,
    pub frames: Vec<Range<usize>>,
}
impl SpriteSet {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let version = r.u32()?;
        let (count, default_palette) = match version {
            1000 => {
                let n = r.u32()?;
                let p = r.u32()?;
                (n, p)
            }
            1001 => {
                let p = r.u32()?;
                let n = r.u32()?;
                (n, p)
            }
            _ => return r.fail(format!("unsupported SPR2 version {version}")),
        };
        let count = count as usize;
        r.count(count, 4)?;
        let mut frames = Vec::with_capacity(count);
        if version == 1000 {
            let mut offsets = Vec::with_capacity(count + 1);
            for _ in 0..count {
                offsets.push(r.u32()? as usize);
            }
            offsets.push(data.len());
            for pair in offsets.windows(2) {
                if pair[0] < r.pos
                    || pair[0] > pair[1]
                    || pair[1] > data.len()
                    || pair[1] - pair[0] < 16
                {
                    return r.fail("invalid SPR2 frame offset/size");
                }
                frames.push(pair[0]..pair[1]);
            }
        } else {
            for _ in 0..count {
                let v = r.u32()?;
                if v != 1001 && v != 1000 {
                    return r.fail(format!("unsupported SPR2 frame version {v}"));
                }
                let n = r.u32()? as usize;
                let start = r.pos;
                r.skip(n)?;
                frames.push(start..r.pos);
            }
        }
        Ok(Self {
            version,
            default_palette,
            frames,
        })
    }
    pub fn decode(&self, data: &[u8], index: usize, iff: &Iff) -> Result<Frame> {
        let range = self
            .frames
            .get(index)
            .ok_or_else(|| Error::new(0, format!("SPR2 frame {index} does not exist")))?;
        let frame = data
            .get(range.clone())
            .ok_or_else(|| Error::new(range.start, "frame outside SPR2"))?;
        let mut r = Reader::new(frame);
        r.skip(8)?;
        let stored = r.u16()?;
        let palette = if self.version == 1000 || stored == 0 || stored == 0xA3A3 {
            self.default_palette
        } else {
            u32::from(stored)
        };
        let palette_id =
            u16::try_from(palette).map_err(|_| Error::new(8, "palette ID exceeds 16 bits"))?;
        let chunk = iff
            .get(b"PALT", palette_id)
            .ok_or_else(|| Error::new(8, format!("missing PALT #{palette_id}")))?;
        Frame::decode(frame, &Palette::parse(iff.data(chunk))?)
    }
}
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub flags: u32,
    pub rgba: Vec<u8>,
    pub depth: Vec<u8>,
}
impl Frame {
    pub fn decode(data: &[u8], palette: &Palette) -> Result<Self> {
        let mut r = Reader::new(data);
        let width = r.u16()? as usize;
        let height = r.u16()? as usize;
        let flags = r.u32()?;
        r.u16()?;
        r.u16()?;
        let y = i32::from(r.i16()?);
        let x = i32::from(r.i16()?);
        let pixels = width
            .checked_mul(height)
            .ok_or_else(|| Error::new(0, "sprite dimensions overflow"))?;
        if pixels > MAX_PIXELS {
            return r.fail("sprite exceeds 16 megapixel decode limit");
        }
        let mut rgba = vec![0; pixels * 4];
        let mut depth = vec![255; pixels];
        let mut row = 0;
        loop {
            // Some shipped frames end immediately after their final complete row.
            if r.remaining() == 0 && row == height {
                break;
            }
            let marker = r.u16()?;
            let command = marker >> 13;
            let count = (marker & 0x1FFF) as usize;
            match command {
                5 => break,
                4 => {
                    if count == 0 || count > height.saturating_sub(row) {
                        return r.fail("transparent row run outside sprite");
                    }
                    row += count;
                }
                0 => {
                    if row >= height || count < 2 {
                        return r.fail("pixel row outside sprite or invalid row size");
                    }
                    let row_data = r.take(count - 2)?;
                    let mut line = Reader::new(row_data);
                    let mut col = 0;
                    while line.remaining() > 0 {
                        let run = line.u16()?;
                        let op = run >> 13;
                        let n = (run & 0x1FFF) as usize;
                        if n > width - col {
                            return r.fail("pixel run exceeds row width");
                        }
                        if !matches!(op, 1 | 2 | 3 | 6) {
                            return r.fail(format!("unknown SPR2 pixel command {op}"));
                        }
                        if op == 3 {
                            col += n;
                            continue;
                        }
                        for _ in 0..n {
                            let z = if op == 6 { 0 } else { line.u8()? };
                            let color = line.u8()? as usize;
                            let alpha = if op == 2 {
                                let a = line.u8()?;
                                if a > 31 {
                                    return r.fail("SPR2 alpha exceeds 5-bit range");
                                }
                                ((u16::from(a) * 255) / 31) as u8
                            } else {
                                255
                            };
                            let rgb = palette.0.get(color).ok_or_else(|| {
                                Error::new(r.pos, format!("palette index {color} out of range"))
                            })?;
                            let i = row * width + col;
                            rgba[i * 4..i * 4 + 3].copy_from_slice(rgb);
                            rgba[i * 4 + 3] = alpha;
                            if flags & 2 != 0 {
                                depth[i] = z;
                            }
                            col += 1;
                        }
                        if matches!(op, 2 | 6) && !n.is_multiple_of(2) {
                            line.skip(1)?;
                        }
                    }
                    row += 1;
                }
                _ => return r.fail(format!("unknown SPR2 row command {command}")),
            }
        }
        Ok(Self {
            width: width as u32,
            height: height as u32,
            x,
            y,
            flags,
            rgba,
            depth,
        })
    }
}
#[derive(Debug, Clone)]
pub struct DrawSprite {
    pub id: u32,
    pub frame: u32,
    pub x: i32,
    pub y: i32,
    pub flags: u32,
    pub object: [f32; 3],
}
#[derive(Debug, Clone)]
pub struct DrawView {
    pub direction: u32,
    pub zoom: u32,
    pub sprites: Vec<DrawSprite>,
}
#[derive(Debug, Clone)]
pub struct DrawGroup {
    pub version: u16,
    pub views: Vec<DrawView>,
}
impl DrawGroup {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let version = r.u16()?;
        if !(20000..=20004).contains(&version) {
            return r.fail(format!("unsupported DGRP version {version}"));
        }
        let count = if version < 20003 {
            r.u16()? as usize
        } else {
            r.u32()? as usize
        };
        r.count(count, 4)?;
        let mut views = Vec::with_capacity(count);
        for _ in 0..count {
            let (n, direction, zoom) = if version < 20003 {
                let n = r.u16()? as usize;
                (n, u32::from(r.u8()?), u32::from(r.u8()?))
            } else {
                let d = r.u32()?;
                let z = r.u32()?;
                (r.u32()? as usize, d, z)
            };
            r.count(n, 12)?;
            let mut sprites = Vec::with_capacity(n);
            for _ in 0..n {
                let (id, frame, x, y, flags, object) = if version < 20003 {
                    r.u16()?;
                    let id = u32::from(r.u16()?);
                    let frame = u32::from(r.u16()?);
                    let flags = u32::from(r.u16()?);
                    let x = i32::from(r.i16()?);
                    let y = i32::from(r.i16()?);
                    let z = if version == 20001 { r.f32()? } else { 0.0 };
                    (id, frame, x, y, flags, [0.0, 0.0, z])
                } else {
                    let id = r.u32()?;
                    let frame = r.u32()?;
                    let x = r.i32()?;
                    let y = r.i32()?;
                    let z = r.f32()?;
                    let flags = r.u32()?;
                    let (ox, oy) = if version == 20004 {
                        (r.f32()?, r.f32()?)
                    } else {
                        (0.0, 0.0)
                    };
                    (id, frame, x, y, flags, [ox, oy, z])
                };
                if object.iter().any(|n| !n.is_finite()) {
                    return r.fail("non-finite DGRP object offset");
                }
                sprites.push(DrawSprite {
                    id,
                    frame,
                    x,
                    y,
                    flags,
                    object,
                });
            }
            views.push(DrawView {
                direction,
                zoom,
                sprites,
            });
        }
        Ok(Self { version, views })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Color,
    Alpha,
    Depth,
}
impl std::fmt::Display for Channel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Color => "Color",
            Self::Alpha => "Alpha",
            Self::Depth => "Depth",
        })
    }
}
#[derive(Debug, Clone)]
pub struct Preview {
    pub width: u32,
    pub height: u32,
    pub origin: [i32; 2],
    pub rgba: Vec<u8>,
    pub depth: Vec<u8>,
    pub notes: Vec<String>,
}
impl Preview {
    pub fn channel(&self, channel: Channel, checker: bool) -> Vec<u8> {
        let mut result = self.rgba.clone();
        for (i, p) in result.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            match channel {
                Channel::Alpha => {
                    let a = p[3];
                    p.copy_from_slice(&[a, a, a, 255]);
                }
                Channel::Depth => {
                    let d = self.depth[i];
                    p.copy_from_slice(&[d, d, d, 255]);
                }
                Channel::Color => {
                    if checker {
                        let x = i % self.width as usize;
                        let y = i / self.width as usize;
                        let bg = if (x / 8 + y / 8).is_multiple_of(2) {
                            38
                        } else {
                            48
                        };
                        let a = u32::from(p[3]);
                        for c in &mut p[..3] {
                            *c = ((u32::from(*c) * a + bg * (255 - a)) / 255) as u8;
                        }
                        p[3] = 255;
                    }
                }
            }
        }
        result
    }
}
pub fn render(iff: &Iff, view: &DrawView) -> Result<Preview> {
    let mut layers = Vec::new();
    let mut notes = Vec::new();
    let mut area = 0usize;
    for sprite in &view.sprites {
        let id =
            u16::try_from(sprite.id).map_err(|_| Error::new(0, "sprite ID exceeds 16 bits"))?;
        let chunk = iff.get(b"SPR2", id).ok_or_else(|| {
            Error::new(
                0,
                format!("missing SPR2 #{id}; SPR# fallback is not implemented"),
            )
        })?;
        let raw = iff.data(chunk);
        let frame = SpriteSet::parse(raw)?.decode(raw, sprite.frame as usize, iff)?;
        area += frame.rgba.len() / 4;
        if area > MAX_PIXELS {
            return Err(Error::new(0, "drawgroup exceeds decode budget"));
        }
        if frame.flags & 2 == 0 {
            notes.push(format!(
                "SPR2 #{id} frame {} has no depth channel",
                sprite.frame
            ));
        }
        if sprite.object != [0.0; 3] {
            notes.push(format!(
                "object offset {:?}; preview uses screen offsets, not world depth projection",
                sprite.object
            ));
        }
        let x = i64::from(sprite.x) + i64::from(frame.x);
        let y = i64::from(sprite.y) + i64::from(frame.y);
        layers.push((sprite, frame, x, y));
    }
    let min_x = layers.iter().map(|l| l.2).min().unwrap_or(0);
    let min_y = layers.iter().map(|l| l.3).min().unwrap_or(0);
    let max_x = layers
        .iter()
        .map(|l| l.2 + i64::from(l.1.width))
        .max()
        .unwrap_or(1);
    let max_y = layers
        .iter()
        .map(|l| l.3 + i64::from(l.1.height))
        .max()
        .unwrap_or(1);
    let w = max_x - min_x;
    let h = max_y - min_y;
    if w <= 0 || h <= 0 || w > 4096 || h > 4096 || w * h > MAX_PIXELS as i64 {
        return Err(Error::new(0, "drawgroup canvas exceeds preview limits"));
    }
    let width = w as usize;
    let height = h as usize;
    let mut rgba = vec![0u8; width * height * 4];
    let mut depth = vec![255u8; width * height];
    for (sprite, frame, x, y) in layers {
        for fy in 0..frame.height as usize {
            for fx in 0..frame.width as usize {
                let sx = if sprite.flags & 1 != 0 {
                    frame.width as usize - 1 - fx
                } else {
                    fx
                };
                let source = fy * frame.width as usize + sx;
                let a = u32::from(frame.rgba[source * 4 + 3]);
                if a == 0 {
                    continue;
                }
                let dst = (y - min_y) as usize * width + fy * width + (x - min_x) as usize + fx;
                if frame.flags & 2 != 0
                    && rgba[dst * 4 + 3] != 0
                    && frame.depth[source] > depth[dst]
                {
                    continue;
                }
                let da = u32::from(rgba[dst * 4 + 3]);
                let alpha = a + da * (255 - a) / 255;
                for c in 0..3 {
                    rgba[dst * 4 + c] = ((u32::from(frame.rgba[source * 4 + c]) * a
                        + u32::from(rgba[dst * 4 + c]) * da * (255 - a) / 255)
                        / alpha) as u8;
                }
                rgba[dst * 4 + 3] = alpha as u8;
                depth[dst] = frame.depth[source];
            }
        }
    }
    Ok(Preview {
        width: width as u32,
        height: height as u32,
        origin: [min_x as i32, min_y as i32],
        rgba,
        depth,
        notes,
    })
}

/// SPR# directories and paletted RLE images, including original wall coverings.
///
/// Versions 502 to 505 share one frame encoding: every 503 and 505 chunk in
/// the original install decodes completely with it. What the version number
/// itself selects is unknown. 501 and 506 do not occur and are rejected.
#[derive(Debug, Clone)]
pub struct LegacySpriteSet {
    pub version: u32,
    pub default_palette: u32,
    pub frames: Vec<Range<usize>>,
    big_endian: bool,
}
impl LegacySpriteSet {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let word = r.u32()?;
        let (version, big_endian) = match word {
            502..=505 => (word, false),
            _ if matches!(word.swap_bytes(), 502..=505) => (word.swap_bytes(), true),
            _ => return r.fail("unsupported legacy SPR# version"),
        };
        let read = |r: &mut Reader<'_>| if big_endian { r.be32() } else { r.u32() };
        let count = read(&mut r)? as usize;
        let default_palette = read(&mut r)?;
        if count > 4096 {
            return r.fail("legacy sprite frame count exceeds limit");
        }
        r.count(count, 4)?;
        let mut offsets = Vec::with_capacity(count + 1);
        for _ in 0..count {
            offsets.push(read(&mut r)? as usize);
        }
        offsets.push(data.len());
        let mut frames = Vec::with_capacity(count);
        for pair in offsets.windows(2) {
            if pair[0] < r.pos
                || pair[0] >= pair[1]
                || pair[1] > data.len()
                || pair[1] - pair[0] < 8
            {
                return r.fail("invalid legacy sprite frame range");
            }
            frames.push(pair[0]..pair[1]);
        }
        Ok(Self {
            version,
            default_palette,
            frames,
            big_endian,
        })
    }
    /// The PALT chunk this set names, or None when the drawing code supplies
    /// the palette (wall masks store 0xFFFFFFFF).
    pub fn palette_id(&self) -> Option<u16> {
        u16::try_from(self.default_palette).ok()
    }
    /// Decodes a frame with the palette the set names. Sets without one
    /// must use `decode_with_palette`.
    pub fn decode(&self, data: &[u8], index: usize, iff: &Iff) -> Result<Frame> {
        let id = self
            .palette_id()
            .ok_or_else(|| Error::new(8, "SPR# names no palette; the caller supplies one"))?;
        let c = iff
            .get(b"PALT", id)
            .ok_or_else(|| Error::new(8, format!("missing PALT #{id}")))?;
        self.decode_with_palette(data, index, &Palette::parse(iff.data(c))?)
    }
    pub fn decode_with_palette(
        &self,
        data: &[u8],
        index: usize,
        palette: &Palette,
    ) -> Result<Frame> {
        let range = self
            .frames
            .get(index)
            .ok_or_else(|| Error::new(0, format!("SPR# frame {index} does not exist")))?;
        let frame = data
            .get(range.clone())
            .ok_or_else(|| Error::new(range.start, "frame outside SPR#"))?;
        crate::legacy_sprite::decode_frame(frame, self.big_endian, palette)
    }
}

#[cfg(test)]
mod legacy_tests {
    use super::*;
    #[test]
    fn legacy_directory_preserves_big_endian_frame_ranges_and_checks_bounds() {
        let mut bytes = Vec::new();
        for n in [502u32, 2, 9, 20, 28] {
            bytes.extend_from_slice(&n.to_be_bytes());
        }
        bytes.resize(36, 0);
        let set = LegacySpriteSet::parse(&bytes).unwrap();
        assert_eq!(set.default_palette, 9);
        assert_eq!(set.frames, vec![20..28, 28..36]);
        for (offset, value) in [(12, 16u32), (16, 19), (16, 35), (16, 37)] {
            let mut bad = bytes.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
            assert!(LegacySpriteSet::parse(&bad).is_err());
        }
        for length in 0..20 {
            assert!(LegacySpriteSet::parse(&bytes[..length]).is_err());
        }
    }

    #[test]
    fn versions_502_to_505_in_either_byte_order_share_the_directory() {
        for version in 501u32..=506 {
            for big_endian in [false, true] {
                let mut bytes = Vec::new();
                for n in [version, 1, 0xFFFF_FFFF, 16] {
                    let word = if big_endian {
                        n.to_be_bytes()
                    } else {
                        n.to_le_bytes()
                    };
                    bytes.extend_from_slice(&word);
                }
                bytes.resize(24, 0);
                let set = LegacySpriteSet::parse(&bytes);
                assert_eq!(set.is_ok(), (502..=505).contains(&version), "{version}");
                if let Ok(set) = set {
                    assert_eq!(set.version, version);
                    assert_eq!(set.palette_id(), None);
                }
            }
        }
    }
}
