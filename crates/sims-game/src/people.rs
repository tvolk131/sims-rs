//! The people, drawn by `sims-people` into the canvas `sims-render` drew
//! the lot in, and found again by a click.
//!
//! [`sims_people::People`] places and poses each person and tests every
//! pixel against the lot's depth, so walls, furniture and the floor above
//! hide the people behind them. A pixel of a person that is drawn writes
//! its depth into the canvas, so a nearer part of the mesh drawn later
//! covers it. People on a level above the camera's are not drawn, so with
//! the camera on the second floor its floor hides everyone downstairs, as
//! it hides the rooms there. How the original gives a person's mesh its
//! depth is **unverified** (see `docs/decisions.md`, "People's meshes go
//! into the lot renderer's canvas").
//!
//! Each frame also marks the pixels a person was drawn in. A click finds
//! a person only on (or within [`REACH`] of) such a pixel, so a person
//! hidden behind a bookcase does not take a click meant for the bookcase.

use sims_people::View;
use sims_people::raster::Target;
use sims_render::depth::Depth;
use sims_render::{Camera, Canvas};
use sims_sim::World;
use std::path::Path;

/// How many different problems are kept; later ones are dropped.
const KEPT: usize = 256;

/// How far from a pixel a person was drawn in a click still finds them,
/// in pixels.
pub const REACH: i32 = 2;

/// The people of a lot, where they were last drawn, and what went wrong
/// drawing them.
pub struct People {
    people: sims_people::People,
    /// The last frame's size, and for each of its pixels whether a person
    /// was drawn in it.
    size: [usize; 2],
    shown: Vec<bool>,
    /// What went wrong, one line each, each said once, for the host to
    /// show.
    pub problems: Vec<String>,
}

/// A canvas that notes which pixels the people are drawn in.
struct Marked<'a> {
    canvas: &'a mut Canvas,
    shown: &'a mut [bool],
}

impl Target for Marked<'_> {
    fn size(&self) -> (u32, u32) {
        Target::size(&*self.canvas)
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        Target::passes(&*self.canvas, x, y, z)
    }

    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], shade: f32) {
        let i = y as usize * self.canvas.width + x as usize;
        let before = (
            self.canvas.color.get(i).copied(),
            self.canvas.depth.get(i).copied(),
        );
        Target::write(&mut *self.canvas, x, y, z, texel, shade);
        // The canvas leaves out a pixel too faint or too far; one it
        // takes changes its colour or its depth.
        let after = (
            self.canvas.color.get(i).copied(),
            self.canvas.depth.get(i).copied(),
        );
        if after != before
            && let Some(shown) = self.shown.get_mut(i)
        {
            *shown = true;
        }
    }
}

impl People {
    /// Reads the people's files from the install at `root`, and the cast
    /// of the neighborhood `house` is in.
    pub fn open(root: &Path, house: &Path) -> Result<Self, String> {
        Ok(Self {
            people: sims_people::People::open(root, house)?,
            size: [0, 0],
            shown: Vec::new(),
            problems: Vec::new(),
        })
    }

    /// Follows `world`: loads the people who appeared or changed clothes
    /// and forgets those who left.
    pub fn refresh(&mut self, world: &World) {
        let problems = self.people.update(world);
        self.note(problems);
    }

    /// Draws every placed person on the levels `camera` shows into
    /// `canvas`, which holds the lot `sims_render::draw` drew with the
    /// same camera, as `sims_people::People::draw_on_lot` does, noting
    /// the pixels drawn.
    pub fn draw(&mut self, world: &World, camera: &Camera, canvas: &mut Canvas) {
        self.size = [canvas.width, canvas.height];
        self.shown.clear();
        self.shown.resize(canvas.width * canvas.height, false);
        let depth = Depth::new(camera);
        let mut marked = Marked {
            canvas,
            shown: &mut self.shown,
        };
        let drawn = self
            .people
            .draw(world, camera, &mut marked, |fine, elevation, level| {
                depth.point(fine, elevation, level)
            });
        self.note(drawn.problems);
    }

    /// Whether a person was drawn within [`REACH`] of `point` in the last
    /// frame.
    fn near_shown(&self, [x, y]: [i32; 2]) -> bool {
        let [w, h] = self.size.map(|n| i32::try_from(n).unwrap_or(i32::MAX));
        (y - REACH..=y + REACH).any(|y| {
            (x - REACH..=x + REACH).any(|x| {
                (0..w).contains(&x) && (0..h).contains(&y) && self.shown[(y * w + x) as usize]
            })
        })
    }

    /// The person a press at `point` (canvas pixels) lands on: one drawn
    /// there in the last frame, the nearest of those whose standing box
    /// (half a tile wide, most of a storey tall, over where they stand, as
    /// `sims-people` places them) holds the point.
    pub fn at(&mut self, world: &World, camera: &Camera, point: [i32; 2]) -> Option<u16> {
        if !self.near_shown(point) {
            return None;
        }
        let half = (camera.zoom.tile_width() / 4) as f32;
        let tall = camera.zoom.floor_height() as f32 * 0.8;
        let view = View::new(camera);
        let depth = Depth::new(camera);
        let (posed, _) = self.people.posed(world, camera.level);
        let [px, py] = point.map(|v| v as f32 + 0.5);
        posed
            .iter()
            .filter_map(|person| {
                let (fine, elevation) = view.place(person.root.translation);
                let [sx, sy] = view.screen(fine, elevation, person.level);
                let (dx, dy) = (px - sx, sy - py);
                (dx.abs() <= half && dy >= -half / 2.0 && dy <= tall)
                    .then(|| (depth.point(fine, elevation, person.level), person.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, id)| id)
    }

    /// Keeps the problems not kept yet, up to [`KEPT`].
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
    fn a_marked_canvas_notes_only_the_pixels_it_takes() {
        let mut canvas = Canvas::new(4, 3, [0, 0, 0]);
        let mut shown = vec![false; 12];
        let mut marked = Marked {
            canvas: &mut canvas,
            shown: &mut shown,
        };
        // Taken: an opaque pixel nearer than the empty canvas.
        marked.write(1, 2, 10.0, [200, 100, 50, 255], 1.0);
        // Left out: a fully transparent one.
        marked.write(2, 0, 10.0, [200, 100, 50, 0], 1.0);
        let taken: Vec<usize> = (0..shown.len()).filter(|&i| shown[i]).collect();
        assert_eq!(taken, vec![2 * 4 + 1]);
    }
}
