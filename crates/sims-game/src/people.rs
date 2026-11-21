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
use sims_people::{math::Vec3, skin};
use sims_render::{Camera, Canvas};
use sims_sim::World;
use std::path::Path;

/// How much of the pie's face the head fills, across or down, whichever
/// is reached first.
pub const HEAD_FILL: f32 = 0.8;

/// The direction a skeleton's face looks, in the root's frame: the root
/// turns it to the person's direction ([`sims_people::placement`]).
const FACE_FORWARD: Vec3 = Vec3::new(0.0, 0.0, 1.0);

/// How far down the pie's camera looks at the head: sin and cos of 10
/// degrees, this crate's choice.
const HEAD_PITCH: [f32; 2] = [0.173_648_18, 0.984_807_7];

/// `posed`'s head (the person `person`) drawn as [`People::head`] says.
fn head(
    posed: Option<&sims_people::lot::Posed>,
    person: u16,
    [width, height]: [u32; 2],
) -> Result<Image, String> {
    let posed = posed.ok_or_else(|| format!("person {person} is not posed on the lot"))?;
    let bone = sims_people::rig::bone(&posed.body.skeleton, "HEAD")
        .ok_or_else(|| format!("person {person}'s skeleton has no HEAD bone"))?;
    let parts: Vec<_> = posed
        .body
        .parts
        .iter()
        .filter(|p| p.censor == 0 && p.bone == bone)
        .map(|p| (p, skin(p, &posed.bones)))
        .collect();
    let count: usize = parts.iter().map(|(_, s)| s.positions.len()).sum();
    if count == 0 {
        return Err(format!("person {person} has no head to draw"));
    }
    // The camera looks the face in the eye: along the face's direction,
    // turned back.
    let face = posed.root.rotation.rotate(FACE_FORWARD);
    let flat = Vec3::new(face.x, 0.0, face.z).normalize();
    let mut camera = sims_people::raster::Camera {
        yaw: [-flat.x, -flat.z],
        pitch: HEAD_PITCH,
        scale: 1.0,
        center: Vec3::ZERO,
    };
    // Centred on the head as the camera sees it, and scaled to fill.
    let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);
    for (_, skinned) in &parts {
        for &p in &skinned.positions {
            let v = camera.view(p);
            low = [low[0].min(v.x), low[1].min(v.y)];
            high = [high[0].max(v.x), high[1].max(v.y)];
        }
    }
    let middle = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0];
    // A view-space point back in the world: the camera's right and up.
    let [sy, cy] = camera.yaw;
    let [sp, cp] = camera.pitch;
    let right = Vec3::new(cy, 0.0, -sy);
    let up = Vec3::new(sy * sp, cp, cy * sp);
    camera.center = right * middle[0] + up * middle[1];
    let extent = [(high[0] - low[0]).max(1e-6), (high[1] - low[1]).max(1e-6)];
    camera.scale = HEAD_FILL * (width as f32 / extent[0]).min(height as f32 / extent[1]);
    let mut canvas = sims_people::raster::Canvas::new(width, height, [0; 4]);
    let forward = camera.forward();
    let light = Vec3::new(-forward.x, 1.0, -forward.z);
    for (part, skinned) in &parts {
        canvas.draw(&camera, part, skinned, light);
    }
    Ok(Image {
        width,
        height,
        rgba: canvas.rgba,
    })
}

/// How many different problems are kept; later ones are dropped.
const KEPT: usize = 256;

/// The people of a lot, and what went wrong drawing them.
pub struct People {
    people: sims_people::People,
    /// What went wrong, one line each, each said once, for the host to
    /// show.
    pub problems: Vec<String>,
}

impl People {
    /// Reads the people's files from the install at `root`, and the cast
    /// of the neighborhood `house` is in.
    pub fn open(root: &Path, house: &Path) -> Result<Self, String> {
        Ok(Self {
            people: sims_people::People::open(root, house)?,
            problems: Vec::new(),
        })
    }

    /// Follows `world`: loads the people who appeared or changed clothes
    /// and forgets those who left.
    pub fn refresh(&mut self, world: &World) {
        let problems = self.people.update(world);
        self.note(problems);
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

    /// The head of `person` as posed now, seen from in front, for the
    /// middle of the pie menu: an image of `size` pixels, clear where the
    /// head is not. The parts bound to the `HEAD` bone are drawn (the head
    /// with its hair, and glasses and the like), filling [`HEAD_FILL`] of
    /// the image, lit from the front and above.
    ///
    /// The original renders the head live into the face bitmap; its camera,
    /// light and size there are not ported, and the head here is **not**
    /// compared with the original's.
    pub fn head(&mut self, world: &World, person: u16, size: [u32; 2]) -> Result<Image, String> {
        let (posed, problems) = self.people.posed(world, i32::MAX);
        let result = head(posed.iter().find(|p| p.id == person), person, size);
        drop(posed);
        self.note(problems);
        result
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
