//! A small software rasterizer: draws skinned, textured meshes into an
//! image with a depth buffer, with no window and no GPU, so tests can
//! render offscreen. It is also the reference the GPU renderer can be
//! checked against later (the old prototype kept one for the same reason:
//! legacy/crates/sims-render/src/software.rs).

use crate::appearance::{Body, Part};
use crate::math::{Transform, Vec3};
use crate::skin::{Skinned, skin};

/// The direction the light comes from in the offscreen renders: above,
/// a little to the side and behind the viewer of the house view. This is
/// our own light, not the original's; a person on a lot is lit as the
/// original lights them ([`Lighting::standard`]).
pub const LIGHT: Vec3 = Vec3::new(0.3, 1.0, -0.5);

/// How a part is lit: a light from `towards` (a direction towards the
/// light), and per vertex a shade of `ambient + diffuse × max(0, n · l)`,
/// where `n` is the vertex's normal and `l` the unit direction; the
/// original's light holds it to 1, as Direct3D holds a vertex's colour.
/// A texel's colour is multiplied by its shade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighting {
    pub towards: Vec3,
    pub ambient: f32,
    pub diffuse: f32,
    /// Whether the shade is held to 1. Our own light is not, so the
    /// offscreen renders pinned before it existed keep their bits.
    pub held: bool,
}

impl Lighting {
    /// Our own light for offscreen renders: a shade from 0.45 in shadow
    /// to 1 facing the light.
    pub fn ours(towards: Vec3) -> Self {
        Self {
            towards,
            ambient: 0.45,
            diffuse: 0.55,
            held: false,
        }
    }

    /// The original's light for a person at `level` (0.5 to 1; see
    /// [`crate::lot::level`]), with the standard light from `towards`.
    /// The original sets the ambient light to the grey
    /// `round(255 × level)` and the standard light's strength to the level
    /// (0x4b49c7), and Direct3D adds them up per vertex. The material is
    /// taken to be white (decisions entry "A person is lit by the
    /// original's standard light, at their room's level").
    pub fn standard(towards: Vec3, level: f32) -> Self {
        Self {
            towards,
            ambient: (255.0 * level).round() / 255.0,
            diffuse: level,
            held: true,
        }
    }

    /// A vertex's shade, for its unit normal, given `towards` already normalized.
    #[inline]
    pub(crate) fn shade_unit(&self, normal: Vec3, towards: Vec3) -> f32 {
        let shade = self.ambient + self.diffuse * normal.dot(towards).max(0.0);
        if self.held { shade.min(1.0) } else { shade }
    }

    /// A vertex's shade, for its unit normal.
    pub fn shade(&self, normal: Vec3) -> f32 {
        self.shade_unit(normal, self.towards.normalize())
    }

    /// Like [`Lighting::shade_with`] with `towards` already normalized.
    #[inline]
    pub(crate) fn shade_with_unit(&self, normal: Vec3, towards: Vec3, lamps: &[Lamp]) -> Shade {
        if lamps.is_empty() {
            return Shade::Grey(self.shade_unit(normal, towards));
        }
        let base = self.ambient + self.diffuse * normal.dot(towards).max(0.0);
        let mut channels = [base; 3];
        for lamp in lamps {
            let cosine = normal.dot(lamp.towards).max(0.0);
            for (channel, colour) in channels.iter_mut().zip(LAMP_COLOUR) {
                *channel += lamp.strength * colour * cosine;
            }
        }
        Shade::Colour(channels.map(|c| if self.held { c.min(1.0) } else { c }))
    }

    /// A vertex's shade with lamps besides: each adds its strength times
    /// [`LAMP_COLOUR`] times the cosine to the lamp, channel by channel.
    /// Each channel is held to 1, as the original's is, when the shade is
    /// (`held`, as [`Lighting::standard`] sets it); otherwise it may pass
    /// 1, as [`Lighting::shade`] may. With no lamps it is the grey
    /// [`Lighting::shade`], to the bit.
    pub fn shade_with(&self, normal: Vec3, lamps: &[Lamp]) -> Shade {
        self.shade_with_unit(normal, self.towards.normalize(), lamps)
    }
}

