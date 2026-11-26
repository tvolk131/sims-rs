//! Draws a lot: terrain, floors, walls and objects, level by level.

use crate::assets::Assets;
use crate::camera::{self, Camera, Walls, Zoom};
#[cfg(test)]
use crate::canvas::Image;
use crate::canvas::{Canvas, CanvasBand, Image as CanvasImage, RowSpans, clip_span, clip_x};
use crate::caps::{self, Cap, Corners, Line, solid};
use crate::cutaway::{self, Mask};
use crate::depth::{Anchor, Depth};
use crate::grass::{self, Grass};
use crate::light::Light;
use crate::lot::{self, Lot, WallCell};
use crate::pools::{self, Pools};
use crate::shadows;
use sims_sim::{CowMap, Object, World};
use std::collections::{BTreeMap, BTreeSet};

/// Rows between a tile's left corner and the top of its first grass
/// column (matched against the original's screen).
const GRASS_ROW: i32 = 0;
const GRASS_COLUMN: i32 = 1;

/// Object sprites sit one pixel right of the point the camera projects
/// them to, measured against the walls and floors around them. The camera
/// origin is fitted to the tile art, and the original places the two
/// kinds of art one pixel apart; at every compared camera (far and
/// medium zoom) the gap is this same pixel. Near zoom is unverified (see
/// the decisions log, 2026-11-14).
const OBJECT_COLUMN: i32 = 1;

/// What the window shows outside the lot by day: the original's grey
/// (`0x737373`, 5-6-5 red 14, green 28, blue 14), widened to 8 bits a
/// channel. See [`background`] for other times of day and house 99.
pub const BACKGROUND: [u8; 3] = [115, 113, 115];

/// What the window shows outside the lot (0x4aa9bd, filled by 0x4aaa26):
/// chosen from the house number (global 10) and the time of day (global 4),
/// then packed into the original's 16-bit display mode (0x49f0ae) and
/// widened back to 8 bits a channel.
///
/// House 99 (the cloud lot) takes `0x14148c` at every time of day. Any
/// other house takes `0x737373` by day (0), `0x404040` at dusk or dawn
/// (1 or 3), `0x1a1a1a` at night (2) and `0x0000ff` for any other value.
pub fn background(time_of_day: i16, house: i16) -> [u8; 3] {
    let rgb = if house == 99 {
        [0x14, 0x14, 0x8c]
    } else {
        match time_of_day {
            0 => [0x73, 0x73, 0x73],
            1 | 3 => [0x40, 0x40, 0x40],
            2 => [0x1a, 0x1a, 0x1a],
            _ => [0x00, 0x00, 0xff],
        }
    };
    crate::canvas::quantize565(rgb)
}

/// Graphics options that control how [`Painter`] renders the lot
/// (`cTSOptions`: object floor shadows `[0x6f0188]` at `0x4e5fb2`/`0x4ae846`,
/// dynamic lamp light pools `[0x6f018a]` at `0x4e5fd5`/`0x4c7c1a`/`0x4c835c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    /// Cast object floor shadows (`[0x6f0188]`).
    pub shadows: bool,
    /// Render dynamic lamp pools on floors and walls (`[0x6f018a]` at
    /// `0x4c7c1a` and `0x4c835c`); base room shading always applies.
    pub lighting: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            shadows: true,
            lighting: true,
        }
    }
}

/// Counts of what one frame drew, for tests and the viewer's status line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub floors: usize,
    pub walls: usize,
    pub objects: usize,
    /// Wall faces whose shape or pattern the game files lack.
    pub missing_walls: usize,
    /// Objects with no sprites for this view.
    pub missing_objects: usize,
}

/// Draws the lot as the camera sees it, every object included. `world`
/// is only read. A frame with people in it draws in three passes instead:
/// [`draw_lot`], the people, then [`draw_held`].
pub fn draw(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    canvas: &mut Canvas,
) -> Stats {
    let viewport = (canvas.width as u32, canvas.height as u32);
    paint(
        world,
        lot,
        assets,
        camera,
        viewport,
        None,
        RenderOptions::default(),
        canvas,
        Pass::All,
    )
}

/// Draws the lot as [`draw`] does, but leaves out the objects that hold a
/// person ([`holds_person`]): the original draws those each frame after
/// its people, not into the lot's picture under them (0x4aacb9).
pub fn draw_lot(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    canvas: &mut Canvas,
) -> Stats {
    let viewport = (canvas.width as u32, canvas.height as u32);
    paint(
        world,
        lot,
        assets,
        camera,
        viewport,
        None,
        RenderOptions::default(),
        canvas,
        Pass::Lot,
    )
}

/// Draws the lot as [`draw_lot`] does, clipping grass columns and blades to
/// the active house `viewport` height (for example `800×500` when the live
/// control strip is expanded, matching `0x49aed6`–`0x49aee2`, `0x4c44dd`–
/// `0x4c4547`, and `0x49dd0a`/`0x49df19`).
pub fn draw_lot_in_viewport(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    viewport: (u32, u32),
    canvas: &mut Canvas,
) -> Stats {
    paint(
        world,
        lot,
        assets,
        camera,
        viewport,
        None,
        RenderOptions::default(),
        canvas,
        Pass::Lot,
    )
}

/// Draws the lot as [`draw_lot_in_viewport`] does, also rendering an invalid
/// Buy Mode preview's off-lot held object at its on-lot `"Tile Cursor"`
/// position (`0x42242f`) when `preview = Some((cursor_lead, held_lead))`.
pub fn draw_lot_with_preview(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    viewport: (u32, u32),
    preview: Option<(u16, u16)>,
    canvas: &mut Canvas,
) -> Stats {
    paint(
        world,
        lot,
        assets,
        camera,
        viewport,
        preview,
        RenderOptions::default(),
        canvas,
        Pass::Lot,
    )
}

/// Draws the lot like [`draw_lot_in_viewport`], applying [`RenderOptions`]
/// (`shadows` `[0x6f0188]` and `lighting` `[0x6f018a]`).
pub fn draw_lot_in_viewport_with_options(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    viewport: (u32, u32),
    options: RenderOptions,
    canvas: &mut Canvas,
) -> Stats {
    paint(
        world,
        lot,
        assets,
        camera,
        viewport,
        None,
        options,
        canvas,
        Pass::Lot,
    )
}

/// Draws the lot like [`draw_lot_with_preview`], applying [`RenderOptions`]
/// (`shadows` `[0x6f0188]` and `lighting` `[0x6f018a]`).
#[allow(clippy::too_many_arguments)]
pub fn draw_lot_with_preview_and_options(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    viewport: (u32, u32),
    preview: Option<(u16, u16)>,
    options: RenderOptions,
    canvas: &mut Canvas,
) -> Stats {
    paint(
        world,
        lot,
        assets,
        camera,
        viewport,
        preview,
        options,
        canvas,
        Pass::Lot,
    )
}

/// Draws the lot like [`draw_lot_with_preview_and_options`] using an explicit
/// horizontal-band worker count (`bands.max(1)`), used by parity tests to prove
/// 0-pixel-diff equivalence between single-threaded (`1`) and multi-threaded
/// band rasterization.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn draw_lot_with_band_count(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    viewport: (u32, u32),
    preview: Option<(u16, u16)>,
    options: RenderOptions,
    canvas: &mut Canvas,
    bands: usize,
) -> Stats {
    let grass = Grass::new(lot, camera);
    let held = holds_person(world);
    paint_with_precomputed(
        world,
        lot,
        assets,
        camera,
        viewport,
        preview,
        options,
        canvas,
        Pass::Lot,
        &grass,
        held,
        None,
        None,
        Some(bands.max(1)),
        false,
        |_, _| {},
    )
}

/// Draws the objects [`draw_lot`] left out, over a canvas that holds the
/// lot and then the people, each pixel tested against the depth there:
/// the original's per-frame pass draws its moving objects after its
/// people (0x4a9458, then 0x4aa1d7 from 0x4a94ab). So an in-use bench's
/// backrest in front of the Sim on it covers the Sim, and its soft top
/// edge blends over the Sim's clothes, not over the floor behind.
pub fn draw_held(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    canvas: &mut Canvas,
) -> Stats {
    draw_held_with_options(world, lot, assets, camera, RenderOptions::default(), canvas)
}

/// Like [`draw_held`], applying [`RenderOptions`].
pub fn draw_held_with_options(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    options: RenderOptions,
    canvas: &mut Canvas,
) -> Stats {
    draw_held_with_attach(
        world,
        lot,
        assets,
        camera,
        options,
        &BTreeMap::new(),
        canvas,
    )
}

/// Where a person draws what its container slots hold, in the lot's
/// unrotated fine units and height units (`[fine x, fine y, altitude]`, as
/// an object's pose): the person's attach points.
///
/// The generic slot walk skips a person holder (`0x437c56`: definition
/// `+0x12 == 2`). Instead the person's own draw (`0x459dc0`..`0x45a131`)
/// draws each held object at an attach point of its animator before
/// drawing the person (`0x59185d` → `0x59293d` → `0x4b966b`, then the
/// object's draw). The attach points are bone-local points of the posed
/// skeleton, set from the person's body strings (`0x5924a4`); see
/// `sims_people::attach`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PersonAttach {
    /// Attach point 0 (animator `+0x140`): `R_HAND`'s "Palm".
    pub palm: Option<[f32; 3]>,
    /// Attach points 1 and 2 (`+0x144`): `HEAD`'s "Top".
    pub head_top: Option<[f32; 3]>,
    /// Attach point 3 (`+0x148`): `PELVIS`.
    pub pelvis: Option<[f32; 3]>,
}

impl PersonAttach {
    /// The attach point the person's draw uses for its container slot
    /// `slot` (as scripts count them): slot 0 (the carry slot) at point 0,
    /// slot 2 at point 3, slot 3 at point 1, slot 4 at point 0
    /// (`0x459e25`, `0x459ecf`, `0x459f7b`, `0x45a027`). Slot 1, the
    /// "HEAD" record, is not drawn there: the animator's tick moves an
    /// animated object held in it (the tail of `0x58dc72`), which is not
    /// ported.
    pub fn for_slot(&self, slot: usize) -> Option<[f32; 3]> {
        match slot {
            0 | 4 => self.palm,
            2 => self.pelvis,
            3 => self.head_top,
            _ => None,
        }
    }

    /// What the person's draw does with its slot `slot`: draws it at a
    /// point, skips it because the point is unbound (`0x591874`), or (slot
    /// 1 and slots past 4) does not draw it at all, which is `None`.
    fn slot_point(&self, slot: usize) -> Option<SlotPoint> {
        let point = match slot {
            0 | 4 => self.palm,
            2 => self.pelvis,
            3 => self.head_top,
            _ => return None,
        };
        Some(point.map_or(SlotPoint::Unbound, SlotPoint::At))
    }
}

/// Where the person's draw puts what one of its slots holds.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SlotPoint {
    /// At this attach point.
    At([f32; 3]),
    /// Nowhere: the slot's point is unbound, so the person's draw skips the
    /// slot (`0x459e2c`, `0x591874`) and what it holds is not drawn.
    Unbound,
}

/// Like [`draw_held_with_options`], drawing what each person in `attach`
/// holds at that person's attach points ([`PersonAttach`]). What sits in a
/// slot whose point is unbound is not drawn, as in the original. Objects
/// held by a person missing from `attach`, or in slot 1, stay at their own
/// position.
pub fn draw_held_with_attach(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    options: RenderOptions,
    attach: &BTreeMap<u16, PersonAttach>,
    canvas: &mut Canvas,
) -> Stats {
    draw_held_except(
        world,
        lot,
        assets,
        camera,
        options,
        attach,
        &BTreeSet::new(),
        canvas,
    )
}

/// Like [`draw_held_with_attach`], leaving out `drawn`: what the people's
/// own draws already drew ([`draw_held_by`]).
#[allow(clippy::too_many_arguments)]
pub fn draw_held_except(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    options: RenderOptions,
    attach: &BTreeMap<u16, PersonAttach>,
    drawn: &BTreeSet<u16>,
    canvas: &mut Canvas,
) -> Stats {
    let mut held = holds_person(world);
    held.retain(|id| !drawn.contains(id));
    draw_set(world, lot, assets, camera, options, attach, held, canvas)
}

/// What `person` holds, as the person's own draw draws it (`0x459dc0`):
/// the objects in the person's slots, and what those hold, at every
/// depth, but no person and nothing a person in it holds. As in
/// [`holds_person`], a slot list's first entry links to the next object on
/// the tile and is not a slot.
pub fn held_by(world: &World, person: u16) -> BTreeSet<u16> {
    held_by_in(&world.objects, person, |id| {
        world.objects.get(&id).is_some_and(|o| o.person.is_some())
    })
}

/// [`held_by`] for `objects`, `is_person` telling the people.
fn held_by_in(
    objects: &CowMap<u16, Object>,
    person: u16,
    is_person: impl Fn(u16) -> bool,
) -> BTreeSet<u16> {
    let registered = |w: i16| {
        u16::try_from(w)
            .ok()
            .filter(|&id| id != 0 && objects.contains_key(&id))
    };
    let slots = |id: u16| {
        objects
            .get(&id)
            .into_iter()
            .flat_map(|o| o.slots.iter().skip(1).filter_map(|s| registered(s.object)))
    };
    let mut tree = BTreeSet::new();
    let mut stack: Vec<u16> = slots(person).collect();
    while let Some(id) = stack.pop() {
        if id == person || is_person(id) || !tree.insert(id) {
            continue;
        }
        stack.extend(slots(id));
    }
    tree
}

/// Draws what `person` holds ([`held_by`]) at the person's attach points
/// `attach`, as the person's own draw does just before it draws the person
/// (`0x459e45`..`0x45a027`, then the person at `0x45a131`). Returns what it
/// drew, for [`draw_held_except`] to leave out.
#[allow(clippy::too_many_arguments)]
pub fn draw_held_by(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    options: RenderOptions,
    person: u16,
    attach: PersonAttach,
    canvas: &mut Canvas,
) -> BTreeSet<u16> {
    let held = held_by(world, person);
    if !held.is_empty() {
        let attach = BTreeMap::from([(person, attach)]);
        draw_set(
            world,
            lot,
            assets,
            camera,
            options,
            &attach,
            held.clone(),
            canvas,
        );
    }
    held
}

/// The held pass over the objects `held`.
#[allow(clippy::too_many_arguments)]
fn draw_set(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    options: RenderOptions,
    attach: &BTreeMap<u16, PersonAttach>,
    held: BTreeSet<u16>,
    canvas: &mut Canvas,
) -> Stats {
    if held.is_empty() {
        return Stats::default();
    }
    let viewport = (canvas.width as u32, canvas.height as u32);
    let mut painter = Painter::new(
        world,
        lot,
        assets,
        camera,
        viewport,
        None,
        options,
        canvas,
        Pass::Held,
        None,
        held,
        Some(1),
    );
    painter.person_attach = attach.clone();
    let top = camera.level.clamp(1, 2);
    for level in 1..=top {
        let cut = cut(lot, camera, level, top, None);
        painter.objects(level, &cut);
    }
    painter.flush_commands();
    painter.stats
}

/// Which objects a pass draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Every object, in the lot's own pass.
    All,
    /// The objects that hold no person.
    Lot,
    /// The objects that hold a person, after the people.
    Held,
}

#[allow(clippy::too_many_arguments)]
fn paint(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    viewport: (u32, u32),
    preview: Option<(u16, u16)>,
    options: RenderOptions,
    canvas: &mut Canvas,
    pass: Pass,
) -> Stats {
    let grass = Grass::new(lot, camera);
    let held = if pass == Pass::All {
        BTreeSet::new()
    } else {
        holds_person(world)
    };
    paint_with_precomputed(
        world,
        lot,
        assets,
        camera,
        viewport,
        preview,
        options,
        canvas,
        pass,
        &grass,
        held,
        None,
        None,
        None,
        false,
        |_, _| {},
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_lot_with_env_hook(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    viewport: (u32, u32),
    preview: Option<(u16, u16)>,
    options: RenderOptions,
    canvas: &mut Canvas,
    grass: &Grass,
    held: BTreeSet<u16>,
    cut_mask: Option<&Mask>,
    level1_openings: Option<&BTreeMap<Edge, (usize, u16)>>,
    after_level1_env: impl FnMut(&Canvas, Stats),
) -> Stats {
    paint_with_precomputed(
        world,
        lot,
        assets,
        camera,
        viewport,
        preview,
        options,
        canvas,
        Pass::Lot,
        grass,
        held,
        cut_mask,
        level1_openings,
        None,
        true,
        after_level1_env,
    )
}

/// Completes `Pass::Lot` starting from a canvas already populated with level-1's
/// static environment (`clear` + `floors(1)` + `walls(1)` + `trims(1)`) and its
/// accumulated [`Stats`]. Draws level-1 non-held objects, plus all of level 2
/// when `camera.level == 2`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_lot_from_env(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    viewport: (u32, u32),
    preview: Option<(u16, u16)>,
    options: RenderOptions,
    canvas: &mut Canvas,
    held: BTreeSet<u16>,
    cut_mask: Option<&Mask>,
    env_stats: Stats,
) -> Stats {
    let mut painter = Painter::new(
        world,
        lot,
        assets,
        camera,
        viewport,
        preview,
        options,
        canvas,
        Pass::Lot,
        None,
        held,
        None,
    );
    painter.stats = env_stats;
    let top = camera.level.clamp(1, 2);
    let cut1 = cut(lot, camera, 1, top, cut_mask);
    painter.objects(1, &cut1);
    if top == 2 {
        painter.floors(2);
        let cut2 = cut(lot, camera, 2, top, cut_mask);
        let corners2 = painter.corners(2, &cut2);
        painter.walls(2, &cut2, &corners2);
        painter.trims(2, &corners2);
        painter.objects(2, &cut2);
    }
    painter.flush_commands();
    painter.stats
}

#[allow(clippy::too_many_arguments)]
fn paint_with_precomputed(
    world: &World,
    lot: &Lot,
    assets: &mut Assets,
    camera: &Camera,
    viewport: (u32, u32),
    preview: Option<(u16, u16)>,
    options: RenderOptions,
    canvas: &mut Canvas,
    pass: Pass,
    grass: &Grass,
    held: BTreeSet<u16>,
    cut_mask: Option<&Mask>,
    level1_openings: Option<&BTreeMap<Edge, (usize, u16)>>,
    band_override: Option<usize>,
    flush_for_env_hook: bool,
    mut after_level1_env: impl FnMut(&Canvas, Stats),
) -> Stats {
    let time_of_day = world.globals.get(4).copied().unwrap_or(lot.time_of_day);
    let house = world.globals.get(10).copied().unwrap_or(0);
    canvas.clear(background(time_of_day, house));
    let mut painter = Painter::new(
        world,
        lot,
        assets,
        camera,
        viewport,
        preview,
        options,
        canvas,
        pass,
        Some(grass),
        held,
        band_override,
    );
    let top = camera.level.clamp(1, 2);
    for level in 1..=top {
        painter.floors(level);
        let cut = cut(lot, camera, level, top, cut_mask);
        let corners = painter.corners(level, &cut);
        if level == 1
            && let Some(openings) = level1_openings
        {
            painter.walls_with_openings(level, &cut, &corners, openings);
        } else {
            painter.walls(level, &cut, &corners);
        }
        painter.trims(level, &corners);
        if level == 1 && flush_for_env_hook {
            painter.flush_commands();
            after_level1_env(painter.canvas, painter.stats);
        }
        painter.objects(level, &cut);
    }
    painter.flush_commands();
    painter.stats
}

/// How a level's walls are cut. Only the top floor shown is cut away; the
/// floors below it stand (0x4a8895, the old prototype's `wall_view.rs`).
fn cut<'a>(
    lot: &Lot,
    camera: &Camera,
    level: i32,
    top: i32,
    precomputed: Option<&'a Mask>,
) -> Cut<'a> {
    if level < top {
        Cut::None
    } else {
        match camera.walls {
            Walls::Up => Cut::None,
            Walls::Down => Cut::All,
            Walls::Cutaway => match precomputed {
                Some(mask) => Cut::Borrowed(mask),
                None => Cut::Mask(Box::new(Mask::new(lot, camera))),
            },
        }
    }
}

/// The objects that hold a person, directly or through other objects, and
/// everything else in the same holder's tree: the original marks them
/// moving as a person in the tree animates. The person's animation code
/// (0x5859fa, 0x585b80, 0x58da79, 0x58f873, 0x591dfb, each on its own
/// condition) calls 0x434101, which climbs the holders to the outermost
/// one (0x4325b5) and marks it and all it holds, at every depth, as moving
/// (0x434657, recursing through its slots at 0x4347bc). A person's own
/// time-out is never reached (vtable +0x18 is 0x7fffffff at 0x4226b6).
/// With the lot paused, the live original still drew the bench Mortimer
/// sat on in its moving pass every frame. People themselves are not in
/// the set; they draw in their own pass.
pub fn holds_person(world: &World) -> BTreeSet<u16> {
    let people = world
        .objects
        .iter()
        .filter(|(_, o)| o.person.is_some())
        .map(|(&id, _)| id)
        .collect();
    held_trees(&world.objects, &people)
}

/// [`holds_person`] for `objects`, of which `people` are the people.
fn held_trees(objects: &CowMap<u16, Object>, people: &BTreeSet<u16>) -> BTreeSet<u16> {
    let word = |id: u16, i: usize| objects.get(&id).and_then(|o| o.data.get(i).copied());
    let registered = |w: i16| {
        u16::try_from(w)
            .ok()
            .filter(|&id| id != 0 && objects.contains_key(&id))
    };
    // An object's holder by its link words, as 0x4325b5 finds it: an
    // object with slot word 0 follows the one before it on its tile, so
    // climb those to the first with a slot word; the object before that
    // one is the holder.
    let holder = |id: u16| {
        let mut at = id;
        let mut seen = BTreeSet::from([id]);
        while word(at, LINK_SLOT)? == 0 {
            let before = registered(word(at, BEFORE)?)?;
            if !seen.insert(before) {
                return None;
            }
            at = before;
        }
        registered(word(at, BEFORE)?)
    };
    // The outermost holder; for holders that hold each other, the lowest
    // ID in their loop, the same from wherever the climb starts.
    let root = |id: u16| {
        let mut at = id;
        let mut seen = BTreeSet::from([id]);
        while let Some(next) = holder(at) {
            if !seen.insert(next) {
                let mut lowest = next;
                let mut on = holder(next);
                while let Some(o) = on.filter(|&o| o != next) {
                    lowest = lowest.min(o);
                    on = holder(o);
                }
                return lowest;
            }
            at = next;
        }
        at
    };
    // Each root and all it holds, through the objects in its slots, at
    // every depth. A saved slot list's first entry is not a slot: it links
    // to the next object on the tile, so the walk starts at the second, as
    // 0x434657 does (0x434681). A person who holds things and is held by
    // nothing is the root of their own tree.
    let mut tree = BTreeSet::new();
    let mut stack: Vec<u16> = people.iter().map(|&id| root(id)).collect();
    while let Some(id) = stack.pop() {
        if !tree.insert(id) {
            continue;
        }
        if let Some(o) = objects.get(&id) {
            stack.extend(o.slots.iter().skip(1).filter_map(|s| registered(s.object)));
        }
    }
    tree.retain(|id| !people.contains(id));
    tree
}

/// Whether the ground is drawn under level 1's tile `[x, y]` (`0x4af029`):
/// unless the tile and its four neighbours all have a floor and all four
/// neighbours are water (`Arry 10` not `0xff`, `0x478f4a`). A split tile's
/// floor counts as a floor, as the original's does.
fn ground_shows(lot: &Lot, [x, y]: [i32; 2]) -> bool {
    let last = lot.size as i32;
    let at = |[x, y]: [i32; 2]| Lot::cell(x as usize, y as usize);
    let floored = |p| lot.levels[0].floors.get(at(p)).is_some_and(|&f| f != 0);
    let water = |p| lot.water.get(at(p)).is_some_and(|&w| w != 0xff);
    let around = [
        [(x - 1).max(0), y],
        [(x + 1).min(last), y],
        [x, (y - 1).max(0)],
        [x, (y + 1).min(last)],
    ];
    !(floored([x, y]) && around.iter().all(|&p| floored(p) && water(p)))
}

/// Which walls of a level are cut down, and which objects hide with them.
enum Cut<'a> {
    None,
    All,
    Mask(Box<Mask>),
    Borrowed(&'a Mask),
}

impl Cut<'_> {
    /// Whether the wall with segment bit `bit` and style `style` of lot
    /// cell `cell` is cut down. Fences never are.
    fn cuts(&self, camera: &Camera, cell: [i32; 2], bit: u8, style: u16) -> bool {
        solid(style)
            && match self {
                Cut::None => false,
                Cut::All => true,
                Cut::Mask(mask) => mask.cuts(camera, cell, bit),
                Cut::Borrowed(mask) => mask.cuts(camera, cell, bit),
            }
    }
}

/// The trim frame for the walls meeting at a corner, by the directions
/// they leave it in (the original's table at 0x6cd468, less one): bits 1,
/// 2, 4 and 8 are view -y, -x, +y and +x, and 0x10 to 0x80 the diagonals
/// between them, from (-1, -1) round to (1, -1). Combinations the game
/// has no frame for are None.
fn trim_frame(mask: u8) -> Option<usize> {
    const DIAGONALS: [(u8, usize); 30] = [
        (0x14, 16),
        (0x18, 17),
        (0x1c, 18),
        (0x20, 19),
        (0x21, 20),
        (0x28, 21),
        (0x29, 22),
        (0x30, 23),
        (0x38, 24),
        (0x40, 25),
        (0x41, 26),
        (0x42, 27),
        (0x43, 28),
        (0x50, 29),
        (0x60, 30),
        (0x61, 31),
        (0x70, 32),
        (0x80, 33),
        (0x82, 34),
        (0x84, 35),
        (0x86, 36),
        (0x90, 37),
        (0x94, 38),
        (0xa0, 39),
        (0xb0, 40),
        (0xc0, 41),
        (0xc2, 42),
        (0xd0, 43),
        (0xe0, 44),
        (0xf0, 45),
    ];
    match mask {
        0 => None,
        1..=0x10 => Some(usize::from(mask) - 1),
        _ => DIAGONALS
            .iter()
            .find(|&&(m, _)| m == mask)
            .map(|&(_, frame)| frame),
    }
}

/// A level's walls and corners, and each wall's lot cell and segment bit.
struct Level {
    corners: Corners,
    faces: Vec<([i32; 2], u8)>,
    /// Each wall's index, by its cell and segment bit.
    index: BTreeMap<([i32; 2], u8), usize>,
}

impl Level {
    /// A wall's flags (see `caps::Corners`), by its cell and segment bit.
    fn flags(&self, face: ([i32; 2], u8)) -> u8 {
        self.index.get(&face).map_or(0, |&i| self.corners.flags[i])
    }
}

/// A wall segment by its two lot corners, the lower first.
pub(crate) type Edge = [[i32; 2]; 2];

/// The objects that cut their own shape into a wall on a level, by the wall
/// they stand in, stored as `(resource, wall_shape)`.
pub(crate) fn wall_objects_on_level(
    world: &World,
    assets: &mut Assets,
    level: i32,
) -> BTreeMap<Edge, (usize, u16)> {
    let mut found = BTreeMap::new();
    for o in world.objects.values() {
        if o.position[2] != level || o.position[0] < 0 || o.position[1] < 0 {
            continue;
        }
        let Some(definition) = assets.definition_ref(o.guid) else {
            continue;
        };
        if definition.shapes_wall() {
            let info = (Assets::resource_of(definition), definition.wall_shape);
            if let Some(e) = object_edge(o.position, &o.data) {
                found.insert(e, info);
            }
        }
    }
    found
}

