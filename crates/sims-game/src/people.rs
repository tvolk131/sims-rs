//! The people, drawn with `sims-people` into the canvas `sims-render` drew
//! the lot in, and found again by a click.
//!
//! [`sims_people::People`] places and poses each person; each is drawn
//! as `sims_people::People::draw_on_lot` draws them, far to near, every
//! pixel tested against the lot's depth (`Depth::point`, by the canvas's
//! sprite rule), so walls, furniture and the floor above hide the people
//! behind them. A pixel of a person that is drawn writes its depth into
//! the canvas, so a nearer part of the mesh drawn later covers it. People
//! on a level above the camera's are not drawn, so with the camera on the
//! second floor its floor hides everyone downstairs, as it hides the
//! rooms there. How the original gives a person's mesh its depth is
//! **unverified** (see `docs/decisions.md`, "People's meshes go into the
//! lot renderer's canvas").
//!
//! The loop is `draw_on_lot`'s, repeated here from `sims-people`'s public
//! parts because it does not say which person drew a pixel. Drawing it
//! here notes, for each pixel of the frame, the person whose pixel the
//! canvas took last: the one shown there. A click finds that person (or
//! the one shown nearest within [`REACH`]); a person hidden behind a
//! bookcase is shown nowhere, so it does not take a click meant for the
//! bookcase.

use sims_people::View;
use sims_people::raster::{Target, draw_part};
use sims_people::{math::Vec3, skin};
use sims_render::canvas::FAINT;
use sims_render::depth::Depth;
use sims_render::{Camera, Canvas};
use sims_sim::World;
use std::path::Path;

/// How many different problems are kept; later ones are dropped.
const KEPT: usize = 256;

/// How far from a pixel a person is shown in a click still finds them,
/// in pixels.
pub const REACH: i32 = 2;

/// The people of a lot, where each was shown in the last frame, and what
/// went wrong drawing them.
pub struct People {
    people: sims_people::People,
    /// The last frame's size, and for each of its pixels the person shown
    /// there.
    size: [usize; 2],
    shown: Vec<Option<u16>>,
    /// What went wrong, one line each, each said once, for the host to
    /// show.
    pub problems: Vec<String>,
}

/// A canvas that notes, for each pixel it takes, whose it is.
struct Marked<'a> {
    canvas: &'a mut Canvas,
    shown: &'a mut [Option<u16>],
    /// The person being drawn.
    person: u16,
}

impl Target for Marked<'_> {
    fn size(&self) -> (u32, u32) {
        Target::size(&*self.canvas)
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        Target::passes(&*self.canvas, x, y, z)
    }

    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], shade: f32) {
        // The canvas's rule (`Canvas::put`): a pixel fainter than FAINT or
        // farther than what is there is left out; any other is taken, a
        // tie too.
        let taken = texel[3] >= FAINT && Target::passes(&*self.canvas, x, y, z);
        Target::write(&mut *self.canvas, x, y, z, texel, shade);
        let i = y as usize * self.canvas.width + x as usize;
        if taken && let Some(shown) = self.shown.get_mut(i) {
            *shown = Some(self.person);
        }
    }
}

