//! A small software rasterizer: draws skinned, textured meshes into an
//! image with a depth buffer, with no window and no GPU, so tests can
//! render offscreen. It is also the reference the GPU renderer can be
//! checked against later (the old prototype kept one for the same reason:
//! legacy/crates/sims-render/src/software.rs).

use crate::appearance::{Body, Part};
use crate::math::{Transform, Vec3};
use crate::skin::{Skinned, skin};

/// The direction the light comes from in the offscreen renders: above,
/// a little to the side and behind the viewer of the house view.
pub const LIGHT: Vec3 = Vec3::new(0.3, 1.0, -0.5);

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
        let light = light.normalize();
        let (w, h) = (self.width as f32, self.height as f32);
        let screen: Vec<Vec3> = skinned
            .positions
            .iter()
            .map(|&p| {
                let v = camera.view(p);
                Vec3::new(
                    w * 0.5 + v.x * camera.scale,
                    h * 0.5 - v.y * camera.scale,
                    v.z,
                )
            })
            .collect();
        let shade: Vec<f32> = skinned
            .normals
            .iter()
            .map(|n| 0.45 + 0.55 * n.dot(light).max(0.0))
            .collect();
        for t in &part.mesh.triangles {
            let [a, b, c] = t.map(|i| i as usize);
            self.triangle(
                [screen[a], screen[b], screen[c]],
                [
                    part.mesh.vertices[a].uv,
                    part.mesh.vertices[b].uv,
                    part.mesh.vertices[c].uv,
                ],
                [shade[a], shade[b], shade[c]],
                part,
            );
        }
    }

    /// Draws every part of `body` on the bones `world` (one per skeleton
    /// bone, from [`crate::rig::Pose::world`]). Censored parts are not
    /// drawn.
    pub fn draw_body(&mut self, camera: &Camera, body: &Body, world: &[Transform]) {
        for part in body.parts.iter().filter(|p| p.censor == 0) {
            self.draw(camera, part, &skin(part, world), LIGHT);
        }
    }

    fn triangle(&mut self, p: [Vec3; 3], uv: [[f32; 2]; 3], shade: [f32; 3], part: &Part) {
        let area = edge(p[0], p[1], p[2]);
        if area.abs() < 1e-8 {
            return;
        }
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
            .min(self.width as f32 - 1.0) as i64;
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
            .min(self.height as f32 - 1.0) as i64;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let q = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, 0.0);
                let w0 = edge(p[1], p[2], q) / area;
                let w1 = edge(p[2], p[0], q) / area;
                let w2 = edge(p[0], p[1], q) / area;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * p[0].z + w1 * p[1].z + w2 * p[2].z;
                let i = (y as u32 * self.width + x as u32) as usize;
                if z >= self.depth[i] {
                    continue;
                }
                let u = w0 * uv[0][0] + w1 * uv[1][0] + w2 * uv[2][0];
                let v = w0 * uv[0][1] + w1 * uv[1][1] + w2 * uv[2][1];
                let s = w0 * shade[0] + w1 * shade[1] + w2 * shade[2];
                let texel = part.texture.sample(u, v);
                self.depth[i] = z;
                for (k, channel) in texel.iter().take(3).enumerate() {
                    self.rgba[i * 4 + k] = (f32::from(*channel) * s).min(255.0) as u8;
                }
                self.rgba[i * 4 + 3] = 255;
            }
        }
    }

    /// How many pixels something was drawn on.
    pub fn covered(&self) -> usize {
        self.depth.iter().filter(|d| d.is_finite()).count()
    }

    /// The image as a PNG file (8-bit RGBA, stored without compression).
    pub fn png(&self) -> Vec<u8> {
        png(self.width, self.height, &self.rgba)
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
}
