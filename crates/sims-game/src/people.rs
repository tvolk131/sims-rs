//! The people, drawn with `sims-people` into the canvas `sims-render` drew
//! the lot in.
//!
//! [`sims_people::People`] places and poses each person and draws them
//! (`draw_on_lot`), far to near, every pixel tested against the lot's
//! depth (`Depth::point`, by the canvas's sprite rule), so walls,
//! furniture and the floor above hide the people behind them. A pixel of
//! a person that is drawn writes its depth into the canvas, so a nearer
//! part of the mesh drawn later covers it, and names the person
//! (`Canvas::id_at`), so a click finds who is shown there
//! ([`crate::pick`]). People on a level above the camera's are not drawn,
//! so with the camera on the second floor its floor hides everyone
//! downstairs, as it hides the rooms there. How the original gives a
//! person's mesh its depth is **unverified** (see `docs/decisions/`,
//! "People's meshes go into the lot renderer's canvas").

use sims_formats::bitmap::Image;
use sims_people::pie_head::{HeadPose, Look};
use sims_render::{Camera, Canvas, clamp_depth, pack_rgb, unpack_rgb};
use sims_sim::World;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Radius of the dark radial shading circle behind the pie menu (`[0x6c4708]`,
/// `0x4ac8ff`).
pub const PIE_SHADE_RADIUS: i32 = 100;

/// Shades the circular backdrop behind an open pie menu directly in `canvas`,
/// clipped to the active 3D house `viewport` (`0x4a94ec` → `0x4ac8c1` →
/// `0x4ac8ff`).
///
/// Inside radius `67` (`radius - radius / 3`), each pixel is desaturated to
/// `grey = (r + g + b) / 6 + 8` (reading the 5-6-5 zero-filled channels as
/// `0x438ef6` does); between `67` and `100` it blends linearly in squared
/// distance back to the original colour and quantizes to 5-6-5 (`0x49f0ae`).
pub fn shade_pie_face(canvas: &mut Canvas, center: [i32; 2], viewport: (u32, u32)) {
    shade_pie_face_with_radius(canvas, center, PIE_SHADE_RADIUS, viewport);
}

/// Like [`shade_pie_face`], with an explicit animated `radius` in pixels
/// (`0..=PIE_SHADE_RADIUS`, matching `PieAnimState::backdrop` during the
/// 400 ms pop-in and submenu center/radius glide).
pub fn shade_pie_face_with_radius(
    canvas: &mut Canvas,
    center: [i32; 2],
    radius: i32,
    viewport: (u32, u32),
) {
    let vp_w = i32::try_from(viewport.0)
        .unwrap_or(i32::MAX)
        .min(canvas.width as i32);
    let vp_h = i32::try_from(viewport.1)
        .unwrap_or(i32::MAX)
        .min(canvas.height as i32);
    if vp_w <= 0 || vp_h <= 0 || radius <= 0 {
        return;
    }
    let r2 = radius * radius;
    let inner = radius - radius / 3;
    let inner2 = inner * inner;
    let span = (r2 - inner2).max(1);
    let x0 = center[0].saturating_sub(radius).max(0);
    let y0 = center[1].saturating_sub(radius).max(0);
    let x1 = center[0].saturating_add(radius).min(vp_w);
    let y1 = center[1].saturating_add(radius).min(vp_h);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let width = canvas.width;
    for y in y0..y1 {
        let dy = y - center[1];
        let dy2 = dy * dy;
        let row = y as usize * width;
        for x in x0..x1 {
            let dx = x - center[0];
            let d2 = dx * dx + dy2;
            if d2 >= r2 {
                continue;
            }
            let idx = row + x as usize;
            // 0x4ac8e7 passes threshold 0; 0x4aca76..0x4aca7a skips pixels
            // whose 16-bit depth is <= 0 (empty sky is EMPTY = 65535 > 0).
            if canvas.depth[idx] == 0 {
                continue;
            }
            let [pr, pg, pb] = unpack_rgb(canvas.color[idx]);
            // 0x438ef6 unpacks 5-6-5 pixels with zero-filled low bits.
            let r = i32::from(pr & 0xf8);
            let g = i32::from(pg & 0xfc);
            let b = i32::from(pb & 0xf8);
            let grey = (r + g + b) / 6 + 8;
            let (out_r, out_g, out_b) = if d2 <= inner2 {
                (grey, grey, grey)
            } else {
                let t = (((d2 - inner2) << 8) / span).clamp(0, 255);
                let inv = 255 - t;
                (
                    (r * t + grey * inv) >> 8,
                    (g * t + grey * inv) >> 8,
                    (b * t + grey * inv) >> 8,
                )
            };
            // 0x49f0ae truncates back to 5-6-5; expand via MSB bit-replication
            // to match the rest of the 8-bit framebuffer.
            let r5 = (out_r.clamp(0, 255) as u8) >> 3;
            let g6 = (out_g.clamp(0, 255) as u8) >> 2;
            let b5 = (out_b.clamp(0, 255) as u8) >> 3;
            canvas.color[idx] = pack_rgb([
                (r5 << 3) | (r5 >> 2),
                (g6 << 2) | (g6 >> 4),
                (b5 << 3) | (b5 >> 2),
            ]);
        }
    }
}

/// Composites a rendered pie-menu 3D `head` into `canvas` at `face_origin`,
/// clipped to the active 3D house `viewport` and depth-tested (`LESSEQUAL`)
/// against `canvas.depth` using each pixel's native 16-bit depth in
/// `head_depth` (`0x4a9500`, `0x5ce959`–`0x5ce977`, `0x4a953e` → `0x587cf1` →
/// `0x462290`). Running this in the house pass before the 2D HUD ensures
/// nearer scene geometry occludes the head, the bottom control strip hides any
/// part below `viewport.1`, and pie labels drawn later still appear on top.
pub fn composite_pie_head(
    canvas: &mut Canvas,
    face_origin: [i32; 2],
    head: &Image,
    head_depth: &[i32],
    viewport: (u32, u32),
) {
    let vp_w = i32::try_from(viewport.0)
        .unwrap_or(i32::MAX)
        .min(canvas.width as i32);
    let vp_h = i32::try_from(viewport.1)
        .unwrap_or(i32::MAX)
        .min(canvas.height as i32);
    let hw = i32::try_from(head.width).unwrap_or(0);
    let hh = i32::try_from(head.height).unwrap_or(0);
    if vp_w <= 0 || vp_h <= 0 || hw <= 0 || hh <= 0 {
        return;
    }
    let sx0 = (-face_origin[0]).max(0);
    let sy0 = (-face_origin[1]).max(0);
    let sx1 = (vp_w - face_origin[0]).min(hw);
    let sy1 = (vp_h - face_origin[1]).min(hh);
    if sx0 >= sx1 || sy0 >= sy1 {
        return;
    }
    let head_w = head.width as usize;
    let canvas_w = canvas.width;
    for sy in sy0..sy1 {
        let dy = (face_origin[1] + sy) as usize;
        let dst_row = dy * canvas_w;
        let src_row = sy as usize * head_w;
        for sx in sx0..sx1 {
            let src_idx = src_row + sx as usize;
            let p = src_idx * 4;
            let a = head.rgba[p + 3];
            if a == 0 {
                continue;
            }
            let Some(&z) = head_depth.get(src_idx) else {
                continue;
            };
            let dx = (face_origin[0] + sx) as usize;
            let dst_idx = dst_row + dx;
            if z > i32::from(canvas.depth[dst_idx]) {
                continue;
            }
            canvas.depth[dst_idx] = clamp_depth(z);
            if a == 255 {
                canvas.color[dst_idx] =
                    pack_rgb([head.rgba[p], head.rgba[p + 1], head.rgba[p + 2]]);
            } else {
                let alpha = u16::from(a);
                let inv = 255 - alpha;
                let mut rgb = unpack_rgb(canvas.color[dst_idx]);
                for (channel, &src) in rgb.iter_mut().zip(&head.rgba[p..p + 3]) {
                    *channel =
                        ((u16::from(src) * alpha + u16::from(*channel) * inv + 127) / 255) as u8;
                }
                canvas.color[dst_idx] = pack_rgb(rgb);
            }
        }
    }
}

/// How many different problems are kept; later ones are dropped.
const KEPT: usize = 256;

/// Temporary directory holding REDB-materialized `Characters/` files for the
/// lifetime of a [`People`] cast, removed automatically on drop.
struct TempCastDir(PathBuf);

impl TempCastDir {
    fn create() -> Result<Self, String> {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let base = std::env::temp_dir();
        for _ in 0..1024 {
            let seq = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("sims-rs-cast-{pid}-{seq}"));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(format!(
                        "creating temporary cast directory {}: {e}",
                        path.display()
                    ));
                }
            }
        }
        Err("could not allocate a unique temporary cast directory".into())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempCastDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Joins a relative REDB character key under `base`, splitting on both `/` and