/// The colour of a lamp's light on a person: (1, 251/255, 209/255), a warm
/// white (the three floats the original gives each lamp's light, 0x4b4c49
/// to 0x4b4c5d).
pub const LAMP_COLOUR: [f32; 3] = [1.0, 251.0 / 255.0, 209.0 / 255.0];

/// A lamp's light on a person: the unit direction towards the lamp and
/// the light's strength, 0 to 1 (`crate::lot::lamps`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lamp {
    pub towards: Vec3,
    pub strength: f32,
}

/// An orthographic camera. The view turns about the vertical axis by an
/// angle given as its sine and cosine (`yaw`), then tilts down by another
/// (`pitch`); `scale` is pixels per unit, and `center` is the point drawn
/// at the middle of the image. Keeping sines and cosines, not angles,
/// lets the house view be built from square roots alone, which give the
/// same bits on every machine, so offscreen renders can be pinned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub yaw: [f32; 2],
    pub pitch: [f32; 2],
    pub scale: f32,
    pub center: Vec3,
}

impl Camera {
    /// The original's house view: turned an eighth of a turn and tilted
    /// down 30 degrees, so a tile is drawn twice as wide as it is deep.
    /// That the 3D people are drawn with this exact angle is
    /// **unverified**.
    pub fn house_view(scale: f32, center: Vec3) -> Self {
        let half = 0.5f32.sqrt();
        Self {
            yaw: [half, half],
            pitch: [0.5, 0.75f32.sqrt()],
            scale,
            center,
        }
    }

    /// The camera turned by a further `radians` about the vertical axis.
    pub fn turned(self, radians: f32) -> Self {
        let (s, c) = radians.sin_cos();
        let [ys, yc] = self.yaw;
        Self {
            yaw: [ys * c + yc * s, yc * c - ys * s],
            ..self
        }
    }

    /// A point in view space: x right, y up, z away from the viewer.
    pub fn view(&self, p: Vec3) -> Vec3 {
        let p = p - self.center;
        let [sy, cy] = self.yaw;
        let [sp, cp] = self.pitch;
        // Turn about Y, then tilt down: what is nearer the viewer is drawn
        // lower, what is higher is drawn higher.
        let x = p.x * cy - p.z * sy;
        let z = p.x * sy + p.z * cy;
        let y = p.y * cp + z * sp;
        let z = z * cp - p.y * sp;
        Vec3::new(x, y, z)
    }

    /// The direction the viewer looks along, in world space.
    pub fn forward(&self) -> Vec3 {
        let [sy, cy] = self.yaw;
        let [sp, cp] = self.pitch;
        Vec3::new(sy * cp, -sp, cy * cp)
    }
}

/// An image with a depth buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    /// Four bytes per pixel, rows top to bottom.
    pub rgba: Vec<u8>,
    depth: Vec<f32>,
}

impl Canvas {
    pub fn new(width: u32, height: u32, background: [u8; 4]) -> Self {
        let n = (width * height) as usize;
        Self {
            width,
            height,
            rgba: background.repeat(n),
            depth: vec![f32::INFINITY; n],
        }
    }

    /// Draws one skinned part, lit by a light from `light` (a direction
    /// towards the light) with some ambient light.
    pub fn draw(&mut self, camera: &Camera, part: &Part, skinned: &Skinned, light: Vec3) {
        let (w, h) = (self.width as f32, self.height as f32);
        draw_part(self, part, skinned, Lighting::ours(light), |p| {
            let v = camera.view(p);
            Vec3::new(
                w * 0.5 + v.x * camera.scale,
                h * 0.5 - v.y * camera.scale,
                v.z,
            )
        });
    }

    /// Draws every part of `body` on the bones `world` (one per skeleton
    /// bone, from [`crate::rig::Pose::world`]). Censored parts are not
    /// drawn.
    pub fn draw_body(&mut self, camera: &Camera, body: &Body, world: &[Transform]) {
        for part in body.parts.iter().filter(|p| p.censor == 0) {
            self.draw(camera, part, &skin(part, world), LIGHT);
        }
    }

