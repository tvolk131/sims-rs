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
//! person's mesh its depth is **unverified** (see `docs/decisions.md`,
//! "People's meshes go into the lot renderer's canvas").

use sims_render::{Camera, Canvas};
use sims_sim::World;
use std::path::Path;

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

    /// Draws every placed person on the levels `camera` shows into
    /// `canvas`, which holds the lot `sims_render::draw` drew with the
    /// same camera, each pixel naming the person drawn in it.
    pub fn draw(&mut self, world: &World, camera: &Camera, canvas: &mut Canvas) {
        let drawn = self.people.draw_on_lot(world, camera, canvas);
        self.note(drawn.problems);
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
