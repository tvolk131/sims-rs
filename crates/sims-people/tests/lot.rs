//! The Goths' lot booted and run by the simulation (`sims_vm::boot::Lot`),
//! with its people drawn from the state it reaches as it runs. Needs
//! `SIMS_INSTALL`; skipped when it is unset. Nothing from the game is
//! kept.

use sims_people::lot::NEAR_PIXELS_PER_UNIT;
use sims_people::raster::Target;
use sims_people::raster::{Camera, Canvas};
use sims_people::{
    Assets, Body, Cast, Library, People, Player, View, base_clip, placement, residents_of,
};
use sims_render::{Camera as LotCamera, Walls, Zoom};
use sims_sim::World;
use sims_vm::boot::{Frames, Lot, StepError};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(root) => Some(PathBuf::from(root)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping the booted lot test");
            None
        }
    }
}

const BACKGROUND: [u8; 4] = [0, 0, 0, 255];

/// Ticks between the states drawn.
const EVERY: u32 = 30;

/// More ticks than the boot runs today; the test stops there if the
/// simulation gets that far.
const LAST: u32 = 3000;

#[test]
fn the_people_of_a_running_lot_are_drawn_from_its_state() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let cast = Cast::of_house(&root, &house).unwrap();
    let assets = Assets::scan(&root).unwrap();
    let library = Library::scan(&assets).unwrap();
    let mut player = Player::new(&assets, &library);
    // Each person's body and base clip, loaded when it first appears.
    let mut bodies = BTreeMap::new();
    // The most people placed in one drawn state, and the states drawn.
    let mut most = 0;
    let mut drawn = 0;
    loop {
        if lot.ticks().is_multiple_of(EVERY) {
            let world = lot.world();
            for resident in residents_of(&cast, world).unwrap() {
                bodies.entry(resident.id).or_insert_with(|| {
                    let body = Body::load(&assets, &resident.skeleton, &resident.outfit).unwrap();
                    (body, base_clip(&resident.person, &resident.skeleton))
                });
            }
            let mut placed = 0;
            for (id, animator) in &world.animators {
                let Some(root) = placement(animator) else {
                    continue;
                };
                let Some((body, base)) = bodies.get(id) else {
                    panic!("tick {}: person {id} is placed but not read", lot.ticks());
                };
                let pose = player
                    .pose(&body.skeleton, animator, world.base_layers.get(id), base)
                    .unwrap();
                let bones = pose.world(&body.skeleton, root);
                // Drawn alone, with the camera on the person, it covers
                // part of the frame, and only part. The camera is on the
                // skeleton's root bone (bone 0, `ROOT`), not on the
                // animator's position: a clip can carry the body tiles
                // away from it (climbing the stairs moves it four tiles
                // from the foot of the stairs).
                let body_root = bones.first().map_or(root.translation, |b| b.translation);
                let camera = Camera::house_view(40.0, body_root);
                let mut canvas = Canvas::new(200, 200, BACKGROUND);
                canvas.draw_body(&camera, body, &bones);
                let covered = canvas
                    .rgba
                    .chunks(4)
                    .filter(|p| *p != BACKGROUND.as_slice())
                    .count();
                assert!(
                    (500..20_000).contains(&covered),
                    "tick {}: person {id} covers {covered} pixels",
                    lot.ticks()
                );
                placed += 1;
            }
            most = most.max(placed);
            drawn += 1;
        }
        if lot.ticks() >= LAST {
            break;
        }
        match lot.step() {
            Ok(_) => {}
            // A tick or a frame that reaches what is not ported yet ends
            // the run; it is not this crate's to fix.
            Err(StepError::Tick { .. } | StepError::Frame { .. }) => break,
            Err(e) => panic!("tick {}: {e}", lot.ticks() + 1),
        }
    }
    eprintln!(
        "{} ticks run, {drawn} states drawn, at most {most} people placed",
        lot.ticks()
    );
    // The family is placed. How far the run gets is the simulation's to
    // say; the state before the first tick is always drawn.
    assert!(most >= 3, "at most {most} people placed");
    assert!(drawn >= 1, "no state was drawn");
}