    /// How many pixels something opaque was drawn on (a pixel only partly
    /// covered by a translucent texel is not counted).
    pub fn covered(&self) -> usize {
        self.depth.iter().filter(|d| d.is_finite()).count()
    }

    /// The image as a PNG file (8-bit RGBA, stored without compression).
    pub fn png(&self) -> Vec<u8> {
        png(self.width, self.height, &self.rgba)
    }
}

/// What the rasterizer draws into: an image with a depth per pixel. A
/// target keeps its own depth units (the projection gives each vertex its
/// depth in them, and the rasterizer interpolates it linearly across a
/// triangle) and its own rule for writing a texel. [`Canvas`] is one; an
/// app draws people into its lot's image through another.
pub trait Target {
    /// Width and height in pixels.
    fn size(&self) -> (u32, u32);
    /// Whether a texel at depth `z` on pixel (`x`, `y`) would be in front
    /// of what is there. Asked before the texture is sampled, so a hidden
    /// pixel costs little; [`Target::write`] may still test again.
    fn passes(&self, x: u32, y: u32, z: f32) -> bool;
    /// Writes a texel that passed: its colour times `shade` (0 to 1, see
    /// [`Lighting`]). Its alpha is never 0.
    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], shade: f32);
    /// Writes a texel that passed, lit channel by channel by `shade`.
    /// Defaults to multiplying each channel by `shade` and calling
    /// [`Target::write`] with shade 1.
    fn write_coloured(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], shade: [f32; 3]) {
        let lit = |k: usize| (f32::from(texel[k]) * shade[k]).clamp(0.0, 255.0) as u8;
        self.write(x, y, z, [lit(0), lit(1), lit(2), texel[3]], 1.0);
    }
    /// Says whose pixels are written from now on: a person's object ID,
    /// or None once they are drawn. A target that keeps, per pixel, what
    /// drew it (the lot renderer's canvas, `sims_render::Canvas::id_at`)
    /// notes it; by default nothing does.
    fn set_id(&mut self, _id: Option<u16>) {}
}

impl Target for Canvas {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        z < self.depth[(y * self.width + x) as usize]
    }

    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], s: f32) {
        let i = (y * self.width + x) as usize;
        let alpha = texel[3];
        let lit = |c: u8| (f32::from(c) * s).min(255.0);
        if alpha == 255 {
            self.depth[i] = z;
            for (out, &c) in self.rgba[i * 4..i * 4 + 3].iter_mut().zip(&texel) {
                *out = lit(c) as u8;
            }
            self.rgba[i * 4 + 3] = 255;
            return;
        }
        // A partly transparent texel is blended over what is drawn and
        // leaves the depth alone, so it never hides what is drawn behind
        // it later. (The game's skins are opaque or fully transparent;
        // this only keeps other textures sane.) The canvas holds straight
        // (not premultiplied) colour, as a PNG does, so what is below
        // counts by its own alpha.
        let a = f32::from(alpha) / 255.0;
        let below = f32::from(self.rgba[i * 4 + 3]) / 255.0 * (1.0 - a);
        let total = a + below;
        for (out, &c) in self.rgba[i * 4..i * 4 + 3].iter_mut().zip(&texel) {
            *out = ((lit(c) * a + f32::from(*out) * below) / total).round() as u8;
        }
        self.rgba[i * 4 + 3] = (total * 255.0).round() as u8;
    }
}

/// Draws one skinned part into `target`, lit by `lighting`. `project`
/// takes a world point to the target: x and y in pixels, z its depth in
/// the target's units.
pub fn draw_part<T: Target + ?Sized>(
    target: &mut T,
    part: &Part,
    skinned: &Skinned,
    lighting: Lighting,
    project: impl Fn(Vec3) -> Vec3,
) {
    let towards = lighting.towards.normalize();
    let shade: Vec<Shade> = skinned
        .normals
        .iter()
        .map(|&n| Shade::Grey(lighting.shade_unit(n, towards)))
        .collect();
    draw_shaded(target, part, skinned, &shade, project);
}