fn edge(a: [i32; 2], b: [i32; 2]) -> Edge {
    if a <= b { [a, b] } else { [b, a] }
}

/// One wall of a cell: its corners and the direction its first side faces.
struct Segment {
    bit: u8,
    from: [i32; 2],
    to: [i32; 2],
    normal: [i32; 2],
}

fn segments(x: i32, y: i32) -> [Segment; 4] {
    let segment = |bit, from, to, normal| Segment {
        bit,
        from,
        to,
        normal,
    };
    [
        segment(lot::WEST, [x, y], [x, y + 1], [1, 0]),
        segment(lot::NORTH, [x, y], [x + 1, y], [0, 1]),
        segment(lot::DESCENDING, [x + 1, y], [x, y + 1], [1, 1]),
        segment(lot::ASCENDING, [x, y], [x + 1, y + 1], [1, -1]),
    ]
}

/// For a straight wall between lot corners `from` and `to`, whether it
/// runs along view y (direction 1; otherwise along view x, direction 2),
/// and the lot tile the original reads its lamps from: the one whose view
/// tile has the wall on its low-x or low-y edge, the side the camera sees
/// (0x53db67 to 0x4aed23). None for a diagonal.
fn seen_tile(camera: &Camera, from: [i32; 2], to: [i32; 2]) -> Option<(bool, [i32; 2])> {
    let [a, b] = [from, to].map(|c| camera.rotate_fine(c.map(|n| n * 16)).map(|n| n / 16));
    let along_y = a[0] == b[0];
    if !along_y && a[1] != b[1] {
        return None;
    }
    let view = [a[0].min(b[0]), a[1].min(b[1])];
    Some((along_y, camera.unrotate_tile(view)))
}

/// A fine position just beside a wall segment's middle, on its normal's
/// side (side 0) or the other (side 1).
fn wall_side(s: &Segment, side: usize) -> [i32; 2] {
    let sign = if side == 0 { 2 } else { -2 };
    [0, 1].map(|i| (s.from[i] + s.to[i]) * 8 + sign * s.normal[i])
}

#[derive(Clone, Copy)]
enum SpriteRef {
    Floor(u16, usize),
    PoolTile(u8, usize),
    PoolCorner(u8, usize),
    Water(u8, usize),
    Pattern(u16, usize, u8),
    Shape(u16, bool, usize, u8),
    ObjectShape(usize, u16, bool, usize, u8),
    Trim(bool, usize, u16),
    GroupPart(usize, u16, u32, usize, usize),
}

