//! Text record layout from 59e746, 594d73/5947e2 and 59a629. The Windows
//! text stream removes CRLF; names are full nonempty lines, numeric fields are
//! whitespace separated. Unsupported property bags/animation sections fail
//! explicitly, as they do in the binary appearance loader.
use super::{Appearance, Binding, Blend, Catalog, Mesh, MeshBinding, Vertex, validate_mesh};
use crate::{Error, Result};

struct Text<'a> {
    data: &'a [u8],
    pos: usize,
}

/// Ordinary CMX record stream. Skeleton/animation text records are an explicit
/// support boundary; all original installed CMX catalogs currently encountered
/// contain appearance records. Counts and trailing data follow the controller,
/// while version/property support limits remain explicit decoding errors.
pub struct Cmx<'a> {
    reader: Text<'a>,
    group: usize,
    remaining: i32,
    failed: bool,
}
impl<'a> Cmx<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self> {
        let mut reader = Text::new(data)?;
        if !reader.line(256)?.starts_with(b"//") || reader.line(256)? != b"version 300" {
            return reader.fail("unsupported CMX header/version");
        }
        Ok(Self {
            reader,
            group: 0,
            remaining: 0,
            failed: false,
        })
    }
    pub fn count(&mut self, kind: super::index::Kind) -> Result<Option<i32>> {
        if self.failed || self.group != kind.index() || self.remaining > 0 {
            return self.reader.fail("CMX group out of sequence");
        }
        self.failed = true;
        if self.reader.data[self.reader.pos..]
            .iter()
            .all(u8::is_ascii_whitespace)
        {
            self.reader.pos = self.reader.data.len();
            return Ok(None);
        }
        let count = self.reader.integer()?;
        if count > [64, 256, 4096][kind.index()] {
            return self.reader.fail("CMX group exceeds supported limit");
        }
        self.group += 1;
        self.remaining = count;
        self.failed = false;
        Ok(Some(count))
    }
    pub fn read(&mut self, kind: super::index::Kind) -> Result<super::catalog::Asset> {
        if self.failed || self.group != kind.index() + 1 || self.remaining <= 0 {
            return self.reader.fail("CMX record out of sequence");
        }
        self.failed = true;
        if kind != super::index::Kind::Appearance {
            return self
                .reader
                .fail("CMX skeleton/animation records not supported");
        }
        let asset = super::catalog::Asset::Appearance(read_appearance(&mut self.reader)?);
        self.remaining -= 1;
        self.failed = false;
        Ok(asset)
    }
}
impl<'a> Text<'a> {
    fn new(data: &'a [u8]) -> Result<Self> {
        if data.contains(&0) {
            return Err(Error::new(0, "NUL in text avatar"));
        }
        Ok(Self { data, pos: 0 })
    }
    fn fail<T>(&self, message: &str) -> Result<T> {
        Err(Error::new(self.pos, message))
    }
    fn line(&mut self, max: usize) -> Result<&'a [u8]> {
        let rest = &self.data[self.pos..];
        let n = rest
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| Error::new(self.pos, "unterminated avatar line"))?;
        let line = rest[..n].strip_suffix(b"\r").unwrap_or(&rest[..n]);
        if line.len() >= max {
            return self.fail("avatar line too long");
        }
        self.pos += n + 1;
        Ok(line)
    }
    fn name(&mut self, max: usize) -> Result<String> {
        loop {
            let line = self.line(max)?;
            if !line.is_empty() {
                return Ok(crate::strings::decode(line, 1));
            }
        }
    }
    fn token(&mut self) -> Result<&'a str> {
        while self.data.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
        let start = self.pos;
        while self
            .data
            .get(self.pos)
            .is_some_and(|b| !b.is_ascii_whitespace())
        {
            self.pos += 1;
        }
        if start == self.pos {
            return self.fail("missing avatar number");
        }
        std::str::from_utf8(&self.data[start..self.pos])
            .map_err(|_| Error::new(start, "non-ASCII avatar number"))
    }
    fn integer(&mut self) -> Result<i32> {
        self.token()?
            .parse()
            .map_err(|_| Error::new(self.pos, "invalid avatar integer"))
    }
    fn count(&mut self, max: usize) -> Result<usize> {
        let n = self.integer()?;
        if n < 0 || n as usize > max {
            return self.fail("invalid avatar count");
        }
        Ok(n as usize)
    }
    fn word(&mut self) -> Result<u32> {
        Ok(self.integer()? as u32)
    }
    fn float(&mut self) -> Result<f32> {
        let v: f32 = self
            .token()?
            .parse()
            .map_err(|_| Error::new(self.pos, "invalid avatar float"))?;
        if !v.is_finite() {
            return self.fail("non-finite avatar float");
        }
        Ok(v)
    }
    fn vector(&mut self) -> Result<[f32; 3]> {
        Ok([self.float()?, self.float()?, self.float()?])
    }
    fn finish(&self) -> Result<()> {
        if self.data[self.pos..].iter().all(u8::is_ascii_whitespace) {
            Ok(())
        } else {
            self.fail("trailing text avatar data")
        }
    }
}