/// How much light a vertex takes: the same for every channel, or one
/// factor each for red, green and blue (a coloured light).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shade {
    Grey(f32),
    Colour([f32; 3]),
}

impl Shade {
    fn channels(self) -> [f32; 3] {
        match self {
            Shade::Grey(s) => [s; 3],
            Shade::Colour(c) => c,
        }
    }
}

/// Draws one skinned part into `target` with each vertex's [`Shade`]
/// given (one per vertex of the part's mesh). A grey shade goes to
/// [`Target::write`]; a coloured shade goes to [`Target::write_coloured`]
/// (whose default pre-lights each channel of the texel and calls
/// [`Target::write`] with shade 1).
pub fn draw_shaded<T: Target + ?Sized>(
    target: &mut T,
    part: &Part,
    skinned: &Skinned,
    shade: &[Shade],
    project: impl Fn(Vec3) -> Vec3,
) {
    draw_shaded_with_cull(target, part, skinned, shade, project, false);
}

/// Like [`draw_shaded`], culling counter-clockwise screen-space back faces
/// (`edge(a, b, c) < 0.0`) as Direct3D 6's default `D3DRENDERSTATE_CULLMODE`
/// (`D3DCULL_CCW`, never overridden in `0x5ce911`–`0x5cea52`,
/// `0x5d34c6`–`0x5d37f1`, or `0x5d38d5`–`0x5d3c69`) does.
pub fn draw_shaded_culled<T: Target + ?Sized>(
    target: &mut T,
    part: &Part,
    skinned: &Skinned,
    shade: &[Shade],
    project: impl Fn(Vec3) -> Vec3,
) {
    draw_shaded_with_cull(target, part, skinned, shade, project, true);
}

fn draw_shaded_with_cull<T: Target + ?Sized>(
    target: &mut T,
    part: &Part,
    skinned: &Skinned,
    shade: &[Shade],
    project: impl Fn(Vec3) -> Vec3,
    cull_ccw: bool,
) {
    let screen: Vec<Vec3> = skinned.positions.iter().map(|&p| project(p)).collect();
    for t in &part.mesh.triangles {
        let [a, b, c] = t.map(|i| i as usize);
        // A mesh whose triangles name vertices it lacks draws nothing
        // there rather than panicking.
        let (Some(&pa), Some(&pb), Some(&pc)) = (screen.get(a), screen.get(b), screen.get(c))
        else {
            continue;
        };
        let (Some(va), Some(vb), Some(vc)) = (
            part.mesh.vertices.get(a),
            part.mesh.vertices.get(b),
            part.mesh.vertices.get(c),
        ) else {
            continue;
        };
        triangle(
            target,
            [pa, pb, pc],
            [va.uv, vb.uv, vc.uv],
            [a, b, c].map(|i| shade.get(i).copied().unwrap_or(Shade::Grey(1.0))),
            part,
            cull_ccw,
        );
    }
}