/// `\` (matching `sims_content::Index::add_character_files` on Unix and
/// Windows) and rejecting absolute paths, parent traversal (`..`), or drive
/// prefixes.
fn safe_relative_path(base: &Path, rel: &str) -> Option<PathBuf> {
    if rel.starts_with('/') || rel.starts_with('\\') {
        return None;
    }
    let mut out = base.to_path_buf();
    let mut has_component = false;
    for part in rel.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') || part.contains('\0') {
            return None;
        }
        out.push(part);
        has_component = true;
    }
    has_component.then_some(out)
}

/// The people of a lot, and what went wrong drawing them.
pub struct People {
    people: sims_people::People,
    /// Keeps any REDB-staged character cast directory alive for `sims_people::Cast`.
    _temp_cast_dir: Option<TempCastDir>,
    /// What went wrong, one line each, each said once, for the host to
    /// show.
    pub problems: Vec<String>,
    /// The stream the censor mosaic draws from ([`People::draw_censor`]).
    censor_random: CensorRandom,
}

/// The censor mosaic's random draws. The original takes them from the
/// global stream (`0x46f457`), which the simulation also draws from. The
/// presentation must never change the world, so the mosaic draws from a
/// copy of the world's global stream instead: taken again after every
/// tick (or whenever the stream, the tick count or the restore generation
/// changes, so a restored save draws as it did when saved), and
/// advanced only by the mosaic in between. So the first frame after a
/// tick draws as the original's first censored frame after it does, and
/// frames while paused keep drawing on. The original's draws would also
/// move the simulation's later draws; this copy's do not.
#[derive(Debug, Default, Clone, Copy)]
struct CensorRandom {
    /// The restore generation, the world's tick count and stream state the
    /// copy was taken from, and the copy.
    copy: Option<((u64, u32, u32), sims_core::rng::GameRng)>,
}