/// A depth image that counts what is drawn into it, for [`People::draw`].
struct Frame {
    width: u32,
    height: u32,
    depth: Vec<f32>,
    /// Whether each pixel was written.
    written: Vec<bool>,
}

impl Frame {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            depth: vec![f32::INFINITY; (width * height) as usize],
            written: vec![false; (width * height) as usize],
        }
    }

    /// How many pixels were written within `reach` pixels of `at`.
    fn near(&self, [x, y]: [f32; 2], reach: f32) -> usize {
        let range = |c: f32, side: u32| {
            let low = (c - reach).max(0.0) as u32;
            let high = ((c + reach).max(0.0) as u32).min(side);
            low..high
        };
        range(y, self.height)
            .flat_map(|py| range(x, self.width).map(move |px| (py * self.width + px) as usize))
            .filter(|&i| self.written[i])
            .count()
    }
}

impl Target for Frame {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        z < self.depth[(y * self.width + x) as usize]
    }

    fn write(&mut self, x: u32, y: u32, z: f32, _: [u8; 4], _: f32) {
        let i = (y * self.width + x) as usize;
        self.depth[i] = z;
        self.written[i] = true;
    }
}

/// A stand-in for the lot renderer's depth: nearer the viewer and higher
/// is nearer. Only the order matters here.
fn stand_in_depth([x, y]: [f32; 2], elevation: f32, level: i32) -> f32 {
    -3.0 * (x + y) - elevation / 2.0 - 116.0 * (level - 1) as f32
}

#[test]
fn the_people_of_a_running_lot_are_drawn_in_the_lot_renderers_view() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let mut people = People::open(&root, &house).unwrap();
    let mut drawn = 0;
    let mut most = 0;
    let mut checked = 0;
    loop {
        if lot.ticks().is_multiple_of(EVERY) {
            let world = lot.world();
            assert_eq!(people.update(world), Vec::<String>::new());
            // Each person drawn alone, in a world holding only them, so
            // the pixels near where they stand are theirs.
            let ids: Vec<u16> = world.animators.keys().copied().collect();
            let solos: Vec<(u16, World)> = ids
                .iter()
                .map(|&id| {
                    let mut solo = world.clone();
                    solo.animators.retain(|&other, _| other == id);
                    (id, solo)
                })
                .collect();
            for rotation in 0..4 {
                let mut camera = LotCamera {
                    zoom: Zoom::Far,
                    rotation,
                    level: 2,
                    origin: [0, 0],
                    walls: Walls::Up,
                    focus: Default::default(),
                };
                // The whole lot fits: its diagonal is 128 tiles of 16
                // pixels at the far zoom, its depth 128 of 8. People stand
                // on the road at the lot's corners.
                camera.center_on([32 * 16, 32 * 16], [1200, 700]);
                let (posed, problems) = people.posed(world, camera.level);
                assert!(problems.is_empty(), "{problems:?}");
                // Where the lot renderer puts each person's position: its
                // own projection, the animator's position rounded to a
                // sixteenth, raised by the ground, one column right as it
                // draws objects. A clip may carry the body away from the
                // position (the stair climb carries it four tiles); such
                // people are left out.
                let stands: Vec<(u16, [i32; 2])> = posed
                    .iter()
                    .filter(|p| {
                        let [r, b] = [p.root.translation, p.bones[0].translation];
                        (r.x - b.x).abs().max((r.z - b.z).abs()) < 1.5
                    })
                    .map(|p| {
                        let [x, y, _] = world.animators[&p.id].position.map(|v| v.to_f32());
                        let fine = [x, y].map(|v| v.round() as i32);
                        let [p0, p1, _] = world.animators[&p.id].position;
                        let ground = sims_sim::head::ground_height(world, [p0, p1])
                            .map(|h| h.to_f32())
                            .unwrap_or(0.0);
                        let elevation = (ground * NEAR_PIXELS_PER_UNIT).round() as i32;
                        let level = if p.level <= 1 { 1 } else { p.level };
                        let [sx, sy] =
                            camera.project_fine(camera.rotate_fine(fine), level, elevation);
                        (p.id, [sx + 1, sy])
                    })
                    .collect();
                let placed = posed.len();
                drop(posed);
                most = most.max(placed);
                for (id, [x, y]) in stands {
                    let solo = &solos.iter().find(|(other, _)| *other == id).unwrap().1;
                    let mut frame = Frame::new(2400, 1400);
                    let report = people.draw(solo, &camera, &mut frame, stand_in_depth);
                    assert_eq!(report.people, 1);
                    assert!(report.problems.is_empty(), "{:?}", report.problems);
                    let near = frame.near([x as f32, y as f32], 20.0);
                    assert!(
                        near > 20,
                        "tick {}, rotation {rotation}: person {id} at {x}, {y}: {near} pixels",
                        lot.ticks(),
                    );
                    checked += 1;
                }
            }
            drawn += 1;
        }
        if lot.ticks() >= LAST {
            break;
        }
        match lot.step() {
            Ok(_) => {}
            Err(StepError::Tick { .. } | StepError::Frame { .. }) => break,
            Err(e) => panic!("tick {}: {e}", lot.ticks() + 1),
        }
    }
    assert!(most >= 3, "at most {most} people placed");
    assert!(drawn >= 1);
    assert!(checked >= 4 * 3, "{checked} people checked");

    // People who change clothes are dressed again; people who leave are
    // forgotten.
    let mut world = lot.world().clone();
    let ids: Vec<u16> = world
        .objects
        .iter()
        .filter(|(_, o)| o.person.is_some())
        .map(|(&id, _)| id)
        .collect();
    let bodies = |people: &mut People, world: &World| -> Vec<(u16, String)> {
        people
            .posed(world, 2)
            .0
            .iter()
            .map(|p| (p.id, p.body.parts[0].mesh.name.clone()))
            .collect()
    };
    people.update(&world);
    let before = bodies(&mut people, &world);
    let (&[first, second], _) = ids.split_first_chunk::<2>().unwrap();
    let theirs = world.objects[&second]
        .person
        .as_ref()
        .unwrap()
        .appearance
        .clone();
    world
        .objects
        .get_mut(&first)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .appearance = theirs;
    assert_eq!(people.update(&world), Vec::<String>::new());
    let after = bodies(&mut people, &world);
    let body_of = |list: &[(u16, String)], id| list.iter().find(|(i, _)| *i == id).cloned();
    if let (Some(a), Some(b)) = (body_of(&after, first), body_of(&before, second)) {
        assert_eq!(a.1, b.1, "person {first} wears person {second}'s body");
    }
    world.objects.remove(&first);
    people.update(&world);
    assert!(
        bodies(&mut people, &world)
            .iter()
            .all(|(id, _)| *id != first)
    );
}

