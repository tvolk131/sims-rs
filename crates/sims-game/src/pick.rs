//! Which person or object a click on the lot lands on: what the frame
//! last drawn shows there.
//!
//! The lot and its people are drawn into one canvas, which notes for
//! each pixel the object or person whose pixel it took last
//! ([`Canvas::id_at`]; `sims_render::draw_lot` and `draw_held` name
//! object sprites,
//! `sims_people::People::draw_on_lot` names people, and terrain, floors
//! and walls name nobody). So a person or object hidden behind a wall or
//! a bookcase is shown nowhere and takes no click meant for what hides
//! it.
//!
//! People are thin, so a click finds the person shown at the pixel or
//! nearest to it within [`REACH`] first; only then the object shown at
//! the pixel itself. Each tile of a multi-tile object is its own object
//! and is picked as itself, not as its group's lead: that is the object
//! the pie menu is listed on, and the simulation sends the interactions
//! of a tile on to the tile that takes them, as the original does
//! (`crate::menu::target`, 0x435286). How the original resolves a click
//! to an object or a Sim is **unverified** (see `docs/decisions/`, "A
//! click picks what the frame shows at the pixel").

use sims_render::Canvas;
use sims_sim::World;

/// How far from a pixel a person is shown in a click still finds them,
/// in pixels.
pub const REACH: i32 = 2;

/// The person or object a press at `point` (canvas pixels) lands on in
/// `canvas`, the frame last drawn of `world`: the person shown nearest
/// within [`REACH`] ([`person`]), else the object shown there
/// ([`object`]).
pub fn at(world: &World, canvas: &Canvas, point: [i32; 2]) -> Option<u16> {
    choose(canvas, point, |id| kind(world, id))
}

/// The person shown at `point` in `canvas`, or else the one shown nearest
/// to it within [`REACH`] (the first in row order among the nearest), if
/// `world` holds them as a person.
pub fn person(world: &World, canvas: &Canvas, point: [i32; 2]) -> Option<u16> {
    nearest(canvas, point, |id| kind(world, id) == Some(Kind::Person))
}

/// The object, not a person, shown at `point` in `canvas`, if `world`
/// still holds it.
pub fn object(world: &World, canvas: &Canvas, [x, y]: [i32; 2]) -> Option<u16> {
    canvas
        .id_at(x, y)
        .filter(|&id| kind(world, id) == Some(Kind::Object))
}

/// What an ID a pixel names is in the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Person,
    Object,
}

/// What `world` holds as `id`, if anything.
fn kind(world: &World, id: u16) -> Option<Kind> {
    world.objects.get(&id).map(|o| match o.person {
        Some(_) => Kind::Person,
        None => Kind::Object,
    })
}

/// [`at`], with `kind` saying what each ID is.
fn choose(canvas: &Canvas, [x, y]: [i32; 2], kind: impl Fn(u16) -> Option<Kind>) -> Option<u16> {
    nearest(canvas, [x, y], |id| kind(id) == Some(Kind::Person)).or_else(|| {
        canvas
            .id_at(x, y)
            .filter(|&id| kind(id) == Some(Kind::Object))
    })
}

/// The ID `keep` takes shown nearest to `point` within [`REACH`], the
/// first in row order among the nearest.
fn nearest(canvas: &Canvas, [x, y]: [i32; 2], keep: impl Fn(u16) -> bool) -> Option<u16> {
    let mut best: Option<(i32, u16)> = None;
    for py in y - REACH..=y + REACH {
        for px in x - REACH..=x + REACH {
            let Some(id) = canvas.id_at(px, py).filter(|&id| keep(id)) else {
                continue;
            };
            let distance = (px - x).pow(2) + (py - y).pow(2);
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, id));
            }
        }
    }
    best.map(|(_, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// People 3, 4 and 5 and objects 7 and 8; nothing else.
    fn kind(id: u16) -> Option<Kind> {
        match id {
            3..=5 => Some(Kind::Person),
            7 | 8 => Some(Kind::Object),
            _ => None,
        }
    }

    /// A 10 by 10 canvas with `pixels` drawn as `(x, y, id)`.
    fn canvas(pixels: &[(i32, i32, u16)]) -> Canvas {
        let mut canvas = Canvas::new(10, 10, [0, 0, 0]);
        for &(x, y, id) in pixels {
            canvas.set_id(Some(id));
            assert!(canvas.put(x, y, [9, 9, 9, 255], 0));
        }
        canvas
    }

    #[test]
    fn a_click_finds_the_person_shown_there_or_nearest() {
        let canvas = canvas(&[(5, 5, 3), (6, 5, 4), (8, 5, 5), (5, 6, 7)]);
        let person = |point| nearest(&canvas, point, |id| kind(id) == Some(Kind::Person));
        assert_eq!(person([5, 5]), Some(3));
        assert_eq!(person([6, 5]), Some(4));
        assert_eq!(person([8, 7]), Some(5));
        // Object 7's pixel is nearer, but is nobody's.
        assert_eq!(person([5, 7]), Some(3));
        // Beyond REACH of every shown pixel, or off the frame.
        assert_eq!(person([1, 1]), None);
        assert_eq!(person([-50, 5]), None);
    }

    #[test]
    fn a_click_finds_a_person_near_before_the_object_shown_there() {
        // Object 7 at (2, 2) beside person 3; object 8 alone at (8, 8);
        // object 9, which the world does not hold, at (0, 9).
        let canvas = canvas(&[(2, 2, 7), (3, 2, 3), (8, 8, 8), (0, 9, 9)]);
        assert_eq!(choose(&canvas, [2, 2], kind), Some(3));
        assert_eq!(choose(&canvas, [8, 8], kind), Some(8));
        // Off the object by a pixel, nobody's pixel names nothing.
        assert_eq!(choose(&canvas, [8, 7], kind), None);
        assert_eq!(choose(&canvas, [0, 9], kind), None);
    }
}
