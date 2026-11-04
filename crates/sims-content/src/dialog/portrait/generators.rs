//! 4e725f / 4e7448 / 4e7671 / 4e81f6 / 4e8400. Image owners and
//! synchronous drawing/blitting are supplied services; generator ordering,
//! layouts, parameters and failure aggregation follow the original callers.
use super::{Layout, Rect};
use serde::{Deserialize, Serialize};

pub const FIXED_HEAD: Rect = [76, 45, 122, 125];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Render {
    pub angles: [f32; 2],
    pub target: [i32; 2],
    pub padding: [i32; 4],
    /// Packed source-surface clear color, not an RGB888 palette key.
    pub background: u32,
    pub fixed: Option<Rect>,
}
pub struct Rendered<I> {
    /// Owns one temporary wrapper reference; release after use, even on a
    /// failed draw. A successful draw may also supply an empty wrapper.
    pub image: Option<I>,
    pub success: bool,
}

pub trait Backend {
    type Image: Copy;
    /// Allocates and constructs an unreferenced image; None is allocation failure.
    fn allocate(&mut self) -> Option<Self::Image>;
    fn create(&mut self, image: Self::Image, width: i32, height: i32) -> bool;
    /// Direct native destruction of an unreferenced image.
    fn destroy(&mut self, image: Self::Image);
    fn template(&mut self, resource: u16) -> Option<Self::Image>;
    fn lock(&mut self, image: Self::Image, mode: u32) -> bool;
    fn unlock(&mut self, image: Self::Image, mode: u32);
    fn color_key(&mut self, image: Self::Image, color: u32);
    /// Read origin pixel; speech callers query byte stride first even though
    /// their sample coordinates are both zero.
    fn first_pixel(&mut self, image: Self::Image, query_stride: bool) -> u16;
    fn rect(&self, image: Self::Image) -> Rect;
    fn render(&mut self, request: Render) -> Rendered<Self::Image>;
    fn blit(&mut self, destination: Self::Image, source: Self::Image, from: Rect, to: Rect)
    -> bool;
    fn release(&mut self, image: Self::Image);
    /// Retain the new image, release the old output, then replace its wrapper.
    fn publish(&mut self, image: Self::Image);
    /// The caller's preexisting output wrapper, used by the thumbnail path.
    fn output(&self) -> Option<Self::Image>;
}

fn translated([l, t, r, b]: Rect, [x, y]: [i32; 2]) -> Rect {
    [
        l.wrapping_add(x),
        t.wrapping_add(y),
        r.wrapping_add(x),
        b.wrapping_add(y),
    ]
}
fn common_angles() -> [f32; 2] {
    [
        std::f32::consts::PI * -0.027777778,
        std::f32::consts::PI * 0.1,
    ]
}
fn request(target: [i32; 2], background: u32) -> Render {
    Render {
        angles: [0.; 2],
        target,
        padding: [0; 4],
        background,
        fixed: Some(FIXED_HEAD),
    }
}
fn pack(color: u32, format: u32) -> u32 {
    let r = (color >> 16) & 0xf8;
    let g = (color >> 8) & if format == 1 { 0xf8 } else { 0xfc };
    ((r << 5 | g) << if format == 1 { 2 } else { 3 }) | (color >> 3 & 31)
}

