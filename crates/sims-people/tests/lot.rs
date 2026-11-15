//! The Goths' lot booted and run by the simulation (`sims_vm::boot::Lot`),
//! with its people drawn from the state it reaches as it runs. Needs
//! `SIMS_INSTALL`; skipped when it is unset. Nothing from the game is
//! kept.

use sims_people::math::Vec3;
use sims_people::raster::{Camera, Canvas};
use sims_people::{Assets, Body, Cast, Library, Player, base_clip, placement, residents_of};
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
                // part of the frame, and only part.
                let camera = Camera::house_view(40.0, root.translation + Vec3::new(0.0, 2.6, 0.0));
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
    // The family is placed, and the run gets past the load. How far it
    // gets is the simulation's to say.
    assert!(most >= 3, "at most {most} people placed");
    assert!(drawn >= 2, "the run ended at tick {}", lot.ticks());
}