impl CensorRandom {
    fn follow(&mut self, world: &World, generation: u64) -> &mut sims_core::rng::GameRng {
        let at = (generation, world.ticks, world.global_rng.state());
        let (seen, copy) = self.copy.get_or_insert((at, world.global_rng));
        if *seen != at {
            *seen = at;
            *copy = world.global_rng;
        }
        copy
    }
}

impl People {
    /// Reads the people's files from the install at `root`, and the cast
    /// of the neighborhood `house` is in.
    pub fn open(root: &Path, house: &Path) -> Result<Self, String> {
        Ok(Self::from_inner(sims_people::People::open(root, house)?))
    }

    /// Wraps an already-constructed [`sims_people::People`] built from shared
    /// startup state.
    pub(crate) fn from_inner(people: sims_people::People) -> Self {
        Self {
            people,
            _temp_cast_dir: None,
            problems: Vec::new(),
            censor_random: CensorRandom::default(),
        }
    }

    /// Reads the people's files from the install at `root`, and the cast
    /// of `neighborhood_dir` (`UserData`, `UserData2`..`UserData8`), without
    /// requiring a house file on disk.
    pub fn open_neighborhood(root: &Path, neighborhood_dir: &Path) -> Result<Self, String> {
        let assets = sims_people::Assets::scan(root)?;
        let library = sims_people::Library::scan(&assets)?;
        let cast = sims_people::Cast::open(root, neighborhood_dir)?;
        Ok(Self::from_inner(sims_people::People::new(
            assets, library, cast,
        )))
    }