impl SpriteRef {
    fn resolve(self, assets: &Assets) -> Option<(&CanvasImage, &RowSpans)> {
        match self {
            SpriteRef::Floor(id, zoom) => assets.floor_cached(id, zoom),
            SpriteRef::PoolTile(mask, zoom) => assets.pool_tile_cached(mask, zoom),
            SpriteRef::PoolCorner(corner, zoom) => assets.pool_corner_cached(corner, zoom),
            SpriteRef::Water(mask, zoom) => assets.water_tile_cached(mask, zoom),
            SpriteRef::Pattern(id, zoom, frame) => assets.pattern_cached(id, zoom, frame as usize),
            SpriteRef::Shape(style, cut, zoom, frame) => {
                assets.shape_cached(style, cut, zoom, frame as usize)
            }
            SpriteRef::ObjectShape(res, ws, cut, zoom, frame) => {
                assets.object_shape_cached(res, ws, cut, zoom, frame as usize)
            }
            SpriteRef::Trim(cut, zoom, frame) => assets.trim_cached(cut, zoom, frame as usize),
            SpriteRef::GroupPart(res, group, dir, zoom, idx) => {
                assets.group_part_cached(res, group, dir, zoom, idx)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum RowLight {
    Flat(Light),
    FloorPools {
        base: u32,
        zoom: usize,
        origins: [Option<[i32; 2]>; 2],
        offset: [i32; 2],
    },
}

#[derive(Clone, Copy)]
enum RowDepthMode {
    Floor {
        base_d: i32,
        first_row: i32,
    },
    FlatTrim {
        view: [i32; 2],
        ground: i32,
        level: i32,
        flat_offset: i32,
        top_y: i32,
    },
}

#[derive(Clone, Copy)]
enum WallLight {
    Flat(Light),
    WallPools {
        base: u32,
        zoom: usize,
        along_y: bool,
        rise: i64,
        lamps: [Option<(i32, usize)>; 2],
    },
}

#[derive(Clone, Copy)]
enum WallDepthMode {
    Straight {
        anchor: Anchor,
        first: i32,
        sprite_left: i32,
    },
    Diagonal {
        anchor: Anchor,
        edge_on: bool,
    },
}

#[derive(Clone, Copy)]
enum SpriteShading {
    Flat(Light),
    Blocked,
}

#[derive(Clone, Copy)]
enum PixelDepthMode {
    Flat(i32),
    Sprite(Anchor),
    Trim { anchor: Anchor, left: i32 },
}

enum DrawCmdKind {
    GrassTile {
        view: [i32; 2],
        heights: [u8; 4],
        shades: [u8; 4],
        colors: [u8; 4],
        left: i32,
        bottom: i32,
        zoom: u32,
        depth: i32,
        depth_clamped: u16,
        clip_bottom: i32,
    },
    TerrainGrid {
        left: i32,
        y_left: i32,
        y_top: i32,
        y_right: i32,
        half_w: i32,
        step: i32,
        mask: u8,
        clip_bottom: i32,
    },
    RowDepthSprite {
        sprite: SpriteRef,
        at: [i32; 2],
        columns: Option<[i32; 2]>,
        sub_rect: Option<[usize; 4]>,
        clip_bottom: Option<i32>,
        light: RowLight,
        depth: RowDepthMode,
    },
    WaterOverlay {
        sprite: SpriteRef,
        at: [i32; 2],
        clip_bottom: i32,
        light: Light,
        water_base: i32,
        first_row: i32,
    },
    ShadowMask {
        zoom: usize,
        code: u8,
        at: [i32; 2],
        strength: i32,
        base_d: i32,
        first_row: i32,
    },
    MaskedWall {
        pattern: SpriteRef,
        mask: SpriteRef,
        at: [i32; 2],
        columns: Option<[i32; 2]>,
        light: WallLight,
        depth: WallDepthMode,
    },
    DepthSprite {
        sprite: SpriteRef,
        at: [i32; 2],
        flip: bool,
        slack: i32,
        columns: Option<[i32; 2]>,
        pen: Option<u16>,
        shading: SpriteShading,
        depth: PixelDepthMode,
    },
}

struct DrawCmd {
    y0: usize,
    y1: usize,
    kind: DrawCmdKind,
}

enum ResolvedCmdKind<'a> {
    GrassTile {
        view: [i32; 2],
        heights: [u8; 4],
        shades: [u8; 4],
        colors: [u8; 4],
        left: i32,
        bottom: i32,
        zoom: u32,
        depth: i32,
        depth_clamped: u16,
        clip_bottom: i32,
    },
    TerrainGrid {
        left: i32,
        y_left: i32,
        y_top: i32,
        y_right: i32,
        half_w: i32,
        step: i32,
        mask: u8,
        clip_bottom: i32,
    },
    RowDepthSprite {
        image: &'a CanvasImage,
        spans: &'a RowSpans,
        at: [i32; 2],
        columns: Option<[i32; 2]>,
        sub_rect: Option<[usize; 4]>,
        clip_bottom: Option<i32>,
        light: RowLight,
        depth: RowDepthMode,
    },
    WaterOverlay {
        image: &'a CanvasImage,
        spans: &'a RowSpans,
        at: [i32; 2],
        clip_bottom: i32,
        light: Light,
        water_base: i32,
        first_row: i32,
    },
    ShadowMask {
        mask: &'a crate::shadows::Mask,
        at: [i32; 2],
        strength: i32,
        base_d: i32,
        first_row: i32,
    },
    MaskedWall {
        pattern: &'a CanvasImage,
        pat_spans: &'a RowSpans,
        mask: &'a CanvasImage,
        mask_spans: &'a RowSpans,
        at: [i32; 2],
        columns: Option<[i32; 2]>,
        light: ResolvedWallLight<'a>,
        depth: WallDepthMode,
    },
    DepthSprite {
        image: &'a CanvasImage,
        spans: &'a RowSpans,
        at: [i32; 2],
        flip: bool,
        slack: i32,
        columns: Option<[i32; 2]>,
        pen: Option<u16>,
        shading: SpriteShading,
        depth: PixelDepthMode,
    },
}

enum ResolvedWallLight<'a> {
    Flat(Light),
    WallPools {
        base: u32,
        rise: i64,
        pools: [Option<pools::WallPoolRef<'a>>; 2],
    },
}

struct ResolvedCmd<'a> {
    y0: usize,
    y1: usize,
    kind: ResolvedCmdKind<'a>,
}

const MAX_BAND_WORKERS: usize = 8;

fn default_band_count(width: usize, height: usize, cmd_count: usize) -> usize {
    if height < 128 || width.saturating_mul(height) < 64 * 64 || cmd_count < 32 {
        return 1;
    }
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .clamp(1, MAX_BAND_WORKERS)
}

fn rasterize_band(
    band: &mut CanvasBand<'_>,
    cmds: &[ResolvedCmd<'_>],
    depth_table: &Depth,
    pools: Option<&Pools>,
    grass: Option<&Grass>,
) {
    let by0 = band.y0;
    let by1 = band.y1;
    for cmd in cmds {
        if cmd.y1 <= by0 || cmd.y0 >= by1 {
            continue;
        }
        match &cmd.kind {
            ResolvedCmdKind::GrassTile {
                view,
                heights,
                shades,
                colors,
                left,
                bottom,
                zoom,
                depth,
                depth_clamped,
                clip_bottom,
            } => {
                let Some(grass) = grass else {
                    continue;
                };
                let canvas_w = i64::try_from(band.width).unwrap_or(i64::MAX);
                let by0_i32 = by0 as i32;
                let by1_i32 = by1 as i32;
                grass::for_each_column(
                    *left,
                    *bottom,
                    *heights,
                    *shades,
                    *colors,
                    *zoom,
                    |column| {
                        if i64::from(column.x) + 4 <= 0 || i64::from(column.x) >= canvas_w {
                            return;
                        }
                        let clipped = column.clipped(0, *clip_bottom);
                        for (row, pixels) in clipped.rows_rgb_in_band(by0_i32, by1_i32) {
                            band.plot_grass_span(column.x, row, *depth, *depth_clamped, pixels);
                        }
                        let index = ((column.x - *left) / 4) as usize;
                        grass.for_each_blade(*view, &column, index, |bx, foot, height, color| {
                            let top = foot.saturating_sub(height - 1).max(0);
                            let bottom = foot.min(*clip_bottom - 1);
                            if bottom >= by0_i32 && top < by1_i32 {
                                band.plot_grass_blade(
                                    bx,
                                    top,
                                    bottom,
                                    *depth,
                                    *depth_clamped,
                                    color,
                                );
                            }
                        });
                    },
                );
            }
            ResolvedCmdKind::TerrainGrid {
                left,
                y_left,
                y_top,
                y_right,
                half_w,
                step,
                mask,
                clip_bottom,
            } => {
                let by0_i32 = by0 as i32;
                let by1_i32 = (by1 as i32).min(*clip_bottom);
                if (*mask & 1) != 0 {
                    let dy = y_top.wrapping_sub(*y_left).wrapping_shl(16) / *half_w;
                    let mut y_fp = y_left.wrapping_shl(16);
                    let mut px = *left;
                    while px < *left + *half_w {
                        let py = y_fp >> 16;
                        if py >= by0_i32 && py < by1_i32 {
                            band.plot_color(px, py, [0, 0, 0]);
                        }
                        px = px.wrapping_add(*step);
                        y_fp = y_fp.wrapping_add(dy.wrapping_mul(*step));
                    }
                }
                if (*mask & 2) != 0 {
                    let dy = y_right.wrapping_sub(*y_top).wrapping_shl(16) / *half_w;
                    let mut y_fp = y_top.wrapping_shl(16);
                    let mut px = *left + *half_w;
                    while px < *left + *half_w * 2 {
                        let py = y_fp >> 16;
                        if py >= by0_i32 && py < by1_i32 {
                            band.plot_color(px, py, [0, 0, 0]);
                        }
                        px = px.wrapping_add(*step);
                        y_fp = y_fp.wrapping_add(dy.wrapping_mul(*step));
                    }
                }
            }
            ResolvedCmdKind::RowDepthSprite {
                image,
                spans,
                at,
                columns,
                sub_rect,
                clip_bottom,
                light,
                depth,
            } => {
                let row_depth = |sy: i32| match *depth {
                    RowDepthMode::Floor { base_d, first_row } => {
                        depth_table.floor_row(base_d, first_row, sy)
                    }
                    RowDepthMode::FlatTrim {
                        view,
                        ground,
                        level,
                        flat_offset,
                        top_y,
                    } => depth_table.flat(view, ground, level, flat_offset, top_y, sy),
                };
                match *light {
                    RowLight::Flat(flat_light) => {
                        band.blit_row_depth_flat(
                            image,
                            spans,
                            *at,
                            flat_light,
                            *columns,
                            *sub_rect,
                            *clip_bottom,
                            None,
                            row_depth,
                        );
                    }
                    RowLight::FloorPools {
                        base,
                        zoom,
                        origins,
                        offset,
                    } => {
                        if let Some(pools) = pools {
                            band.blit_row_depth_shaded(
                                image,
                                spans,
                                *at,
                                |ix, iy| {
                                    let mut sum = base;
                                    for o in origins.into_iter().flatten() {
                                        sum += u32::from(pools.at(
                                            zoom,
                                            o[0] + offset[0] + ix as i32,
                                            o[1] + offset[1] + iy as i32,
                                        ));
                                    }
                                    Light::from_sum(sum)
                                },
                                *columns,
                                *sub_rect,
                                *clip_bottom,
                                None,
                                row_depth,
                            );
                        }
                    }
                }
            }
            ResolvedCmdKind::WaterOverlay {
                image,
                spans,
                at,
                clip_bottom,
                light,
                water_base,
                first_row,
            } => {
                band.draw_water_overlay(image, spans, *at, *clip_bottom, *light, None, |sy| {
                    depth_table.floor_row(*water_base, *first_row, sy)
                });
            }
            ResolvedCmdKind::ShadowMask {
                mask,
                at,
                strength,
                base_d,
                first_row,
            } => {
                band.shade_mask(mask, *at, *strength, |row| {
                    depth_table
                        .floor_row(*base_d, *first_row, row)
                        .saturating_sub(65)
                });
            }
            ResolvedCmdKind::MaskedWall {
                pattern,
                pat_spans,
                mask,
                mask_spans,
                at,
                columns,
                light,
                depth,
            } => {
                let depth_fn = |sx: i32, sy: i32| match *depth {
                    WallDepthMode::Straight {
                        anchor,
                        first,
                        sprite_left,
                    } => depth_table.wall(anchor, sy, first, sx.saturating_sub(sprite_left)),
                    WallDepthMode::Diagonal { anchor, edge_on } => {
                        depth_table.diagonal(anchor, sy, edge_on)
                    }
                };
                match *light {
                    ResolvedWallLight::Flat(plain) => {
                        band.draw_wall_masked_flat(
                            pattern, pat_spans, mask, mask_spans, *at, *columns, plain, None,
                            depth_fn,
                        );
                    }
                    ResolvedWallLight::WallPools {
                        base,
                        rise,
                        pools: wall_pools,
                    } => {
                        band.draw_wall_masked_shaded(
                            pattern,
                            pat_spans,
                            mask,
                            mask_spans,
                            *at,
                            *columns,
                            |x, y| {
                                let mut sum = base;
                                let py = y as i64 - rise;
                                for p in wall_pools.into_iter().flatten() {
                                    sum += u32::from(p.at(x as i64, py));
                                }
                                Light::from_sum(sum)
                            },
                            None,
                            depth_fn,
                        );
                    }
                }
            }
            ResolvedCmdKind::DepthSprite {
                image,
                spans,
                at,
                flip,
                slack,
                columns,
                pen,
                shading,
                depth,
            } => {
                let depth_fn = |sx: i32, sy: i32, byte: u8| match *depth {
                    PixelDepthMode::Flat(d) => Some(d),
                    PixelDepthMode::Sprite(anchor) => depth_table.sprite(anchor, byte),
                    PixelDepthMode::Trim { anchor, left } => {
                        depth_table.trim(anchor, sy, sx.saturating_sub(left), byte)
                    }
                };
                match *shading {
                    SpriteShading::Flat(light) => {
                        band.blit_depth_sprite_flat(
                            image, spans, *at, *flip, light, *slack, *columns, *pen, depth_fn,
                        );
                    }
                    SpriteShading::Blocked => {
                        band.blit_depth_sprite_tinted(
                            image, spans, *at, *flip, *slack, *columns, *pen, depth_fn,
                        );
                    }
                }
            }
        }
    }
}

type ObjectEntry<'a> = (Anchor, [i32; 2], u16, &'a Object, [f32; 3], [i32; 2], bool);

struct Painter<'a> {
    pass: Pass,
    /// The objects that hold a person, for passes other than `All`.
    held: BTreeSet<u16>,
    /// Off-lot Buy Mode held preview parts rendered at their on-lot
    /// `"Tile Cursor"` positions (`0x42242f`), mapped to `([x, y, level], lift)`.
    preview_overrides: BTreeMap<u16, ([i32; 3], f32)>,
    /// Active house viewport bottom (`word ptr [0x6f0c14]`), clamped to the
    /// canvas height.
    clip_bottom: i32,
    /// Graphics options (`shadows` `[0x6f0188]`, `lighting` `[0x6f018a]`).
    options: RenderOptions,
    world: &'a World,
    lot: &'a Lot,
    camera: &'a Camera,
    assets: &'a mut Assets,
    canvas: &'a mut Canvas,
    grass: Option<&'a Grass>,
    depth: Depth,
    stats: Stats,
    band_override: Option<usize>,
    commands: Vec<DrawCmd>,
    object_scratch: Vec<ObjectEntry<'a>>,
    /// Where each drawn person draws what it holds ([`PersonAttach`]).
    person_attach: BTreeMap<u16, PersonAttach>,
}

impl<'a> Painter<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        world: &'a World,
        lot: &'a Lot,
        assets: &'a mut Assets,
        camera: &'a Camera,
        viewport: (u32, u32),
        preview: Option<(u16, u16)>,
        options: RenderOptions,
        canvas: &'a mut Canvas,
        pass: Pass,
        grass: Option<&'a Grass>,
        held: BTreeSet<u16>,
        band_override: Option<usize>,
    ) -> Self {
        let clip_bottom = (viewport.1 as usize).min(canvas.height) as i32;
        Painter {
            pass,
            held,
            preview_overrides: buy_preview_overrides(world, preview),
            clip_bottom,
            options,
            world,
            lot,
            camera,
            assets,
            canvas,
            grass,
            depth: Depth::new(camera),
            stats: Stats::default(),
            band_override,
            commands: Vec::with_capacity(512),
            object_scratch: Vec::new(),
            person_attach: BTreeMap::new(),
        }
    }

    fn flush_commands(&mut self) {
        if self.commands.is_empty() {
            return;
        }
        if self.canvas.width == 0 || self.canvas.height == 0 {
            self.commands.clear();
            return;
        }
        let assets: &Assets = self.assets;
        let pools = assets.pools();
        let mut resolved = Vec::with_capacity(self.commands.len());
        for cmd in self.commands.drain(..) {
            let kind = match cmd.kind {
                DrawCmdKind::GrassTile {
                    view,
                    heights,
                    shades,
                    colors,
                    left,
                    bottom,
                    zoom,
                    depth,
                    depth_clamped,
                    clip_bottom,
                } => ResolvedCmdKind::GrassTile {
                    view,
                    heights,
                    shades,
                    colors,
                    left,
                    bottom,
                    zoom,
                    depth,
                    depth_clamped,
                    clip_bottom,
                },
                DrawCmdKind::TerrainGrid {
                    left,
                    y_left,
                    y_top,
                    y_right,
                    half_w,
                    step,
                    mask,
                    clip_bottom,
                } => ResolvedCmdKind::TerrainGrid {
                    left,
                    y_left,
                    y_top,
                    y_right,
                    half_w,
                    step,
                    mask,
                    clip_bottom,
                },
                DrawCmdKind::RowDepthSprite {
                    sprite,
                    at,
                    columns,
                    sub_rect,
                    clip_bottom,
                    light,
                    depth,
                } => {
                    let Some((image, spans)) = sprite.resolve(assets) else {
                        continue;
                    };
                    ResolvedCmdKind::RowDepthSprite {
                        image,
                        spans,
                        at,
                        columns,
                        sub_rect,
                        clip_bottom,
                        light,
                        depth,
                    }
                }
                DrawCmdKind::WaterOverlay {
                    sprite,
                    at,
                    clip_bottom,
                    light,
                    water_base,
                    first_row,
                } => {
                    let Some((image, spans)) = sprite.resolve(assets) else {
                        continue;
                    };
                    ResolvedCmdKind::WaterOverlay {
                        image,
                        spans,
                        at,
                        clip_bottom,
                        light,
                        water_base,
                        first_row,
                    }
                }
                DrawCmdKind::ShadowMask {
                    zoom,
                    code,
                    at,
                    strength,
                    base_d,
                    first_row,
                } => {
                    let Some(mask) = assets.shadow_mask_cached(zoom, code) else {
                        continue;
                    };
                    ResolvedCmdKind::ShadowMask {
                        mask,
                        at,
                        strength,
                        base_d,
                        first_row,
                    }
                }
                DrawCmdKind::MaskedWall {
                    pattern,
                    mask,
                    at,
                    columns,
                    light,
                    depth,
                } => {
                    let (Some((pat, pat_spans)), Some((msk, mask_spans))) =
                        (pattern.resolve(assets), mask.resolve(assets))
                    else {
                        continue;
                    };
                    let light = match light {
                        WallLight::Flat(plain) => ResolvedWallLight::Flat(plain),
                        WallLight::WallPools {
                            base,
                            zoom,
                            along_y,
                            rise,
                            lamps,
                        } => {
                            let resolved_pools = lamps.map(|item| {
                                let (along, dist) = item?;
                                pools?.wall_ref(along_y, zoom, along, dist)
                            });
                            ResolvedWallLight::WallPools {
                                base,
                                rise,
                                pools: resolved_pools,
                            }
                        }
                    };
                    ResolvedCmdKind::MaskedWall {
                        pattern: pat,
                        pat_spans,
                        mask: msk,
                        mask_spans,
                        at,
                        columns,
                        light,
                        depth,
                    }
                }
                DrawCmdKind::DepthSprite {
                    sprite,
                    at,
                    flip,
                    slack,
                    columns,
                    pen,
                    shading,
                    depth,
                } => {
                    let Some((image, spans)) = sprite.resolve(assets) else {
                        continue;
                    };
                    ResolvedCmdKind::DepthSprite {
                        image,
                        spans,
                        at,
                        flip,
                        slack,
                        columns,
                        pen,
                        shading,
                        depth,
                    }
                }
            };
            resolved.push(ResolvedCmd {
                y0: cmd.y0,
                y1: cmd.y1,
                kind,
            });
        }
        if resolved.is_empty() {
            return;
        }
        let num_bands = self
            .band_override
            .map(|b| b.clamp(1, MAX_BAND_WORKERS))
            .unwrap_or_else(|| {
                default_band_count(self.canvas.width, self.canvas.height, resolved.len())
            });
        let depth_table = &self.depth;
        let grass = self.grass;
        let mut bands = self.canvas.split_bands(num_bands);
        if bands.len() <= 1 {
            if let Some(band) = bands.first_mut() {
                rasterize_band(band, &resolved, depth_table, pools, grass);
            }
        } else {
            let cmds = &resolved;
            let mut iter = bands.into_iter();
            let Some(mut first) = iter.next() else {
                return;
            };
            let rest_slots: Vec<std::sync::Mutex<Option<CanvasBand<'_>>>> =
                iter.map(|b| std::sync::Mutex::new(Some(b))).collect();
            std::thread::scope(|scope| {
                let mut fallback = Vec::new();
                for (i, slot) in rest_slots.iter().enumerate() {
                    if std::thread::Builder::new()
                        .spawn_scoped(scope, move || {
                            if let Some(mut band) =
                                slot.lock().unwrap_or_else(|e| e.into_inner()).take()
                            {
                                rasterize_band(&mut band, cmds, depth_table, pools, grass);
                            }
                        })
                        .is_err()
                    {
                        fallback.push(i);
                    }
                }
                rasterize_band(&mut first, cmds, depth_table, pools, grass);
                for i in fallback {
                    if let Some(mut band) = rest_slots[i]
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take()
                    {
                        rasterize_band(&mut band, cmds, depth_table, pools, grass);
                    }
                }
            });
        }
        self.canvas.rasterize_partial_tail_row(|band| {
            rasterize_band(band, &resolved, depth_table, pools, grass);
        });
    }

    /// The elevation of a lot corner in near-zoom pixels: 16 per terrain
    /// step.
    fn corner_elevation(&self, [cx, cy]: [i32; 2]) -> i32 {
        // A tile's four heights are its corners (0, 1), (0, 0), (1, 0) and
        // (1, 1), in that order (`legacy/crates/sims-render/src/walls.rs`).
        let last = self.lot.size as i32 - 1;
        let (tx, ty) = (cx.clamp(0, last), cy.clamp(0, last));
        let h = self.lot.heights[Lot::cell(tx as usize, ty as usize)];
        let corner = match (cx - tx, cy - ty) {
            (0, 1) => 0,
            (0, 0) => 1,
            (1, 0) => 2,
            _ => 3,
        };
        i32::from(h[corner]) * 16
    }

    /// The lamps whose pools reach tile (`x`, `y`) of a level, as view
    /// offsets from the lamp to the tile: those of the tile's record
    /// within the pool map's reach each way (0x4c8049).
    fn lamps_at(
        lot: &Lot,
        camera: &Camera,
        level: usize,
        [x, y]: [i32; 2],
    ) -> [Option<[i32; 2]>; 2] {
        let record = usize::try_from(x)
            .ok()
            .zip(usize::try_from(y).ok())
            .and_then(|(x, y)| lot.lamps[level].get(Lot::cell(x, y)).copied())
            .unwrap_or_default();
        record.0.map(|lamp| {
            let offset = lamp?.map(i32::from);
            offset
                .iter()
                .all(|n| n.abs() <= pools::REACH)
                .then(|| rotate_direction(camera.rotation, offset))
        })
    }

    /// Returns `(along, dist)` lamp table keys for a straight wall face along
    /// view y (`along_y`) or x from the lamps marked on lot tile `[x, y]`.
    fn wall_lamp_coords(
        lot: &Lot,
        camera: &Camera,
        pools: Option<&Pools>,
        level: usize,
        along_y: bool,
        [x, y]: [i32; 2],
    ) -> [Option<(i32, usize)>; 2] {
        let Some(pools) = pools else {
            return [None, None];
        };
        let Some(record) = usize::try_from(x)
            .ok()
            .zip(usize::try_from(y).ok())
            .and_then(|(x, y)| lot.lamps[level].get(Lot::cell(x, y)).copied())
        else {
            return [None, None];
        };
        let zoom = camera.zoom.index();
        let mut out = [None, None];
        let mut count = 0;
        for lamp in record.0.into_iter().flatten() {
            let offset = rotate_direction(camera.rotation, lamp.map(i32::from));
            if let Some((along, dist)) = pools::wall_lamp(along_y, offset)
                && pools.wall_ref(along_y, zoom, along, dist).is_some()
            {
                out[count] = Some((along, dist));
                count += 1;
            }
        }
        out
    }

    /// Rotates a lot corner (0 to 64) into the view.
    fn rotate_corner(&self, [x, y]: [i32; 2]) -> [i32; 2] {
        let m = lot::FULL as i32;
        match self.camera.rotation & 3 {
            1 => [m - y, x],
            2 => [m - x, m - y],
            3 => [y, m - x],
            _ => [x, y],
        }
    }

    /// Rotates a lot direction into the view.
    fn rotate_direction(&self, direction: [i32; 2]) -> [i32; 2] {
        rotate_direction(self.camera.rotation, direction)
    }

    /// The screen point of a lot corner on a level, at its terrain height.
    fn corner_screen(&self, corner: [i32; 2], level: i32) -> [i32; 2] {
        let view = self.rotate_corner(corner);
        self.camera.project_fine(
            [view[0] * 16, view[1] * 16],
            level,
            self.corner_elevation(corner),
        )
    }

    fn floors(&mut self, level: i32) {
        // The ground shows wherever a floor leaves it uncovered, such as
        // half of a split tile. All of it goes first: a grass column can
        // reach a pixel or two into the next tile, and floors win there.
        // The original draws no ground on the lot's outermost tiles
        // (0x49cecc).
        if level == 1 {
            let inner = 1..self.lot.size as i32 - 1;
            for x in inner.clone() {
                for y in inner.clone() {
                    if ground_shows(self.lot, [x, y]) {
                        self.terrain([x, y]);
                    }
                }
            }
            // Buy (`2`) and Build (`1`) modes draw the dotted black lot
            // placement grid over unfloored buildable terrain (`0x4ce44f` →
            // `0x4c4222`, `0x4c42f0`, `0x4c3ba4`, `0x4c3c23`, `0x4c37f2`).
            if matches!(self.world.globals.get(19).copied().unwrap_or(0), 1 | 2) {
                for x in inner.clone() {
                    for y in inner.clone() {
                        self.terrain_grid([x, y]);
                    }
                }
            }
        }
        let index = (level - 1) as usize;
        let zoom = self.camera.zoom.index();
        // Constant rotation-indexed top corner offset (the corner with minimum
        // rotated `v[0] + v[1]` is strictly invariant across all tiles).
        let top_offset = match self.camera.rotation & 3 {
            1 => [0, 1],
            2 => [1, 1],
            3 => [1, 0],
            _ => [0, 0],
        };
        let wide = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        let canvas_w_wide = wide(self.canvas.width);
        let canvas_h_wide = wide(self.canvas.height);
        for x in 0..self.lot.size as i32 {
            for y in 0..self.lot.size as i32 {
                let cell = Lot::cell(x as usize, y as usize);
                let floor = self.lot.levels[index].floors[cell];
                let top = [x + top_offset[0], y + top_offset[1]];
                let p = self.corner_screen(top, level);
                let (view, height) = (self.rotate_corner(top), self.corner_elevation(top) / 16);
                if floor == 0 {
                    if level == 1 {
                        self.pool([x, y], view, height);
                        self.water([x, y], view, height, p);
                    }
                    self.shadow(level, [x, y], view, height);
                    continue;
                }
                let wall = self.lot.levels[index].walls[cell];
                let halves = if floor == lot::SPLIT_FLOOR {
                    wall.split_floors().map(Some)
                } else {
                    [
                        Some(floor_for_rotation(
                            floor,
                            level,
                            self.camera.rotation,
                            self.lot.floor_rotation,
                        )),
                        None,
                    ]
                };
                for (half, id) in halves.into_iter().enumerate() {
                    let Some(id) = id.filter(|&id| id != 0) else {
                        continue;
                    };
                    let Some((image, pools)) = self.assets.floor_with_pools(id, zoom) else {
                        continue;
                    };
                    let img_w = image.width;
                    let img_h = image.height;
                    let has_pools = pools.is_some();
                    let at = [p[0] - img_w as i32 / 2, p[1]];
                    let ax = i64::from(at[0]);
                    let ay = i64::from(at[1]);
                    if ax.saturating_add(wide(img_w)) <= 0
                        || ax >= canvas_w_wide
                        || ay.saturating_add(wide(img_h)) <= 0
                        || ay >= canvas_h_wide
                    {
                        self.stats.floors += 1;
                        continue;
                    }
                    let sub_rect = (floor == lot::SPLIT_FLOOR)
                        .then(|| split_half_rect(self.camera, img_w, img_h, wall.segments, half));
                    let (mut y0, mut y1, ay_clip) = clip_span(at[1], img_h, self.canvas.height);
                    if let Some([ix0, iy0, ix1, iy1]) = sub_rect {
                        if ix0 >= ix1 || iy0 >= iy1 {
                            self.stats.floors += 1;
                            continue;
                        }
                        let lim_y = canvas_h_wide;
                        y0 = y0.max(ay_clip.saturating_add(wide(iy0)).clamp(0, lim_y) as usize);
                        y1 = y1.min(ay_clip.saturating_add(wide(iy1)).clamp(0, lim_y) as usize);
                    }
                    if y0 >= y1 {
                        self.stats.floors += 1;
                        continue;
                    }
                    // A point inside the half, for its room's light.
                    let inside = match (floor == lot::SPLIT_FLOOR, wall.segments, half) {
                        (false, _, _) => [8, 8],
                        (true, s, 0) if s & lot::DESCENDING != 0 => [4, 4],
                        (true, s, _) if s & lot::DESCENDING != 0 => [12, 12],
                        (true, _, 0) => [4, 12],
                        (true, _, _) => [12, 4],
                    };
                    let byte = self
                        .lot
                        .light_at(index, [x * 16 + inside[0], y * 16 + inside[1]]);
                    let lamps = if self.options.lighting {
                        Self::lamps_at(self.lot, self.camera, index, [x, y])
                    } else {
                        [None; 2]
                    };
                    let (base_d, first_row) = self.depth.floor_base(view, height, level, at[1]);
                    let light = match (Light::drawn_byte(byte, self.lot.brightest()), has_pools) {
                        // A room in full light shows no pools (0x4c83d4).
                        (Some(base), true) if lamps.iter().any(Option::is_some) => {
                            let tile = self.camera.zoom.tile_width();
                            let origins = lamps.map(|lamp| lamp.map(|a| Pools::origin(tile, a)));
                            RowLight::FloorPools {
                                base,
                                zoom,
                                origins,
                                offset: [0, 0],
                            }
                        }
                        _ => RowLight::Flat(Light::from_byte(byte, self.lot.brightest())),
                    };
                    self.commands.push(DrawCmd {
                        y0,
                        y1,
                        kind: DrawCmdKind::RowDepthSprite {
                            sprite: SpriteRef::Floor(id, zoom),
                            at,
                            columns: None,
                            sub_rect,
                            clip_bottom: None,
                            light,
                            depth: RowDepthMode::Floor { base_d, first_row },
                        },
                    });
                    self.stats.floors += 1;
                }
                if level == 1 {
                    self.pool([x, y], view, height);
                    self.water([x, y], view, height, p);
                }
                self.shadow(level, [x, y], view, height);
            }
        }
    }

    /// Draws the swimming pool water/coping overlay and inner-corner coping
    /// patches for level-1 tile `[x, y]` (`Arry 9`, `0x4ae63b`–`0x4ae83e`,
    /// `SPR2 0x400..=0x43b` and floor `12` for mask `0x10`).
    fn pool(&mut self, [x, y]: [i32; 2], view: [i32; 2], height: i32) {
        let cell = Lot::cell(x as usize, y as usize);
        let raw = self.lot.pool[cell];
        if raw > 0x10 {
            return;
        }
        let zoom = self.camera.zoom.index();
        let rot_val = rotate_cardinal_mask(raw, self.camera.rotation);
        let tile_w = self
            .assets
            .floor(0, zoom)
            .map_or(self.camera.zoom.tile_width() as usize, |img| img.width);
        let at = shadow_origin(self.camera, self.lot, [x, y], 1, tile_w);
        let byte = self.lot.light_at(0, [x * 16 + 8, y * 16 + 8]);
        let lamps = if self.options.lighting {
            Self::lamps_at(self.lot, self.camera, 0, [x, y])
        } else {
            [None; 2]
        };
        let tile_px = self.camera.zoom.tile_width();
        let brightest = self.lot.brightest();
        let (base_d, first_row) = self.depth.floor_base(view, height, 1, at[1]);
        let clip_bottom = self.clip_bottom;
        let max_h = self.canvas.height.min(clip_bottom.max(0) as usize);
        let (main_sprite, main_dims, main_has_pools) = if rot_val < 0x10 {
            match self.assets.pool_tile_with_pools(rot_val, zoom) {
                Some((img, pools)) => (
                    Some(SpriteRef::PoolTile(rot_val, zoom)),
                    (img.width, img.height),
                    pools.is_some(),
                ),
                None => (None, (0, 0), false),
            }
        } else {
            match self.assets.floor_with_pools(12, zoom) {
                Some((img, pools)) => (
                    Some(SpriteRef::Floor(12, zoom)),
                    (img.width, img.height),
                    pools.is_some(),
                ),
                None => (None, (0, 0), false),
            }
        };
        if let Some(sprite) = main_sprite {
            let (x0, x1, _) = clip_span(at[0], main_dims.0, self.canvas.width);
            let (y0, y1, _) = clip_span(at[1], main_dims.1, max_h);
            if x0 < x1 && y0 < y1 {
                let light = match (Light::drawn_byte(byte, brightest), main_has_pools) {
                    (Some(base), true) if lamps.iter().any(Option::is_some) => {
                        let origins = lamps.map(|lamp| lamp.map(|a| Pools::origin(tile_px, a)));
                        RowLight::FloorPools {
                            base,
                            zoom,
                            origins,
                            offset: [0, 0],
                        }
                    }
                    _ => RowLight::Flat(Light::from_byte(byte, brightest)),
                };
                self.commands.push(DrawCmd {
                    y0,
                    y1,
                    kind: DrawCmdKind::RowDepthSprite {
                        sprite,
                        at,
                        columns: None,
                        sub_rect: None,
                        clip_bottom: Some(clip_bottom),
                        light,
                        depth: RowDepthMode::Floor { base_d, first_row },
                    },
                });
            }
        }
        // Inner-corner coping patches (`0x4ae710`–`0x4ae83e` -> `0x461e39`):
        // depth bias -1 relative to the pool tile (`0x4ae744: push -1`), with
        // `first_row` anchored at `at[1]` before adding `frame.y`
        // (`0x4c8345`–`0x4c8351`).
        let corner_bits = pool_inner_corners(self.lot, x, y, self.camera.rotation);
        if corner_bits == 0 {
            return;
        }
        let corner_base = base_d.saturating_sub(1);
        for ci in 0..4u8 {
            if (corner_bits & (1 << ci)) == 0 {
                continue;
            }
            let Some((image, offset, pools)) = self.assets.pool_corner_with_pools(ci, zoom) else {
                continue;
            };
            let img_w = image.width;
            let img_h = image.height;
            let has_pools = pools.is_some();
            let cat = [
                at[0].saturating_add(offset[0]),
                at[1].saturating_add(offset[1]),
            ];
            let (x0, x1, _) = clip_span(cat[0], img_w, self.canvas.width);
            let (y0, y1, _) = clip_span(cat[1], img_h, max_h);
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            let light = match (Light::drawn_byte(byte, brightest), has_pools) {
                (Some(base), true) if lamps.iter().any(Option::is_some) => {
                    let origins = lamps.map(|lamp| lamp.map(|a| Pools::origin(tile_px, a)));
                    RowLight::FloorPools {
                        base,
                        zoom,
                        origins,
                        offset,
                    }
                }
                _ => RowLight::Flat(Light::from_byte(byte, brightest)),
            };
            self.commands.push(DrawCmd {
                y0,
                y1,
                kind: DrawCmdKind::RowDepthSprite {
                    sprite: SpriteRef::PoolCorner(ci, zoom),
                    at: cat,
                    columns: None,
                    sub_rect: None,
                    clip_bottom: Some(clip_bottom),
                    light,
                    depth: RowDepthMode::Floor {
                        base_d: corner_base,
                        first_row,
                    },
                },
            });
        }
    }

    /// Draws the pond/natural water sprite overlay for inner level-1 tile
    /// `[x, y]` (`Arry 10`, `0x4af9d3`–`0x4b010c` → `0x4c1eea`–`0x4c2059` →
    /// `0x4c6594`, `GameData/floors.iff` `SPR2 0x800..=0x80f`, frame `2 - zoom`).
    fn water(&mut self, [x, y]: [i32; 2], view: [i32; 2], height: i32, top_screen: [i32; 2]) {
        let limit = self.lot.size as i32 - 1;
        if x < 1 || x >= limit || y < 1 || y >= limit {
            return;
        }
        let cell = Lot::cell(x as usize, y as usize);
        let raw = self.lot.water[cell];
        if raw == 0xff {
            return;
        }
        let zoom = self.camera.zoom.index();
        let rot_val = rotate_cardinal_mask(raw, self.camera.rotation) & 0x0f;
        let half_w = self
            .assets
            .floor(0, zoom)
            .map_or(self.camera.zoom.tile_width() / 2, |img| {
                img.width as i32 / 2
            });
        let Some(image) = self.assets.water_tile(rot_val, zoom) else {
            return;
        };
        let img_w = image.width;
        let img_h = image.height;
        // `0x4afaac`–`0x4afad6`: `y` shifts up by 4 at Near zoom and 1 at
        // Far/Medium zoom; `x` shifts left by 1 relative to normal floors.
        let y_adj = if self.camera.zoom == Zoom::Near { 4 } else { 1 };
        let at = [top_screen[0] - half_w - 1, top_screen[1] - y_adj];
        let max_h = self.canvas.height.min(self.clip_bottom.max(0) as usize);
        let (x0, x1, _) = clip_span(at[0], img_w, self.canvas.width);
        let (y0, y1, _) = clip_span(at[1], img_h, max_h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let center = [x * 16 + 8, y * 16 + 8];
        let byte = self.lot.light_at(0, center);
        let light = Light::from_byte(byte, self.lot.brightest());
        // `0x4c1f9d`–`0x4c1fb1`: ground depth + 0x50 (80 = `floor_base + 15`),
        // stepping row-by-row through `R[y - origin_y]` anchored at `at[1]`.
        let (floor_base, first_row) = self.depth.floor_base(view, height, 1, at[1]);
        let water_base = floor_base.saturating_add(15);
        self.commands.push(DrawCmd {
            y0,
            y1,
            kind: DrawCmdKind::WaterOverlay {
                sprite: SpriteRef::Water(rot_val, zoom),
                at,
                clip_bottom: self.clip_bottom,
                light,
                water_base,
                first_row,
            },
        });
    }

    /// Casts the floor shadow of tile (`x`, `y`) on `level`, whose view top
    /// corner is `view` at terrain height `height`: that of the first object on
    /// the tile that has one (see [`crate::shadows`]).
    ///
    /// Placement follows `0x4adac0` → `0x4b9613` → `0x4b97ea` and
    /// `0x4ae8cc`–`0x4ae9d3`: project the tile centre `[x * 16 + 8, y * 16 + 8]`
    /// at its rounded terrain altitude (`0x478dfe`), then subtract half the
    /// tile's width (`[esi + 0x5c]`) and height (`[esi + 0x64]`). Depth still
    /// anchors from the tile's top corner (`0x4c9e38`) and steps from the
    /// shadow's top row (`0x4ae93e`–`0x4ae9ab`).
    fn shadow(&mut self, level: i32, [x, y]: [i32; 2], view: [i32; 2], height: i32) {
        if !self.options.shadows {
            return;
        }
        let Some((code, strength)) =
            shadow_of_tile(self.world, self.assets, self.camera.rotation, level, [x, y])
        else {
            return;
        };
        let index = (level - 1) as usize;
        let center = [x * 16 + 8, y * 16 + 8];
        let byte = self.lot.light_at(index, center);
        let strength = i32::from(strength) + 255
            - Light::drawn_byte(byte, self.lot.brightest()).unwrap_or(255) as i32;
        let zoom = self.camera.zoom.index();
        // The mask sits where the plain floor tile would, centred by the
        // tile's width rather than its own.
        let tile = self.assets.floor(0, zoom).map(|image| image.width);
        let Some(mask) = self.assets.shadow_mask(zoom, code) else {
            return;
        };
        let (mask_w, mask_h) = (mask.width, mask.height);
        let at = shadow_origin(self.camera, self.lot, [x, y], level, tile.unwrap_or(mask_w));
        let (x0, x1, _) = clip_span(at[0], mask_w, self.canvas.width);
        let (y0, y1, _) = clip_span(at[1], mask_h, self.canvas.height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (base_d, first_row) = self.depth.floor_base(view, height, level, at[1]);
        self.commands.push(DrawCmd {
            y0,
            y1,
            kind: DrawCmdKind::ShadowMask {
                zoom,
                code,
                at,
                strength,
                base_d,
                first_row,
            },
        });
    }

    /// Draws a grass tile in the original's four-pixel columns (see
    /// [`crate::grass`]), straight from the palette: no room's light falls
    /// on it (0x49da21).
    fn terrain(&mut self, tile: [i32; 2]) {
        let Some(grass) = self.grass else {
            return;
        };
        let view = self.camera.rotate_tile(tile);
        let Some((shades, colors)) = grass.tile(view) else {
            return;
        };
        let rotation = usize::from(self.camera.rotation & 3);
        let h = self.lot.heights[Lot::cell(tile[0] as usize, tile[1] as usize)];
        let heights: [u8; 4] = std::array::from_fn(|n| h[(n + 4 - rotation) & 3]);
        // The left corner, at height 0; columns start a row above it.
        let corner = self.camera.project([view[0], view[1] + 1], 1, 0);
        let zoom = self.camera.zoom.level();
        let width = 16 << zoom;
        // Skip a tile wholly off the canvas, even on the highest ground.
        let reach = width + (255 << (zoom + 1));
        let size = [self.canvas.width, self.canvas.height].map(|n| n as i32);
        if (0..2).any(|i| corner[i] < -reach || corner[i] > size[i].saturating_add(reach)) {
            return;
        }
        let left = corner[0] + GRASS_COLUMN;
        let bottom = corner[1] + GRASS_ROW;
        let canvas_w = i64::try_from(self.canvas.width).unwrap_or(i64::MAX);
        if i64::from(left) + i64::from(width) <= 0 || i64::from(left) >= canvas_w {
            return;
        }
        let shift = zoom + 1;
        let y_left = bottom.saturating_sub(i32::from(heights[0]) << shift);
        let y_top = bottom.saturating_sub((i32::from(heights[1]) + 2) << shift);
        let y_right = bottom.saturating_sub(i32::from(heights[2]) << shift);
        let y_bot = bottom.saturating_sub((i32::from(heights[3]) - 2) << shift);
        let min_y = y_left.min(y_top).min(y_right).min(y_bot).saturating_sub(8);
        let max_y = y_left
            .max(y_top)
            .max(y_right)
            .max(y_bot)
            .saturating_add(4 << shift)
            .saturating_add(8);
        let max_h = self.canvas.height.min(self.clip_bottom.max(0) as usize);
        let y0 = min_y.clamp(0, max_h as i32) as usize;
        let y1 = max_y.clamp(y0 as i32, max_h as i32) as usize;
        if y0 >= y1 {
            return;
        }
        let depth = self.depth.grass(view, i32::from(heights[1]));
        let depth_clamped = crate::canvas::clamp_depth(depth);
        self.commands.push(DrawCmd {
            y0,
            y1,
            kind: DrawCmdKind::GrassTile {
                view,
                heights,
                shades,
                colors,
                left,
                bottom,
                zoom,
                depth,
                depth_clamped,
                clip_bottom: self.clip_bottom,
            },
        });
    }

    /// Whether view tile `[vx, vy]` is buildable for the Buy/Build Mode lot
    /// grid (`0x4c3c23`: lot tile inside `1..size - 1` with ground flag `0x20`
    /// clear in array `8`).
    fn tile_buildable(&self, [vx, vy]: [i32; 2]) -> bool {
        let [lx, ly] = self.camera.unrotate_tile([vx, vy]);
        let limit = self.lot.size as i32 - 1;
        if lx < 1 || lx >= limit || ly < 1 || ly >= limit {
            return false;
        }
        let Some(flags) = self.world.arrays.get(&8) else {
            return true;
        };
        let cell = Lot::cell(lx as usize, ly as usize);
        flags.data.get(cell).is_none_or(|&byte| (byte & 0x20) == 0)
    }

    /// Draws the Buy/Build Mode dotted black lot placement grid on an
    /// unfloored ground tile (`0x4c42f0`, `0x4c3ba4`, `0x4c37f2` / `0x4c391d`):
    /// bit 1 is the tile's top-left edge and bit 2 its top-right edge, stepped
    /// by 2 pixels on flat tiles and 4 pixels on sloped tiles (`0x4c3af9` /
    /// `0x4c3c72`).
    fn terrain_grid(&mut self, [x, y]: [i32; 2]) {
        let cell = Lot::cell(x as usize, y as usize);
        // Native `0x4c42f0` calls `0x4862ed` and skips tiles with a floor.
        if self.lot.levels[0].floors[cell] != 0 {
            return;
        }
        let view = self.camera.rotate_tile([x, y]);
        let mask = if self.tile_buildable(view) {
            3u8
        } else {
            let mut m = 0u8;
            if view[0] > 0 && self.tile_buildable([view[0] - 1, view[1]]) {
                m |= 1;
            }
            if view[1] > 0 && self.tile_buildable([view[0], view[1] - 1]) {
                m |= 2;
            }
            m
        };
        if mask == 0 {
            return;
        }
        let rotation = usize::from(self.camera.rotation & 3);
        let h = self.lot.heights[cell];
        let heights: [u8; 4] = std::array::from_fn(|n| h[(n + 4 - rotation) & 3]);
        let corner = self.camera.project([view[0], view[1] + 1], 1, 0);
        let zoom = self.camera.zoom.level();
        let half_w = 8i32 << zoom;
        let reach = (half_w * 2) + (255 << (zoom + 1));
        let size = [self.canvas.width, self.canvas.height].map(|n| n as i32);
        if (0..2).any(|i| corner[i] < -reach || corner[i] > size[i].saturating_add(reach)) {
            return;
        }
        let left = corner[0] + GRASS_COLUMN;
        let base_y = corner[1] + GRASS_ROW;
        let shift = zoom + 1;
        let y_left = base_y - (i32::from(heights[0]) << shift);
        let y_top = base_y - ((i32::from(heights[1]) + 2) << shift);
        let y_right = base_y - (i32::from(heights[2]) << shift);
        let flat = heights[0] == heights[1] && heights[0] == heights[2] && heights[0] == heights[3];
        let step = if flat { 2 } else { 4 };
        let max_h = self.canvas.height.min(self.clip_bottom.max(0) as usize);
        let fixed_safe = [y_left, y_top, y_right]
            .into_iter()
            .all(|v| (-32768..=32767).contains(&v));
        let (y0, y1) = if fixed_safe {
            let min_y = y_left.min(y_top).min(y_right).saturating_sub(2);
            let max_y = y_left.max(y_top).max(y_right).saturating_add(3);
            let y0 = min_y.clamp(0, max_h as i32) as usize;
            let y1 = max_y.clamp(y0 as i32, max_h as i32) as usize;
            (y0, y1)
        } else {
            (0, max_h)
        };
        if y0 >= y1 {
            return;
        }
        self.commands.push(DrawCmd {
            y0,
            y1,
            kind: DrawCmdKind::TerrainGrid {
                left,
                y_left,
                y_top,
                y_right,
                half_w,
                step,
                mask,
                clip_bottom: self.clip_bottom,
            },
        });
    }

    /// The wallpaper on each side of a wall, first along its normal (the
    /// old prototype's `faces` in
    /// `legacy/crates/sims-render/src/walls.rs`, checked there against the
    /// original's wall records).
    fn sides(&self, level: usize, x: i32, y: i32, bit: u8) -> [u16; 2] {
        let walls = &self.lot.levels[level].walls;
        let cell = |x: i32, y: i32| -> WallCell {
            if self.lot.contains(x, y) {
                walls[Lot::cell(x as usize, y as usize)]
            } else {
                WallCell::default()
            }
        };
        let t = cell(x, y);
        match bit {
            lot::WEST => [t.patterns[0], cell(x - 1, y).patterns[3]],
            lot::NORTH => [t.patterns[1], cell(x, y - 1).patterns[2]],
            _ => [t.patterns[3], t.patterns[2]],
        }
    }

    /// The objects that cut their own shape into a wall on a level, by the
    /// wall they stand in (the old prototype's `object_apertures` in
    /// `legacy/crates/sims-render/src/walls.rs`), stored as `(resource, wall_shape)`.
    fn wall_objects(&mut self, level: i32) -> BTreeMap<Edge, (usize, u16)> {
        wall_objects_on_level(self.world, self.assets, level)
    }

    /// Draws a level's walls.
    fn walls(&mut self, level: i32, cut: &Cut, corners: &Level) {
        let openings = self.wall_objects(level);
        self.walls_with_openings(level, cut, corners, &openings);
    }

    /// Draws a level's walls using precomputed wall-shaping object apertures.
    fn walls_with_openings(
        &mut self,
        level: i32,
        cut: &Cut,
        corners: &Level,
        openings: &BTreeMap<Edge, (usize, u16)>,
    ) {
        let index = (level - 1) as usize;
        let zoom = self.camera.zoom.index();
        // The original draws the walls run by run, in its wall graph's
        // order ([`wall_order`]).
        let ordered = wall_order(self.camera, self.lot, index);
        for (x, y, s) in ordered {
            let wall = self.lot.levels[index].walls[Lot::cell(x as usize, y as usize)];
            let style = if s.bit == lot::WEST {
                wall.styles[0]
            } else {
                wall.styles[1]
            };
            let patterns = self.sides(index, x, y, s.bit);
            let n = self.rotate_direction(s.normal);
            let front = shown_side(n);
            let cut = cut.cuts(self.camera, [x, y], s.bit, style);
            // In live and buy modes (`0x416920 != 0`), a cut door-like
            // wall (styles 3, 5, 6, 15 and 23) zeroes its pass count
            // (`0x53e9ce` to `0x53ea2b`) and draws no wall stub across
            // the doorway.
            if cut && caps::door_like(style) {
                continue;
            }
            let key = edge(s.from, s.to);
            let opening = openings.get(&key).copied();
            let pattern = patterns[front];
            let ends = [s.from, s.to].map(|c| self.corner_screen(c, level));
            let heights = [s.from, s.to].map(|c| self.corner_elevation(c) / 16);
            let frame = wall_frame(ends);
            // A cut wall marked for a cap at one end stands full
            // height there: the original draws it in shape 7 or 8,
            // then lays a strip of trim-coloured pattern (252 or
            // 253, shape 9 or 10) down that end (0x53ea55 to
            // 0x53ebb5). Door-like walls are drawn as usual.
            let flags = corners.flags(([x, y], s.bit));
            let capped = (flags & 6 != 0 && !caps::door_like(style)).then_some(if flags & 2 != 0 {
                8
            } else {
                7
            });
            // The side that shows takes the light of the room on
            // that side: sampled just beside the wall's middle, so
            // a split tile gives the triangle touching the wall.
            // Without a room there the wall pass keeps the light it
            // last read, first 1 (0x53dbb4, 0x53dc78); ours takes 1.
            let fraction = self
                .lot
                .room_fraction_at(index, wall_side(&s, front))
                .unwrap_or(1.0);
            let seen = seen_tile(self.camera, s.from, s.to);
            // Fences have no cut shape and stay up.
            let drew = match capped {
                Some(shape) => self.place_wall_parts(
                    pattern,
                    shape,
                    None,
                    false,
                    zoom,
                    frame,
                    ends,
                    heights,
                    level,
                    Some(fraction),
                    seen,
                    None,
                    0,
                ),
                None => {
                    self.place_wall_parts(
                        pattern,
                        style,
                        opening,
                        cut,
                        zoom,
                        frame,
                        ends,
                        heights,
                        level,
                        Some(fraction),
                        seen,
                        None,
                        0,
                    ) || (cut
                        && self.place_wall_parts(
                            pattern,
                            style,
                            opening,
                            false,
                            zoom,
                            frame,
                            ends,
                            heights,
                            level,
                            Some(fraction),
                            seen,
                            None,
                            0,
                        ))
                }
            };
            if !drew {
                self.stats.missing_walls += 1;
                continue;
            }
            if let Some(shape) = capped {
                // In room 0's light, or 1 without one, held by the time
                // of day's cap, and no pools (0x53eba8 to 0x53ebb5).
                let outdoor = self.lot.outdoor_light.unwrap_or(1.0);
                // On an edge-on diagonal the strip is always pattern 252
                // in shape 9, whichever end is marked (0x53eb6d to
                // 0x53ebf5), moved up before drawing.
                if frame == 3 {
                    let lift = edge_on_strip_lift(self.camera.zoom);
                    self.place_edge_on_strip(zoom, ends, heights, level, outdoor, lift);
                } else {
                    let (trim, mask) = if shape == 7 { (252, 9) } else { (253, 10) };
                    self.place_wall_parts(
                        trim,
                        mask,
                        None,
                        false,
                        zoom,
                        frame,
                        ends,
                        heights,
                        level,
                        Some(outdoor),
                        None,
                        None,
                        0,
                    );
                }
            }
            self.stats.walls += 1;
        }
    }

    /// Places a wall face from borrowed `(pattern, shape)` sprites in `assets`,
    /// without cloning or pre-allocating a masked image. Returns `false` if the
    /// pattern/shape sprites are missing or incompatible in size.
    #[allow(clippy::too_many_arguments)]
    fn place_wall_parts(
        &mut self,
        pattern: u16,
        style: u16,
        opening: Option<(usize, u16)>,
        cut: bool,
        zoom: usize,
        frame: usize,
        ends: [[i32; 2]; 2],
        heights: [i32; 2],
        level: i32,
        light: Option<f32>,
        seen: Option<(bool, [i32; 2])>,
        columns: Option<[i32; 2]>,
        raise: i32,
    ) -> bool {
        let Some((pat, shp, pools)) = self
            .assets
            .wall_parts(pattern, style, opening, cut, zoom, frame)
        else {
            return false;
        };
        let fills = |image: &CanvasImage| {
            image
                .width
                .checked_mul(image.height)
                .and_then(|n| n.checked_mul(4))
                == Some(image.rgba.len())
        };
        if (pat.width, pat.height) != (shp.width, shp.height) || !fills(pat) || !fills(shp) {
            return false;
        }
        if pat.is_empty() || self.canvas.color.is_empty() {
            return true;
        }
        let pat_w = pat.width;
        let pat_h = pat.height;
        let (at, anchor) = wall_placement(
            self.camera,
            &self.depth,
            pat_w,
            pat_h,
            frame,
            ends,
            heights,
            level,
            raise,
        );
        let Some((x0, x1, _, _, _, _)) = clip_x(at[0], pat_w, self.canvas.width, columns) else {
            return true;
        };
        let (y0, y1, _) = clip_span(at[1], pat_h, self.canvas.height);
        if x0 >= x1 || y0 >= y1 {
            return true;
        }
        let sprite_left = at[0];
        let first = self.depth.wall_start(frame == 0);
        let brightest = self.lot.brightest();
        let plain = light.map_or(Light::FULL, |f| Light::from_fraction(f, brightest));
        let index = (level - 1) as usize;
        let drawn_base = light.and_then(|f| Light::drawn_fraction(f, brightest));
        // `0x4c7c1a`: `[0x6f018a]` skips only dynamic wall lamp pools while
        // preserving base room shading.
        let wall_light = match (self.options.lighting, drawn_base, seen) {
            (true, Some(base), Some((along_y, tile))) => {
                let lamps =
                    Self::wall_lamp_coords(self.lot, self.camera, pools, index, along_y, tile);
                if lamps[0].is_some() {
                    WallLight::WallPools {
                        base,
                        zoom,
                        along_y,
                        rise: pools::wall_rise(zoom) as i64,
                        lamps,
                    }
                } else {
                    WallLight::Flat(plain)
                }
            }
            _ => WallLight::Flat(plain),
        };
        let use_plain_cut = cut && style == 22;
        let use_object = !use_plain_cut && matches!(style, 22 | 23) && opening.is_some();
        let mask_ref = if use_object {
            let (resource, wall_shape) = opening.expect("opening checked");
            SpriteRef::ObjectShape(resource, wall_shape, false, zoom, frame as u8)
        } else if use_plain_cut {
            SpriteRef::Shape(1, true, zoom, frame as u8)
        } else {
            SpriteRef::Shape(style, cut, zoom, frame as u8)
        };
        let depth_mode = match frame {
            0 | 1 => WallDepthMode::Straight {
                anchor,
                first,
                sprite_left,
            },
            _ => WallDepthMode::Diagonal {
                anchor,
                edge_on: frame == 3,
            },
        };
        self.commands.push(DrawCmd {
            y0,
            y1,
            kind: DrawCmdKind::MaskedWall {
                pattern: SpriteRef::Pattern(pattern, zoom, frame as u8),
                mask: mask_ref,
                at,
                columns,
                light: wall_light,
                depth: depth_mode,
            },
        });
        true
    }

    /// Places the edge-on diagonal cap strip (pattern 252 masked top-left by
    /// shape 9, frame 3) without cloning either sprite.
    fn place_edge_on_strip(
        &mut self,
        zoom: usize,
        ends: [[i32; 2]; 2],
        heights: [i32; 2],
        level: i32,
        outdoor_light: f32,
        raise: i32,
    ) {
        let Some((pat, shp)) = self.assets.edge_on_strip_parts(zoom) else {
            return;
        };
        let fills = |image: &CanvasImage| {
            image
                .width
                .checked_mul(image.height)
                .and_then(|n| n.checked_mul(4))
                == Some(image.rgba.len())
        };
        if shp.width < pat.width || shp.height < pat.height || !fills(pat) || !fills(shp) {
            return;
        }
        if pat.is_empty() || self.canvas.color.is_empty() {
            return;
        }
        let pat_w = pat.width;
        let pat_h = pat.height;
        let (at, anchor) = wall_placement(
            self.camera,
            &self.depth,
            pat_w,
            pat_h,
            3,
            ends,
            heights,
            level,
            raise,
        );
        let Some((x0, x1, _, _, _, _)) = clip_x(at[0], pat_w, self.canvas.width, None) else {
            return;
        };
        let (y0, y1, _) = clip_span(at[1], pat_h, self.canvas.height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let light = Light::from_fraction(outdoor_light, self.lot.brightest());
        self.commands.push(DrawCmd {
            y0,
            y1,
            kind: DrawCmdKind::MaskedWall {
                pattern: SpriteRef::Pattern(252, zoom, 3),
                mask: SpriteRef::Shape(9, false, zoom, 3),
                at,
                columns: None,
                light: WallLight::Flat(light),
                depth: WallDepthMode::Diagonal {
                    anchor,
                    edge_on: true,
                },
            },
        });
    }

    /// A level's walls as the original's corner pass sees them (see
    /// `caps`).
    fn corners(&self, level: i32, cut: &Cut) -> Level {
        let index = (level - 1) as usize;
        let size = self.lot.size as i32;
        let mut walls = Vec::new();
        let mut faces = Vec::new();
        for x in 0..size {
            for y in 0..size {
                let wall = self.lot.levels[index].walls[Lot::cell(x as usize, y as usize)];
                for s in segments(x, y) {
                    if wall.segments & s.bit == 0 {
                        continue;
                    }
                    let style = if s.bit == lot::WEST {
                        wall.styles[0]
                    } else {
                        wall.styles[1]
                    };
                    walls.push(caps::Wall {
                        lot: [s.from, s.to],
                        view: [s.from, s.to].map(|c| self.rotate_corner(c)),
                        style,
                        cut: cut.cuts(self.camera, [x, y], s.bit, style),
                    });
                    faces.push(([x, y], s.bit));
                }
            }
        }
        Level {
            corners: Corners::new(walls),
            index: faces.iter().enumerate().map(|(i, &f)| (f, i)).collect(),
            faces,
        }
    }

    /// The wood trim along the tops of a level's walls and the caps on
    /// their exposed ends (0x53c227). Each corner a wall touches takes one
    /// trim sprite whose frame shows the walls meeting there: standing
    /// walls' trim at the storey's height, cut walls' at the cut height.
    /// Which walls count, and the cap, follow the original's corner
    /// records (see `caps`). Every trim takes the outdoors' light, not its
    /// room's.
    fn trims(&mut self, level: i32, corners: &Level) {
        let Level { corners, faces, .. } = corners;
        let zoom = self.camera.zoom;
        let half = zoom.tile_width() / 2;
        let cut_height = [6, 12, 23][zoom.index()];
        let (corner_light, run_light) = trim_lights(self.lot.outdoor_light, self.lot.brightest());
        for (&corner, c) in &corners.corners {
            let light = if c.record { corner_light } else { run_light };
            let at = self.corner_screen(corner, level);
            let view = self.rotate_corner(corner);
            let ground = self.corner_elevation(corner) / 16;
            let record_trims = [(c.cut, true), (c.standing, false)];
            let trims: &[(u8, bool)] = if c.record {
                &record_trims
            } else {
                &c.run_trims
            };
            for &(bits, is_cut) in trims {
                let height = if is_cut {
                    cut_height
                } else {
                    zoom.floor_height()
                };
                let Some(frame) = trim_frame(bits) else {
                    continue;
                };
                // Cut trim running along a view diagonal through a corner
                // record's top or bottom (bits 0x10 and 0x40, `0x53c41d`–
                // `0x53c4fa`) comes from the standing walls' set; lone 0x10
                // sits a little higher when zoomed in. Non-record wall-run cut
                // trims always take the depth-sprite cut set (`0x53e2fa`–
                // `0x53e362`), with runs along view `(1, 1)` (`bits & 0x50`)
                // lifted by `2 >> (3 - zoom)` (`0x53e300`–`0x53e313`).
                let diagonal = is_cut && c.record && bits & 0x50 != 0;
                let lift = trim_lift(is_cut, c.record, bits, zoom);
                let set = is_cut && !diagonal;
                let flat_offset = flat_trim_offset(is_cut, c.record, bits, level);
                // The original subtracts one more pixel from x; its corner
                // point sits one to the right of ours (fitted to the
                // screenshot).
                let Some(x) = at[0].checked_sub(half) else {
                    continue;
                };
                let mut pieces: [(bool, i32, Option<[i32; 2]>); 3] =
                    [(set, lift, None), (false, 0, None), (false, 0, None)];
                let mut piece_count = 1usize;
                // With 0x10, the cut set's frame is laid again over the
                // part of the corner where a wall leaves along +x (bit 8,
                // right of a point just past the corner) or +y (bit 4,
                // left of a point just short of it), at the plain cut
                // height (0x53c509 to 0x53c640).
                if diagonal && bits & 0x10 != 0 {
                    let z = zoom.index() as i32 + 1;
                    let (step, nudge) = (8 << z, 8 >> (3 - z));
                    if bits & 8 != 0 {
                        pieces[piece_count] = (true, 0, Some([x + step + nudge, i32::MAX]));
                        piece_count += 1;
                    }
                    if bits & 4 != 0 {
                        pieces[piece_count] = (true, 0, Some([i32::MIN, x + step - nudge]));
                        piece_count += 1;
                    }
                }
                // A cut trim is placed as an object would be at the
                // middle of the view tile below the corner, on the ground
                // there (0x4b9613 from 0x53e24d; the old prototype's
                // `geometry_side.rs` reads the same point).
                let last = self.lot.size as i32 - 1;
                let anchor = trim_anchor(&self.depth, self.camera, view, level, |[x, y]| {
                    let (x, y) = (x.clamp(0, last), y.clamp(0, last));
                    self.lot.heights[Lot::cell(x as usize, y as usize)]
                });
                let left = x;
                for &(set, lift, columns) in &pieces[..piece_count] {
                    let rise = zoom.tile_height() / 2 + height + lift;
                    let Some(y) = at[1].checked_sub(rise) else {
                        continue;
                    };
                    let Some(image) = self.assets.trim(set, zoom).get(frame) else {
                        continue;
                    };
                    let img_w = image.width;
                    let img_h = image.height;
                    let has_depth = image.depth.is_some();
                    if set && !has_depth {
                        continue;
                    }
                    let Some((x0, x1, _, _, _, _)) = clip_x(x, img_w, self.canvas.width, columns)
                    else {
                        continue;
                    };
                    let (y0, y1, _) = clip_span(y, img_h, self.canvas.height);
                    if x0 >= x1 || y0 >= y1 {
                        continue;
                    }
                    let sprite = SpriteRef::Trim(set, zoom.index(), frame as u16);
                    if set {
                        // A cut trim takes its depth from its depth sprite,
                        // by column from the whole sprite's left, and is
                        // not drawn without one (0x4c8b1f).
                        self.commands.push(DrawCmd {
                            y0,
                            y1,
                            kind: DrawCmdKind::DepthSprite {
                                sprite,
                                at: [x, y],
                                flip: false,
                                slack: 1,
                                columns,
                                pen: None,
                                shading: SpriteShading::Flat(light),
                                depth: PixelDepthMode::Trim { anchor, left },
                            },
                        });
                        continue;
                    }
                    // The trim lies on the wall tops, one depth a row
                    // (0x4c8849, with `flat_offset` for diagonal trims).
                    self.commands.push(DrawCmd {
                        y0,
                        y1,
                        kind: DrawCmdKind::RowDepthSprite {
                            sprite,
                            at: [x, y],
                            columns,
                            sub_rect: None,
                            clip_bottom: None,
                            light: RowLight::Flat(light),
                            depth: RowDepthMode::FlatTrim {
                                view,
                                ground,
                                level,
                                flat_offset,
                                top_y: y,
                            },
                        },
                    });
                }
            }
            if c.record {
                if let Some((cap, low)) = c.cap {
                    self.cap(level, corners, faces, corner, c, cap, low, None);
                }
            } else {
                for &(cap, low, owner, light_wall) in &c.run_caps {
                    self.cap(
                        level,
                        corners,
                        faces,
                        corner,
                        c,
                        cap,
                        low,
                        Some((owner, light_wall)),
                    );
                }
            }
        }
    }

    /// Draws a corner's cap (0x53c6b7 for corner records, 0x53e3c0 for
    /// mid-run caps): a narrow upright slice of a wall face on the cap's
    /// line, in the plain wall shape and the pattern of the wall in the
    /// cap's slot, standing in the tile the cap names and cut to a column
    /// beside the corner.
    #[allow(clippy::too_many_arguments)]
    fn cap(
        &mut self,
        level: i32,
        corners: &Corners,
        faces: &[([i32; 2], u8)],
        corner: [i32; 2],
        c: &caps::Corner,
        cap: Cap,
        low: bool,
        mid_run: Option<(usize, usize)>,
    ) {
        let owner = match mid_run {
            Some((owner, _)) => owner,
            None => {
                let Some(owner) = c.slots[cap.slot] else {
                    return;
                };
                owner
            }
        };
        let index = (level - 1) as usize;
        let face = |i: usize| {
            let ([x, y], bit) = faces[i];
            let s = segments(x, y).into_iter().find(|s| s.bit == bit)?;
            let n = self.rotate_direction(s.normal);
            Some(([x, y], s, usize::from(n[0] + n[1] < 0)))
        };
        let Some(([x, y], s, front)) = face(owner) else {
            return;
        };
        let pattern = self.sides(index, x, y, s.bit)[front];
        // At a corner record, the light of the first wall at the corner on
        // the cap's line, else of the slot's wall (0x53c97f). Along a run
        // (`mid_run`), the run's seen-side room stored at `0x53f95b..0x53fac7`
        // and read once at `0x53dc6d` / reused at `0x53e701`.
        let lit = match mid_run {
            Some((_, light_wall)) => light_wall,
            None => c
                .slots
                .iter()
                .flatten()
                .copied()
                .find(|&i| corners.walls[i].line() == cap.line)
                .unwrap_or(owner),
        };
        // At a corner record, in the light its room shows through the setter
        // (0x53c9df), or without a room there in full light (0x53c9e9). Along
        // a run (`mid_run`, 0x53e701), without a room the run's light stays
        // at 1 through the setter (`0x53dbb4`), so after dark it is capped.
        let fraction = match face(lit) {
            Some((_, s, front)) => self.lot.room_fraction_at(index, wall_side(&s, front)),
            None => return,
        };
        let fraction = if mid_run.is_some() {
            Some(fraction.unwrap_or(1.0))
        } else {
            fraction
        };
        let [tx, ty] = [c.view[0] + cap.offset[0], c.view[1] + cap.offset[1]];
        let ends = match cap.line {
            Line::Y => [[tx, ty], [tx, ty + 1]],
            Line::X => [[tx, ty], [tx + 1, ty]],
            Line::Rising => [[tx, ty], [tx + 1, ty + 1]],
            Line::Falling => [[tx + 1, ty], [tx, ty + 1]],
        };
        // The phantom face stays within the lot's corners.
        let full = lot::FULL as i32;
        if ends.iter().flatten().any(|v| !(0..=full).contains(v)) {
            return;
        }
        // It stands on the corner's own ground (0x53c7e2 projects the
        // corner's position).
        let ground = self.corner_elevation(corner);
        let ends = ends.map(|[x, y]| self.camera.project_fine([x * 16, y * 16], level, ground));
        let zoom = self.camera.zoom.index();
        let frame = wall_frame(ends);
        // The plain wall's shape, or style 255's for a corner-record wall of
        // that style (0x53c8c1); mid-run caps always use style 1 (0x53e652).
        let style = if mid_run.is_none() && corners.walls[owner].style == 255 {
            255
        } else {
            PLAIN_STYLE
        };
        // The column beside the corner, in the original's corner point
        // (one pixel right of ours).
        let cx = self.corner_screen(corner, level)[0].saturating_add(1);
        let width = cap.width << zoom;
        let columns = if cap.right {
            [
                cx.saturating_sub(1),
                cx.saturating_sub(1).saturating_add(width),
            ]
        } else {
            [cx.saturating_sub(width), cx]
        };
        let step = ground / 16;
        // The cap takes the pools of the lamps on its own seen side, as a
        // wall on its line would (0x53ca0a, 0x53ca41). Its ends are view
        // corners; the view tile with it on its low edge starts at the
        // first.
        let seen = match cap.line {
            Line::Y => Some((true, self.camera.unrotate_tile([tx, ty]))),
            Line::X => Some((false, self.camera.unrotate_tile([tx, ty]))),
            Line::Rising | Line::Falling => None,
        };
        self.place_wall_parts(
            pattern,
            style,
            None,
            low,
            zoom,
            frame,
            ends,
            [step; 2],
            level,
            fraction,
            seen,
            Some(columns),
            0,
        );
    }

    fn objects(&mut self, level: i32, cut: &Cut) {
        let zoom = self.camera.zoom.index();
        let mut list = std::mem::take(&mut self.object_scratch);
        list.clear();
        for (&id, o) in &self.world.objects {
            let preview_at = self.preview_overrides.get(&id).copied();
            let [x, y, z] = preview_at.map_or(o.position, |(pos, _)| pos);
            if o.person.is_some() || z != level || x < 0 || y < 0 {
                continue;
            }
            let wanted = match self.pass {
                Pass::All => true,
                Pass::Lot => !self.held.contains(&id),
                Pass::Held => self.held.contains(&id),
            };
            if !wanted {
                continue;
            }
            // A script can hide an object (data word 34); the original
            // skips it when drawing, as the old prototype did
            // (`legacy/crates/sims-render/src/lib.rs`).
            if o.data.get(HIDDEN).is_some_and(|&w| w != 0) {
                continue;
            }
            let tile = [x >> 4, y >> 4];
            if !self.lot.contains(tile[0], tile[1]) {
                continue;
            }
            let assets = &mut *self.assets;
            let slot = |guid, i: usize| assets.definition_ref(guid)?.slots.get(i).copied();
            let person_attach = &self.person_attach;
            let attach = |person: u16, i: usize| person_attach.get(&person)?.slot_point(i);
            // The object's base altitude comes from `0x478dfe` (`0x4b9613`),
            // and its base screen position and depth anchor from `0x4b97ea`
            // (unheld) or `0x4b9991` (held in a slot, `0x437fdd`, or an
            // invalid Buy preview drawn via `0x42242f`).
            let base_altitude = self.lot.altitude_at([x, y]);
            let (pose, screen, anchor) = if let Some((_, lift)) = preview_at {
                let pose = [x as f32, y as f32, base_altitude + lift];
                let (screen, anchor) =
                    place_moved(&self.depth, self.camera, self.lot, [x, y], level, pose[2]);
                (pose, screen, anchor)
            } else {
                let Some((is_held, pose)) = held(
                    &self.world.objects,
                    slot,
                    attach,
                    id,
                    [x as f32, y as f32, base_altitude],
                ) else {
                    // In a person's slot with an unbound point: not drawn.
                    continue;
                };
                let (screen, anchor) = if is_held {
                    let fine = [
                        (pose[0] + 0.5).trunc() as i32,
                        (pose[1] + 0.5).trunc() as i32,
                    ];
                    place_moved(&self.depth, self.camera, self.lot, fine, level, pose[2])
                } else {
                    place_base(
                        &self.depth,
                        self.camera,
                        self.lot,
                        [x, y],
                        level,
                        base_altitude,
                    )
                };
                (pose, screen, anchor)
            };
            list.push((anchor, screen, id, o, pose, [x, y], preview_at.is_some()));
        }
        object_order(&mut list, self.camera, |item| (item.2, item.3, item.5));
        for &(base_anchor, base_screen, id, o, pose, pos, blocked_preview) in &list {
            let Some(definition) = self.assets.definition_ref(o.guid) else {
                self.stats.missing_objects += 1;
                continue;
            };
            let resource = Assets::resource_of(definition);
            let def_graphic = definition.graphic;
            let def_graphics = definition.graphics;
            let def_kind = definition.kind;
            let def_dynamic_base = definition.dynamic_base;
            let def_shapes_wall = definition.shapes_wall();
            let def_lit_at_brightest = definition.lit_at_brightest();
            // Doors, windows and the like hide where the walls are cut
            // (0x4aa446). Their scripts mark them when the lot loads; in a
            // house read without running those, the objects that shape a
            // wall still hide.
            let tile = [pos[0] >> 4, pos[1] >> 4];
            let walled = cutaway::hides_with_walls(&o.data) || def_shapes_wall;
            let hidden = walled
                && match cut {
                    Cut::None => false,
                    Cut::All => true,
                    Cut::Mask(mask) => mask.hides(self.camera, tile, &o.data),
                    Cut::Borrowed(mask) => mask.hides(self.camera, tile, &o.data),
                };
            if hidden {
                continue;
            }
            let Some(group) = u16::try_from(o.data.first().copied().unwrap_or(0))
                .ok()
                .filter(|&g| g < def_graphics)
                .and_then(|g| def_graphic.checked_add(g))
            else {
                continue;
            };
            let direction = view_direction(o.data.get(1).copied().unwrap_or(0), self.camera);
            let parts = self
                .assets
                .group_by_resource(resource, group, direction, zoom);
            if parts.is_empty() {
                self.stats.missing_objects += 1;
                continue;
            }
            let facing = o.data.get(1).copied().unwrap_or(0);
            // An object takes the light of the room at its position (the
            // original samples the tile light map there, 0x467482). One
            // that gives light (data word 51), other than a window, shows
            // in full light (0x437f7d, 0x4b455f). Then one whose master's
            // word 69 is 7 takes a light of 1 through the setter, held by
            // the time of day's cap (0x437adb to 0x437aed).
            // 0x437abd..0x437aed: `0x437f7d` sets full light first, then
            // master word 69 == 7 (`lit_at_brightest`) overwrites it with the
            // time-of-day cap (`0x4b4525(1.0)`).
            let glows = o.data.get(LIGHT).is_some_and(|&w| w != 0) && def_kind != WINDOW;
            let light = if def_lit_at_brightest {
                Light::from_fraction(1.0, self.lot.brightest())
            } else if glows {
                Light::FULL
            } else {
                Light::from_byte(
                    self.lot.light_at((level - 1) as usize, pos),
                    self.lot.brightest(),
                )
            };
            let shading = if blocked_preview {
                SpriteShading::Blocked
            } else {
                SpriteShading::Flat(light)
            };
            for (part_idx, part) in parts.iter().enumerate() {
                if !sprite_visible(part.sprite, def_dynamic_base, &o.dynamic_sprites) {
                    continue;
                }
                // Offsets come from the files and may be anything; the
                // canvas clips whatever this gives.
                let height = i32::try_from(part.image.height).unwrap_or(i32::MAX);
                // When the sprite has no world offset (`0x437af2`–`0x437b1a`:
                // `fcomp 0.0; jne`, so both `0.0` and `NaN` stay on the base
                // path), it uses the object's base screen position and depth
                // anchor. Otherwise `0x437b7e` adds the rotated offset and
                // calls `0x4b9991`, projecting onto the sloped terrain at the
                // moved position and lifting by `(total_z - ground_here) * (2 << zoom)`.
                let (screen, anchor) = if part.object.iter().all(|&v| v == 0.0 || v.is_nan()) {
                    (base_screen, base_anchor)
                } else {
                    let fine = part_fine([pose[0], pose[1]], facing, part.object);
                    let total_z = pose[2] + sprite_offset(part.object)[2];
                    place_moved(&self.depth, self.camera, self.lot, fine, level, total_z)
                };
                let at = [
                    screen[0]
                        .saturating_add(part.offset[0])
                        .saturating_add(OBJECT_COLUMN),
                    screen[1]
                        .saturating_add(part.offset[1])
                        .saturating_sub(height),
                ];
                let pixels = part.image.width.checked_mul(part.image.height);
                if pixels.and_then(|n| n.checked_mul(4)) != Some(part.image.rgba.len())
                    || pixels == Some(0)
                    || self.canvas.color.is_empty()
                {
                    continue;
                }
                let Some((x0, x1, _, _, rect_x, rect_w)) =
                    clip_x(at[0], part.image.width, self.canvas.width, None)
                else {
                    continue;
                };
                // Record the object's full drawn rectangle on the main thread
                // (preserving exact `Canvas::drawn(id)` semantics even for
                // off-screen parts).
                self.canvas
                    .record_rect_for(id, rect_x, at[1], rect_w, part.image.height);
                let (y0, y1, _) = clip_span(at[1], part.image.height, self.canvas.height);
                if x0 >= x1 || y0 >= y1 {
                    continue;
                }
                let depth_mode = if part.image.depth.is_none() {
                    PixelDepthMode::Flat(anchor.depth)
                } else {
                    PixelDepthMode::Sprite(anchor)
                };
                self.commands.push(DrawCmd {
                    y0,
                    y1,
                    kind: DrawCmdKind::DepthSprite {
                        sprite: SpriteRef::GroupPart(resource, group, direction, zoom, part_idx),
                        at,
                        flip: part.flip,
                        slack: 0,
                        columns: None,
                        pen: Some(id),
                        shading,
                        depth: depth_mode,
                    },
                });
            }
            self.canvas.set_id(None);
            self.stats.objects += 1;
        }
        list.clear();
        self.object_scratch = list;
    }
}

/// Rotates a lot direction into the view at `rotation`, as
/// [`Camera::rotate_fine`] turns positions.
fn rotate_direction(rotation: u8, [x, y]: [i32; 2]) -> [i32; 2] {
    match rotation & 3 {
        1 => [-y, x],
        2 => [-x, -y],
        3 => [y, -x],
        _ => [x, y],
    }
}

/// Maps off-lot Buy Mode held preview parts to their on-lot `"Tile Cursor"`
/// positions and surface lift (`0x42242f` + `0x46def8`), so blocked placements
/// still draw the held object on top of the red cursor.
fn buy_preview_overrides(
    world: &World,
    preview: Option<(u16, u16)>,
) -> BTreeMap<u16, ([i32; 3], f32)> {
    let mut out = BTreeMap::new();
    let Some((cursor_id, held_id)) = preview else {
        return out;
    };
    let resolve_lead = |id: u16| -> u16 {
        world
            .objects
            .get(&id)
            .and_then(|o| o.multipart.as_ref())
            .and_then(|m| u16::try_from(m.parent).ok())
            .filter(|&p| p != 0 && world.objects.contains_key(&p))
            .unwrap_or(id)
    };
    let cursor_lead = resolve_lead(cursor_id);
    let held_lead = resolve_lead(held_id);
    let (Some(cursor_obj), Some(held_obj)) = (
        world.objects.get(&cursor_lead),
        world.objects.get(&held_lead),
    ) else {
        return out;
    };
    let collect_group = |lead: u16, is_group: bool| -> Vec<u16> {
        if !is_group {
            return vec![lead];
        }
        let parent_id = lead as i16;
        world
            .objects
            .iter()
            .filter_map(|(&id, o)| {
                let belongs =
                    id == lead || o.multipart.as_ref().is_some_and(|m| m.parent == parent_id);
                belongs.then_some(id)
            })
            .collect()
    };
    let cursor_parts = collect_group(cursor_lead, cursor_obj.multipart.is_some());
    let held_parts = collect_group(held_lead, held_obj.multipart.is_some());
    let mut pairs = Vec::with_capacity(held_parts.len());
    if held_obj.multipart.is_some() && cursor_obj.multipart.is_some() {
        for &hp in &held_parts {
            let offset = world.objects[&hp].multipart.as_ref().map(|m| m.offset);
            let matched = cursor_parts
                .iter()
                .copied()
                .find(|&cp| world.objects[&cp].multipart.as_ref().map(|m| m.offset) == offset);
            if let Some(cp) = matched {
                pairs.push((cp, hp));
            }
        }
    } else {
        for (&cp, &hp) in cursor_parts.iter().zip(held_parts.iter()) {
            pairs.push((cp, hp));
        }
    }
    for (cp, hp) in pairs {
        let (Some(c), Some(h)) = (world.objects.get(&cp), world.objects.get(&hp)) else {
            continue;
        };
        if h.position[0] >= 0 && h.position[1] >= 0 {
            continue;
        }
        if c.position[0] < 0 || c.position[1] < 0 {
            continue;
        }
        if c.data.get(HIDDEN).is_some_and(|&w| w != 0) {
            continue;
        }
        out.insert(hp, (c.position, preview_surface_lift(&h.data)));
    }
    out
}

/// Default surface-height lift for a non-floor Buy Mode preview drawn via
/// `0x42242f` (`0x46def8`: height codes 2 = 2.5, 3 = 4.0, 4 = 4.05, 8 = 4.0).
fn preview_surface_lift(data: &[i16]) -> f32 {
    let allowed = data.get(4).copied().unwrap_or(0) as u16;
    if allowed & 1 != 0 {
        return 0.0;
    }
    for (code, height) in [(2u32, 2.5f32), (3, 4.0), (4, 4.05), (8, 4.0)] {
        if allowed & (1u16 << (code - 1)) != 0 {
            return height;
        }
    }
    0.0
}

/// An object's data word that hides it when not zero (the simulation's
/// `HIDDEN`).
const HIDDEN: usize = 34;
/// Object data word 1: the way the object faces, in eighths of a turn.
const FACING: usize = 1;
/// Object data word 8, its flags, and the flag of an object that takes no
/// room on its tile.
const FLAGS: usize = 8;
const ZERO_EXTENT: i16 = 4;

/// An object's data word holding the light it gives.
const LIGHT: usize = 51;

/// The object type of a window (OBJD word 9).
const WINDOW: i16 = 8;

/// The data words naming the object that holds this one and which of
/// its slots it is in.
const CONTAINER: usize = 2;
const CONTAINER_SLOT: usize = 3;

/// The data words that link an object into the lot (the simulation's
/// placement): its slot in its holder, counted from 1 (-1 for none, 0 for
/// an object that follows another on its tile), and the object it is in
/// or follows.
const LINK_SLOT: usize = 14;
const BEFORE: usize = 26;

/// Computes a held object's floating-point pose `[x, y, z]` starting from
/// `base = [x, y, base_altitude]`, and reports whether it is drawn away from
/// its own position: held by a non-person container (`0x437fdd`), or at a
/// person's attach point (`0x59185d`).
///
/// The original walks top-down from the root holder to each child (`0x437c56`
/// -> `0x437fdd`): a hidden holder still traverses its slots (`0x4378fa` ->
/// `0x437c56`), each slot's horizontal offset is turned by the holder's facing
/// with odd facings clearing both `x` and `y` (`0x438044` -> `0x439109`), and
/// each step adds `[dx, dy, dz]` sequentially onto the running `f32` pose
/// (`0x438049`–`0x438090`). A person holder is not walked that way
/// (`0x437c56` skips definition type 2): the person's draw puts what its
/// slot holds at the attach point `attach(person, slot)` gives
/// ([`PersonAttach::for_slot`]), and the steps below it stack on that. A
/// slot whose point is unbound is skipped with all it holds, which is
/// `None`. With no attach point (the person has none this frame, or slot
/// 1), or one that is not finite or more than a lot's width from `base`,
/// an object a Sim holds gets no offset from the Sim. A holder cycle
/// returns `(false, base)`. Each accumulated coordinate stays within a
/// lot's width of its start so malformed slot data cannot overflow.
fn held(
    objects: &CowMap<u16, Object>,
    mut slot: impl FnMut(u32, usize) -> Option<[f32; 3]>,
    attach: impl Fn(u16, usize) -> Option<SlotPoint>,
    id: u16,
    base: [f32; 3],
) -> Option<(bool, [f32; 3])> {
    let Some(mut cursor) = objects.get(&id) else {
        return Some((false, base));
    };
    // Fast path: the vast majority of objects are not contained in any slot,
    // so check the immediate container word before allocating `seen`/`steps`.
    let first_container = cursor.data.get(CONTAINER).copied().unwrap_or(0);
    if u16::try_from(first_container)
        .ok()
        .filter(|&c| c != 0)
        .and_then(|c| objects.get(&c))
        .is_none()
    {
        return Some((false, base));
    }
    let mut seen = BTreeSet::from([id]);
    let mut steps = Vec::new();
    let mut start = None;
    loop {
        let container = cursor.data.get(CONTAINER).copied().unwrap_or(0);
        let Some((holder_id, holder)) = u16::try_from(container)
            .ok()
            .filter(|&c| c != 0)
            .and_then(|c| Some((c, objects.get(&c)?)))
        else {
            break;
        };
        let in_slot = usize::try_from(cursor.data.get(CONTAINER_SLOT).copied().unwrap_or(-1)).ok();
        if holder.person.is_some() {
            match in_slot.and_then(|i| attach(holder_id, i)) {
                Some(SlotPoint::At(point)) => start = Some(point),
                Some(SlotPoint::Unbound) => return None,
                None => {}
            }
            break;
        }
        if !seen.insert(holder_id) {
            return Some((false, base));
        }
        let at = in_slot
            .and_then(|i| slot(holder.guid, i))
            .unwrap_or_default();
        let [mut x, mut y, z] = sprite_offset(at);
        let facing = holder.data.get(FACING).copied().unwrap_or(0);
        if facing & 1 != 0 {
            [x, y] = [0.0; 2];
        } else {
            for _ in 0..(i32::from(facing) / 2).rem_euclid(4) {
                (x, y) = (-y, x);
            }
        }
        steps.push([x, y, z]);
        cursor = holder;
    }
    // A point that is not finite, or further than a lot's width from the
    // object's own position (which is its holder's), is no point: the
    // object stays where it was, drawn as an unheld one. This keeps a
    // malformed pose from overflowing the integer projection.
    let limit = (camera::ARRAY_SIZE * 16) as f32;
    let start =
        start.filter(|p| (0..3).all(|i| p[i].is_finite() && (p[i] - base[i]).abs() <= limit));
    if steps.is_empty() && start.is_none() {
        return Some((false, base));
    }
    let start = start.unwrap_or(base);
    let mut pose = start;
    for [dx, dy, dz] in steps.into_iter().rev() {
        let delta = [dx, dy, dz];
        for i in 0..3 {
            let next = pose[i] + delta[i];
            pose[i] = if next.is_finite() {
                next.clamp(start[i] - limit, start[i] + limit)
            } else {
                start[i]
            };
        }
    }
    Some((true, pose))
}

/// A draw group sprite's world offset (`object`), made safe: x and y in
/// sixteenths of a tile, z in height units, finite and clamped to a lot's
/// width so nonsense in a file cannot overflow anything.
fn sprite_offset(object: [f32; 3]) -> [f32; 3] {
    let limit = (camera::ARRAY_SIZE * 16) as f32;
    object.map(|v| {
        if v.is_finite() {
            v.clamp(-limit, limit)
        } else {
            0.0
        }
    })
}

/// How many screen pixels a sprite rises for its world offset's height
/// `z`: `z * (2 << zoom)`, rounded as the original's `__ftol` does (add a
/// half, then truncate toward zero). This is the original's catalog
/// preview (0x438ae4 to 0x438b58, read by the old prototype in
/// `legacy/crates/sims-render/src/object_preview/software.rs`); that the
/// lot view places sprites the same way is **unverified**.
#[cfg(test)]
fn sprite_rise(object: [f32; 3], zoom: Zoom) -> i32 {
    let z = sprite_offset(object)[2];
    if z == 0.0 {
        return 0;
    }
    (z * (2 << zoom.level()) as f32 + 0.5).trunc() as i32
}

/// Draws one grass tile (`0x49dacd`, `0x49da21`, `0x49d8fb`), clipping its
/// columns and blades vertically to `[0, clip_bottom)` (the active house
/// viewport bottom `word ptr [0x6f0c14]`).
#[cfg(test)]
fn draw_grass_tile(
    lot: &Lot,
    grass: &Grass,
    depth_table: &Depth,
    camera: &Camera,
    [x, y]: [i32; 2],
    clip_bottom: i32,
    canvas: &mut Canvas,
) {
    let view = camera.rotate_tile([x, y]);
    let Some((shades, colors)) = grass.tile(view) else {
        return;
    };
    let rotation = usize::from(camera.rotation & 3);
    let h = lot.heights[Lot::cell(x as usize, y as usize)];
    let heights: [u8; 4] = std::array::from_fn(|n| h[(n + 4 - rotation) & 3]);
    let corner = camera.project([view[0], view[1] + 1], 1, 0);
    let zoom = camera.zoom.level();
    let width = 16 << zoom;
    let reach = width + (255 << (zoom + 1));
    let size = [canvas.width, canvas.height].map(|n| n as i32);
    if (0..2).any(|i| corner[i] < -reach || corner[i] > size[i].saturating_add(reach)) {
        return;
    }
    let depth = depth_table.grass(view, i32::from(heights[1]));
    let depth_clamped = crate::canvas::clamp_depth(depth);
    let left = corner[0] + GRASS_COLUMN;
    let canvas_w = i64::try_from(canvas.width).unwrap_or(i64::MAX);
    grass::for_each_column(
        left,
        corner[1] + GRASS_ROW,
        heights,
        shades,
        colors,
        zoom,
        |column| {
            if i64::from(column.x) + 4 <= 0 || i64::from(column.x) >= canvas_w {
                return;
            }
            for (row, pixels) in column
                .clipped(0, clip_bottom)
                .rows_rgb_in_band(i32::MIN, i32::MAX)
            {
                canvas.plot_grass_span(column.x, row, depth, depth_clamped, pixels);
            }
            let index = ((column.x - left) / 4) as usize;
            grass.for_each_blade(view, &column, index, |bx, foot, height, color| {
                let top = foot.saturating_sub(height - 1).max(0);
                let bottom = foot.min(clip_bottom - 1);
                canvas.plot_grass_blade(bx, top, bottom, depth, depth_clamped, color);
            });
        },
    );
}

/// Extra upward screen lift in pixels for diagonal cut wall trims:
/// - Corner records with lone bit `0x10` (`0x53c47a`–`0x53c496`): `[0, 2, 5]`
///   at far, medium, and near zoom.
/// - Non-record wall-run cut trims along view `(1, 1)` (`bits & 0x50 != 0`,
///   `0x53e300`–`0x53e313`): `2 >> (3 - zoom)` = `[0, 1, 2]` before `0x4c8b1f`.
/// - All other trims: `0`.
fn trim_lift(is_cut: bool, is_record: bool, bits: u8, zoom: Zoom) -> i32 {
    if !is_cut {
        return 0;
    }
    if is_record {
        if bits == 0x10 {
            [0, 2, 5][zoom.index()]
        } else {
            0
        }
    } else if bits & 0x50 != 0 {
        [0, 1, 2][zoom.index()]
    } else {
        0
    }
}

/// Depth offset passed to `0x4c8849` for flat-depth wall trims:
/// - Cut diagonal corner trims (`bits & 0x50 != 0`, `0x53c478`–`0x53c4af`):
///   `900` (`0x384`) for `0x10`, `770` (`0x302`) for `0x40` on level 1, and
///   `700` (`0x2bc`) for `0x40` on level 2 (`0x40` wins when both bits set).
/// - Standing corner trims (`0x53c65f`–`0x53c675`): `-100` (`-0x64`) when
///   `bits & 0x40 != 0` on level 2.
/// - All other flat trims (including wall-run trims at `0x540473`): `0`.
fn flat_trim_offset(is_cut: bool, is_record: bool, bits: u8, level: i32) -> i32 {
    if !is_record {
        return 0;
    }
    if is_cut && bits & 0x50 != 0 {
        if bits & 0x40 != 0 {
            if level == 2 { 700 } else { 770 }
        } else {
            900
        }
    } else if !is_cut && bits & 0x40 != 0 && level == 2 {
        -100
    } else {
        0
    }
}

/// The lights of a level's trims, in room 0's light (the outdoors') as a
/// fraction, `outdoor` (None without a room 0), at a time of day whose
/// brightest is `brightest`: (at a corner record, along a run between
/// them). Both take room 0's light through the drawing's light setter
/// (0x4b4525). Without a room 0 the corner pass resets to full light,
/// uncapped (0x53c3a7 to 0x53c3c6), but the wall pass, which draws the
/// trim along a run, passes 1 through the setter (0x53dbb4, 0x53e2f5), so
/// after dark that trim is held at the cap.
fn trim_lights(outdoor: Option<f32>, brightest: f32) -> (Light, Light) {
    let corner = outdoor.map_or(Light::FULL, |light| Light::from_fraction(light, brightest));
    let run = Light::from_fraction(outdoor.unwrap_or(1.0), brightest);
    (corner, run)
}

/// Where a cut wall's trim at view corner `view` on `level` takes its
/// depth from: the middle of the view tile whose top corner that is, on
/// the terrain there. `heights` gives a lot tile's four corner heights.
/// The original looks the terrain up at the middle (0x478dfe), which
/// falls on the diagonal between the tile's corners (0, 0) and (1, 1)
/// ([`sims_sim::head::altitude_at`]).
fn trim_anchor(
    depth: &Depth,
    camera: &Camera,
    view: [i32; 2],
    level: i32,
    heights: impl Fn([i32; 2]) -> [u8; 4],
) -> Anchor {
    let tile = camera.unrotate_tile(view);
    let ground = sims_sim::head::altitude_at(heights(tile), 8, 8).to_f32();
    let middle = [view[0] * 16 + 8, view[1] * 16 + 8];
    place(depth, camera, middle, level, ground)
}

/// Places a point at fine view position `view` on `level`, `height`
/// terrain steps up (16 near-zoom pixels each): the original projects it
/// at its height rounded to a whole step, then takes its depth anchor
/// (0x4b97ea to 0x4c9ceb).
fn place(depth: &Depth, camera: &Camera, view: [i32; 2], level: i32, height: f32) -> Anchor {
    let steps = (height + 0.5).trunc() as i32;
    let rise = steps.saturating_mul(2 << camera.zoom.level());
    let y = camera.project_fine(view, level, 0)[1].saturating_sub(rise);
    depth.anchor(y, height, level)
}

/// The fine position of a draw group sprite that sits `object` away from
/// an object at fine position `position` facing `facing` (data word 1).
/// The position is a float: a held object's slot offset stays exact
/// until here. The offset's x and y are sixteenths of a tile in the
/// object's own frame. The original turns them a quarter turn per two
/// steps of facing and leaves both at 0 for an odd facing (0x439109),
/// then adds them to the object's position and rounds, adding a half and
/// truncating (0x437b7e, 0x4b9991).
fn part_fine(position: [f32; 2], facing: i16, object: [f32; 3]) -> [i32; 2] {
    let [mut x, mut y, _] = sprite_offset(object);
    if facing & 1 != 0 {
        [x, y] = [0.0; 2];
    }
    for _ in 0..(i32::from(facing) / 2).rem_euclid(4) {
        (x, y) = (-y, x);
    }
    [
        (position[0] + x + 0.5) as i32,
        (position[1] + y + 0.5) as i32,
    ]
}

/// Projects rotated fine position `view_fine` on `level` using `0x4a9dc7`
/// (and `0x4a9c50`). When `fixed_steps` is `Some(s)`, all four corners of
/// the view tile take height `s` (`0x4a9e9b`, used by `0x4b97ea`). When
/// `None`, the view tile's rotated terrain heights are interpolated across
/// its two triangles (`0x4a9e22`–`0x4a9f90`, used by `0x4b9991`), falling
/// back to `[0; 4]` on the lot's outermost ring or outside it.
fn project_fine_terrain(
    camera: &Camera,
    lot: &Lot,
    view_fine: [i32; 2],
    level: i32,
    fixed_steps: Option<u8>,
) -> [i32; 2] {
    let zoom = camera.zoom.level();
    let [vx, vy] = view_fine;
    let [bx, by] = [vx >> 4, vy >> 4];
    let [sx, sy] = [vx & 15, vy & 15];
    let heights = match fixed_steps {
        Some(steps) => [steps; 4],
        None => {
            let tile = camera.unrotate_tile([bx, by]);
            let inner = 1..(lot.size as i32 - 1);
            if inner.contains(&tile[0]) && inner.contains(&tile[1]) {
                let rotation = usize::from(camera.rotation & 3);
                let h = lot.heights[Lot::cell(tile[0] as usize, tile[1] as usize)];
                std::array::from_fn(|n| h[(n + 4 - rotation) & 3])
            } else {
                [0; 4]
            }
        }
    };
    let base = camera.project([bx, by], level, i32::from(heights[1]));
    let shift = zoom + 1;
    let half_w = 8i32 << zoom;
    let h = heights.map(i32::from);
    let (dx, dy) = if sx == sy {
        let dy = (((h[1] - h[3] + 4) << shift) * sx) / 16;
        (0, dy)
    } else if sx > sy {
        let y_edge = (((h[1] - h[2] + 2) << shift) * sx) / 16;
        let y_diag = (((h[1] - h[3] + 4) << shift) * sx) / 16;
        let dy = y_edge + ((y_diag - y_edge) * sy) / sx;
        let x_edge = (half_w * sx) / 16;
        let dx = x_edge - (x_edge * sy) / sx;
        (dx, dy)
    } else {
        let y_edge = (((h[1] - h[0] + 2) << shift) * sy) / 16;
        let y_diag = (((h[1] - h[3] + 4) << shift) * sy) / 16;
        let dy = y_edge + ((y_diag - y_edge) * sx) / sy;
        let x_edge = (half_w * sy) / -16;
        let dx = x_edge - (x_edge * sx) / sy;
        (dx, dy)
    };
    [base[0].saturating_add(dx), base[1].saturating_add(dy)]
}

/// Rotates a 4-bit cardinal connectivity mask (`Arry 9` pool / `Arry 10` water)
/// right by `rotation & 3` bits while preserving the high nibble (`0x4614d6`,
/// rotation table `[0x6f4b70]` built at `0x53d310`–`0x53d39c`).
fn rotate_cardinal_mask(mask: u8, rotation: u8) -> u8 {
    let rot = rotation & 3;
    if rot == 0 {
        return mask;
    }
    let lo = mask & 0x0f;
    let rotated = ((lo >> rot) | (lo << (4 - rot))) & 0x0f;
    (mask & 0xf0) | rotated
}

/// Computes the 4-bit view-rotated inner-corner coping mask for swimming pool
/// tile `[x, y]` (`0x461e39`–`0x461f75`, table `[0x6e52a0..0x6e52c8]`
/// initialized at `0x461d2f`–`0x461e38`): bits 0..3 select `SPR2 0x430..0x433`
/// (+ `zoom * 4`).
fn pool_inner_corners(lot: &Lot, x: i32, y: i32, rotation: u8) -> u8 {
    struct CornerRule {
        self_req: u8,
        d1: [i32; 2],
        forbid1: u8,
        need1: u8,
        d2: [i32; 2],
        forbid2: u8,
        need2: u8,
        out_bit: u8,
    }
    const RULES: [CornerRule; 4] = [
        CornerRule {
            self_req: 0x03,
            d1: [-1, 0],
            forbid1: 0x01,
            need1: 0x08,
            d2: [0, -1],
            forbid2: 0x02,
            need2: 0x04,
            out_bit: 1,
        },
        CornerRule {
            self_req: 0x06,
            d1: [-1, 0],
            forbid1: 0x04,
            need1: 0x08,
            d2: [0, 1],
            forbid2: 0x02,
            need2: 0x01,
            out_bit: 2,
        },
        CornerRule {
            self_req: 0x09,
            d1: [0, -1],
            forbid1: 0x08,
            need1: 0x04,
            d2: [1, 0],
            forbid2: 0x01,
            need2: 0x02,
            out_bit: 8,
        },
        CornerRule {
            self_req: 0x0c,
            d1: [0, 1],
            forbid1: 0x08,
            need1: 0x01,
            d2: [1, 0],
            forbid2: 0x04,
            need2: 0x02,
            out_bit: 4,
        },
    ];
    if !lot.contains(x, y) {
        return 0;
    }
    let self_val = lot.pool[Lot::cell(x as usize, y as usize)];
    let mut bits = 0u8;
    for rule in &RULES {
        if (self_val & rule.self_req) != rule.self_req {
            continue;
        }
        let (x1, y1) = (x + rule.d1[0], y + rule.d1[1]);
        let (x2, y2) = (x + rule.d2[0], y + rule.d2[1]);
        if !lot.contains(x1, y1) || !lot.contains(x2, y2) {
            continue;
        }
        let v1 = lot.pool[Lot::cell(x1 as usize, y1 as usize)];
        if (v1 & rule.forbid1) != 0 || (v1 & rule.need1) == 0 {
            continue;
        }
        let v2 = lot.pool[Lot::cell(x2 as usize, y2 as usize)];
        if (v2 & rule.forbid2) != 0 || (v2 & rule.need2) == 0 {
            continue;
        }
        bits |= rule.out_bit;
    }
    rotate_cardinal_mask(bits, rotation)
}

/// Screen top-left of a tile's floor shadow mask (`0x4adac0` → `0x4b9613` →
/// `0x4b97ea`, `0x4ae8cc`–`0x4ae8e3`): project the tile centre at its rounded
/// terrain altitude (`0x478dfe`), then subtract half `tile_width` (matching
/// plain floor sprite alignment) and half the tile's screen height.
fn shadow_origin(
    camera: &Camera,
    lot: &Lot,
    [x, y]: [i32; 2],
    level: i32,
    tile_width: usize,
) -> [i32; 2] {
    let center = [x * 16 + 8, y * 16 + 8];
    let altitude = lot.altitude_at(center);
    let steps = (altitude + 0.5).trunc().clamp(0.0, 255.0) as u8;
    let screen = project_fine_terrain(camera, lot, camera.rotate_fine(center), level, Some(steps));
    [
        screen[0] - tile_width as i32 / 2,
        screen[1] - camera.zoom.tile_height() / 2,
    ]
}

/// The wall segments of level `index + 1`, in the order the original
/// draws them: by its wall graph (`0x540619`).
///
/// - **Corners.** Each lot corner gets a mask of the eight segments that
///   meet it (bits 0 to 7: the west wall above it, the north wall to its
///   left, the west and north walls of its own tile, the ascending
///   diagonal from the tile above-left, the descending diagonal of the
///   tile to its left, its tile's ascending diagonal, and the descending
///   diagonal of the tile above it). A valid corner where runs meet or
///   turn is a junction; one with a single segment is an end
///   ([`wall_corner`]).
/// - **Runs.** From each junction or end, in the order of their keys,
///   a run starts down each of its two forward slots: south (the west
///   wall, else the ascending diagonal) and east (the north wall, else the
///   descending diagonal), and follows straight on through plain corners
///   (`0x53cfa7`). Each is inserted before the runs with an equal start key
///   (`0x5409ab`, a lower bound).
/// - **View.** For the camera's rotation, each run's corners are turned
///   and its ends swapped when the far end now has the smaller key, which
///   reverses its segments (`0x5404b5`); then the runs are sorted by their
///   start keys with the original's sort (`0x540809`, `0x541389`).
///
/// The key of a corner is `(x + y, x - y)` in view coordinates. The
/// original turns the graph by each rotation's difference and sorts again
/// each time (`0x479360`), so the order of runs with equal keys follows
/// the rotations the view went through; this builds the graph at rotation
/// 0 and turns it to the camera's rotation in one step. Segments no run
/// covers (corners the original holds invalid, such as a wall and a
/// diagonal leaving one corner on the same side) go last, nearest first by
/// their corners' keys.
fn wall_order(camera: &Camera, lot: &Lot, index: usize) -> Vec<(i32, i32, Segment)> {
    let size = lot.size as i32;
    let walls = &lot.levels[index].walls;
    let has = |x: i32, y: i32, bit: u8| {
        (0..size).contains(&x)
            && (0..size).contains(&y)
            && walls[Lot::cell(x as usize, y as usize)].segments & bit != 0
    };
    let mask = |[x, y]: [i32; 2]| -> u8 {
        [
            has(x, y - 1, lot::WEST),
            has(x - 1, y, lot::NORTH),
            has(x, y, lot::WEST),
            has(x, y, lot::NORTH),
            has(x - 1, y - 1, lot::ASCENDING),
            has(x - 1, y, lot::DESCENDING),
            has(x, y, lot::ASCENDING),
            has(x, y - 1, lot::DESCENDING),
        ]
        .iter()
        .enumerate()
        .fold(0, |m, (i, &b)| m | (u8::from(b) << i))
    };
    let stops = |c: [i32; 2]| {
        let (junction, end) = wall_corner(mask(c));
        junction || end
    };
    let key = |[x, y]: [i32; 2]| (x + y, x - y);
    // A step from corner `c` down a forward slot: its direction, and the
    // segment's tile and bit.
    let step = |c: [i32; 2], m: u8, slot: u8| -> Option<([i32; 2], [i32; 2], u8)> {
        let [x, y] = c;
        match slot {
            0x44 if m & 0x04 != 0 => Some(([0, 1], c, lot::WEST)),
            0x44 if m & 0x40 != 0 => Some(([1, 1], c, lot::ASCENDING)),
            0x88 if m & 0x08 != 0 => Some(([1, 0], c, lot::NORTH)),
            0x88 if m & 0x80 != 0 => Some(([1, -1], [x, y - 1], lot::DESCENDING)),
            _ => None,
        }
    };
    let mut corners = Vec::new();
    for y in 0..=size {
        for x in 0..=size {
            if stops([x, y]) {
                corners.push([x, y]);
            }
        }
    }
    corners.sort_by_key(|&c| key(c));
    // Each run: its two ends, and its segments (tile and bit) from the
    // first end.
    type Run = ([i32; 2], [i32; 2], Vec<[i32; 3]>);
    let mut runs: Vec<Run> = Vec::new();
    let mut covered = BTreeSet::new();
    for &start in &corners {
        let m = mask(start);
        for slot in [0x44, 0x88] {
            let Some((dir, tile, bit)) = step(start, m, slot) else {
                continue;
            };
            if !has(tile[0], tile[1], bit) {
                continue;
            }
            let mut segments = vec![[tile[0], tile[1], i32::from(bit)]];
            let mut at = [start[0] + dir[0], start[1] + dir[1]];
            while !stops(at) && mask(at) != 0 {
                let Some((d, tile, bit)) = step(at, mask(at), slot) else {
                    break;
                };
                if d != dir || !has(tile[0], tile[1], bit) || segments.len() > 4 * lot::FULL {
                    break;
                }
                segments.push([tile[0], tile[1], i32::from(bit)]);
                at = [at[0] + dir[0], at[1] + dir[1]];
            }
            covered.extend(segments.iter().copied());
            let k = key(start);
            let i = runs.partition_point(|r| key(r.0) < k);
            runs.insert(i, (start, at, segments));
        }
    }
    let m = lot::FULL as i32;
    let turn = |[x, y]: [i32; 2]| match camera.rotation & 3 {
        1 => [m - y, x],
        2 => [m - x, m - y],
        3 => [y, m - x],
        _ => [x, y],
    };
    let mut view: Vec<((i32, i32), usize, bool)> = runs
        .iter()
        .enumerate()
        .map(|(i, (a, b, _))| {
            let (a, b) = (key(turn(*a)), key(turn(*b)));
            if b < a { (b, i, true) } else { (a, i, false) }
        })
        .collect();
    sims_core::sort::sort_by(&mut view, |a, b| a.0 < b.0);
    let segment = |[x, y, bit]: [i32; 3]| {
        segments(x, y)
            .into_iter()
            .find(|s| i32::from(s.bit) == bit)
            .map(|s| (x, y, s))
    };
    let mut ordered = Vec::new();
    for (_, i, reversed) in view {
        let run = &runs[i].2;
        if reversed {
            ordered.extend(run.iter().rev().filter_map(|&s| segment(s)));
        } else {
            ordered.extend(run.iter().filter_map(|&s| segment(s)));
        }
    }
    let mut rest = Vec::new();
    for x in 0..size {
        for y in 0..size {
            for s in segments(x, y) {
                if has(x, y, s.bit) && !covered.contains(&[x, y, i32::from(s.bit)]) {
                    let (lo, hi) = wall_draw_key(camera, s.from, s.to);
                    rest.push((lo, hi, x, y, s));
                }
            }
        }
    }
    rest.sort_by_key(|&(lo, hi, x, y, ref s)| (lo, hi, x, y, s.bit));
    ordered.extend(rest.into_iter().map(|(_, _, x, y, s)| (x, y, s)));
    ordered
}

/// Whether a wall corner with segment mask `m` ([`wall_order`]) is a
/// junction, where runs meet or turn, and whether it is a run's end (one
/// segment). A corner where a straight wall and a diagonal leave on the
/// same side is invalid, and neither (`0x540619`).
fn wall_corner(m: u8) -> (bool, bool) {
    let valid = !(m & 0x10 != 0 && m & 0x03 != 0
        || m & 0x20 != 0 && m & 0x06 != 0
        || m & 0x40 != 0 && m & 0x04 != 0
        || m & 0x80 != 0 && m & 0x09 != 0);
    let junction = m & 0x01 != 0 && m & 0xfa != 0
        || m & 0x02 != 0 && m & 0xf5 != 0
        || m & 0x08 != 0 && m & 0xf5 != 0
        || m & 0x10 != 0 && m & 0xaf != 0
        || m & 0x40 != 0 && m & 0xaf != 0
        || m & 0x80 != 0 && m & 0x5f != 0
        || m & 0x20 != 0 && m & 0x5f != 0;
    (valid && junction, valid && m.count_ones() == 1)
}

/// Puts a level's objects in the order the original draws them
/// (`0x4aa1d7`'s list, kept by the presenter): the objects not in another's
/// slot sorted by their tiles' view keys, `(x + y, x - y)` with the tile
/// turned to the view (`63 - y, x` and so on), with the original's sort,
/// from object ID order; each followed by what its slots hold, slot by
/// slot and at every depth, as the holder's draw walks them (`0x437c56`).
/// An object whose holder is not in `list` (a person, or one the pass
/// leaves out) is sorted as one in no slot.
///
/// The original sorts its registry, whose order this does not track; it
/// matters only for objects on the same tile key. It also leaves out
/// objects on the lot's outermost tiles, which are still drawn here.
fn object_order<T: Copy>(
    list: &mut Vec<T>,
    camera: &Camera,
    of: impl Fn(&T) -> (u16, &Object, [i32; 2]),
) {
    let ids: BTreeMap<u16, usize> = list.iter().enumerate().map(|(i, t)| (of(t).0, i)).collect();
    let holder = |t: &T| {
        let o = of(t).1;
        u16::try_from(o.data.get(CONTAINER).copied().unwrap_or(0))
            .ok()
            .filter(|&c| c != 0 && c != of(t).0)
            .and_then(|c| ids.get(&c).copied())
    };
    let mut children: BTreeMap<usize, Vec<(i16, u16, usize)>> = BTreeMap::new();
    let mut top = Vec::new();
    for (i, t) in list.iter().enumerate() {
        let (id, o, _) = of(t);
        match holder(t) {
            Some(h) => {
                let slot = o.data.get(CONTAINER_SLOT).copied().unwrap_or(-1);
                children.entry(h).or_default().push((slot, id, i));
            }
            None => top.push(i),
        }
    }
    let key = |i: &usize| {
        let [x, y] = of(&list[*i]).2.map(|n| n >> 4);
        let [vx, vy] = match camera.rotation & 3 {
            1 => [63 - y, x],
            2 => [63 - x, 63 - y],
            3 => [y, 63 - x],
            _ => [x, y],
        };
        (vx + vy, vx - vy)
    };
    sims_core::sort::sort_by(&mut top, |a, b| key(a) < key(b));
    for v in children.values_mut() {
        v.sort();
    }
    let mut order = Vec::with_capacity(list.len());
    let mut placed = vec![false; list.len()];
    for root in top {
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut placed[i], true) {
                continue;
            }
            order.push(i);
            if let Some(held) = children.get(&i) {
                stack.extend(held.iter().rev().map(|&(_, _, c)| c));
            }
        }
    }
    // Holders that hold each other, which no root reaches.
    order.extend((0..list.len()).filter(|&i| !placed[i]));
    let sorted: Vec<T> = order.into_iter().map(|i| list[i]).collect();
    *list = sorted;
}