fn triangle<T: Target + ?Sized>(
    target: &mut T,
    mut p: [Vec3; 3],
    mut uv: [[f32; 2]; 3],
    mut shade: [Shade; 3],
    part: &Part,
    cull_ccw: bool,
) {
    let grey = shade.iter().all(|s| matches!(s, Shade::Grey(_)));
    let (width, height) = target.size();
    let mut area = edge(p[0], p[1], p[2]);
    if area.abs() < 1e-8 || !area.is_finite() {
        return;
    }
    if area < 0.0 {
        if cull_ccw {
            return;
        }
        p.swap(1, 2);
        uv.swap(1, 2);
        shade.swap(1, 2);
        area = -area;
    }
    let channels = shade.map(Shade::channels);
    // The top-left rule: a pixel centre exactly on an edge belongs to
    // the triangle only if that is its top or left edge, so a pixel on
    // an edge two triangles share is drawn once, not twice.
    let top_left = |a: Vec3, b: Vec3| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        dy < 0.0 || (dy == 0.0 && dx > 0.0)
    };
    let owns = [
        top_left(p[1], p[2]),
        top_left(p[2], p[0]),
        top_left(p[0], p[1]),
    ];
    let min_x = p
        .iter()
        .map(|v| v.x)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as i64;
    let max_x = p
        .iter()
        .map(|v| v.x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(width as f32 - 1.0) as i64;
    let min_y = p
        .iter()
        .map(|v| v.y)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.0) as i64;
    let max_y = p
        .iter()
        .map(|v| v.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(height as f32 - 1.0) as i64;
    if min_x > max_x || min_y > max_y {
        return;
    }
    let e0 = Edge::new(p[1], p[2]);
    let e1 = Edge::new(p[2], p[0]);
    let e2 = Edge::new(p[0], p[1]);
    let outside = |w: f32, owned: bool| w < 0.0 || (w == 0.0 && !owned);
    for y in min_y..=max_y {
        let qy = y as f32 + 0.5;
        let r0 = e0.row(qy);
        let r1 = e1.row(qy);
        let r2 = e2.row(qy);
        let yu = y as u32;
        for x in min_x..=max_x {
            let qx = x as f32 + 0.5;
            let v0 = e0.eval(r0, qx);
            if outside(v0, owns[0]) {
                continue;
            }
            let v1 = e1.eval(r1, qx);
            if outside(v1, owns[1]) {
                continue;
            }
            let v2 = e2.eval(r2, qx);
            if outside(v2, owns[2]) {
                continue;
            }
            let w0 = v0 / area;
            let w1 = v1 / area;
            let w2 = v2 / area;
            let z = w0 * p[0].z + w1 * p[1].z + w2 * p[2].z;
            let xu = x as u32;
            if !target.passes(xu, yu, z) {
                continue;
            }
            let u = w0 * uv[0][0] + w1 * uv[1][0] + w2 * uv[2][0];
            let v = w0 * uv[0][1] + w1 * uv[1][1] + w2 * uv[2][1];
            let texel = part.texture.sample(u, v);
            // A transparent texel (such as one a run-length bitmap skips)
            // draws nothing and hides nothing.
            if texel[3] == 0 {
                continue;
            }
            if grey {
                let s0 = w0 * channels[0][0] + w1 * channels[1][0] + w2 * channels[2][0];
                target.write(xu, yu, z, texel, s0);
            } else {
                let s = [0, 1, 2]
                    .map(|k| w0 * channels[0][k] + w1 * channels[1][k] + w2 * channels[2][k]);
                target.write_coloured(xu, yu, z, texel, s);
            }
        }
    }
}

/// Precomputed directed edge matching [`edge`] with canonical endpoint
/// ordering (always evaluated from the lexicographically smaller endpoint so
/// shared edges produce exact negated values).
struct Edge {
    ax: f32,
    ay: f32,
    dx: f32,
    dy: f32,
    negate: bool,
}

impl Edge {
    fn new(a: Vec3, b: Vec3) -> Self {
        if (a.x, a.y) <= (b.x, b.y) {
            Self {
                ax: a.x,
                ay: a.y,
                dx: b.x - a.x,
                dy: b.y - a.y,
                negate: false,
            }
        } else {
            Self {
                ax: b.x,
                ay: b.y,
                dx: a.x - b.x,
                dy: a.y - b.y,
                negate: true,
            }
        }
    }

    #[inline(always)]
    fn row(&self, qy: f32) -> f32 {
        self.dx * (qy - self.ay)
    }

    #[inline(always)]
    fn eval(&self, row_term: f32, qx: f32) -> f32 {
        let v = row_term - self.dy * (qx - self.ax);
        if self.negate { -v } else { v }
    }
}

