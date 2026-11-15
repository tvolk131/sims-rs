//! The Goths' lot booted and run by the simulation (`sims_vm::boot::Lot`),
//! with its people drawn from the state it reaches as it runs. Needs
//! `SIMS_INSTALL`; skipped when it is unset. Nothing from the game is
//! kept.

use sims_people::lot::NEAR_PIXELS_PER_UNIT;
use sims_people::raster::Target;
use sims_people::raster::{Camera, Canvas};
use sims_people::{
    Assets, Body, Cast, Library, People, Player, base_clip, placement, residents_of,
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
                let pose = player.pose(&body.skeleton, animator, base).unwrap();
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