/// Back-to-front view sort key for a wall segment (`0x54045b` → `0x540809` →
/// `0x54088d` → `0x44054c`), ordering its two rotated corners by
/// `(vx + vy, vx - vy)` ascending.
fn wall_draw_key(camera: &Camera, from: [i32; 2], to: [i32; 2]) -> ((i32, i32), (i32, i32)) {
    let m = lot::FULL as i32;
    let rot = |[x, y]: [i32; 2]| match camera.rotation & 3 {
        1 => [m - y, x],
        2 => [m - x, m - y],
        3 => [y, m - x],
        _ => [x, y],
    };
    let key = |[vx, vy]: [i32; 2]| (vx + vy, vx - vy);
    let va = key(rot(from));
    let vb = key(rot(to));
    if va <= vb { (va, vb) } else { (vb, va) }
}

/// Base screen position and depth anchor for an unheld object at unrotated
/// fine position `fine` on `level` with terrain `altitude` (`0x4b9613` ->
/// `0x4b97ea`): projects at the rounded whole terrain step `steps`, then
/// anchors depth from that screen row and exact `altitude`.
fn place_base(
    depth: &Depth,
    camera: &Camera,
    lot: &Lot,
    fine: [i32; 2],
    level: i32,
    altitude: f32,
) -> ([i32; 2], Anchor) {
    let steps = (altitude + 0.5).trunc().clamp(0.0, 255.0) as u8;
    let screen = project_fine_terrain(camera, lot, camera.rotate_fine(fine), level, Some(steps));
    let anchor = depth.anchor(screen[1], altitude, level);
    (screen, anchor)
}