/// How many pixels of `canvas` differ from `base`, within `area` (left,
/// top, right, bottom; all of it when None).
fn changed(
    base: &sims_render::Canvas,
    canvas: &sims_render::Canvas,
    area: Option<[i32; 4]>,
) -> usize {
    let [l, t, r, b] = area.unwrap_or([0, 0, base.width as i32, base.height as i32]);
    (t.max(0)..b.min(base.height as i32))
        .flat_map(|y| (l.max(0)..r.min(base.width as i32)).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let i = y as usize * base.width + x as usize;
            base.color[i] != canvas.color[i]
        })
        .count()
}

/// The lot renderer's canvas, noting which pixels it takes by its sprite
/// rule (not faint, no farther than what is there), apart from its IDs.
struct Taken {
    canvas: sims_render::Canvas,
    taken: Vec<bool>,
}

impl Target for Taken {
    fn size(&self) -> (u32, u32) {
        Target::size(&self.canvas)
    }

    fn passes(&self, x: u32, y: u32, z: f32) -> bool {
        Target::passes(&self.canvas, x, y, z)
    }

    fn write(&mut self, x: u32, y: u32, z: f32, texel: [u8; 4], shade: f32) {
        if texel[3] >= sims_render::canvas::FAINT && Target::passes(&self.canvas, x, y, z) {
            self.taken[y as usize * self.canvas.width + x as usize] = true;
        }
        Target::write(&mut self.canvas, x, y, z, texel, shade);
    }

    fn set_id(&mut self, id: Option<u16>) {
        Target::set_id(&mut self.canvas, id);
    }
}

