//! Shipped FNTF101 bitmap fonts. Gimex60019bd1 reads the header/table offsets;
//!6001a36b reads11-byte glyphs;6001b5be expands high then low coverage nibbles.
//! Coverage indices and signed bearings remain data, without a display/GPU.
use crate::{Error, Result, reader::Reader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glyph {
    pub code: u16,
    pub width: u8,
    pub height: u8,
    pub atlas_x: u16,
    pub atlas_y: u16,
    pub first_bearing: i8,
    pub bearing_x: i8,
    pub bearing_y: i8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Font {
    pub flags: u32,
    pub glyphs: Vec<Glyph>,
    pub width: u16,
    pub height: u16,
    /// Top-down, one byte per pixel; values0..15, before palette/color mapping.
    pub coverage: Vec<u8>,
}
impl Font {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        if r.take(4)? != b"FNTF" {
            return r.fail("not an FNTF bitmap font");
        }
        if r.u32()? as usize != bytes.len() {
            return r.fail("FNTF length disagrees with file");
        }
        if r.u16()? != 101 {
            return r.fail("unsupported FNTF version");
        }
        let count = r.u16()? as usize;
        let flags = r.u32()?;
        r.skip(4)?;
        let table = r.u32()? as usize;
        r.skip(4)?;
        let atlas = r.u32()? as usize;
        if table < 32 || table > atlas || count > (atlas - table) / 11 || atlas > bytes.len() {
            return r.fail("FNTF glyph table outside file");
        }
        r.pos = table;
        let mut glyphs = Vec::with_capacity(count);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..count {
            let code = r.u16()?;
            if !seen.insert(code) {
                return r.fail("duplicate FNTF glyph code");
            }
            glyphs.push(Glyph {
                code,
                width: r.u8()?,
                height: r.u8()?,
                atlas_x: r.u16()?,
                atlas_y: r.u16()?,
                first_bearing: r.u8()? as i8,
                bearing_x: r.u8()? as i8,
                bearing_y: r.u8()? as i8,
            });
        }
        r.pos = atlas;
        if r.u32()? != 0x7a {
            return r.fail("unsupported FNTF atlas encoding");
        }
        let width = r.u16()?;
        let height = r.u16()?;
        r.skip(8)?;
        if width == 0 || height == 0 {
            return r.fail("empty FNTF atlas");
        }
        for glyph in &glyphs {
            if u32::from(glyph.atlas_x) + u32::from(glyph.width) > u32::from(width)
                || u32::from(glyph.atlas_y) + u32::from(glyph.height) > u32::from(height)
            {
                return Err(Error::new(table, "FNTF glyph outside atlas"));
            }
        }
        let stride = usize::from(width).div_ceil(2);
        let packed = r.take(stride * usize::from(height))?;
        if r.remaining() != 0 {
            return r.fail("trailing FNTF atlas bytes");
        }
        let mut coverage = Vec::with_capacity(usize::from(width) * usize::from(height));
        for row in packed.chunks_exact(stride) {
            coverage.extend(
                (0..usize::from(width))
                    .map(|x| (row[x / 2] >> if x % 2 == 0 { 4 } else { 0 }) & 15),
            );
        }
        Ok(Self {
            flags,
            glyphs,
            width,
            height,
            coverage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Vec<u8> {
        let mut b = vec![0; 32];
        b[..4].copy_from_slice(b"FNTF");
        b[8..10].copy_from_slice(&101u16.to_le_bytes());
        b[10..12].copy_from_slice(&2u16.to_le_bytes());
        b[12..16].copy_from_slice(&9u32.to_le_bytes());
        b[20..24].copy_from_slice(&32u32.to_le_bytes());
        b[28..32].copy_from_slice(&54u32.to_le_bytes());
        for code in [65u16, 66] {
            b.extend(code.to_le_bytes());
            b.extend([1, 2, 0, 0, 0, 0, 0, 1, 255]);
        }
        b.extend(0x7au32.to_le_bytes());
        b.extend(3u16.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend([0; 8]);
        b.extend([0x1f, 0xa0, 0x62, 0xb0]);
        let len = b.len() as u32;
        b[4..8].copy_from_slice(&len.to_le_bytes());
        b
    }
    #[test]
    fn coverage_rows_and_signed_bearings() {
        let f = Font::parse(&sample()).unwrap();
        assert_eq!((f.width, f.height), (3, 2));
        assert_eq!(f.coverage, [1, 15, 10, 6, 2, 11]);
        assert_eq!(f.glyphs[0].bearing_y, -1);
    }
    #[test]
    fn malformed_tables_and_atlas_are_rejected() {
        let b = sample();
        for end in 0..b.len() {
            assert!(Font::parse(&b[..end]).is_err());
        }
        let mut duplicate = b.clone();
        duplicate[43..45].copy_from_slice(&65u16.to_le_bytes());
        assert!(Font::parse(&duplicate).is_err());
        let mut outside = b.clone();
        outside[36..38].copy_from_slice(&3u16.to_le_bytes());
        assert!(Font::parse(&outside).is_err());
        let mut version = b;
        version[8] = 102;
        assert!(Font::parse(&version).is_err());
    }
}