/// Screen position and depth anchor for a held object (`0x437fdd`) or an
/// offset draw-group sprite (`0x437b7e`) via `0x4b9991`: projects onto the
/// sloped terrain at `fine` (`0x4a9dc7` with no altitude override), lifts by
/// `((total_z - ground_here) * (2 << zoom) + 0.5).trunc()`, and anchors
/// depth from the resulting screen row and `total_z`.
pub(crate) fn place_moved(
    depth: &Depth,
    camera: &Camera,
    lot: &Lot,
    fine: [i32; 2],
    level: i32,
    total_z: f32,
) -> ([i32; 2], Anchor) {
    let mut screen = project_fine_terrain(camera, lot, camera.rotate_fine(fine), level, None);
    let ground_here = lot.altitude_at(fine);
    let rise = ((total_z - ground_here) * (2 << camera.zoom.level()) as f32 + 0.5).trunc() as i32;
    screen[1] = screen[1].saturating_sub(rise);
    let anchor = depth.anchor(screen[1], total_z, level);
    (screen, anchor)
}

/// Where on the screen a draw group sprite that sits `object` away from
/// an object at `position` facing `facing` is placed, before its pixel
/// offset and rise: the camera's projection of [`part_fine`], on `level`
/// at `ground`. The sprite's x and y offset move it, as they move the
/// object's position (0x437b7e: the original projects the moved
/// position, 0x4b9991).
#[cfg(test)]
fn part_screen(
    camera: &Camera,
    position: [f32; 2],
    facing: i16,
    object: [f32; 3],
    level: i32,
    ground: i32,
) -> [i32; 2] {
    let fine = part_fine(position, facing, object);
    camera.project_fine(camera.rotate_fine(fine), level, ground)
}