/// Twice the signed area of the triangle `a`, `b`, `c` on screen.
fn edge(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// Encodes an RGBA image as a PNG file with stored (uncompressed) deflate
/// blocks: no compression library is needed.
pub fn png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * 4 + 1) as usize * height as usize);
    for row in rgba.chunks_exact(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let n = block.len() as u16;
        z.extend(n.to_le_bytes());
        z.extend((!n).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    for (tag, data) in [(b"IHDR", header), (b"IDAT", z), (b"IEND", Vec::new())] {
        out.extend((data.len() as u32).to_be_bytes());
        let mut chunk = tag.to_vec();
        chunk.extend(data);
        out.extend_from_slice(&chunk);
        out.extend(crc32(&chunk).to_be_bytes());
    }
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

    #[test]
    fn crc_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn a_png_has_its_chunks() {
        let file = png(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&file[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&file[12..16], b"IHDR");
        assert_eq!(&file[file.len() - 8..file.len() - 4], b"IEND");
    }

    /// A one-bone body: a square one unit wide facing -Z, standing on the
    /// origin, with `texture`.
    fn square(texture: crate::texture::Texture) -> (Body, Vec<Transform>) {
        use sims_formats::avatar::{Bone, Mesh, MeshBinding, Skeleton, Vertex, Wiggle};
        let skeleton = Skeleton {
            name: "test".into(),
            bones: vec![Bone {
                name: "ROOT".into(),
                parent: None,
                translation: [0.0; 3],
                rotation: [0.0, 0.0, 0.0, 1.0],
                properties: Vec::new(),
                flags: [0; 3],
                wiggle: Wiggle::default(),
            }],
        };
        let vertex = |x: f32, y: f32| Vertex {
            position: [x, y, 0.0],
            normal: [0.0, 0.0, -1.0],
            uv: [0.5, 0.5],
            bone: 0,
        };
        let mesh = Mesh {
            name: "square".into(),
            texture: String::new(),
            bones: vec!["ROOT".into()],
            bindings: vec![MeshBinding {
                bone: 0,
                first_vertex: 0,
                vertex_count: 4,
                first_blend: 0,
                blend_count: 0,
            }],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            vertices: vec![
                vertex(-0.5, 0.0),
                vertex(0.5, 0.0),
                vertex(0.5, 1.0),
                vertex(-0.5, 1.0),
            ],
            blends: Vec::new(),
        };
        let body = Body {
            skeleton,
            parts: vec![Part {
                appearance: "test".into(),
                bone: 0,
                bones: vec![0],
                mesh,
                texture_name: None,
                texture,
                censor: 0,
            }],
        };
        let world =
            crate::rig::Pose::rest(&body.skeleton).world(&body.skeleton, Transform::IDENTITY);
        (body, world)
    }

    /// Looking straight along +Z at the square's middle, 20 pixels a unit.
    fn front() -> Camera {
        Camera {
            yaw: [0.0, 1.0],
            pitch: [0.0, 1.0],
            scale: 20.0,
            center: Vec3::new(0.0, 0.5, 0.0),
        }
    }

    fn pixel(canvas: &Canvas, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * canvas.width + x) * 4) as usize;
        [
            canvas.rgba[i],
            canvas.rgba[i + 1],
            canvas.rgba[i + 2],
            canvas.rgba[i + 3],
        ]
    }

    #[test]
    fn transparent_texels_draw_nothing_and_hide_nothing() {
        use crate::texture::Texture;
        let background = [1, 2, 3, 255];
        let (clear, world) = square(Texture::solid([0, 0, 0, 0]));
        let mut canvas = Canvas::new(40, 40, background);
        canvas.draw_body(&front(), &clear, &world);
        assert_eq!(canvas.covered(), 0);
        assert_eq!(pixel(&canvas, 20, 20), background);
        // An opaque square drawn behind it afterwards still shows.
        let (red, _) = square(Texture::solid([255, 0, 0, 255]));
        let behind = [Transform {
            rotation: crate::math::Quat::IDENTITY,
            translation: Vec3::new(0.0, 0.0, 1.0),
        }];
        canvas.draw_body(&front(), &red, &behind);
        assert_eq!(canvas.covered(), 400);
        assert_eq!(pixel(&canvas, 20, 20)[3], 255);
        assert!(pixel(&canvas, 20, 20)[0] > 100);
    }

    #[test]
    fn translucent_texels_blend_and_leave_the_depth_alone() {
        use crate::texture::Texture;
        // Half-transparent white over black: about half grey, and the
        // pixel is not marked as covered.
        let (glass, world) = square(Texture::solid([255, 255, 255, 128]));
        let mut canvas = Canvas::new(40, 40, [0, 0, 0, 255]);
        canvas.draw_body(&front(), &glass, &world);
        assert_eq!(canvas.covered(), 0);
        let (white, _) = square(Texture::solid([255, 255, 255, 255]));
        let mut lit = Canvas::new(40, 40, [0, 0, 0, 255]);
        lit.draw_body(&front(), &white, &world);
        let full = f32::from(pixel(&lit, 20, 20)[0]);
        let [r, g, b, a] = pixel(&canvas, 20, 20);
        assert!(
            (f32::from(r) - full * 128.0 / 255.0).abs() <= 1.0,
            "{r} {full}"
        );
        assert!(r == g && g == b && r > 0);
        assert_eq!(a, 255);
        // An opaque square behind it, drawn later, is not hidden by it.
        let (red, _) = square(Texture::solid([255, 0, 0, 255]));
        let behind = [Transform {
            rotation: crate::math::Quat::IDENTITY,
            translation: Vec3::new(0.0, 0.0, 1.0),
        }];
        canvas.draw_body(&front(), &red, &behind);
        assert_eq!(canvas.covered(), 400);
        let [r, g, _, _] = pixel(&canvas, 20, 20);
        assert!(r > 100 && g == 0);
        // Over a transparent background the alpha adds up, and the
        // colour is the texel's own, not darkened by the empty pixel.
        let mut clear = Canvas::new(40, 40, [0, 0, 0, 0]);
        clear.draw_body(&front(), &glass, &world);
        let [r, _, _, a] = pixel(&clear, 20, 20);
        assert_eq!(a, 128);
        assert!((f32::from(r) - full).abs() <= 1.0, "{r} {full}");
    }

    /// Draws the screen-space triangles `triangles` half transparent onto a
    /// clear canvas.
    fn translucent(triangles: &[[Vec3; 3]], size: u32) -> Canvas {
        let (glass, _) = square(crate::texture::Texture::solid([255, 255, 255, 128]));
        let mut canvas = Canvas::new(size, size, [0, 0, 0, 0]);
        for t in triangles {
            triangle(
                &mut canvas,
                *t,
                [[0.5; 2]; 3],
                [Shade::Grey(1.0); 3],
                &glass.parts[0],
                false,
            );
        }
        canvas
    }

    #[test]
    fn a_shared_edge_is_drawn_once_wherever_it_falls() {
        // An edge whose ends are not on the pixel grid, with the pixel
        // centre (1.5, 1.5) within rounding of it: one triangle or the
        // other takes the pixel, never both and never neither.
        let (a, b) = (
            Vec3::new(1.258_336_7, 0.405_250_9, 0.5),
            Vec3::new(1.718_175_8, 2.488_349, 0.5),
        );
        let left = Vec3::new(0.0, 2.0, 0.5);
        let right = Vec3::new(3.0, 1.0, 0.5);
        let canvas = translucent(&[[a, b, left], [b, a, right]], 4);
        assert_eq!(pixel(&canvas, 1, 1)[3], 128);
        // A fan of seven triangles around a point off the grid: every
        // pixel is drawn at most once, and none inside is missed.
        let centre = Vec3::new(10.37, 9.81, 0.5);
        let rim: Vec<Vec3> = (0..7)
            .map(|k| {
                let [s, c] = crate::person::heading(k as f32 * 8.0 / 7.0 + 0.4);
                centre + Vec3::new(8.0 * c, 8.0 * s, 0.0)
            })
            .collect();
        let fan: Vec<[Vec3; 3]> = (0..7).map(|k| [centre, rim[k], rim[(k + 1) % 7]]).collect();
        let canvas = translucent(&fan, 21);
        for y in 0..21 {
            for x in 0..21 {
                let alpha = pixel(&canvas, x, y)[3];
                assert!(alpha == 0 || alpha == 128, "({x}, {y}): {alpha}");
                let (dx, dy) = (x as f32 + 0.5 - centre.x, y as f32 + 0.5 - centre.y);
                if dx * dx + dy * dy < 36.0 {
                    assert_eq!(alpha, 128, "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_house_view_looks_down_at_the_center() {
        let camera = Camera::house_view(1.0, Vec3::new(1.0, 2.0, 3.0));
        assert!(camera.view(Vec3::new(1.0, 2.0, 3.0)).length() < 1e-6);
        // A point straight above the center is drawn straight above it.
        let up = camera.view(Vec3::new(1.0, 3.0, 3.0));
        assert!(up.x.abs() < 1e-6 && up.y > 0.0);
        // The view direction maps to view-space +Z.
        let ahead = camera.view(Vec3::new(1.0, 2.0, 3.0) + camera.forward());
        assert!(
            (ahead - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-5,
            "{ahead:?}"
        );
    }

    #[test]
    fn draw_shaded_culled_drops_counter_clockwise_back_faces() {
        use sims_formats::avatar::{Mesh, MeshBinding, Vertex};

        let make_part = |triangles: Vec<[u32; 3]>, color: [u8; 4]| Part {
            appearance: "tri".into(),
            bone: 0,
            bones: vec![0],
            mesh: Mesh {
                name: "tri".into(),
                texture: String::new(),
                bones: vec!["ROOT".into()],
                bindings: vec![MeshBinding {
                    bone: 0,
                    first_vertex: 0,
                    vertex_count: 3,
                    first_blend: 0,
                    blend_count: 0,
                }],
                triangles,
                vertices: vec![
                    Vertex {
                        position: [1.0, 1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                        uv: [0.5, 0.5],
                        bone: 0,
                    },
                    Vertex {
                        position: [7.0, 1.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                        uv: [0.5, 0.5],
                        bone: 0,
                    },
                    Vertex {
                        position: [4.0, 7.0, 0.0],
                        normal: [0.0, 0.0, 1.0],
                        uv: [0.5, 0.5],
                        bone: 0,
                    },
                ],
                blends: Vec::new(),
            },
            texture_name: None,
            texture: crate::texture::Texture::solid(color),
            censor: 0,
        };
        // Screen-space clockwise (y grows downward): edge(a, b, c) > 0 (front).
        let front = make_part(vec![[0, 1, 2]], [200, 50, 50, 255]);
        // Reversed winding: edge(a, b, c) < 0 (back face).
        let back = make_part(vec![[0, 2, 1]], [50, 200, 50, 255]);
        let skinned = |z: f32| Skinned {
            positions: vec![
                Vec3::new(1.0, 1.0, z),
                Vec3::new(7.0, 1.0, z),
                Vec3::new(4.0, 7.0, z),
            ],
            normals: vec![Vec3::new(0.0, 0.0, 1.0); 3],
        };
        let shade = [Shade::Grey(1.0); 3];

        // Unculled: the nearer back face overwrites the front face.
        let mut unculled = Canvas::new(8, 8, [0, 0, 0, 255]);
        draw_shaded(&mut unculled, &front, &skinned(5.0), &shade, |p| p);
        draw_shaded(&mut unculled, &back, &skinned(1.0), &shade, |p| p);
        assert_eq!(pixel(&unculled, 4, 3), [50, 200, 50, 255]);

        // Culled (`D3DCULL_CCW`): the nearer back face is dropped, keeping the front face.
        let mut culled = Canvas::new(8, 8, [0, 0, 0, 255]);
        draw_shaded_culled(&mut culled, &front, &skinned(5.0), &shade, |p| p);
        draw_shaded_culled(&mut culled, &back, &skinned(1.0), &shade, |p| p);
        assert_eq!(pixel(&culled, 4, 3), [200, 50, 50, 255]);
    }
}