/// Read appearance-only CMX catalogs, including the original expansion outfits.
/// Skeleton and animation text sections remain explicit unsupported formats;
/// their existing BCF counterparts use the binary readers.
pub fn read_cmx(data: &[u8]) -> Result<Catalog> {
    let mut r = Text::new(data)?;
    // 59e746 consumes exactly two header lines. Restrict supported versions
    // instead of interpreting an unknown layout as version 300.
    if !r.line(256)?.starts_with(b"//") || r.line(256)? != b"version 300" {
        return r.fail("unsupported CMX header/version");
    }
    if r.integer()? != 0 {
        return r.fail("CMX skeleton sections not supported");
    }
    let mut appearances = Vec::new();
    for _ in 0..r.count(256)? {
        appearances.push(read_appearance(&mut r)?);
    }
    if r.integer()? != 0 {
        return r.fail("CMX animation sections not supported");
    }
    r.finish()?;
    Ok(Catalog {
        skeletons: vec![],
        appearances,
    })
}

fn read_appearance(r: &mut Text<'_>) -> Result<Appearance> {
    // The native appearance reader lowercases this name, unlike its mesh
    // and bone names. Retain the remaining source spelling and coordinates.
    let name = r.name(256)?.to_ascii_lowercase();
    let kind = r.word()?;
    if r.integer()? != 0 {
        return r.fail("CMX appearance properties not supported");
    }
    let mut bindings = Vec::new();
    for _ in 0..r.count(256)? {
        let bone = r.name(1024)?;
        let mesh = r.name(1024)?;
        let censor = r.word()?;
        if r.integer()? != 0 {
            return r.fail("CMX binding properties not supported");
        }
        bindings.push(Binding { bone, mesh, censor });
    }
    Ok(Appearance {
        name,
        kind,
        bindings,
    })
}

pub fn read_skn(data: &[u8]) -> Result<Mesh> {
    let mut r = Text::new(data)?;
    let name = r.name(1024)?;
    let texture = r.name(1024)?;
    let mut bones = Vec::new();
    for _ in 0..r.count(256)? {
        let bone = r.name(1024)?;
        // 599dfa deduplicates the table. Reject malformed indexed duplicates
        // until their dependent-index behavior is represented explicitly.
        if bones.contains(&bone) {
            return r.fail("duplicate SKN bone name");
        }
        bones.push(bone);
    }
    let mut triangles = Vec::new();
    for _ in 0..r.count(100_000)? {
        triangles.push([r.word()?, r.word()?, r.word()?]);
    }
    let mut bindings = Vec::new();
    for _ in 0..r.count(1024)? {
        bindings.push(MeshBinding {
            bone: r.word()? as usize,
            first_vertex: r.word()? as usize,
            vertex_count: r.word()? as usize,
            first_blend: r.word()? as usize,
            blend_count: r.word()? as usize,
        });
    }
    let mut vertices = Vec::new();
    for _ in 0..r.count(100_000)? {
        vertices.push(Vertex {
            position: [0.; 3],
            normal: [0.; 3],
            uv: [r.float()?, r.float()?],
            bone: usize::MAX,
        });
    }
    let mut blends = Vec::new();
    for _ in 0..r.count(100_000)? {
        // SKN writes index before fixed-point weight; BMF stores weight first.
        let vertex = r.word()? as usize;
        let weight = r.word()?;
        if weight > 32768 {
            return r.fail("invalid blend weight");
        }
        blends.push(Blend {
            vertex,
            weight: weight as f32 / 32768.,
            position: [0.; 3],
            normal: [0.; 3],
            bone: usize::MAX,
        });
    }
    if r.count(200_000)? != vertices.len() + blends.len() {
        return r.fail("mesh sample count disagrees with vertex counts");
    }
    for v in &mut vertices {
        v.position = r.vector()?;
        v.normal = r.vector()?;
    }
    for v in &mut blends {
        v.position = r.vector()?;
        v.normal = r.vector()?;
    }
    r.finish()?;
    let mut mesh = Mesh {
        name,
        texture,
        bones,
        bindings,
        triangles,
        vertices,
        blends,
    };
    validate_mesh(&mut mesh, r.pos)?;
    Ok(mesh)
}