/// Every pixel of `canvas`, as (x, y).
fn pixels(canvas: &sims_render::Canvas) -> impl Iterator<Item = (i32, i32)> + use<> {
    let [w, h] = [canvas.width as i32, canvas.height as i32];
    (0..h).flat_map(move |y| (0..w).map(move |x| (x, y)))
}

/// Drawn over the lot the lot renderer drew, people are hidden by what
/// stands in front of them, and not by the floor or ground they stand
/// on. Each person is drawn alone and compared with the same person laid
/// over everything. The camera shows the ground level, where the Goths
/// are, with the walls down; seen from upstairs, the upper floor hides
/// everyone below it.
#[test]
fn the_lot_hides_the_people_behind_it() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let mut opened = sims_render::open_house(&root, &house).unwrap();
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let mut people = People::open(&root, &house).unwrap();
    // Per person and view: pixels shown and laid, of the whole body and
    // of the feet.
    let mut seen: Vec<(u16, [usize; 4])> = Vec::new();
    while lot.ticks() < 600 {
        if lot.ticks().is_multiple_of(120) {
            let world = lot.world();
            assert_eq!(people.update(world), Vec::<String>::new());
            let shape = sims_render::Lot::from_world(world).unwrap();
            let ids: Vec<u16> = world.animators.keys().copied().collect();
            for rotation in 0..4 {
                let mut camera = LotCamera {
                    zoom: Zoom::Medium,
                    rotation,
                    level: 1,
                    origin: [0, 0],
                    walls: Walls::Down,
                    focus: Default::default(),
                };
                camera.center_on(shape.built_center(), [400, 300]);
                let view = View::new(&camera);
                let mut base = sims_render::Canvas::new(800, 600, [0, 0, 0]);
                sims_render::draw(world, &shape, &mut opened.assets, &camera, &mut base);
                // The lot alone names objects at some pixels, never a
                // person.
                let named = pixels(&base).filter_map(|(x, y)| base.id_at(x, y));
                let named: Vec<u16> = named.collect();
                assert!(!named.is_empty());
                assert!(named.iter().all(|i| world.objects[i].person.is_none()));
                for &id in &ids {
                    let mut solo = world.clone();
                    solo.animators.retain(|&other, _| other == id);
                    let (posed, _) = people.posed(&solo, camera.level);
                    let Some(feet) = posed.first().map(|p| {
                        let (fine, elevation) = view.place(p.root.translation);
                        view.screen(fine, elevation, p.level).map(|v| v as i32)
                    }) else {
                        continue;
                    };
                    drop(posed);
                    let mut on = base.clone();
                    let report = people.draw_on_lot(&solo, &camera, &mut on);
                    assert!(report.problems.is_empty(), "{:?}", report.problems);
                    // Drawn again, noting apart from the IDs which pixels
                    // the canvas took: those name the person, or the
                    // plumb bob they hold where the bob's own draw took
                    // them, and every other pixel names what it named
                    // before. Which pixels are the bob's is told by a
                    // draw without it: a pixel the body took there, with
                    // the same colour, is the body's.
                    let bob = sims_people::bob::held_by(&solo, id);
                    let depth = sims_render::depth::Depth::new(&camera);
                    let mut note = |world: &World| {
                        let mut noted = Taken {
                            canvas: base.clone(),
                            taken: vec![false; base.width * base.height],
                        };
                        people.draw(world, &camera, &mut noted, |f, e, l| depth.point(f, e, l));
                        noted
                    };
                    let noted = note(&solo);
                    assert_eq!(noted.canvas, on);
                    let mut bobless = solo.clone();
                    if bob.is_some() {
                        bobless.objects.get_mut(&id).unwrap().slots[2].object = 0;
                    }
                    let body = note(&bobless);
                    for (x, y) in pixels(&on) {
                        let i = y as usize * on.width + x as usize;
                        let expected = if noted.taken[i] {
                            let bodys = body.taken[i] && body.canvas.color[i] == on.color[i];
                            if bodys { Some(id) } else { bob.or(Some(id)) }
                        } else {
                            assert_eq!(on.color[i], base.color[i], "{x}, {y}");
                            base.id_at(x, y)
                        };
                        assert_eq!(on.id_at(x, y), expected, "{x}, {y}");
                    }
                    // Laid over everything: nearer than any depth there is.
                    let mut over = base.clone();
                    people.draw(&solo, &camera, &mut over, |_, _, _| -1.0e9);
                    // The feet: a band a few pixels above the point the
                    // person stands on.
                    let [x, y] = feet;
                    let band = Some([x - 16, y - 10, x + 16, y + 3]);
                    let counts = [
                        changed(&base, &on, None),
                        changed(&base, &over, None),
                        changed(&base, &on, band),
                        changed(&base, &over, band),
                    ];
                    let [shown, laid, ..] = counts;
                    assert!(
                        shown <= laid,
                        "tick {}, rotation {rotation}, person {id}: {shown} > {laid}",
                        lot.ticks()
                    );
                    seen.push((id, counts));
                }
            }
        }
        match lot.step() {
            Ok(_) => {}
            Err(StepError::Tick { .. } | StepError::Frame { .. }) => break,
            Err(e) => panic!("tick {}: {e}", lot.ticks() + 1),
        }
    }
    // Something in front of someone hides them at times (in this run, a
    // hedge, the porch railing, a bookcase, a bed's posts).
    assert!(seen.iter().any(|(_, [shown, laid, ..])| shown < laid));
    // Their feet show on the floor and grass they stand on, unless
    // something stands in front of them: in most views, some of the feet
    // are drawn. A floor or ground drawn nearer than the person would
    // hide nearly all of them.
    let feet: Vec<bool> = seen
        .iter()
        .filter(|(_, c)| c[3] > 0)
        .map(|(_, c)| c[2] > 0)
        .collect();
    let shown = feet.iter().filter(|&&f| f).count();
    assert!(
        feet.len() >= 20 && shown * 4 >= feet.len() * 3,
        "feet shown in {shown} of {} views",
        feet.len()
    );
    // Nobody is hidden through the whole run.
    let mut by_person: BTreeMap<u16, [usize; 2]> = BTreeMap::new();
    for (id, [shown, laid, ..]) in &seen {
        let total = by_person.entry(*id).or_default();
        total[0] += shown;
        total[1] += laid;
    }
    for (id, [shown, laid]) in by_person {
        assert!(
            shown * 4 >= laid,
            "person {id}: {shown} of {laid} pixels shown"
        );
    }
}

