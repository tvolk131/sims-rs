//! The Goths' house played headless: it boots, ticks, draws its people
//! over the lot and the panel over both. Needs `SIMS_INSTALL`; skipped
//! when it is unset. Nothing from the game is kept.

use sims_game::game::goth_house;
use sims_game::people::People;
use sims_game::view::View;
use sims_game::{Game, Options, pick};
use sims_render::{Canvas, open_house};
use sims_sim::animator::UNSET;
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping the Goth house tests");
            None
        }
    }
}

fn options(install: PathBuf) -> Options {
    Options {
        house: goth_house(&install),
        install,
        answers: vec![3],
        strict: false,
    }
}

#[test]
fn the_goths_play_and_are_drawn() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.stopped(), None, "the load completes");
    assert_eq!(game.run(100), 100);
    assert_eq!(game.lot().ticks(), 100);
    assert_eq!(game.stopped(), None);
    game.resize([800, 600]);
    let canvas = game.render();
    assert_eq!((canvas.width, canvas.height), (800, 600));
    // Paused, the lot owes no ticks however long it waits.
    game.pace.toggle_pause();
    assert_eq!(game.advance(10_000_000), 0);
    game.pace.toggle_pause();
    assert_eq!(game.advance(100_000), 2);
}

#[test]
fn people_stand_where_a_click_finds_them() {
    let Some(install) = install() else {
        return;
    };
    let house = goth_house(&install);
    let options = options(install.clone());
    let mut lot = sims_vm::boot::Lot::load(
        &install,
        &house,
        &options.answers,
        sims_vm::boot::Frames::default(),
    )
    .expect("the Goth house boots");
    for _ in 0..100 {
        lot.step().expect("a tick runs");
    }
    let world = lot.world();
    let mut opened = open_house(&install, &house).expect("the house opens");
    let shape = sims_render::Lot::from_world(world).expect("the lot's shape");
    let view = View::new([800, 600], shape.built_center(), opened.rotation);
    let mut canvas = Canvas::new(800, 600, [0, 0, 0]);
    sims_render::draw(world, &shape, &mut opened.assets, &view.camera, &mut canvas);
    let bare = canvas.color.clone();
    let mut people = People::open(&install, &house).expect("the people's files");
    people.refresh(world);
    people.draw(world, &shape, &view.camera, &mut canvas);
    assert!(people.problems.is_empty(), "{:?}", people.problems);
    let drawn = bare
        .iter()
        .zip(&canvas.color)
        .filter(|(a, b)| a != b)
        .count();
    assert!(drawn > 200, "only {drawn} pixels of people drawn");
    // Each placed person on the ground floor is found by a click on its
    // feet, a little above where it stands.
    let mut found = 0;
    for (&id, animator) in &world.animators {
        let Some(object) = world.objects.get(&id) else {
            continue;
        };
        if object.person.is_none() || animator.position.contains(&UNSET) {
            continue;
        }
        let [x, y, _] = animator.position.map(|v| v.to_f32() as i32);
        let camera = &view.camera;
        let ground = sims_game::people::terrain(&shape, [x as f32, y as f32]) as i32;
        let [sx, sy] = camera.project_fine(camera.rotate_fine([x, y]), 1, ground);
        if (0..800).contains(&sx) && (0..600).contains(&sy) {
            assert!(pick::person(world, &shape, camera, [sx, sy - 4]).is_some());
            found += 1;
        }
    }
    assert!(found > 0, "no person stands in view");
}

#[test]
fn play_mode_runs_the_goths_past_the_hour() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    // Past the hour turning at tick 878, through the errors play mode
    // catches.
    assert_eq!(game.run(3000), 3000);
    assert_eq!(game.stopped(), None);
    assert!(game.errors().len() as u64 <= game.caught());
}