/// The draw group view (one bit of 1, 4, 16 and 64) for an object facing
/// `facing` (data word 1: 0, 2, 4 or 6), turned by the camera's rotation
/// (the old prototype's reading in `legacy/crates/sims-render/src/lib.rs`).
pub fn view_direction(facing: i16, camera: &Camera) -> u32 {
    let bits = (1u32 << (facing & 7)).rotate_left(u32::from(camera.rotation & 3) * 2);
    (bits & 255) | (bits >> 8)
}

/// The wall an object in a wall stands in. A door or window faces along
/// its placement bit (data word 13) turned by its facing (data word 1),
/// as the original's 0x4358c3 does (the old prototype's `normal` in
/// `legacy/crates/sims-sim/src/lot_presenter/object_cutaway.rs`).
fn object_edge(position: [i32; 3], data: &[i16]) -> Option<Edge> {
    let placement = data.get(13).copied().unwrap_or(0) as u16;
    let facing = data.get(1).copied().unwrap_or(0);
    let bit: u16 = match placement & 5 {
        1 => 2,
        4 => 8,
        _ => return None,
    };
    let turn = (i32::from(facing) / 2).rem_euclid(4) as u32;
    let normal = ((bit << turn) | (bit >> (4 - turn))) & 15;
    let [x, y] = [position[0] >> 4, position[1] >> 4];
    let (a, b) = match normal {
        1 => ([x, y], [x, y + 1]),
        2 => ([x, y], [x + 1, y]),
        4 => ([x + 1, y], [x + 1, y + 1]),
        8 => ([x, y + 1], [x + 1, y + 1]),
        _ => return None,
    };
    Some(edge(a, b))
}

/// The wall style whose cut-down shape a cut window wall (style 22) and wall
/// caps take (`0x53ead5`, `0x53e652`).
const PLAIN_STYLE: u16 = 1;

/// The shadow code, turned into the view by `rotation`, and strength of the
/// first object on tile `[x, y]` of `level` that casts a shadow (`0x4ae844`,
/// `0x434e23`).
pub(crate) fn shadow_of_tile(
    world: &World,
    assets: &mut Assets,
    rotation: u8,
    level: i32,
    [x, y]: [i32; 2],
) -> Option<(u8, i16)> {
    let grid = world
        .arrays
        .get(&[3, 103][usize::try_from(level - 1).ok()?])?;
    let cell = Lot::cell(usize::try_from(x).ok()?, usize::try_from(y).ok()?) * 2;
    let word = grid.data.get(cell..cell + 2)?;
    let head = i16::from_le_bytes([word[0], word[1]]);
    shadow_of_chain(world, assets, rotation, head)
}

/// Walks an object-grid linked list head `next` and returns the shadow code
/// (turned by `rotation`) and strength of the first visible shadow caster.
pub(crate) fn shadow_of_chain(
    world: &World,
    assets: &mut Assets,
    rotation: u8,
    mut next: i16,
) -> Option<(u8, i16)> {
    // A chain longer than the lot's objects has looped.
    for _ in 0..world.objects.len() {
        let o = u16::try_from(next)
            .ok()
            .and_then(|id| world.objects.get(&id))?;
        let visible = o.data.get(HIDDEN).is_none_or(|&w| w == 0)
            && o.data.get(FLAGS).is_none_or(|&w| w & ZERO_EXTENT == 0);
        if visible
            && let Some(definition) = assets.definition_ref(o.guid)
            && !matches!(definition.kind, 2 | 9)
            && let Some(code) = definition.shadow
        {
            let facing = o.data.get(FACING).copied().unwrap_or(0) / 2;
            let code = shadows::rotate(code, facing.max(0) as u8);
            let code = shadows::rotate(code, rotation);
            return Some((code, definition.shadow_strength));
        }
        next = o.slots.first()?.object;
    }
    None
}

/// Computes `(at, anchor)` for a wall face of dimensions `(width, height)`
/// standing between projected ends `[a, b]` on `level` with terrain `heights`.
#[allow(clippy::too_many_arguments)]
fn wall_placement(
    camera: &Camera,
    depth: &Depth,
    width: usize,
    height: usize,
    frame: usize,
    [a, b]: [[i32; 2]; 2],
    heights: [i32; 2],
    level: i32,
    raise: i32,
) -> ([i32; 2], Anchor) {
    let rise = 4 << camera.zoom.level();
    let left = if a[0] <= b[0] { a } else { b };
    let (w, h) = (width as i32, height as i32);
    // The sprite's bottom row sits one pixel above the corner's projected point
    // (matched against the original's screen, 2026-11-14).
    let at = match frame {
        0 | 2 => [left[0], left[1] - h],
        1 => [left[0], left[1] - (h - rise)],
        // An edge-on diagonal stands centred on its column, a pixel higher at
        // far zoom, two at medium and four at near than the other frames.
        _ => {
            let bottom = if a[1] > b[1] { a } else { b };
            let lift = 1 << camera.zoom.index();
            [bottom[0] - w / 2, bottom[1] - h - lift]
        }
    };
    let anchor = match (frame, a[1].cmp(&b[1])) {
        (2, _) if a[0] <= b[0] => depth.anchor(a[1], heights[0] as f32, level),
        (2, _) => depth.anchor(b[1], heights[1] as f32, level),
        (3, _) => {
            let mean = (heights[0] + heights[1]) as f32 / 2.0;
            depth.anchor((a[1] + b[1]) / 2, mean, level)
        }
        (_, std::cmp::Ordering::Less) => depth.anchor(b[1], heights[1] as f32, level),
        _ => depth.anchor(a[1], heights[0] as f32, level),
    };
    ([at[0], at[1] - raise], anchor)
}

/// How far up the original moves the strip on a marked edge-on diagonal:
/// `(92 >> (3 - zoom)) + 1`, or `+ 2` at near zoom, passed to OffsetRect
/// before the strip is drawn (0x53eb6d to 0x53ebf5).
fn edge_on_strip_lift(zoom: crate::Zoom) -> i32 {
    let level = zoom.level() as i32;
    (92 >> (3 - level)) + if level == 3 { 2 } else { 1 }
}

/// Which side of a wall shows, as an index into `Scene::sides` (0 the
/// side its normal points to, 1 the other), from its normal turned into
/// the view. A wall shows the side facing the camera. An edge-on
/// diagonal (normal (1, −1) or (−1, 1) in the view) faces neither way and
/// shows the side facing view direction (−1, 1), as the original's runs
/// store it (0x53f6c1, turned with the view at 0x53d93a).
fn shown_side(normal: [i32; 2]) -> usize {
    if normal[0] + normal[1] == 0 {
        usize::from(normal != [-1, 1])
    } else {
        usize::from(normal[0] + normal[1] < 0)
    }
}

/// The frame of a wall by its direction on screen, from the screen points
/// of its two ends.
fn wall_frame([a, b]: [[i32; 2]; 2]) -> usize {
    let (left, right) = if a[0] <= b[0] { (a, b) } else { (b, a) };
    if left[0] == right[0] {
        3
    } else if left[1] == right[1] {
        2
    } else if left[1] > right[1] {
        0
    } else {
        1
    }
}

/// The part of an image drawn with its left edge at screen column `x`
/// that falls within screen columns `from` (inclusive) to `to`
/// (exclusive), and how many columns were cut off its left; None if
/// nothing is left.
#[cfg(test)]
fn within(image: &Image, x: i32, [from, to]: [i32; 2]) -> Option<(Image, i32)> {
    let width = i64::try_from(image.width).ok()?;
    let left = (i64::from(from) - i64::from(x)).clamp(0, width);
    let right = (i64::from(to) - i64::from(x)).clamp(left, width);
    if left == right {
        return None;
    }
    let (left, right) = (usize::try_from(left).ok()?, usize::try_from(right).ok()?);
    let w = right - left;
    let mut rgba = Vec::with_capacity(w * image.height * 4);
    let mut depth = image
        .depth
        .as_ref()
        .map(|_| Vec::with_capacity(w * image.height));
    for row in 0..image.height {
        let start = row * image.width;
        rgba.extend_from_slice(image.rgba.get((start + left) * 4..(start + right) * 4)?);
        if let (Some(out), Some(d)) = (depth.as_mut(), image.depth.as_ref()) {
            out.extend_from_slice(d.get(start + left..start + right)?);
        }
    }
    let part = Image {
        width: w,
        height: image.height,
        rgba,
        depth,
    };
    Some((part, i32::try_from(left).ok()?))
}

/// Image sub-rectangle `[ix0, iy0, ix1, iy1]` kept for one half of a split
/// floor tile (identical to `split_half` without cloning the image).
fn split_half_rect(
    camera: &Camera,
    width: usize,
    height: usize,
    segments: u8,
    half: usize,
) -> [usize; 4] {
    let half_h = ((4 << camera.zoom.level()) as usize).min(height);
    let corner = ((8 << camera.zoom.level()) as usize).min(width);
    let level = (segments & lot::DESCENDING != 0) != (camera.rotation & 1 == 1);
    let [x, y] = unrotate_local(camera, if level { [4, 4] } else { [4, 12] });
    let first = if segments & lot::DESCENDING != 0 {
        x + y < 16
    } else {
        x < y
    };
    let upper = first == (half == 0);
    match (level, upper) {
        (true, true) => [0, 0, width, half_h],
        (true, false) => [0, half_h, width, height],
        (false, true) => [0, 0, corner, height],
        (false, false) => [corner, 0, width, height],
    }
}

/// One half of a floor image for a tile a diagonal wall splits: half 0
/// holds corner (0, 0) under a descending wall, or corner (0, 1) under an
/// ascending one.
#[cfg(test)]
fn split_half(camera: &Camera, image: &Image, segments: u8, half: usize) -> Image {
    let [ix0, iy0, ix1, iy1] = split_half_rect(camera, image.width, image.height, segments, half);
    let mut out = image.clone();
    for py in 0..image.height {
        for px in 0..image.width {
            if !(ix0..ix1).contains(&px) || !(iy0..iy1).contains(&py) {
                out.rgba[(py * image.width + px) * 4 + 3] = 0;
            }
        }
    }
    out
}

/// Floor sprite ID for a non-split tile storing `id` on `level` (`1` or `2`),
/// drawn at camera rotation `camera` when the lot's level-1 floor array was
/// saved at `stored` ([`Lot::floor_rotation`]).
///
/// Floor IDs `10` and `11` are the road's centre-line dashes, one running
/// along each horizontal axis. The original has no rotated floor sprites: on
/// each odd-parity camera turn (`abs(old - new) & 1 != 0`), `0x4a7dfe`
/// (`0x4a8042` to `0x4a80b1`) swaps `10` and `11` (`id ^ 1`) in place across
/// `0..lot.size` of level 1's floor array (`[lot + 0x24]`), leaving level 2
/// (`[lot + 0x28]`) and split-tile wall floors alone. Loading passes
/// `force_no_swap = 1` (`0x4a861f`), so saved arrays keep the orientation of
/// their saved `camera_direction` (`HOUS 0`).
fn floor_for_rotation(id: u16, level: i32, camera: u8, stored: u8) -> u16 {
    if level == 1 && (camera ^ stored) & 1 != 0 && matches!(id, 10 | 11) {
        id ^ 1
    } else {
        id
    }
}

/// A position inside a tile (sixteenths), from view to lot axes.
fn unrotate_local(camera: &Camera, [x, y]: [i32; 2]) -> [i32; 2] {
    match camera.rotation & 3 {
        1 => [y, 16 - x],
        2 => [16 - x, 16 - y],
        3 => [16 - y, x],
        _ => [x, y],
    }
}