/// A person upstairs stands on the upper floor, which the terrain under
/// the house lifts as it lifts the ground floor. At tick 1000 Bella
/// (person 18) plays with the dollhouse on level 2; seen from there with
/// the walls down, most of her shows over the floor she kneels on (the
/// front wall's strip hides the rest). Standing at the floor's height
/// without the terrain, the floor hid all of her.
#[test]
fn a_person_upstairs_stands_on_the_upper_floor() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let mut opened = sims_render::open_house(&root, &house).unwrap();
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let mut people = People::open(&root, &house).unwrap();
    while lot.ticks() < 1000 {
        lot.step().unwrap();
    }
    let world = lot.world();
    assert_eq!(World::tile(world.objects[&18].position), [25, 25, 2]);
    assert_eq!(people.update(world), Vec::<String>::new());
    let mut solo = world.clone();
    solo.animators.retain(|&id, _| id == 18);
    let camera = LotCamera {
        zoom: Zoom::Medium,
        rotation: 0,
        level: 2,
        origin: [400, -350],
        walls: Walls::Down,
        focus: Default::default(),
    };
    let shape = sims_render::Lot::from_world(world).unwrap();
    let mut base = sims_render::Canvas::new(800, 600, [0, 0, 0]);
    sims_render::draw(world, &shape, &mut opened.assets, &camera, &mut base);
    let mut on = base.clone();
    let report = people.draw_on_lot(&solo, &camera, &mut on);
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    assert_eq!(report.people, 1);
    let mut over = base.clone();
    people.draw(&solo, &camera, &mut over, |_, _, _| -1.0e9);
    let [shown, laid] = [changed(&base, &on, None), changed(&base, &over, None)];
    assert!(laid > 200, "{laid}");
    assert!(shown * 3 >= laid * 2, "{shown} of {laid} pixels shown");
}

