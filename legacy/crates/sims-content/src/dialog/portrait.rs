//! 4e7082 portrait atlas composition, with image operations supplied by a host.
//! This preserves native control flow; a backend is responsible for allocation,
//! rectangle copying, color keys and image ownership. It does not emulate GDI.

pub type Rect = [i32; 4];
pub mod generators;
pub mod resize;
pub mod blit;
pub mod bmp;
pub mod images;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Faces,
    Background,
}

/// Native global words 6f2650..6f2664, supplied explicitly until full startup
/// owns them: cell width/height, face x/y offset, face width/height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout(pub [i32; 6]);

pub trait Backend {
    /// Allocate and construct one native-equivalent image owner. False means
    /// allocation failed; construction is not attempted in that case.
    fn allocate(&mut self) -> bool;
    fn create(&mut self, width: i32, height: i32) -> bool;
    fn destroy(&mut self);
    fn lock(&mut self, mode: u32) -> bool;
    fn unlock(&mut self, mode: u32);
    fn fill(&mut self, key: u32) -> bool;
    fn color_key(&mut self, key: u32);
    fn blit(&mut self, source: Source, from: Rect, to: Rect) -> bool;
    /// Retain the allocated image in the caller's output wrapper. This occurs
    /// even after a fill/blit failure, but never after allocation/create failure.
    fn publish(&mut self);
}

pub fn compose(
    layout: Layout,
    faces: Rect,
    background: Rect,
    key: u32,
    backend: &mut impl Backend,
) -> bool {
    let [width, height, x, y, face_width, face_height] = layout.0;
    if !backend.allocate() {
        return false;
    }
    if !backend.create(width.wrapping_mul(4), height.wrapping_mul(3)) {
        backend.destroy();
        return false;
    }
    let mut success = true;
    if backend.lock(1) {
        success &= backend.fill(key);
        backend.color_key(key);
        backend.unlock(1);
    }
    if backend.lock(16) {
        let mut to = background;
        let step = background[3].wrapping_sub(background[1]);
        for _ in 0..3 {
            success &= backend.blit(Source::Background, background, to);
            to[1] = to[1].wrapping_add(step);
            to[3] = to[3].wrapping_add(step);
        }
        for row in 0..3 {
            // 4e719f re-reads the original source rectangle on EVERY row.
            // The increment at 4e7228 is overwritten: all rows use its first
            // third. Do not substitute one of the other saved face samples.
            let from = [
                faces[0],
                faces[1],
                faces[0].wrapping_add(faces[2].wrapping_sub(faces[0]) / 3),
                faces[3],
            ];
            let top = y.wrapping_add(height.wrapping_mul(row));
            let mut to = [
                x,
                top,
                x.wrapping_add(face_width),
                top.wrapping_add(face_height),
            ];
            for _ in 0..4 {
                success &= backend.blit(Source::Faces, from, to);
                to[0] = to[0].wrapping_add(width);
                to[2] = to[2].wrapping_add(width);
            }
        }
        backend.unlock(16);
    }
    backend.publish();
    success
}