/// The person shown at `point` in a frame of `size` whose pixels show
/// `shown`, or else the one shown nearest to it within [`REACH`] (the
/// first in row order among the nearest).
fn shown_near(shown: &[Option<u16>], size: [usize; 2], [x, y]: [i32; 2]) -> Option<u16> {
    let [w, h] = size.map(|n| i32::try_from(n).unwrap_or(i32::MAX));
    let mut best: Option<(i32, u16)> = None;
    for py in y - REACH..=y + REACH {
        for px in x - REACH..=x + REACH {
            if !(0..w).contains(&px) || !(0..h).contains(&py) {
                continue;
            }
            let Some(Some(id)) = shown.get((py * w + px) as usize) else {
                continue;
            };
            let distance = (px - x).pow(2) + (py - y).pow(2);
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, *id));
            }
        }
    }
    best.map(|(_, id)| id)
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
    /// same camera, noting who is shown in each pixel.
    pub fn draw(&mut self, world: &World, camera: &Camera, canvas: &mut Canvas) {
        self.size = [canvas.width, canvas.height];
        self.shown.clear();
        self.shown.resize(canvas.width * canvas.height, None);
        let view = View::new(camera);
        let light = view.light();
        let depth = Depth::new(camera);
        let (mut posed, problems) = self.people.posed(world, camera.level);
        // Far to near by the root's depth, then by ID, as `draw_on_lot`
        // orders them.
        let far = |p: &sims_people::lot::Posed| {
            let (fine, elevation) = view.place(p.root.translation);
            depth.point(fine, elevation, p.level)
        };
        posed.sort_by(|a, b| far(b).total_cmp(&far(a)).then(a.id.cmp(&b.id)));
        let mut marked = Marked {
            canvas,
            shown: &mut self.shown,
            person: 0,
        };
        for person in &posed {
            marked.person = person.id;
            for part in person.body.parts.iter().filter(|p| p.censor == 0) {
                let skinned = skin(part, &person.bones);
                draw_part(&mut marked, part, &skinned, light, |p| {
                    let (fine, elevation) = view.place(p);
                    let [x, y] = view.screen(fine, elevation, person.level);
                    Vec3::new(x, y, depth.point(fine, elevation, person.level))
                });
            }
        }
        drop(posed);
        self.note(problems);
    }

    /// The person a press at `point` (canvas pixels) lands on: the one
    /// shown there in the last frame drawn, which is the frame on screen,
    /// or the one shown nearest within [`REACH`], if `world` still holds
    /// them as a person.
    pub fn at(&self, world: &World, point: [i32; 2]) -> Option<u16> {
        shown_near(&self.shown, self.size, point)
            .filter(|id| world.objects.get(id).is_some_and(|o| o.person.is_some()))
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
    fn each_pixel_shows_the_person_the_canvas_took_last() {
        let mut canvas = Canvas::new(4, 3, [0, 0, 0]);
        let mut shown = vec![None; 12];
        let mut marked = Marked {
            canvas: &mut canvas,
            shown: &mut shown,
            person: 7,
        };
        let opaque = [200, 100, 50, 255];
        marked.write(1, 2, 10.0, opaque, 1.0);
        // Too faint to be taken.
        marked.write(2, 0, 10.0, [200, 100, 50, FAINT - 1], 1.0);
        // A nearer person takes the pixel; a farther one does not.
        marked.person = 8;
        marked.write(1, 2, 5.0, opaque, 1.0);
        marked.person = 9;
        marked.write(1, 2, 6.0, opaque, 1.0);
        // A tie is taken, even when it changes nothing in the canvas.
        marked.person = 10;
        marked.write(1, 2, 5.0, opaque, 1.0);
        let taken: Vec<(usize, u16)> = (0..shown.len())
            .filter_map(|i| shown[i].map(|id| (i, id)))
            .collect();
        assert_eq!(taken, vec![(2 * 4 + 1, 10)]);
    }

    #[test]
    fn a_click_finds_the_person_shown_there_or_nearest() {
        let size = [10, 10];
        let mut shown = vec![None; 100];
        shown[5 * 10 + 5] = Some(3);
        shown[5 * 10 + 6] = Some(4);
        shown[5 * 10 + 8] = Some(5);
        assert_eq!(shown_near(&shown, size, [5, 5]), Some(3));
        assert_eq!(shown_near(&shown, size, [6, 5]), Some(4));
        assert_eq!(shown_near(&shown, size, [8, 7]), Some(5));
        // Beyond REACH of every shown pixel, or off the frame.
        assert_eq!(shown_near(&shown, size, [1, 1]), None);
        assert_eq!(shown_near(&shown, size, [-50, 5]), None);
        assert_eq!(shown_near(&[], [0, 0], [0, 0]), None);
    }
}