/// The bob's green: green well above red and blue, and bright.
fn bob_green([r, g, b]: [u8; 3]) -> bool {
    let [r, g, b] = [r, g, b].map(i32::from);
    g > r + 50 && g > b + 50 && g > 120
}

/// The plumb bob over the selected person: in a state where Mortimer
/// (person 21) reads on the porch bench, as in a screenshot of the
/// original at medium zoom (7:38 AM, camera origin (335, −560), cutaway),
/// its green pixels span the box the original's do, x 427 to 434 and
/// y 74 to 97, each edge within a pixel; the pixels inside the box are
/// not compared one by one. Taken out of his head slot, it is not drawn.
#[test]
fn the_plumb_bob_floats_over_the_selected_persons_head() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let mut opened = sims_render::open_house(&root, &house).unwrap();
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    let mut people = People::open(&root, &house).unwrap();
    while lot.ticks() < 1100 {
        lot.step().unwrap();
    }
    let world = lot.world();
    assert_eq!(world.selected(), Some(21));
    assert_eq!(sims_people::bob::held_by(world, 21), Some(383));
    let camera = LotCamera {
        zoom: Zoom::Medium,
        rotation: 0,
        level: 1,
        origin: [335, -560],
        walls: Walls::Cutaway,
        focus: Default::default(),
    };
    let shape = sims_render::Lot::from_world(world).unwrap();
    let mut base = sims_render::Canvas::new(800, 600, [0, 0, 0]);
    sims_render::draw(world, &shape, &mut opened.assets, &camera, &mut base);
    assert_eq!(people.update(world), Vec::<String>::new());
    let mut green = |world: &World| {
        let mut canvas = base.clone();
        let drawn = people.draw_on_lot(world, &camera, &mut canvas);
        assert!(drawn.problems.is_empty(), "{:?}", drawn.problems);
        let found: Vec<(i32, i32)> = pixels(&canvas)
            .filter(|&(x, y)| (400..470).contains(&x) && (50..130).contains(&y))
            .filter(|&(x, y)| bob_green(canvas.color[y as usize * canvas.width + x as usize]))
            .collect();
        (found, canvas)
    };
    let (found, canvas) = green(world);
    assert!(found.len() > 60, "{}", found.len());
    let xs = found.iter().map(|p| p.0);
    let ys = found.iter().map(|p| p.1);
    let [x0, x1, y0, y1] = [
        xs.clone().min().unwrap(),
        xs.max().unwrap(),
        ys.clone().min().unwrap(),
        ys.max().unwrap(),
    ];
    assert!(
        (x0 - 427).abs() <= 1 && (x1 - 434).abs() <= 1,
        "x {x0}..{x1}"
    );
    assert!((y0 - 74).abs() <= 1 && (y1 - 97).abs() <= 1, "y {y0}..{y1}");
    // Brighter on the viewer's left than on the right, as in the
    // screenshot.
    let middle = (x0 + x1) / 2;
    let mean = |side: &dyn Fn(i32) -> bool| {
        let g: Vec<u32> = found
            .iter()
            .filter(|p| side(p.0))
            .map(|&(x, y)| u32::from(canvas.color[y as usize * canvas.width + x as usize][1]))
            .collect();
        g.iter().sum::<u32>() / g.len() as u32
    };
    assert!(mean(&|x| x < middle) > mean(&|x| x > middle));
    // Its pixels name the bob, not Mortimer.
    assert!(found.iter().all(|&(x, y)| canvas.id_at(x, y) == Some(383)));
    // Out of the slot, no bob.
    let mut without = world.clone();
    without.objects.get_mut(&21).unwrap().slots[2].object = 0;
    assert_eq!(green(&without).0, Vec::new());
}

/// Each Goth is lit at the light of the room they stand in, held between
/// a half and full light. A little after 7 in the morning (tick 1100)
/// Mortimer reads on the porch, outdoors in daylight; Cassandra is
/// indoors in room 4, at its light; and Bella is upstairs in a room whose
/// light is 0, which shows 0.8 because someone is in it.
#[test]
fn each_person_is_lit_at_their_rooms_light() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    while lot.ticks() < 1100 {
        lot.step().unwrap();
    }
    let world = lot.world();
    let level = |id| sims_people::lot::level(world, id);
    let room = |id: u16| world.room_at(world.objects[&id].position).unwrap();
    assert_eq!((room(21), level(21)), (0, 1.0));
    let kitchen = sims_render::lot::room_light(&world.rooms.rooms[&room(16)]);
    assert_eq!((room(16), level(16)), (4, f32::from(kitchen) / 255.0));
    assert!(level(16) > 0.5 && level(16) < 1.0, "{}", level(16));
    assert_eq!((room(18), level(18)), (1025, 204.0 / 255.0));
}