    /// Reads the people's files from the install at `root` and builds the
    /// neighborhood character cast directly from REDB's `character_files`
    /// snapshot, matching the VM and HUD even for REDB-only or
    /// `TemplateUserData`-imported characters without reading loose `UserData`
    /// files on disk.
    pub fn open_from_character_files(
        root: &Path,
        character_files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, String> {
        let staging = TempCastDir::create()?;
        let empty_iff = sims_formats::iff::write(&[])
            .map_err(|e| format!("writing staged Neighborhood.iff: {e}"))?;
        std::fs::write(staging.path().join("Neighborhood.iff"), empty_iff)
            .map_err(|e| format!("staging Neighborhood.iff: {e}"))?;
        if !character_files.is_empty() {
            let chars_dir = staging.path().join("Characters");
            std::fs::create_dir_all(&chars_dir)
                .map_err(|e| format!("creating staged Characters dir: {e}"))?;
            for (rel, bytes) in character_files {
                let is_dir = rel.ends_with('/') || rel.ends_with('\\');
                let trimmed = rel.trim_end_matches(['/', '\\']);
                let Some(dest) = safe_relative_path(&chars_dir, trimmed) else {
                    continue;
                };
                if is_dir {
                    std::fs::create_dir_all(&dest)
                        .map_err(|e| format!("creating staged dir {}: {e}", dest.display()))?;
                } else {
                    if let Some(parent) = dest.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            format!("creating staged parent {}: {e}", parent.display())
                        })?;
                    }
                    std::fs::write(&dest, bytes)
                        .map_err(|e| format!("staging character {}: {e}", dest.display()))?;
                }
            }
        }
        let assets = sims_people::Assets::scan(root)?;
        let library = sims_people::Library::scan(&assets)?;
        let cast = sims_people::Cast::open(root, staging.path())?;
        Ok(Self {
            people: sims_people::People::new(assets, library, cast),
            _temp_cast_dir: Some(staging),
            problems: Vec::new(),
            censor_random: CensorRandom::default(),
        })
    }

    /// Follows `world`: loads the people who appeared or changed clothes
    /// and forgets those who left.
    pub fn refresh(&mut self, world: &World) {
        let problems = self.people.update(world);
        self.note(problems);
    }

    /// Whether the object `id` is drawn as a plumb bob (the standard, date
    /// or star bob) when a person holds it in their head slot
    /// ([`sims_people::People::is_bob`]).
    pub fn is_bob(&self, world: &World, id: u16) -> bool {
        self.people.is_bob(world, id)
    }

    /// The plumb bobs ([`People::is_bob`]) the Sims of `world` hold in
    /// their head slots: the only bobs drawn, so the only ones that spin
    /// and that the pick notes (the animator's tick moves only the
    /// head-slot object, 0x58f8ff). Only the head-slot occupants are
    /// looked up, not every object.
    pub fn bobs(&self, world: &World) -> std::collections::BTreeSet<u16> {
        self.held_in_heads(world)
            .filter(|&id| self.is_bob(world, id))
            .collect()
    }

    /// Whether a Sim of `world` holds a plumb bob ([`People::bobs`]).
    pub fn has_bob(&self, world: &World) -> bool {
        self.held_in_heads(world).any(|id| self.is_bob(world, id))
    }

    /// What each Sim of `world` holds in their head slot.
    fn held_in_heads<'w>(&self, world: &'w World) -> impl Iterator<Item = u16> + 'w {
        world
            .objects
            .iter()
            .filter(|(_, o)| o.person.is_some())
            .filter_map(move |(&person, _)| sims_people::bob::in_head_slot(world, person))
    }

    /// Sets the clock, in milliseconds, that spins the plumb bob
    /// ([`sims_people::People::set_clock`]).
    pub fn set_clock(&mut self, milliseconds: u32) {
        self.people.set_clock(milliseconds);
    }

    /// Draws every placed person on the levels `camera` shows into
    /// `canvas`, which holds the lot `sims_render::draw_lot` drew with the
    /// same camera, each pixel naming the person drawn in it. The objects
    /// that hold a person go in after (`sims_render::draw_held`).
    pub fn draw(&mut self, world: &World, camera: &Camera, canvas: &mut Canvas) {
        let viewport = (canvas.width as u32, canvas.height as u32);
        self.draw_in_viewport(world, camera, viewport, canvas);
    }

    /// Like [`People::draw`], passing the active 3D house `viewport` size
    /// (`0x49ae5e`, `0x4d363b`, `0x53b2eb` → `0x4b136b` → `0x49b0bf`) for
    /// the house camera's eye (`sims_people::lot::View::eye`).
    pub fn draw_in_viewport(
        &mut self,
        world: &World,
        camera: &Camera,
        viewport: (u32, u32),
        canvas: &mut Canvas,
    ) {
        let drawn = self
            .people
            .draw_on_lot_in_viewport(world, camera, viewport, canvas);
        self.note(drawn.problems);
    }

    /// Like [`People::draw_in_viewport`], additionally recording the tight
    /// screen bounding rectangles `[x0, y0, x1, y1)` of every pixel written
    /// for each drawn person and plumb bob into `written_rects`.
    pub fn draw_in_viewport_tracked(
        &mut self,
        world: &World,
        camera: &Camera,
        viewport: (u32, u32),
        canvas: &mut Canvas,
        written_rects: &mut Vec<sims_render::canvas::Rect>,
    ) {
        let drawn = self.people.draw_on_lot_in_viewport_tracked(
            world,
            camera,
            viewport,
            canvas,
            written_rects,
        );
        self.note(drawn.problems);
    }

    /// Like [`People::draw_in_viewport_tracked`], calling `held` just
    /// before each person is drawn, with their ID, attach points and the
    /// canvas, to draw what they hold first
    /// ([`sims_people::People::draw_on_lot_in_viewport_tracked_holding`]).
    pub fn draw_in_viewport_tracked_holding(
        &mut self,
        world: &World,
        camera: &Camera,
        viewport: (u32, u32),
        canvas: &mut Canvas,
        written_rects: &mut Vec<sims_render::canvas::Rect>,
        held: impl FnMut(u16, sims_render::PersonAttach, &mut Canvas),
    ) {
        let drawn = self.people.draw_on_lot_in_viewport_tracked_holding(
            world,
            camera,
            viewport,
            canvas,
            written_rects,
            held,
        );
        self.note(drawn.problems);
    }

    /// Draws the censor mosaic over the people the last draw drew whose
    /// censor word is set (`sims_render::draw_censor`), into `canvas`,
    /// which holds the lot `lot` and those people drawn with `camera`;
    /// `lot_depth` is the lot's depth from before the people, with what
    /// their held objects wrote; `restore_generation` is the game's
    /// ([`CensorRandom`]). Pushes the rectangles it changed onto `rects`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_censor(
        &mut self,
        world: &World,
        lot: &sims_render::Lot,
        camera: &Camera,
        viewport: (u32, u32),
        lot_depth: &[u16],
        restore_generation: u64,
        canvas: &mut Canvas,
        rects: &mut Vec<sims_render::canvas::Rect>,
    ) {
        let censored = self.people.censored();
        if censored.is_empty() {
            return;
        }
        let random = self.censor_random.follow(world, restore_generation);
        sims_render::draw_censor(
            lot,
            camera,
            viewport,
            censored,
            lot_depth,
            || random.next_u32(),
            canvas,
            rects,
        );
    }

    /// Where each person the last draw drew draws what they hold
    /// ([`sims_people::People::held_attach`]).
    pub fn held_attach(
        &self,
        camera: &Camera,
    ) -> std::collections::BTreeMap<u16, sims_render::PersonAttach> {
        self.people.held_attach(camera)
    }

    /// The head of `person` for the middle of the pie menu in neutral pose
    /// ([`People::head_looking`] with [`Look::neutral`]).
    pub fn head(&mut self, world: &World, person: u16, size: [u32; 2]) -> Result<Image, String> {
        self.head_looking(world, person, size, Look::neutral())
    }

    /// The head of `person` for the middle of the pie menu, rendered as
    /// `Sims.exe` renders it (`0x462290`, [`sims_people::pie_head`]):
    /// bind-pose `HEAD` parts, zoom- and rotation-invariant `56√2` px/unit
    /// orthographic projection through the house camera's 30° tilt, origin
    /// 14 px below center, pointer-tracking pitch/yaw and 8-way compass
    /// highlight turn, ambient-only lighting (`1.0` highlighted / `0.8` idle),
    /// and 5-6-5 texel expansion.
    pub fn head_looking(
        &mut self,
        world: &World,
        person: u16,
        size: [u32; 2],
        look: Look,
    ) -> Result<Image, String> {
        self.refresh(world);
        let canvas = self.people.pie_head(person, size, look)?;
        Ok(Image {
            width: canvas.width,
            height: canvas.height,
            rgba: canvas.rgba,
        })
    }

    /// Renders `person`'s pie-menu 3D head positioned at `center` inside the
    /// active house `viewport` (`0x462621` → `0x462985`), returning its
    /// top-left viewport origin, tightly bounded RGBA image, and per-pixel
    /// native 16-bit screen depths clipped to the house viewport and native
    /// near/far frustum for `zoom` (never cropped by the 201×201 face bitmap).
    pub fn head_in_viewport(
        &mut self,
        world: &World,
        person: u16,
        viewport: (u32, u32),
        center: [i32; 2],
        zoom: sims_render::Zoom,
        look: Look,
    ) -> Result<([i32; 2], Image, Vec<i32>), String> {
        self.refresh(world);
        let drawn = self.people.pie_head_in_viewport(
            person,
            [viewport.0, viewport.1],
            center,
            zoom,
            look,
        )?;
        Ok((
            drawn.origin,
            Image {
                width: drawn.canvas.width,
                height: drawn.canvas.height,
                rgba: drawn.canvas.rgba,
            },
            drawn.depth,
        ))
    }

    /// Renders `person`'s pie-menu 3D head using an animated [`HeadPose`] from
    /// [`sims_people::pie_head::PieAnimState::head_pose`] (`0x4625ae`–`0x462822`).
    pub fn head_in_viewport_pose(
        &mut self,
        world: &World,
        person: u16,
        viewport: (u32, u32),
        center: [i32; 2],
        zoom: sims_render::Zoom,
        pose: HeadPose,
    ) -> Result<([i32; 2], Image, Vec<i32>), String> {
        self.refresh(world);
        if pose.scale <= 0.0 {
            return Ok((
                center,
                Image {
                    width: 0,
                    height: 0,
                    rgba: Vec::new(),
                },
                Vec::new(),
            ));
        }
        let (posed, problems) = self.people.posed(world, i32::MAX);
        let body = posed
            .iter()
            .find(|p| p.id == person)
            .map(|p| p.body)
            .ok_or_else(|| format!("person {person} has no loaded body"))?;
        let drawn = sims_people::pie_head::draw_body_in_viewport_pose(
            body,
            [viewport.0, viewport.1],
            center,
            zoom,
            pose,
        )?;
        self.note(problems);
        Ok((
            drawn.origin,
            Image {
                width: drawn.canvas.width,
                height: drawn.canvas.height,
                rgba: drawn.canvas.rgba,
            },
            drawn.depth,
        ))
    }

    /// Keeps the problems not kept yet, up to `KEPT` (256).
    pub fn note(&mut self, problems: Vec<String>) {
        for line in problems {
            if self.problems.len() < KEPT && !self.problems.contains(&line) {
                self.problems.push(line);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shade_pie_face_desaturates_inner_disk_fades_outer_ring_and_clips_to_viewport() {
        let mut canvas = Canvas::new(260, 260, [200, 120, 40]);
        // Drawn scene pixel (depth > 0), empty sky pixel (EMPTY = 65535 > 0),
        // and native threshold pixel (depth <= 0, skipped by 0x4aca76..0x4aca7a).
        canvas.depth[130 * 260 + 130] = 1000;
        canvas.depth[130 * 260 + 131] = sims_render::EMPTY_DEPTH;
        canvas.depth[130 * 260 + 132] = 0;
        canvas.depth[130 * 260 + 133] = clamp_depth(-5);
        shade_pie_face(&mut canvas, [130, 130], (260, 200));

        // Inside inner radius 67 with depth > 0 (both drawn depth and EMPTY):
        // desaturated to grey = (200 + 120 + 40) / 6 + 8 = 68, quantized to
        // 5-6-5 with MSB bit-replication:
        // r5 = 68 >> 3 = 8 -> 66, g6 = 68 >> 2 = 17 -> 69, b5 = 8 -> 66.
        assert_eq!(canvas.pixel_rgb(130, 130), [66, 69, 66]);
        assert_eq!(canvas.pixel_rgb(131, 130), [66, 69, 66]);
        // Pixels with depth <= 0 are skipped.
        assert_eq!(canvas.pixel_rgb(132, 130), [200, 120, 40]);
        assert_eq!(canvas.pixel_rgb(133, 130), [200, 120, 40]);
        // Outer ring (d = 85): partway between desaturated grey and original.
        let ring = canvas.pixel_rgb(130 + 85, 130);
        assert!(ring[0] > 66 && ring[0] < 200, "ring red {ring:?}");
        assert!(ring[2] < 66 && ring[2] > 40, "ring blue {ring:?}");
        // Outside radius 100: untouched.
        assert_eq!(canvas.pixel_rgb(130 + 100, 130), [200, 120, 40]);
        // Below viewport height 200 (even inside circle radius 100): untouched.
        assert_eq!(canvas.pixel_rgb(130, 205), [200, 120, 40]);
    }

    #[test]
    fn composite_pie_head_clips_to_house_viewport_and_handles_negative_origins() {
        let mut canvas = Canvas::new(20, 20, [10, 20, 30]);
        let mut head = Image {
            width: 6,
            height: 6,
            rgba: vec![0; 6 * 6 * 4],
        };
        // Fill head with opaque red, except (0, 0) transparent and (2, 2) half-alpha green.
        for y in 0..6usize {
            for x in 0..6usize {
                let p = (y * 6 + x) * 4;
                head.rgba[p..p + 4].copy_from_slice(&[200, 40, 20, 255]);
            }
        }
        head.rgba[0..4].copy_from_slice(&[0, 0, 0, 0]);
        let mid = (2 * 6 + 2) * 4;
        head.rgba[mid..mid + 4].copy_from_slice(&[0, 200, 0, 128]);

        let depth = vec![12_000; 6 * 6];
        // Place across the bottom of a 20×12 house viewport.
        // Put a nearer foreground pixel at (6, 9) (depth 8,000 < 12,000) and a
        // tie at (7, 9) (depth 12,000 == 12,000, LESSEQUAL passes).
        canvas.depth[9 * 20 + 6] = 8_000;
        canvas.depth[9 * 20 + 7] = 12_000;
        composite_pie_head(&mut canvas, [4, 9], &head, &depth, (20, 12));
        // Transparent pixel at (4, 9) leaves canvas untouched.
        assert_eq!(canvas.pixel_rgb(4, 9), [10, 20, 30]);
        // Opaque pixels inside y < 12 with depth <= scene depth are written and
        // update the scene depth buffer.
        assert_eq!(canvas.pixel_rgb(5, 9), [200, 40, 20]);
        assert_eq!(canvas.depth[9 * 20 + 5], 12_000);
        // Nearer foreground at (6, 9) rejects the head pixel; equal depth at (7, 9) passes.
        assert_eq!(canvas.pixel_rgb(6, 9), [10, 20, 30]);
        assert_eq!(canvas.depth[9 * 20 + 6], 8_000);
        assert_eq!(canvas.pixel_rgb(7, 9), [200, 40, 20]);
        assert_eq!(canvas.pixel_rgb(6, 11), [5, 110, 15]);
        // Rows y >= 12 are clipped out of the house viewport.
        for y in 12..20 {
            for x in 0..20 {
                assert_eq!(canvas.pixel_rgb(x, y), [10, 20, 30], "({x}, {y})");
            }
        }

        // Negative origin clips cleanly to x >= 0, y >= 0 without panicking.
        let mut edge = Canvas::new(10, 10, [10, 20, 30]);
        composite_pie_head(&mut edge, [-2, -2], &head, &depth, (10, 10));
        assert_eq!(edge.pixel_rgb(0, 0), [5, 110, 15]);
        assert_eq!(edge.pixel_rgb(3, 3), [200, 40, 20]);
        assert_eq!(edge.pixel_rgb(4, 4), [10, 20, 30]);
    }

    #[test]
    fn safe_relative_path_normalizes_both_separators_and_rejects_traversal() {
        let base = Path::new("/tmp/cast/Characters");
        assert_eq!(
            safe_relative_path(base, r"ignored.dir\Sim.iff"),
            Some(base.join("ignored.dir").join("Sim.iff"))
        );
        assert_eq!(
            safe_relative_path(base, "sub/User00001.iff"),
            Some(base.join("sub").join("User00001.iff"))
        );
        assert_eq!(safe_relative_path(base, r"..\escape.iff"), None);
        assert_eq!(safe_relative_path(base, "/abs.iff"), None);
        assert_eq!(safe_relative_path(base, r"\abs.iff"), None);
        assert_eq!(safe_relative_path(base, r"C:\drive.iff"), None);
    }
}