/// Slot one is composed separately by 4e7082. `person_word` is the explicit
/// caller word used by 4e8400's failed-template-lock fallback, where the native
/// code accidentally uses its person pointer as a color. It is not a neighbor
/// lookup. Normal initialized Rust image buffers always permit this lock.
pub fn generate(
    slot: usize,
    layout: Layout,
    format: u32,
    key: u32,
    person_word: u32,
    host: &mut impl Backend,
) -> Result<bool, String> {
    let [_, _, x, y, width, height] = layout.0;
    match slot {
        0 | 2 => {
            let Some(destination) = host.allocate() else {
                return Ok(false);
            };
            let size = if slot == 0 {
                [width.wrapping_mul(3), height]
            } else {
                [200, 40]
            };
            if !host.create(destination, size[0], size[1]) {
                host.destroy(destination);
                return Ok(false);
            }
            let mut success = true;
            if host.lock(destination, 16) {
                let colors = [0xd10000, 0x882e2f, 0x405d5f, 0x20934c, 0xca39];
                for n in 0..if slot == 0 { 3 } else { 5 } {
                    let mut draw = request(
                        if slot == 0 { [width, height] } else { [40, 40] },
                        if slot == 0 {
                            10
                        } else {
                            pack(colors[n], format)
                        },
                    );
                    if slot == 0 {
                        draw.angles = match n {
                            0 => [0.; 2],
                            1 => [
                                std::f32::consts::PI * 0.05,
                                std::f32::consts::PI * -0.03888889,
                            ],
                            _ => [
                                std::f32::consts::PI * -0.05,
                                std::f32::consts::PI * 0.03888889,
                            ],
                        };
                    } else {
                        draw.angles = common_angles();
                        draw.padding = [5; 4];
                    }
                    let rendered = host.render(draw);
                    success &= rendered.success;
                    if let Some(image) = rendered.image {
                        let from = host.rect(image);
                        let to = if slot == 0 {
                            let left = width.wrapping_mul(n as i32);
                            [left, 0, left.wrapping_add(width), height]
                        } else {
                            translated(from, [n as i32 * 40, 0])
                        };
                        success &= host.blit(destination, image, from, to);
                        host.release(image);
                    }
                }
                host.unlock(destination, 16);
            }
            if slot == 0 {
                host.color_key(destination, key);
            }
            host.publish(destination);
            Ok(success)
        }
        3 => {
            let destination = host.output().ok_or("portrait thumbnail input absent")?;
            let rect = host.rect(destination);
            let draw = request(
                [
                    rect[2].wrapping_sub(rect[0]).wrapping_sub(2),
                    rect[3].wrapping_sub(rect[1]).wrapping_sub(2),
                ],
                0,
            );
            let rendered = host.render(draw);
            let mut success = rendered.success;
            if let Some(image) = rendered.image {
                let from = host.rect(image);
                if host.lock(destination, 16) {
                    let copied = host.blit(destination, image, from, translated(from, [1, 1]));
                    host.unlock(destination, 16);
                    success &= copied;
                }
                host.release(image);
            }
            Ok(success)
        }
        4 | 5 => {
            let Some(template) = host.template(if slot == 4 { 9000 } else { 9001 }) else {
                return Ok(false);
            };
            host.color_key(template, key);
            let background = if host.lock(template, 64) {
                let sample = host.first_pixel(template, true);
                host.unlock(template, 64);
                u32::from(sample)
            } else {
                65535
            };
            let mut draw = request([if slot == 4 { 16 } else { 32 }; 2], background);
            draw.angles = common_angles();
            draw.fixed = None;
            let rendered = host.render(draw);
            let mut success = rendered.success;
            if let Some(image) = rendered.image {
                if host.lock(template, 16) {
                    let from = host.rect(image);
                    // 4e83b8 translates by the caller's [0,0] offset.
                    success &= host.blit(template, image, from, from);
                    host.unlock(template, 16);
                }
            }
            host.publish(template);
            if let Some(image) = rendered.image {
                host.release(image);
            }
            Ok(success)
        }
        6 => {
            // The native web caller allocates a separate unused image, then
            // destroys it before publishing the loaded template. Retain even
            // this allocation/failure boundary rather than shortcutting it.
            let Some(unused) = host.allocate() else {
                return Ok(false);
            };
            if !host.create(unused, width, height) {
                host.destroy(unused);
                return Ok(false);
            }
            let Some(template) = host.template(4602) else {
                host.destroy(unused);
                return Ok(false);
            };
            let background = if host.lock(template, 64) {
                let sample = host.first_pixel(template, false);
                host.unlock(template, 64);
                u32::from(sample)
            } else {
                person_word
            };
            let rendered = host.render(request([width, height], background));
            let mut success = rendered.success;
            if success {
                if let Some(image) = rendered.image {
                    if host.lock(template, 16) {
                        let from = host.rect(image);
                        let copied = host.blit(template, image, from, translated(from, [x, y]));
                        host.unlock(template, 16);
                        success &= copied;
                    }
                }
            }
            host.destroy(unused);
            host.publish(template);
            if let Some(image) = rendered.image {
                host.release(image);
            }
            Ok(success)
        }
        _ => Ok(false),
    }
}