/// At 8 in the morning (tick 2675) Mortimer stands at the bookcase in room
/// 4, whose two lamps are on, as they are in the original then (track 2's
/// recording of its day has data word 16 at 1 on both). The nearer, four
/// tiles away, lights him at full strength from in front of the viewer;
/// the farther, five tiles away, faintly, twice. Cassandra, outside room 4,
/// takes no lamp.
#[test]
fn the_lamps_of_a_persons_room_light_them() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot = Lot::load(&root, &house, &[3], Frames::default()).unwrap();
    while lot.ticks() < 2675 {
        lot.step().unwrap();
    }
    let world = lot.world();
    let at = |id: u16| {
        let animator = &world.animators[&id];
        let mut root = placement(animator).unwrap().translation;
        let fine = [animator.position[0], animator.position[1]];
        root.y = sims_sim::head::ground_height(world, fine).unwrap().to_f32();
        root
    };
    let room = |id: u16| world.room_at(world.objects[&id].position).unwrap();
    assert_eq!(room(21), 4);
    let lit: Vec<u16> = world
        .objects
        .iter()
        .filter(|(_, o)| sims_render::lot::gives_light(o))
        .map(|(&id, _)| id)
        .collect();
    assert_eq!(lit, [71, 194]);
    let lamps = sims_people::lot::lamps(world, 1, at(21));
    // The farther lamp's tile is listed twice in room 4, so it lights
    // twice.
    assert_eq!(lamps.len(), 3, "{lamps:?}");
    assert_eq!(lamps[0].strength, 1.0);
    assert!(
        lamps[1].strength > 0.01 && lamps[1].strength < 0.1,
        "{lamps:?}"
    );
    assert_eq!(lamps[1], lamps[2]);
    // From the lamp's side: towards larger fine y, which is the drawing
    // frame's smaller x, and from above.
    assert!(
        lamps[0].towards.x < -0.5 && lamps[0].towards.y > 0.0,
        "{lamps:?}"
    );
    assert_ne!(room(16), 4);
    assert!(sims_people::lot::lamps(world, 1, at(16)).is_empty());
    // And the drawing takes them: with both lamps off, Mortimer is drawn
    // darker, and nobody else changes.
    let mut people = People::open(&root, &house).unwrap();
    assert_eq!(people.update(world), Vec::<String>::new());
    let camera = LotCamera {
        zoom: Zoom::Medium,
        rotation: 0,
        level: 1,
        origin: [335, -560],
        walls: Walls::Cutaway,
        focus: Default::default(),
    };
    let base = sims_render::Canvas::new(800, 600, [0, 0, 0]);
    let mut draw = |world: &World| {
        let mut canvas = base.clone();
        let drawn = people.draw_on_lot(world, &camera, &mut canvas);
        assert!(drawn.problems.is_empty(), "{:?}", drawn.problems);
        canvas
    };
    let lit = draw(world);
    let mut dark = world.clone();
    for lamp in [71, 194] {
        dark.objects.get_mut(&lamp).unwrap().data[16] = 0;
    }
    assert!(sims_people::lot::lamps(&dark, 1, at(21)).is_empty());
    let unlit = draw(&dark);
    let changed: Vec<usize> = (0..lit.color.len())
        .filter(|&i| lit.color[i] != unlit.color[i])
        .collect();
    assert!(changed.len() > 300, "{}", changed.len());
    let sum = |canvas: &sims_render::Canvas| -> u64 {
        changed
            .iter()
            .map(|&i| canvas.color[i].iter().map(|&c| u64::from(c)).sum::<u64>())
            .sum()
    };
    assert!(sum(&lit) > sum(&unlit) + changed.len() as u64 * 6);
}