/// Whether a dynamic sprite shows: an object keeps one word per dynamic
/// sprite, counted from its definition's first dynamic sprite ID, and a
/// sprite outside that range always shows (the original's 0x437f99, read
/// by `legacy/crates/sims-render/src/lib.rs`).
pub fn sprite_visible(sprite: u16, base: i16, flags: &[i16]) -> bool {
    let index = i32::from(sprite) - i32::from(base);
    match usize::try_from(index).ok().and_then(|i| flags.get(i)) {
        Some(&flag) => flag != 0,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trim_along_a_run_is_capped_without_room_0_but_a_corners_is_not() {
        let night = crate::light::brightest(2);
        let (corner, run) = trim_lights(None, night);
        assert_eq!(corner, Light::FULL);
        assert_eq!(run, Light::from_fraction(0.8, 1.0));
        // By day both are full; with room 0 both take its light.
        assert_eq!(trim_lights(None, 1.0), (Light::FULL, Light::FULL));
        let room = Light::from_fraction(0.6, night);
        assert_eq!(trim_lights(Some(0.6), night), (room, room));
    }

    #[test]
    fn the_edge_on_strip_is_lifted_as_the_original_offsets_it() {
        // (92 >> (3 - zoom)) + 1, + 2 at near (0x53eb6d to 0x53ebf5);
        // these put the strip's top row on the original's at each zoom
        // (House02, 2026-11-20).
        assert_eq!(edge_on_strip_lift(Zoom::Far), 24);
        assert_eq!(edge_on_strip_lift(Zoom::Medium), 47);
        assert_eq!(edge_on_strip_lift(Zoom::Near), 94);
    }

    fn camera(rotation: u8) -> Camera {
        Camera {
            zoom: Zoom::Near,
            rotation,
            level: 1,
            origin: [0, 0],
            walls: Walls::Up,
            focus: crate::Focus::default(),
        }
    }

    #[test]
    fn trim_frames_follow_the_original_table() {
        assert_eq!(trim_frame(0), None);
        assert_eq!(trim_frame(1), Some(0));
        // A straight wall through a corner, each way.
        assert_eq!(trim_frame(1 | 4), Some(4));
        assert_eq!(trim_frame(2 | 8), Some(9));
        assert_eq!(trim_frame(0x10), Some(15));
        assert_eq!(trim_frame(0x14), Some(16));
        assert_eq!(trim_frame(0x10 | 0x40), Some(29));
        assert_eq!(trim_frame(0xf0), Some(45));
        // No frame for a diagonal with a wall at right angles to it.
        assert_eq!(trim_frame(0x11), None);
        assert_eq!(trim_frame(0xff), None);
    }

    #[test]
    fn a_wall_marks_opposite_bits_at_its_two_ends() {
        for d in [[0, -1], [-1, 0], [-1, -1], [-1, 1]] {
            let (a, b) = (caps::bit(d), caps::bit([-d[0], -d[1]]));
            assert_ne!(a, 0);
            // Both ends of a wall, straight through a corner, have a frame.
            assert!(trim_frame(a | b).is_some(), "{d:?}");
            assert_eq!(a.count_ones() + b.count_ones(), (a | b).count_ones());
        }
        assert_eq!(caps::bit([2, 0]), 0);
    }

    #[test]
    fn a_wall_takes_light_from_the_triangle_touching_it() {
        // Sample points land inside the tile, in the triangle that holds
        // the wall's side (1 SW, 2 SE, 3 NE, 4 NW; see `Lot::room_at`).
        let [west, north, descending, ascending] = segments(3, 5);
        assert_eq!(wall_side(&west, 0), [50, 88]);
        assert_eq!(wall_side(&west, 1), [46, 88]);
        assert_eq!(wall_side(&north, 0), [56, 82]);
        assert_eq!(wall_side(&descending, 0), [58, 90]);
        assert_eq!(wall_side(&descending, 1), [54, 86]);
        assert_eq!(wall_side(&ascending, 0), [58, 86]);
        assert_eq!(wall_side(&ascending, 1), [54, 90]);
    }

    /// A lamp's offset to a tile turns as the camera turns their
    /// positions, so its pool stays centred on it in every view, the
    /// off-axis tiles' gradients too.
    #[test]
    fn lamp_offsets_turn_as_the_camera_turns_positions() {
        for rotation in 0..4 {
            let camera = camera(rotation);
            for (lamp, offset) in [([10, 20], [3, 0]), ([10, 20], [-2, 1]), ([40, 5], [1, -3])] {
                let tile = [lamp[0] + offset[0], lamp[1] + offset[1]];
                let [a, b] = [tile, lamp].map(|t| camera.rotate_fine(t.map(|n| n * 16)));
                let turned = rotate_direction(rotation, offset).map(|n| n * 16);
                assert_eq!([a[0] - b[0], a[1] - b[1]], turned, "{rotation} {offset:?}");
            }
        }
    }

    /// A straight wall's lamps come from the lot tile beside it that the
    /// camera sees, at every rotation; diagonals have none.
    #[test]
    fn a_wall_reads_the_lamps_of_the_tile_on_its_seen_side() {
        // The west edge of lot tile (10, 20): between tiles (9, 20) and
        // (10, 20). At rotation 0 the view is the lot: the wall runs along
        // y and (10, 20) is on its +x side, the side seen.
        let west = ([10, 20], [10, 21]);
        let north = ([10, 20], [11, 20]);
        let seen = |rotation, (from, to)| seen_tile(&camera(rotation), from, to);
        assert_eq!(seen(0, west), Some((true, [10, 20])));
        assert_eq!(seen(0, north), Some((false, [10, 20])));
        // Turned a quarter, lot +x is view +y, lot +y is view -x: the
        // west wall runs along view x with (10, 20) on its seen side, and
        // the north wall along view y with (10, 19) on its seen side.
        assert_eq!(seen(1, west), Some((false, [10, 20])));
        assert_eq!(seen(1, north), Some((true, [10, 19])));
        assert_eq!(seen(2, west), Some((true, [9, 20])));
        assert_eq!(seen(2, north), Some((false, [10, 19])));
        assert_eq!(seen(3, west), Some((false, [9, 20])));
        assert_eq!(seen(3, north), Some((true, [10, 20])));
        assert_eq!(seen(0, ([10, 20], [11, 21])), None);
        // The tile is beside the wall, on the side whose view tile has the
        // wall on its low edge.
        for rotation in 0..4 {
            let camera = camera(rotation);
            for (from, to) in [west, north] {
                let (along_y, tile) = seen_tile(&camera, from, to).unwrap();
                let view = camera.rotate_tile(tile);
                let [a, b] = [from, to].map(|c| camera.rotate_fine(c.map(|n| n * 16)));
                let edge = if along_y { a[0] } else { a[1] } / 16;
                assert_eq!(view[usize::from(!along_y)], edge, "{rotation}");
                assert_eq!(
                    a[usize::from(along_y)].min(b[usize::from(along_y)]) / 16,
                    view[usize::from(along_y)]
                );
            }
        }
    }

    #[test]
    fn an_edge_on_diagonal_shows_the_side_facing_view_minus_one_one() {
        // Face-on walls show the side facing the camera.
        assert_eq!(shown_side([1, 0]), 0);
        assert_eq!(shown_side([1, 1]), 0);
        assert_eq!(shown_side([0, -1]), 1);
        assert_eq!(shown_side([-1, -1]), 1);
        // The Goths' bay ends: at rotation 0 the ascending diagonal's
        // normal (1, -1) points away from (-1, 1), so its other side (the
        // shingles) shows; at rotation 1 the descending one's turns to
        // (-1, 1), and its own side shows. Both checked on screen.
        assert_eq!(shown_side(rotate_direction(0, [1, -1])), 1);
        assert_eq!(shown_side(rotate_direction(1, [1, 1])), 0);
    }

    #[test]
    fn view_directions_turn_with_the_camera() {
        assert_eq!(view_direction(0, &camera(0)), 1);
        assert_eq!(view_direction(2, &camera(0)), 4);
        assert_eq!(view_direction(0, &camera(1)), 4);
        assert_eq!(view_direction(6, &camera(1)), 1);
        assert_eq!(view_direction(4, &camera(3)), 4);
    }

    fn no_attach(_: u16, _: usize) -> Option<SlotPoint> {
        None
    }

    /// [`held`] for an object that is drawn.
    fn held_drawn(
        objects: &CowMap<u16, Object>,
        slot: impl FnMut(u32, usize) -> Option<[f32; 3]>,
        attach: impl Fn(u16, usize) -> Option<SlotPoint>,
        id: u16,
        base: [f32; 3],
    ) -> (bool, [f32; 3]) {
        held(objects, slot, attach, id, base).expect("drawn")
    }

    fn person_object(id: u16, position: [i32; 3]) -> Object {
        use sims_formats::saved_objects::{Interaction, Person};
        let idle = Interaction {
            join: 0,
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: -1,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: 0.0,
            flags: 0,
        };
        let mut o = object(id, 77, position, &[]);
        o.person = Some(Person {
            animation_events: 0,
            engaged: 0,
            appearance: Default::default(),
            accessories: Vec::new(),
            animations: Default::default(),
            routing_state: 0,
            floats: [0.0; 9],
            previous_motives: [0.0; 16],
            motives: [0.0; 16],
            data: vec![0; 128],
            routing_frames: 0,
            active: idle.clone(),
            last: idle,
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        });
        o
    }

    /// What a Sim holds is drawn at the Sim's attach point for its slot
    /// (0x459dc0 → 0x59185d), not at the Sim's feet: a plate in the carry
    /// slot (slot 0) follows the given palm point, food on the plate
    /// stacks the plate's slot offset on top, and with no attach point
    /// (the person not drawn, or slot 1) the plate stays where it is.
    #[test]
    fn person_held_objects_follow_the_persons_attach_point() {
        const PLATE: u32 = 8;
        let slot = |guid: u32, i: usize| match (guid, i) {
            (PLATE, 0) => Some([0.0, 0.0, 0.25]),
            _ => None,
        };
        let palm = [131.25, 207.5, 9.75];
        let pelvis = [128.0, 200.0, 6.0];
        let hands = PersonAttach {
            palm: Some(palm),
            head_top: Some([128.0, 199.0, 12.0]),
            pelvis: Some(pelvis),
        };
        let attach = |person: u16, i: usize| (person == 1).then(|| hands.slot_point(i)).flatten();
        let mut objects = CowMap::new();
        objects.insert(1, person_object(1, [128, 200, 1]));
        // The plate in the Sim's carry slot, its own position the Sim's.
        objects.insert(
            2,
            object(
                2,
                PLATE,
                [128, 200, 1],
                &[(CONTAINER, 1), (CONTAINER_SLOT, 0)],
            ),
        );
        // Food on the plate's slot 0.
        objects.insert(
            3,
            object(3, 9, [128, 200, 1], &[(CONTAINER, 2), (CONTAINER_SLOT, 0)]),
        );
        let base = [128.0, 200.0, 0.0];
        assert_eq!(held_drawn(&objects, slot, attach, 2, base), (true, palm));
        assert_eq!(
            held_drawn(&objects, slot, attach, 3, base),
            (true, [palm[0], palm[1], palm[2] + 0.25])
        );
        // The point moves with the hand from frame to frame.
        let moved = PersonAttach {
            palm: Some([140.0, 190.0, 8.0]),
            ..hands
        };
        let follow = |_: u16, i: usize| moved.slot_point(i);
        assert_eq!(
            held_drawn(&objects, slot, follow, 2, base),
            (true, [140.0, 190.0, 8.0])
        );
        // Slot 2 draws at the pelvis; slot 1 (the "HEAD" record) has no
        // point here, so the plate keeps its own position.
        objects.get_mut(&2).unwrap().data[CONTAINER_SLOT] = 2;
        assert_eq!(held_drawn(&objects, slot, attach, 2, base), (true, pelvis));
        objects.get_mut(&2).unwrap().data[CONTAINER_SLOT] = 1;
        assert_eq!(held_drawn(&objects, slot, attach, 2, base), (false, base));
        // Without the person's points, as before: no offset from the Sim.
        objects.get_mut(&2).unwrap().data[CONTAINER_SLOT] = 0;
        assert_eq!(
            held_drawn(&objects, slot, no_attach, 2, base),
            (false, base)
        );
        // A non-finite point is no point: the object's own position, drawn
        // as unheld (not through the moved-object projection).
        let broken = |_: u16, _: usize| Some(SlotPoint::At([f32::NAN, 0.0, 0.0]));
        assert_eq!(held_drawn(&objects, slot, broken, 2, base), (false, base));
        // Nor is a finite point more than a lot's width away, which would
        // overflow the integer projection.
        let far = |_: u16, _: usize| Some(SlotPoint::At([1.0e9, 0.0, 0.0]));
        assert_eq!(held_drawn(&objects, slot, far, 2, base), (false, base));
        let off_lot = base[0] + (camera::ARRAY_SIZE * 16) as f32 + 1.0;
        let past = |_: u16, _: usize| Some(SlotPoint::At([off_lot, base[1], base[2]]));
        assert_eq!(held_drawn(&objects, slot, past, 2, base), (false, base));
        // A slot whose point is unbound (the Side Show Assistant's hand) is
        // skipped by the person's draw, with what it holds: the plate and
        // the food on it are not drawn.
        let handless = PersonAttach {
            palm: None,
            ..hands
        };
        let unbound = |_: u16, i: usize| handless.slot_point(i);
        assert_eq!(held(&objects, slot, unbound, 2, base), None);
        assert_eq!(held(&objects, slot, unbound, 3, base), None);
        // The person's other points still draw.
        objects.get_mut(&2).unwrap().data[CONTAINER_SLOT] = 2;
        assert_eq!(held_drawn(&objects, slot, unbound, 2, base), (true, pelvis));
    }

    #[test]
    fn person_attach_slots_follow_the_persons_draw() {
        let p = PersonAttach {
            palm: Some([1.0; 3]),
            head_top: Some([2.0; 3]),
            pelvis: Some([3.0; 3]),
        };
        assert_eq!(p.for_slot(0), Some([1.0; 3]));
        assert_eq!(p.for_slot(1), None);
        assert_eq!(p.for_slot(2), Some([3.0; 3]));
        assert_eq!(p.for_slot(3), Some([2.0; 3]));
        assert_eq!(p.for_slot(4), Some([1.0; 3]));
        assert_eq!(p.for_slot(5), None);
        let handless = PersonAttach { palm: None, ..p };
        assert_eq!(handless.slot_point(0), Some(SlotPoint::Unbound));
        assert_eq!(handless.slot_point(4), Some(SlotPoint::Unbound));
        assert_eq!(handless.slot_point(1), None, "slot 1 is not drawn here");
        assert_eq!(handless.slot_point(3), Some(SlotPoint::At([2.0; 3])));
    }

    fn object(id: u16, guid: u32, position: [i32; 3], words: &[(usize, i16)]) -> Object {
        let mut data = vec![0; 64];
        for &(i, v) in words {
            data[i] = v;
        }
        Object {
            id,
            guid,
            footprint: [0; 4],
            position,
            unknown_short: 0,
            attributes: Vec::new(),
            temps: Vec::new(),
            data,
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: 0,
            relationships: Vec::new(),
            slots: Vec::new(),
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: None,
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        }
    }

    #[test]
    fn held_objects_stand_on_their_holders_slot() {
        const TABLE: u32 = 7;
        const TRAY: u32 = 8;
        let slot = |guid: u32, i: usize| match (guid, i) {
            (TABLE, 1) => Some([4.0, 0.0, 4.0]),
            (TABLE, 2) => Some([0.1, 0.0, 0.0]),
            (TRAY, 0) => Some([-9.6, 0.0, 0.0]),
            _ => None,
        };
        let mut objects = CowMap::new();
        // A table facing 2 (one quarter turn) holds a lamp in its slot 1.
        objects.insert(1, object(1, TABLE, [100, 200, 1], &[(1, 2)]));
        objects.insert(
            2,
            object(2, 9, [100, 200, 1], &[(CONTAINER, 1), (CONTAINER_SLOT, 1)]),
        );
        let base = [100.0, 200.0, 0.0];
        // The slot's x turns with the table into y; its height lifts.
        assert_eq!(
            held_drawn(&objects, slot, no_attach, 2, base),
            (true, [100.0, 204.0, 4.0])
        );
        assert_eq!(
            held_drawn(&objects, slot, no_attach, 1, base),
            (false, base)
        );
        // An odd holder facing clears both horizontal offsets (0x438044 -> 0x439109).
        objects.get_mut(&1).unwrap().data[FACING] = 3;
        assert_eq!(
            held_drawn(&objects, slot, no_attach, 2, base),
            (true, [100.0, 200.0, 4.0])
        );
        objects.get_mut(&1).unwrap().data[FACING] = 2;
        // A holder without that slot puts it at its own place.
        objects.get_mut(&2).unwrap().data[CONTAINER_SLOT] = 5;
        assert_eq!(held_drawn(&objects, slot, no_attach, 2, base), (true, base));
        objects.get_mut(&2).unwrap().data[CONTAINER_SLOT] = 1;
        // A hidden holder still traverses its slots (0x4378fa -> 0x437c56).
        objects.get_mut(&1).unwrap().data[HIDDEN] = 1;
        assert_eq!(
            held_drawn(&objects, slot, no_attach, 2, base),
            (true, [100.0, 204.0, 4.0])
        );
        objects.get_mut(&1).unwrap().data[HIDDEN] = 0;
        // Multi-level slots accumulate sequentially in root-to-child f32 order
        // (0x438049–0x438090): ((32.0 + 0.1) + -9.6) + 0.5 truncates to 22,
        // whereas summing offsets first gives 32.0 + -9.5 + 0.5 = 23.
        objects.insert(4, object(4, TABLE, [32, 0, 1], &[(FACING, 0)]));
        objects.insert(
            5,
            object(
                5,
                TRAY,
                [32, 0, 1],
                &[(FACING, 0), (CONTAINER, 4), (CONTAINER_SLOT, 2)],
            ),
        );
        objects.insert(
            6,
            object(6, 9, [32, 0, 1], &[(CONTAINER, 5), (CONTAINER_SLOT, 0)]),
        );
        let (is_held, pose) = held_drawn(&objects, slot, no_attach, 6, [32.0, 0.0, 0.0]);
        assert!(is_held);
        assert_eq!((pose[0] + 0.5).trunc() as i32, 22);
        // Holders that hold each other, or an object that holds itself,
        // give no offset, however many other objects there are.
        objects.get_mut(&1).unwrap().data[CONTAINER] = 2;
        assert_eq!(
            held_drawn(&objects, slot, no_attach, 2, base),
            (false, base)
        );
        objects.insert(
            3,
            object(3, TABLE, [0, 0, 1], &[(CONTAINER, 3), (CONTAINER_SLOT, 1)]),
        );
        assert_eq!(
            held_drawn(&objects, slot, no_attach, 3, [0.0; 3]),
            (false, [0.0; 3])
        );
    }

    #[test]
    fn a_persons_holders_and_all_they_hold_draw_after_the_people() {
        use sims_formats::saved_objects::Slot;
        let mut objects = CowMap::new();
        // Sets entry `index` of `at`'s saved slot list to `id`. Entry 0
        // links to the next object on the tile; slot `n` (from 0) is entry
        // `n + 1`.
        let entry = |objects: &mut CowMap<u16, Object>, at: u16, index: usize, id: u16| {
            let slots = &mut objects.get_mut(&at).unwrap().slots;
            if slots.len() <= index {
                let empty = Slot {
                    unknown: 0,
                    object: 0,
                };
                slots.resize(index + 1, empty);
            }
            slots[index].object = id as i16;
        };
        // `at` holds `id` in its slot `slot` (from 0), by the link words
        // and in the holder's slot list.
        let put = |objects: &mut CowMap<u16, Object>, id: u16, at: u16, slot: usize| {
            let o = objects.get_mut(&id).unwrap();
            o.data[LINK_SLOT] = slot as i16 + 1;
            o.data[BEFORE] = at as i16;
            entry(objects, at, slot + 1, id);
        };
        // `id` follows `before` on its tile.
        let follow = |objects: &mut CowMap<u16, Object>, id: u16, before: u16| {
            let o = objects.get_mut(&id).unwrap();
            o.data[LINK_SLOT] = 0;
            o.data[BEFORE] = before as i16;
            entry(objects, before, 0, id);
        };
        for id in 1..=10 {
            objects.insert(id, object(id, id.into(), [0, 0, 1], &[(LINK_SLOT, -1)]));
        }
        // A bench (1) holds a person (2) and a book (3); the person holds
        // a plumb bob (4); a table (5) holds a lamp (6); a person (7)
        // stands alone; a person (8) holds a drink (9).
        put(&mut objects, 2, 1, 1);
        put(&mut objects, 3, 1, 0);
        put(&mut objects, 4, 2, 0);
        put(&mut objects, 6, 5, 0);
        put(&mut objects, 9, 8, 0);
        // Object 10 follows the bench on its tile: a neighbour, not held.
        follow(&mut objects, 10, 1);
        let people = BTreeSet::from([2, 7, 8]);
        assert_eq!(held_trees(&objects, &people), BTreeSet::from([1, 3, 4, 9]));
        // What each person's own draw draws: not the bench they sit on,
        // nor its book.
        let is_person = |id| people.contains(&id);
        assert_eq!(held_by_in(&objects, 2, is_person), BTreeSet::from([4]));
        assert_eq!(held_by_in(&objects, 8, is_person), BTreeSet::from([9]));
        assert_eq!(held_by_in(&objects, 7, is_person), BTreeSet::new());
        // The bench's tree, as a person's, stops at the person it holds.
        assert_eq!(held_by_in(&objects, 1, is_person), BTreeSet::from([3]));
        // Nor is an object that follows a person on their tile.
        entry(&mut objects, 1, 0, 0);
        follow(&mut objects, 10, 8);
        assert_eq!(held_trees(&objects, &people), BTreeSet::from([1, 3, 4, 9]));
        entry(&mut objects, 8, 0, 0);
        objects.get_mut(&10).unwrap().data[LINK_SLOT] = -1;
        objects.get_mut(&10).unwrap().data[BEFORE] = 0;
        // Data word 2 is not the link: a stale one changes nothing.
        objects.get_mut(&2).unwrap().data[CONTAINER] = 5;
        assert_eq!(held_trees(&objects, &people), BTreeSet::from([1, 3, 4, 9]));
        // A person who follows another object on its tile is held by what
        // that first object is in: here object 10, in the table's slot 2,
        // with the bench in its slot 1. The tree is what the table's slots
        // reach, so the person, linked from 10's tile entry, and their own
        // bob are left out.
        put(&mut objects, 1, 5, 1);
        entry(&mut objects, 1, 2, 0);
        put(&mut objects, 10, 5, 2);
        follow(&mut objects, 2, 10);
        assert_eq!(
            held_trees(&objects, &people),
            BTreeSet::from([1, 3, 5, 6, 9, 10])
        );
        // Holders that hold each other are one tree, whichever of them
        // the climb starts from.
        put(&mut objects, 5, 3, 1);
        assert_eq!(
            held_trees(&objects, &people),
            BTreeSet::from([1, 3, 5, 6, 9, 10])
        );
    }

    #[test]
    fn a_long_chain_of_huge_slots_stays_within_a_lot() {
        let huge = |_: u32, _: usize| Some([f32::MAX, -1e30, f32::INFINITY]);
        let mut objects = CowMap::new();
        objects.insert(1, object(1, 1, [0, 0, 1], &[]));
        for id in 2..200u16 {
            let below = (id - 1) as i16;
            objects.insert(
                id,
                object(id, 1, [0, 0, 1], &[(CONTAINER, below), (CONTAINER_SLOT, 0)]),
            );
        }
        let limit = camera::ARRAY_SIZE * 16;
        let (is_held, [x, y, lift]) = held_drawn(&objects, huge, no_attach, 199, [0.0; 3]);
        assert!(is_held);
        assert!(x.abs() <= limit as f32 && y.abs() <= limit as f32);
        assert!(lift.abs() <= limit as f32);
    }

    #[test]
    fn slot_heights_follow_the_category() {
        use crate::assets::slot_height;
        assert_eq!(slot_height(1, 3.0), 0.0);
        assert_eq!(slot_height(2, 0.0), 2.5);
        assert_eq!(slot_height(3, 0.0), 4.0);
        assert_eq!(slot_height(8, 5.0), 4.0);
        assert_eq!(slot_height(4, 0.0).to_bits(), 0x4081_999a);
        assert_eq!(slot_height(0, 5.0), 5.0);
        assert_eq!(slot_height(5, 4.0), 4.0);
    }

    fn flat_lot(height: u8) -> Lot {
        let full = lot::FULL * lot::FULL;
        let empty = || lot::Level {
            floors: vec![0; full],
            walls: vec![WallCell::default(); full],
            rooms: None,
        };
        Lot {
            size: 52,
            heights: vec![[height; 4]; full],
            grass: vec![0; full],
            pool: vec![0xff; full],
            water: vec![0xff; full],
            levels: [empty(), empty()],
            outdoor_rooms: BTreeSet::new(),
            room_lights: BTreeMap::new(),
            room_fractions: BTreeMap::new(),
            outdoor_light: None,
            time_of_day: 0,
            house: 0,
            halves: BTreeMap::new(),
            lamps: [
                vec![Default::default(); full],
                vec![Default::default(); full],
            ],
            floor_rotation: 0,
        }
    }

    #[test]
    fn sprite_world_offsets_move_depth_with_the_object_and_camera() {
        let position = [100.0, 200.0];
        let base = 2.0;
        let lot = flat_lot(2);
        let d = Depth::new(&camera(0));
        let at =
            |c: u8, fine, height| place(&d, &camera(c), camera(c).rotate_fine(fine), 1, height);
        let part = |c: u8, facing, object: [f32; 3]| {
            let fine = part_fine(position, facing, object);
            let total_z = base + sprite_offset(object)[2];
            place_moved(&d, &camera(c), &lot, fine, 1, total_z).1
        };
        assert_eq!(part(0, 0, [0.0; 3]), at(0, [100, 200], base));
        // One height unit up is one terrain step, and nearer.
        assert_eq!(part(0, 0, [0.0, 0.0, 1.0]), at(0, [100, 200], base + 1.0));
        assert!(part(0, 0, [0.0, 0.0, 1.0]).depth < part(0, 0, [0.0; 3]).depth);
        // Sixteen units along x is one tile, at facing 0; at facing 2 (a
        // quarter turn) x becomes y.
        assert_eq!(part(0, 0, [16.0, 0.0, 0.0]), at(0, [116, 200], base));
        assert_eq!(part(0, 2, [16.0, 0.0, 0.0]), at(0, [100, 216], base));
        // The camera's rotation turns the moved point like any other.
        assert_eq!(part(1, 2, [16.0, 0.0, 0.0]), at(1, [100, 216], base));
        // An odd facing leaves the offset out (0x439109).
        assert_eq!(part(0, 1, [16.0, 0.0, 0.0]), at(0, [100, 200], base));
        // Nonsense in a file stays bounded.
        for bad in [f32::NAN, f32::INFINITY, f32::MAX, f32::MIN] {
            let _ = part_fine(position, i16::MIN, [bad, bad, bad]);
            let _ = part(3, i16::MIN, [bad, bad, bad]);
            let _ = sprite_rise([bad, bad, bad], Zoom::Near);
        }
    }

    #[test]
    fn sloped_objects_round_base_altitude_and_interpolate_moved_sprites() {
        let mut lot = flat_lot(0);
        // Tile (10, 12) has corners (0,1)=0, (0,0)=0, (1,0)=0, (1,1)=1.
        // At its center (8, 8), 0x478dfe returns 0.5 (halfway between (0,0)
        // and (1,1)), whereas the four-corner mean is 0.25.
        lot.heights[Lot::cell(10, 12)] = [0, 0, 0, 1];
        let center = [10 * 16 + 8, 12 * 16 + 8];
        assert_eq!(lot.altitude_at(center), 0.5);
        // Off-diagonal point checks (y & 15, x & 15) argument order:
        // tile (10, 13) has (0,1)=4, (0,0)=0, (1,0)=0, (1,1)=0.
        lot.heights[Lot::cell(10, 13)] = [4, 0, 0, 0];
        assert_eq!(lot.altitude_at([10 * 16 + 2, 13 * 16 + 8]), 1.5);
        assert_eq!(lot.altitude_at([10 * 16 + 8, 13 * 16 + 2]), 0.0);
        // Beyond active lot size (e.g. House 09 size 43, tile (1, 43)),
        // 0x478e2a–0x478e4e still samples the raw 64×64 altitude array, while
        // coordinates outside 0..64 return 0.0.
        lot.size = 43;
        lot.heights[Lot::cell(1, 43)] = [0, 0, 0, 2];
        assert_eq!(lot.altitude_at([16 + 8, 43 * 16 + 8]), 1.0);
        assert_eq!(lot.altitude_at([-1, 0]), 0.0);
        assert_eq!(lot.altitude_at([lot::FULL as i32 * 16, 0]), 0.0);
        lot.size = 52;

        // At far zoom, 0x4b97ea rounds altitude 0.5 up to 1 whole step,
        // lifting screen y by 1 << 2 = 4 px relative to flat ground.
        let cam = Camera {
            zoom: Zoom::Far,
            ..camera(0)
        };
        let depth = Depth::new(&cam);
        let flat = flat_lot(0);
        let (sloped_screen, _) = place_base(&depth, &cam, &lot, center, 1, 0.5);
        let (flat_screen, _) = place_base(&depth, &cam, &flat, center, 1, 0.0);
        assert_eq!(sloped_screen[0], flat_screen[0]);
        assert_eq!(flat_screen[1] - sloped_screen[1], 4);

        // Moved sprite (0x4b9991 + 0x4a9dc7) on the sloped tile's center
        // lifts by 0x4a9dc7's integer triangle slope (2 px at far zoom for
        // h1=0, h3=1 at sx=sy=8: ((0 - 1 + 4) << 2) * 8 / 16 = 6 vs 8 flat).
        let (moved_screen, _) = place_moved(&depth, &cam, &lot, center, 1, 0.5);
        assert_eq!(moved_screen[0], flat_screen[0]);
        assert_eq!(flat_screen[1] - moved_screen[1], 2);
        // Adding 1.0 of lift above ground_here raises screen y by 4 more px.
        let (lifted_screen, _) = place_moved(&depth, &cam, &lot, center, 1, 1.5);
        assert_eq!(moved_screen[1] - lifted_screen[1], 4);
    }

    #[test]
    fn flat_trim_depth_offsets_match_binary_table() {
        assert_eq!(flat_trim_offset(true, true, 0x10, 1), 900);
        assert_eq!(flat_trim_offset(true, true, 0x10, 2), 900);
        assert_eq!(flat_trim_offset(true, true, 0x40, 1), 770);
        assert_eq!(flat_trim_offset(true, true, 0x40, 2), 700);
        // 0x40 overrides 0x10 when both diagonal bits are set (0x53c497).
        assert_eq!(flat_trim_offset(true, true, 0x50, 1), 770);
        assert_eq!(flat_trim_offset(true, true, 0x50, 2), 700);
        // Standing corner trims only apply -100 for bit 0x40 on level 2.
        assert_eq!(flat_trim_offset(false, true, 0x40, 2), -100);
        assert_eq!(flat_trim_offset(false, true, 0x40, 1), 0);
        assert_eq!(flat_trim_offset(false, true, 0x10, 2), 0);
        // Non-record wall-run trims (standing or cut diagonal) always pass 0.
        assert_eq!(flat_trim_offset(false, false, 0x40, 2), 0);
        assert_eq!(flat_trim_offset(true, false, 0x50, 1), 0);
        assert_eq!(flat_trim_offset(true, false, 0x50, 2), 0);
        assert_eq!(flat_trim_offset(true, false, 0x10, 1), 0);
    }

    #[test]
    fn diagonal_cut_trim_lift_matches_corner_and_run_tables() {
        // Corner record with lone 0x10 lifts [0, 2, 5] (0x53c47a–0x53c496).
        assert_eq!(Zoom::ALL.map(|z| trim_lift(true, true, 0x10, z)), [0, 2, 5]);
        assert_eq!(Zoom::ALL.map(|z| trim_lift(true, true, 0x50, z)), [0, 0, 0]);
        // Non-record cut run along view (1, 1) (bits 0x50 / 0x10 / 0x40) lifts
        // 2 >> (3 - zoom) = [0, 1, 2] before 0x4c8b1f (0x53e300–0x53e313).
        for bits in [0x50, 0x10, 0x40] {
            assert_eq!(
                Zoom::ALL.map(|z| trim_lift(true, false, bits, z)),
                [0, 1, 2]
            );
            assert_eq!(
                Zoom::ALL.map(|z| trim_lift(false, false, bits, z)),
                [0, 0, 0]
            );
        }
        // Runs along view (1, -1) (0xa0) or straight edges (0x05) get no lift.
        assert_eq!(
            Zoom::ALL.map(|z| trim_lift(true, false, 0xa0, z)),
            [0, 0, 0]
        );
        assert_eq!(
            Zoom::ALL.map(|z| trim_lift(true, false, 0x05, z)),
            [0, 0, 0]
        );
    }

    #[test]
    fn grass_clips_to_active_house_viewport_height() {
        let lot = flat_lot(0);
        // Place a flat near-zoom tile (10, 10) so its columns cross row 500
        // with an odd unclipped end (row 505): clipping to an 800×500 house
        // viewport flips dither parity above row 500 and draws no grass or
        // blades in rows >= 500.
        let cam = Camera {
            zoom: Zoom::Near,
            rotation: 0,
            level: 1,
            origin: [400, -199],
            walls: Walls::Up,
            focus: crate::cutaway::Focus::default(),
        };
        let grass = Grass::new(&lot, &cam);
        let depth = Depth::new(&cam);
        let bg = background(0, 0);
        let mut full = Canvas::new(800, 600, bg);
        draw_grass_tile(&lot, &grass, &depth, &cam, [10, 10], 600, &mut full);
        let mut strip = Canvas::new(800, 600, bg);
        draw_grass_tile(&lot, &grass, &depth, &cam, [10, 10], 500, &mut strip);
        // Columns crossing row 500 clip to 500 (even) instead of 505 (odd),
        // changing pixels just above row 500, and draw no grass at row >= 500.
        let diff_above = (470..500)
            .flat_map(|y| (336..464).map(move |x| (x, y)))
            .any(|(x, y)| full.color[y * 800 + x] != strip.color[y * 800 + x]);
        assert!(
            diff_above,
            "viewport clip at 500 should change grass dither parity above 500"
        );
        assert!(
            (500..600)
                .flat_map(|y| (0..800).map(move |x| (x, y)))
                .all(|(x, y)| strip.color[y * 800 + x] == crate::canvas::pack_rgb(bg)),
            "viewport clip at 500 should draw no grass or blades in rows 500..600"
        );
    }

    /// A dining chair scooted in to the table draws its graphic 5, whose
    /// sprite sits 4 units back along the chair's own y. The original
    /// moves the drawn sprite by that, not only its depth (0x437b7e).
    #[test]
    fn a_sprite_offset_moves_the_fine_position_it_is_drawn_at() {
        let position = [100.0, 200.0];
        let scooted = [0.0, -4.0, 0.0];
        assert_eq!(part_fine(position, 0, scooted), [100, 196]);
        // Each two steps of facing turn (x, y) to (-y, x).
        assert_eq!(part_fine(position, 2, scooted), [104, 200]);
        assert_eq!(part_fine(position, 4, scooted), [100, 204]);
        assert_eq!(part_fine(position, 6, scooted), [96, 200]);
        assert_eq!(part_fine(position, 3, scooted), [100, 200]);
        // Rounded by adding a half and dropping the fraction.
        assert_eq!(part_fine(position, 0, [0.0, -0.8, 0.0]), [100, 199]);
        assert_eq!(part_fine(position, 0, [0.0, -2.4, 0.0]), [100, 198]);
        assert_eq!(part_fine(position, 0, [0.5, 0.0, 0.0]), [101, 200]);
        // A held object's slot offset is added exactly, then rounded with
        // the sprite's: 100.4 + 0.4 is 101, where rounding each first
        // would give 100.
        assert_eq!(part_fine([100.4, 200.0], 0, [0.4, 0.0, 0.0]), [101, 200]);
        // Rounding drops the fraction towards zero, as the original does.
        assert_eq!(part_fine([-1.0, 0.0], 0, [0.0, 0.0, 1.0]), [0, 0]);
        // It moves where the sprite is drawn on the screen: a quarter tile
        // back along y is 16 pixels right and 8 up at the near zoom.
        let camera = camera(0);
        let screen = |object| part_screen(&camera, position, 0, object, 1, 0);
        let [moved, still] = [screen(scooted), screen([0.0; 3])];
        assert_eq!([moved[0] - still[0], moved[1] - still[1]], [16, -8]);
        assert_eq!(
            still,
            camera.project_fine(camera.rotate_fine([100, 200]), 1, 0)
        );
    }

    #[test]
    fn a_cut_trim_is_anchored_at_the_middle_of_the_tile_below_its_corner() {
        let camera = camera(1);
        let depth = Depth::new(&camera);
        // Lot tile (20, 53), which view tile (10, 20) is at this rotation,
        // has corners (0, 1), (0, 0), (1, 0) and (1, 1) at 10, 2, 6 and 5
        // steps; any other tile is flat.
        let heights = |tile: [i32; 2]| {
            if tile == [20, 53] {
                [10, 2, 6, 5]
            } else {
                [0; 4]
            }
        };
        let view = [10, 20];
        assert_eq!(camera.unrotate_tile(view), [20, 53]);
        // Its middle, fine view position (168, 328), is halfway between
        // corners (0, 0) and (1, 1): 3.5 steps, not the corners' mean.
        let anchor = trim_anchor(&depth, &camera, view, 1, heights);
        assert_eq!(anchor, place(&depth, &camera, [168, 328], 1, 3.5));
        assert_ne!(anchor, place(&depth, &camera, [168, 328], 1, 5.75));
    }

    #[test]
    fn a_sprite_rises_by_its_height_at_each_zoom() {
        // 2 << zoom pixels per height unit, rounded half up toward zero.
        assert_eq!(sprite_rise([0.0, 0.0, 0.25], Zoom::Near), 4);
        assert_eq!(sprite_rise([0.0, 0.0, 0.25], Zoom::Medium), 2);
        assert_eq!(sprite_rise([0.0, 0.0, 0.25], Zoom::Far), 1);
        assert_eq!(sprite_rise([0.0, 0.0, 0.4082483], Zoom::Near), 7);
        assert_eq!(sprite_rise([5.0, -3.0, 0.0], Zoom::Near), 0);
        // __ftol truncates after adding a half: -0.25 at the near zoom is
        // -4 + 0.5 = -3.5, which truncates to -3.
        assert_eq!(sprite_rise([0.0, 0.0, -0.25], Zoom::Near), -3);
    }

    #[test]
    fn dynamic_sprites_hide_only_when_their_word_is_zero() {
        assert!(sprite_visible(10, 20, &[0, 0]));
        assert!(!sprite_visible(20, 20, &[0, 1]));
        assert!(sprite_visible(21, 20, &[0, 1]));
        assert!(sprite_visible(22, 20, &[0, 1]));
        assert!(sprite_visible(20, 20, &[]));
    }

    #[test]
    fn doors_stand_in_the_wall_their_placement_names() {
        let at = [5 * 16 + 8, 7 * 16 + 8, 1];
        let mut data = vec![0; 14];
        data[13] = 1;
        assert_eq!(object_edge(at, &data), Some([[5, 7], [6, 7]]));
        data[1] = 2;
        assert_eq!(object_edge(at, &data), Some([[6, 7], [6, 8]]));
        data[13] = 4;
        data[1] = 0;
        assert_eq!(object_edge(at, &data), Some([[5, 8], [6, 8]]));
        data[13] = 0;
        assert_eq!(object_edge(at, &data), None);
    }

    #[test]
    fn within_keeps_only_the_columns_asked_for() {
        let image = Image {
            width: 4,
            height: 2,
            rgba: (0..32).collect(),
            depth: Some((0..8).collect()),
        };
        let (part, skip) = within(&image, 10, [11, 13]).unwrap();
        assert_eq!((part.width, part.height, skip), (2, 2, 1));
        assert_eq!(part.rgba[..8], image.rgba[4..12]);
        assert_eq!(part.depth, Some(vec![1, 2, 5, 6]));
        // Nothing left, and bounds at the integer limits.
        assert_eq!(within(&image, 10, [0, 10]), None);
        assert_eq!(within(&image, i32::MAX, [i32::MIN, i32::MAX]), None);
        let (all, skip) = within(&image, i32::MIN, [i32::MIN, i32::MAX]).unwrap();
        assert_eq!((all.width, skip), (4, 0));
    }

    #[test]
    fn split_floors_keep_complementary_halves() {
        let image = Image {
            width: 127,
            height: 64,
            rgba: [9, 9, 9, 255].repeat(127 * 64),
            depth: None,
        };
        for rotation in 0..4 {
            for segments in [lot::DESCENDING, lot::ASCENDING] {
                let c = camera(rotation);
                let a = split_half(&c, &image, segments, 0);
                let b = split_half(&c, &image, segments, 1);
                let opaque = |i: &Image| {
                    (0..i.rgba.len() / 4)
                        .filter(|&n| i.rgba[n * 4 + 3] != 0)
                        .count()
                };
                assert_eq!(opaque(&a) + opaque(&b), 127 * 64);
                assert!(opaque(&a) > 127 * 16 && opaque(&b) > 127 * 16);
            }
        }
    }

    #[test]
    fn a_split_floor_is_cut_straight_along_the_corners_row_or_half_a_tile_in() {
        // As wide as the game's near floor sprites, a pixel short of the
        // tile.
        let image = Image {
            width: 127,
            height: 64,
            rgba: [9, 9, 9, 255].repeat(127 * 64),
            depth: None,
        };
        let shows = |i: &Image, [x, y]: [usize; 2]| i.rgba[(y * i.width + x) * 4 + 3] != 0;
        for rotation in 0..4 {
            let c = camera(rotation);
            for segments in [lot::DESCENDING, lot::ASCENDING] {
                let halves = [0, 1].map(|h| split_half(&c, &image, segments, h));
                // Which half holds the corner it is named for, from a point
                // well inside it: (4, 4) or (4, 12) of the lot tile.
                let inside = if segments & lot::DESCENDING != 0 {
                    [4, 4]
                } else {
                    [4, 12]
                };
                let view = match rotation {
                    1 => [16 - inside[1], inside[0]],
                    2 => [16 - inside[0], 16 - inside[1]],
                    3 => [inside[1], 16 - inside[0]],
                    _ => inside,
                };
                // View sixteenths to pixels: a tile 128 wide and 64 tall
                // placed half a tile left of its top corner.
                let px = [64 + (view[0] - view[1]) * 4, (view[0] + view[1]) * 2];
                let px = px.map(|n| n as usize);
                assert!(
                    shows(&halves[0], px),
                    "rotation {rotation} segments {segments}"
                );
                assert!(
                    !shows(&halves[1], px),
                    "rotation {rotation} segments {segments}"
                );
                let level = (segments == lot::DESCENDING) == (rotation % 2 == 0);
                // The upper (or left) half holds the top (or left) corner.
                let corner = if level { [64, 0] } else { [0, 32] };
                let (upper, lower) = if shows(&halves[0], corner) {
                    (&halves[0], &halves[1])
                } else {
                    (&halves[1], &halves[0])
                };
                for n in 0..127 {
                    if level {
                        // The corners' row is wholly the lower half's.
                        assert!(!shows(upper, [n, 32]) && shows(lower, [n, 32]));
                        assert!(shows(upper, [n, 31]) && !shows(lower, [n, 31]));
                    } else if n < 64 {
                        // Column 64, half a tile in, is wholly the right
                        // half's; the sprite's middle column 63 the left's.
                        assert!(!shows(upper, [64, n]) && shows(lower, [64, n]));
                        assert!(shows(upper, [63, n]) && !shows(lower, [63, n]));
                    }
                }
            }
        }
    }

    #[test]
    fn background_follows_time_of_day_and_house_99() {
        assert_eq!(background(0, 5), BACKGROUND);
        assert_eq!(background(0, 5), [115, 113, 115]);
        assert_eq!(background(1, 5), [66, 65, 66]);
        assert_eq!(background(2, 5), [24, 24, 24]);
        assert_eq!(background(3, 5), [66, 65, 66]);
        // Any time of day outside 0..=3 keeps the fallback 0x0000ff.
        assert_eq!(background(-1, 5), [0, 0, 255]);
        assert_eq!(background(4, 5), [0, 0, 255]);
        // House 99 overrides every time of day with 0x14148c (5-6-5: 2, 5, 17).
        for time_of_day in [-1, 0, 1, 2, 3, 4] {
            assert_eq!(background(time_of_day, 99), [16, 20, 140], "{time_of_day}");
        }
    }

    #[test]
    fn road_dashes_swap_only_on_level_1_at_odd_parity_from_saved_rotation() {
        for stored in 0..4u8 {
            for camera in 0..4u8 {
                let odd = (camera ^ stored) & 1 != 0;
                // Level 1 swaps 10 <-> 11 when parity differs from stored.
                assert_eq!(
                    floor_for_rotation(10, 1, camera, stored),
                    if odd { 11 } else { 10 },
                    "stored={stored} camera={camera}"
                );
                assert_eq!(
                    floor_for_rotation(11, 1, camera, stored),
                    if odd { 10 } else { 11 },
                    "stored={stored} camera={camera}"
                );
                // Level 2 never swaps (0x4a8080 reads [lot + 0x24] only).
                assert_eq!(floor_for_rotation(10, 2, camera, stored), 10);
                assert_eq!(floor_for_rotation(11, 2, camera, stored), 11);
                // Other floor IDs never swap.
                for other in [0u16, 1, 9, 12, lot::SPLIT_FLOOR, 256] {
                    assert_eq!(floor_for_rotation(other, 1, camera, stored), other);
                    assert_eq!(floor_for_rotation(other, 2, camera, stored), other);
                }
            }
        }
    }

    #[test]
    fn floor_shadows_project_tile_center_at_rounded_altitude() {
        let mut lot = flat_lot(0);
        // Sloped tile (10, 12) has corners (0,1)=0, (0,0)=0, (1,0)=0, (1,1)=1;
        // center altitude is 0.5, which 0x4b97ea rounds up to 1 terrain step.
        lot.heights[Lot::cell(10, 12)] = [0, 0, 0, 1];
        let flat = flat_lot(0);
        for zoom in Zoom::ALL {
            let floor_w = (zoom.tile_width() - 1) as usize;
            for rotation in 0..4 {
                let cam = Camera {
                    zoom,
                    rotation,
                    ..camera(rotation)
                };
                let depth = Depth::new(&cam);
                let view = cam.rotate_tile([10, 12]);
                for level in [1, 2] {
                    let flat_at = shadow_origin(&cam, &flat, [10, 12], level, floor_w);
                    let sloped_at = shadow_origin(&cam, &lot, [10, 12], level, floor_w);
                    // On a flat tile, shadow_origin matches the top corner minus
                    // half floor_w identically across all zooms, rotations, and levels.
                    let top_p = cam.project(view, level, 0);
                    assert_eq!(flat_at, [top_p[0] - floor_w as i32 / 2, top_p[1]]);
                    // On the sloped tile, the shadow lifts by 1 terrain step
                    // (2 << zoom pixels) with x unchanged.
                    assert_eq!(sloped_at[0], flat_at[0]);
                    assert_eq!(flat_at[1] - sloped_at[1], 2 << zoom.level());
                    // Depth anchors from the top corner with level offset and
                    // rebases from the mask's own first row (0x4ae93e..0x4ae9ab).
                    let (base_d, first_row) = depth.floor_base(view, 0, level, sloped_at[1]);
                    assert_eq!(
                        depth
                            .floor_row(base_d, first_row, sloped_at[1])
                            .saturating_sub(65),
                        depth.shadow(view, 0, level, sloped_at[1], sloped_at[1]),
                    );
                }
                // Level 2 lifts by one storey height and shifts depth nearer by LEVEL_DEPTH.
                let l1 = shadow_origin(&cam, &flat, [10, 12], 1, floor_w);
                let l2 = shadow_origin(&cam, &flat, [10, 12], 2, floor_w);
                assert_eq!(l2[0], l1[0]);
                assert!(l2[1] < l1[1]);
                let (b1, f1) = depth.floor_base(view, 0, 1, l1[1]);
                let (b2, f2) = depth.floor_base(view, 0, 2, l2[1]);
                assert_eq!(
                    depth.floor_row(b2, f2, l2[1]) - depth.floor_row(b1, f1, l1[1]),
                    -967
                );
            }
        }
    }

    fn walled(lot: &mut Lot, walls: &[(i32, i32, u8)]) {
        for &(x, y, bit) in walls {
            lot.levels[0].walls[Lot::cell(x as usize, y as usize)].segments |= bit;
        }
    }

    fn order(lot: &Lot, rotation: u8) -> Vec<(i32, i32, u8)> {
        wall_order(&camera(rotation), lot, 0)
            .into_iter()
            .map(|(x, y, s)| (x, y, s.bit))
            .collect()
    }

    #[test]
    fn wall_corners_are_junctions_ends_or_plain() {
        // One segment: an end.
        assert_eq!(wall_corner(0x04), (false, true));
        // Straight on, either way: plain.
        assert_eq!(wall_corner(0x05), (false, false));
        assert_eq!(wall_corner(0x0a), (false, false));
        assert_eq!(wall_corner(0x50), (false, false));
        assert_eq!(wall_corner(0xa0), (false, false));
        // A turn or a tee: a junction.
        assert_eq!(wall_corner(0x09), (true, false));
        assert_eq!(wall_corner(0x0d), (true, false));
        // A wall and a diagonal leaving on the same side: invalid.
        assert_eq!(wall_corner(0x44), (false, false));
        assert_eq!(wall_corner(0x00), (false, false));
    }

    #[test]
    fn walls_are_drawn_run_by_run_in_view_order() {
        let mut lot = flat_lot(0);
        // A room's north wall from (10, 10) to (13, 10), its west wall
        // from (10, 10) to (10, 12), and a lone diagonal.
        walled(
            &mut lot,
            &[
                (10, 10, lot::NORTH),
                (11, 10, lot::NORTH),
                (12, 10, lot::NORTH),
                (10, 10, lot::WEST),
                (10, 11, lot::WEST),
                (20, 20, lot::ASCENDING),
            ],
        );
        // Both runs start at the room's corner; the east one was inserted
        // before the south one, and the sort keeps them so.
        assert_eq!(
            order(&lot, 0),
            vec![
                (10, 10, lot::NORTH),
                (11, 10, lot::NORTH),
                (12, 10, lot::NORTH),
                (10, 10, lot::WEST),
                (10, 11, lot::WEST),
                (20, 20, lot::ASCENDING),
            ]
        );
        // Half a turn: each run starts from its other end, and its
        // segments go backwards. The north run's far end, (13, 10), turns
        // to key (105, -3), before the west run's (10, 12) at (106, 2).
        assert_eq!(
            order(&lot, 2),
            vec![
                (20, 20, lot::ASCENDING),
                (12, 10, lot::NORTH),
                (11, 10, lot::NORTH),
                (10, 10, lot::NORTH),
                (10, 11, lot::WEST),
                (10, 10, lot::WEST),
            ]
        );
        for rotation in 0..4 {
            let mut got = order(&lot, rotation);
            got.sort();
            let mut want = order(&lot, 0);
            want.sort();
            assert_eq!(got, want, "rotation {rotation}");
        }
    }

    #[test]
    fn segments_no_run_covers_are_drawn_last() {
        let mut lot = flat_lot(0);
        // A wall and a diagonal leave (5, 5) southwards: the corner is
        // invalid, so neither starts a run there.
        walled(
            &mut lot,
            &[
                (5, 5, lot::WEST),
                (5, 5, lot::ASCENDING),
                (2, 2, lot::NORTH),
            ],
        );
        assert_eq!(
            order(&lot, 0),
            vec![
                (2, 2, lot::NORTH),
                (5, 5, lot::WEST),
                (5, 5, lot::ASCENDING)
            ]
        );
    }

    #[test]
    fn the_ground_is_left_out_only_under_floors_ringed_by_floored_water() {
        let mut lot = flat_lot(0);
        let at = |x: usize, y: usize| Lot::cell(x, y);
        assert!(ground_shows(&lot, [5, 5]));
        for [x, y] in [[5, 5], [4, 5], [6, 5], [5, 4], [5, 6]] {
            lot.levels[0].floors[at(x, y)] = 1;
        }
        // Floored all round, but no water: the ground shows.
        assert!(ground_shows(&lot, [5, 5]));
        for [x, y] in [[4, 5], [6, 5], [5, 4], [5, 6]] {
            lot.water[at(x, y)] = 0;
        }
        assert!(!ground_shows(&lot, [5, 5]));
        // One neighbour without water, or without a floor, shows it.
        lot.water[at(5, 6)] = 0xff;
        assert!(ground_shows(&lot, [5, 5]));
        lot.water[at(5, 6)] = 0;
        lot.levels[0].floors[at(4, 5)] = 0;
        assert!(ground_shows(&lot, [5, 5]));
        // The tile's own floor counts too.
        lot.levels[0].floors[at(4, 5)] = 1;
        lot.levels[0].floors[at(5, 5)] = 0;
        assert!(ground_shows(&lot, [5, 5]));
    }

    #[test]
    fn objects_go_by_tile_key_each_followed_by_what_it_holds() {
        // A table (1) at tile (3, 3) holds a plate (2) in slot 1 and a cup
        // (3) in slot 0; a chair (4) is at tile (2, 2); a lamp (5) at
        // (3, 2).
        let objects = [
            object(1, 1, [3 * 16, 3 * 16, 1], &[]),
            object(
                2,
                2,
                [3 * 16, 3 * 16, 1],
                &[(CONTAINER, 1), (CONTAINER_SLOT, 1)],
            ),
            object(
                3,
                3,
                [3 * 16, 3 * 16, 1],
                &[(CONTAINER, 1), (CONTAINER_SLOT, 0)],
            ),
            object(4, 4, [2 * 16, 2 * 16, 1], &[]),
            object(5, 5, [3 * 16, 2 * 16, 1], &[]),
        ];
        let ids = |rotation: u8| {
            let mut list: Vec<&Object> = objects.iter().collect();
            object_order(&mut list, &camera(rotation), |o| {
                (o.id, *o, [o.position[0], o.position[1]])
            });
            list.iter().map(|o| o.id).collect::<Vec<_>>()
        };
        assert_eq!(ids(0), vec![4, 5, 1, 3, 2]);
        // Half a turn: the table's tile is now the back one.
        assert_eq!(ids(2), vec![1, 3, 2, 5, 4]);
    }

    #[test]
    fn wall_draw_key_orders_back_to_front_at_every_rotation() {
        // At corner (26, 24), tile (26, 24)'s WEST wall runs (26,24)-(26,25)
        // and tile (25, 24)'s NORTH wall runs (25,24)-(26,24).
        let west = ([26, 24], [26, 25]);
        let north = ([25, 24], [26, 24]);
        // Rotation 0: north's start (49, 1) < west's start (50, 2).
        let cam0 = camera(0);
        assert!(wall_draw_key(&cam0, north.0, north.1) < wall_draw_key(&cam0, west.0, west.1));
        // Rotation 1: west's start (65, 13) < north's start (65, 15).
        let cam1 = camera(1);
        assert!(wall_draw_key(&cam1, west.0, west.1) < wall_draw_key(&cam1, north.0, north.1));
        // Rotation 2: west's start (77, -1) < north's start (78, -2).
        let cam2 = camera(2);
        assert!(wall_draw_key(&cam2, west.0, west.1) < wall_draw_key(&cam2, north.0, north.1));
        // Rotation 3: both share start (62, -14); endpoint tie-break orders
        // north's far end (63, -15) before west's far end (63, -13).
        let cam3 = camera(3);
        let k_north = wall_draw_key(&cam3, north.0, north.1);
        let k_west = wall_draw_key(&cam3, west.0, west.1);
        assert_eq!(k_north.0, k_west.0);
        assert!(k_north < k_west);
        // Regardless of endpoint order, the key is invariant at every rotation.
        for rot in 0..4 {
            let cam = camera(rot);
            assert_eq!(
                wall_draw_key(&cam, west.0, west.1),
                wall_draw_key(&cam, west.1, west.0)
            );
        }
    }

    #[test]
    fn rotate_cardinal_mask_cycles_low_nibble_and_preserves_deck_bit() {
        // 0x4614d6 right-rotates bits 0..3 (N=1 -> E=8 -> S=4 -> W=2 -> N=1)
        // and leaves the high nibble (including 0x10 pool deck) untouched.
        assert_eq!(rotate_cardinal_mask(1, 0), 1);
        assert_eq!(rotate_cardinal_mask(1, 1), 8);
        assert_eq!(rotate_cardinal_mask(1, 2), 4);
        assert_eq!(rotate_cardinal_mask(1, 3), 2);
        assert_eq!(rotate_cardinal_mask(0x0f, 1), 0x0f);
        assert_eq!(rotate_cardinal_mask(0x00, 3), 0x00);
        for rot in 0..4 {
            assert_eq!(rotate_cardinal_mask(0x10, rot), 0x10);
        }
    }

    #[test]
    fn pool_inner_corners_detects_l_shaped_inside_corners_and_rotates() {
        let mut lot = flat_lot(0);
        // Build an L-shaped pool:
        // (10,10)=pool  (11,10)=pool
        // (10,11)=pool  (11,11)=empty  <-- inner corner on tile (10,10) facing (+x,+y)
        let pool_coords = [(10usize, 10usize), (11, 10), (10, 11)];
        let is_pool = |x: usize, y: usize| pool_coords.contains(&(x, y));
        for &(x, y) in &pool_coords {
            let mut m = 0u8;
            if is_pool(x, y - 1) {
                m |= 1;
            }
            if is_pool(x - 1, y) {
                m |= 2;
            }
            if is_pool(x, y + 1) {
                m |= 4;
            }
            if is_pool(x + 1, y) {
                m |= 8;
            }
            lot.pool[Lot::cell(x, y)] = m;
        }
        // Tile (10,10) has S(4) and E(8) neighbors, whose shared corner (11,11) is dry ->
        // rule 4 emits bit 4 (index 2) at rotation 0, cycling 4 -> 2 -> 1 -> 8.
        assert_eq!(pool_inner_corners(&lot, 10, 10, 0), 4);
        assert_eq!(pool_inner_corners(&lot, 10, 10, 1), 2);
        assert_eq!(pool_inner_corners(&lot, 10, 10, 2), 1);
        assert_eq!(pool_inner_corners(&lot, 10, 10, 3), 8);
        // The two outer arms have no inner corners.
        assert_eq!(pool_inner_corners(&lot, 11, 10, 0), 0);
        assert_eq!(pool_inner_corners(&lot, 10, 11, 0), 0);

        // Lot::exact_eq detects edits to pool and water arrays (invalidating LotCache).
        let base = flat_lot(0);
        assert!(!base.exact_eq(&lot));
        let mut pond = flat_lot(0);
        pond.water[Lot::cell(12, 12)] = 0;
        assert!(!base.exact_eq(&pond));
    }
}
