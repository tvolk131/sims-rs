//! Original49dacd clipped columns,49d79d particles,49da21 colors and49d8fb blades.
//! Tables/buffers are explicit retained inputs; one ordered store per advance.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    pub left: i32,
    pub bottom: i32,
    pub corners: [u8; 4],
    pub zoom: u32,
    pub clip: [i16; 4],
    pub clipped: bool,
    pub rectangle: [i32; 4],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticleColumn {
    pub begin: u32,
    pub records: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Data {
    pub colors: usize,
    pub palette: Vec<Vec<u8>>,
    pub particles: Vec<ParticleColumn>,
    pub color: Vec<u8>,
    pub depth: Vec<u8>,
    pub color_pitch: i32,
    pub depth_pitch: i32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Owner(usize),
    Particle(usize, usize),
    Color(usize),
    Depth(usize),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    pub target: Target,
    pub width: u8,
    pub value: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub index: usize,
    pub x: i32,
    pub y: i32,
    pub height: i32,
    pub texture: Option<[i32; 2]>,
    pub shade: Option<[i32; 2]>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Prepare,
    Preparing,
    Prepared,
    Color,
    Coloring,
    Draw,
    Drawing,
    Next,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub spec: Spec,
    pub columns: Vec<Column>,
    column: usize,
    helpers: bool,
    stage: Stage,
    queue: Vec<Write>,
    next: usize,
    last: Option<Write>,
}
fn word(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn int(b: &[u8], i: usize) -> i32 {
    word(b, i) as i32
}
fn div(a: i32, b: i32) -> Result<i32, String> {
    a.checked_div(b)
        .ok_or_else(|| "terrain raster native division fault".into())
}
fn add(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}
fn sub(a: i32, b: i32) -> i32 {
    a.wrapping_sub(b)
}
fn mul(a: i32, b: i32) -> i32 {
    a.wrapping_mul(b)
}
impl Data {
    pub fn validate(&self, owner: &[u8]) -> Result<(), String> {
        if owner.len() < 0x388
            || self.colors == 0
            || self.palette.len() != word(owner, 0x7c) as usize
            || self.colors != word(owner, 0x4c) as usize
            || self.palette.iter().any(|p| p.len() != self.colors * 16)
            || self.particles.len() > 32
            || self.color_pitch <= 0
            || self.depth_pitch <= 0
        {
            return Err("terrain raster data shape differs".into());
        }
        if word(owner, 0x37c) != 0 {
            return Err("terrain alternate-mode palette is unbound".into());
        }
        for (i, p) in self.particles.iter().enumerate() {
            if p.records.len() % 40 != 0
                || word(owner, 0xe0 + i * 20) != p.begin
                || word(owner, 0xe4 + i * 20) != p.begin.wrapping_add(p.records.len() as u32)
            {
                return Err("terrain raster particle span differs".into());
            }
        }
        Ok(())
    }
    pub fn apply(&mut self, w: &Write, owner: &mut [u8]) -> Result<(), String> {
        let (b, i) = match w.target {
            Target::Owner(i) => (owner, i),
            Target::Particle(c, i) => (
                self.particles
                    .get_mut(c)
                    .ok_or("raster particle column absent")?
                    .records
                    .as_mut_slice(),
                i,
            ),
            Target::Color(i) => (self.color.as_mut_slice(), i),
            Target::Depth(i) => (self.depth.as_mut_slice(), i),
        };
        let n = usize::from(w.width);
        if ![1, 2, 4].contains(&n) || i.checked_add(n).is_none_or(|z| z > b.len()) {
            return Err("terrain raster store outside retained span".into());
        }
        b[i..i + n].copy_from_slice(&w.value.to_le_bytes()[..n]);
        Ok(())
    }
}
fn gradients(a: i32, b: i32, n: i32) -> Result<Option<[i32; 2]>, String> {
    if n == 0 {
        Ok(None)
    } else {
        Ok(Some([a, div(sub(b, a), n)?]))
    }
}
fn columns(s: &Spec, owner: &[u8]) -> Result<Vec<Column>, String> {
    if !(1..=3).contains(&s.zoom) {
        return Err("terrain raster zoom differs".into());
    }
    let h = s.corners.map(i32::from);
    let t: [i32; 4] = std::array::from_fn(|i| i32::from(owner[0xc + i]));
    let a: [i32; 4] = std::array::from_fn(|i| i32::from(owner[0x360 + i]));
    let width = 16i32 << s.zoom;
    let half = width / 2;
    let shift = s.zoom + 1;
    let c = s.clip.map(i32::from);
    let end = if s.clipped && add(s.left, width) > c[3] {
        sub(c[3], s.left) & 0xfffc
    } else {
        width
    };
    let start = if s.clipped && s.left < c[1] {
        add(sub(c[1], s.left), 3) & 0xfffc
    } else {
        0
    };
    if start < 0 || end > width || end < 0 || start > width {
        return Ok(vec![]);
    }
    let slope = add(sub(h[1], h[3]), 4);
    let dy = add(sub(h[1], h[0]), 2);
    let mut height = add(sub(mul(start / 4, slope), h[3]), add(2, h[0]));
    let mut y = add(sub(mul(start / -4, dy), h[0] << shift), s.bottom);
    let d = |x: i32, z: i32| mul(sub(x, z), 256) >> shift;
    let tu = d(t[1], t[0]);
    let tl = d(t[3], t[0]);
    let au = d(a[1], a[0]);
    let al = d(a[3], a[0]);
    let mut tv = [
        add(mul(t[0], 256), mul(tu, start / 4)),
        add(mul(t[0], 256), mul(tl, start / 4)),
    ];
    let mut av = [
        add(mul(a[0], 256), mul(au, start / 4)),
        add(mul(a[0], 256), mul(al, start / 4)),
    ];
    let mut out = vec![];
    let mut oldt = None;
    let mut olda = None;
    let mut x = start;
    while x < half.min(end) {
        if height != 0 {
            oldt = gradients(tv[0], tv[1], height)?;
            olda = gradients(av[0], av[1], height)?;
        }
        out.push(Column {
            index: (x / 4) as usize,
            x: add(s.left, x),
            y,
            height,
            texture: oldt,
            shade: olda,
        });
        x = add(x, 4);
        y = sub(y, dy);
        height = add(height, slope);
        tv = [add(tv[0], tu), add(tv[1], tl)];
        av = [add(av[0], au), add(av[1], al)];
    }
    let skip = sub(width, end) / 4;
    let dy = add(sub(h[1], h[2]), 2);
    let mut height = add(sub(mul(skip, slope), h[3]), add(2, h[2]));
    let mut y = sub(sub(s.bottom, mul(skip, dy)), h[2] << shift);
    let tu = d(t[1], t[2]);
    let tl = d(t[3], t[2]);
    let au = d(a[1], a[2]);
    let al = d(a[3], a[2]);
    let mut tv = [
        add(mul(t[2], 256), mul(tu, skip)),
        add(mul(t[2], 256), mul(tl, skip)),
    ];
    let mut av = [
        add(mul(a[2], 256), mul(au, skip)),
        add(mul(a[2], 256), mul(al, skip)),
    ];
    let mut x = sub(end, 4);
    while x >= start.max(half) {
        tv = [add(tv[0], tu), add(tv[1], tl)];
        av = [add(av[0], au), add(av[1], al)];
        if height != 0 {
            oldt = gradients(tv[0], tv[1], height)?;
            olda = gradients(av[0], av[1], height)?;
        }
        out.push(Column {
            index: (x / 4) as usize,
            x: add(s.left, x),
            y,
            height,
            texture: oldt,
            shade: olda,
        });
        x = sub(x, 4);
        y = sub(y, dy);
        height = add(height, slope);
    }
    Ok(out)
}
fn select(p: &ParticleColumn, threshold: i32, start: usize) -> usize {
    let count = p.records.len() / 40;
    let mut step = count / 2;
    let mut at = start;
    while step != 0 {
        let i = step + at;
        if i < count && int(&p.records, i * 40 + 4) < threshold {
            at = i;
        } else {
            step >>= 1;
        }
    }
    at
}
fn push(q: &mut Vec<Write>, target: Target, width: u8, value: u32) {
    q.push(Write {
        target,
        width,
        value,
    });
}
fn pixel_offset(x: i32, y: i32, pitch: i32, n: usize, len: usize) -> Result<usize, String> {
    let i = add(mul(y, pitch), mul(x, 2));
    let i = usize::try_from(i).map_err(|_| "negative terrain raster pixel address")?;
    if i.checked_add(n).is_none_or(|z| z > len) {
        return Err("terrain raster pixel outside retained plane".into());
    }
    Ok(i)
}
impl Frame {
    pub fn new(spec: Spec, owner: &[u8]) -> Result<Self, String> {
        if owner.len() < 0x388 {
            return Err("terrain raster owner is short".into());
        }
        let columns = columns(&spec, owner)?;
        let helpers = spec.clipped || word(owner, 0x370) != 0;
        Ok(Self {
            spec,
            columns,
            column: 0,
            helpers,
            stage: Stage::Entry,
            queue: vec![],
            next: 0,
            last: None,
        })
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn last_store(&self) -> Option<&Write> {
        self.last.as_ref()
    }
    pub fn current_column(&self) -> Option<&Column> {
        self.columns.get(self.column)
    }
    pub fn validate(&self, d: &Data, owner: &[u8]) -> Result<(), String> {
        d.validate(owner)?;
        if self.column > self.columns.len()
            || self.next > self.queue.len()
            || self.columns.iter().any(|c| c.index >= d.particles.len())
        {
            return Err("terrain raster cursor differs".into());
        }
        Ok(())
    }
    fn prepare(&self, d: &Data, owner: &[u8]) -> Result<Vec<Write>, String> {
        let c = &self.columns[self.column];
        let desc = 0xe0 + c.index * 20;
        let p = &d.particles[c.index];
        let mut q = vec![];
        if c.height < 1 || word(owner, 0x370) == 0 {
            push(&mut q, Target::Owner(desc + 16), 4, word(owner, desc + 4));
            push(&mut q, Target::Owner(desc + 12), 4, word(owner, desc + 4));
            return Ok(q);
        }
        let lo = select(p, int(owner, 0x374), 0);
        let hi = select(p, add(int(owner, 0x374), int(owner, 0x370)), lo);
        push(
            &mut q,
            Target::Owner(desc + 12),
            4,
            p.begin.wrapping_add((lo * 40) as u32),
        );
        push(
            &mut q,
            Target::Owner(desc + 16),
            4,
            p.begin.wrapping_add((hi * 40) as u32),
        );
        let t = c.texture.ok_or("particle texture gradient absent")?;
        let s = c.shade.ok_or("particle shade gradient absent")?;
        for i in lo..hi {
            let off = i * 40;
            let r = &p.records;
            let x = add(int(r, off + 12), c.x);
            push(&mut q, Target::Particle(c.index, off + 28), 4, x as u32);
            let dy = (c.height as f32) * f32::from_bits(word(r, off + 16));
            if !dy.is_finite() || dy < i64::MIN as f32 || dy >= 9223372036854775808.0 {
                return Err("particle conversion needs native indefinite owner".into());
            }
            let y = add(dy.trunc() as i64 as i32, c.y);
            push(&mut q, Target::Particle(c.index, off + 24), 4, y as u32);
            let mut h = int(r, off + 8);
            push(&mut q, Target::Particle(c.index, off + 36), 4, h as u32);
            if owner[0x378] != 0 {
                h = sub(h, 1);
                push(&mut q, Target::Particle(c.index, off + 36), 4, h as u32);
                if h == 0 {
                    push(&mut q, Target::Particle(c.index, off + 36), 4, 1);
                }
            }
            let dy = sub(y, c.y);
            let tex =
                add(add(mul(t[1], dy), t[0]) >> 8, int(r, off + 20)).clamp(0, d.colors as i32 - 1);
            let shade = (add(mul(s[1], dy), s[0]) >> 8).clamp(0, d.palette.len() as i32 - 1);
            let color = u16::from_le_bytes(
                d.palette[shade as usize][tex as usize * 16..tex as usize * 16 + 2]
                    .try_into()
                    .unwrap(),
            );
            push(
                &mut q,
                Target::Particle(c.index, off + 32),
                2,
                u32::from(color),
            );
        }
        Ok(q)
    }
    fn color(&self, d: &Data, owner: &[u8]) -> Result<Vec<Write>, String> {
        let c = &self.columns[self.column];
        let mut y = c.y;
        let mut count = c.height;
        let Some(mut t) = c.texture else {
            return Ok(vec![]);
        };
        let Some(mut s) = c.shade else {
            return Ok(vec![]);
        };
        let clip = self.spec.clip.map(i32::from);
        if self.spec.clipped {
            if y < clip[0] {
                let n = sub(clip[0], y);
                t[0] = add(t[0], mul(t[1], n));
                s[0] = add(s[0], mul(s[1], n));
                count = sub(count, n);
                y = clip[0];
            }
            if add(y, count) > clip[2] {
                count = sub(clip[2], y);
            }
        }
        let mut parity = add(y, count) & 1;
        let mut q = vec![];
        for yy in 0..count.max(0) {
            let ti = t[0] >> 8;
            let si = s[0] >> 8;
            if ti < 0 || ti as usize >= d.colors || si < 0 || si as usize >= d.palette.len() {
                return Err("column palette sample outside retained rows".into());
            }
            let tex = &d.palette[si as usize];
            let src = ti as usize * 16 + parity as usize * 8;
            parity ^= 1;
            t[0] = add(t[0], t[1]);
            s[0] = add(s[0], s[1]);
            let a = pixel_offset(c.x, add(y, yy), d.color_pitch, 8, d.color.len())?;
            let z = pixel_offset(c.x, add(y, yy), d.depth_pitch, 8, d.depth.len())?;
            push(&mut q, Target::Color(a), 4, word(tex, src));
            push(&mut q, Target::Color(a + 4), 4, word(tex, src + 4));
            push(&mut q, Target::Depth(z), 4, word(owner, 0x18));
            push(&mut q, Target::Depth(z + 4), 4, word(owner, 0x1c));
        }
        Ok(q)
    }
    fn draw(&self, d: &Data, owner: &[u8]) -> Result<Vec<Write>, String> {
        if word(owner, 0x37c) != 0 {
            return Ok(vec![]);
        }
        let c = &self.columns[self.column];
        let p = &d.particles[c.index];
        let desc = 0xe0 + c.index * 20;
        let lo = word(owner, desc + 12).wrapping_sub(p.begin) as usize;
        let hi = word(owner, desc + 16).wrapping_sub(p.begin) as usize;
        if lo > hi || hi > p.records.len() || lo % 40 != 0 || hi % 40 != 0 {
            return Err("active particle range differs".into());
        }
        let clip = self.spec.clip.map(i32::from);
        let mut q = vec![];
        for off in (lo..hi).step_by(40) {
            let r = &p.records;
            let x = int(r, off + 28);
            let mut base = int(r, off + 24);
            let h = int(r, off + 36);
            let mut top = add(sub(base, h), 1);
            if self.spec.clipped {
                if x < 0 || x >= clip[3] || base < clip[0] || top >= clip[2] {
                    continue;
                }
                top = top.max(clip[0]);
                if base >= clip[2] {
                    base = sub(clip[2], 1);
                }
            }
            if base < top {
                continue;
            }
            let color = u16::from_le_bytes(r[off + 32..off + 34].try_into().unwrap());
            let depth = u16::from_le_bytes(owner[0x18..0x1a].try_into().unwrap());
            for y in (top..=base).rev() {
                let a = pixel_offset(x, y, d.color_pitch, 2, d.color.len())?;
                let z = pixel_offset(x, y, d.depth_pitch, 2, d.depth.len())?;
                push(&mut q, Target::Color(a), 2, u32::from(color));
                push(&mut q, Target::Depth(z), 2, u32::from(depth));
            }
        }
        Ok(q)
    }
    pub fn advance(&mut self, d: &mut Data, owner: &mut [u8]) -> Result<bool, String> {
        self.validate(d, owner)?;
        self.last = None;
        if self.complete() {
            return Ok(false);
        }
        match self.stage {
            Stage::Entry => {
                self.stage = if self.columns.is_empty() {
                    Stage::Complete
                } else {
                    if self.helpers {
                        Stage::Prepare
                    } else {
                        Stage::Color
                    }
                }
            }
            Stage::Prepare => {
                self.queue = self.prepare(d, owner)?;
                self.next = 0;
                self.stage = Stage::Preparing;
            }
            Stage::Prepared => self.stage = Stage::Color,
            Stage::Color => {
                self.queue = self.color(d, owner)?;
                self.next = 0;
                self.stage = Stage::Coloring;
            }
            Stage::Draw => {
                self.queue = self.draw(d, owner)?;
                self.next = 0;
                self.stage = Stage::Drawing;
            }
            Stage::Preparing | Stage::Coloring | Stage::Drawing => {
                if let Some(w) = self.queue.get(self.next) {
                    d.apply(w, owner)?;
                    self.last = Some(w.clone());
                    self.next += 1;
                } else {
                    self.queue.clear();
                    self.next = 0;
                    self.stage = match self.stage {
                        Stage::Preparing => Stage::Prepared,
                        Stage::Coloring => {
                            if self.helpers {
                                Stage::Draw
                            } else {
                                Stage::Next
                            }
                        }
                        Stage::Drawing => Stage::Next,
                        _ => unreachable!(),
                    };
                }
            }
            Stage::Next => {
                self.column += 1;
                self.stage = if self.column == self.columns.len() {
                    Stage::Complete
                } else {
                    if self.helpers {
                        Stage::Prepare
                    } else {
                        Stage::Color
                    }
                };
            }
            Stage::Complete => return Ok(false),
        }
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x49dacd => self.stage == Stage::Entry,
            0x49d79d | 0x49d887 => self.stage == Stage::Prepare,
            0x49d7d1 => self.stage == Stage::Preparing && self.next == 2,
            0x49d880 => self.stage == Stage::Prepared,
            0x49da21 => self.stage == Stage::Color,
            0x49d8fb => self.stage == Stage::Draw,
            0x49df77 => self.complete(),
            _ => false,
        }
    }
}
