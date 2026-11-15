//! The Goths' house played headless: it boots, ticks, draws its people
//! over the lot and the panel over both. Needs `SIMS_INSTALL`; skipped
//! when it is unset. Nothing from the game is kept.

use sims_game::game::goth_house;
use sims_game::people::People;
use sims_game::view::View;
use sims_game::{Game, Options};
use sims_render::{Canvas, open_house};
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
        play: false,
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
    let mut theirs = canvas.clone();
    let mut people = People::open(&install, &house).expect("the people's files");
    people.refresh(world);
    people.draw(world, &view.camera, &mut canvas);
    assert!(people.problems.is_empty(), "{:?}", people.problems);
    // The game draws them as sims-people's own call does, pixel for pixel.
    let mut reference = sims_people::People::open(&install, &house).expect("the people's files");
    assert!(reference.update(world).is_empty());
    let drawn = reference.draw_on_lot(world, &view.camera, &mut theirs);
    assert!(drawn.people > 0 && drawn.problems.is_empty(), "{drawn:?}");
    assert!(theirs.color == canvas.color && theirs.depth == canvas.depth);
    let drawn = bare
        .iter()
        .zip(&canvas.color)
        .filter(|(a, b)| a != b)
        .count();
    assert!(drawn > 200, "only {drawn} pixels of people drawn");
    // A click on a pixel a person was drawn in finds a person; one far
    // from every person finds none.
    let mut found = std::collections::BTreeSet::new();
    let changed: Vec<usize> = (0..bare.len())
        .filter(|&i| bare[i] != canvas.color[i])
        .collect();
    for &i in &changed {
        let point = [(i % 800) as i32, (i / 800) as i32];
        let id = people
            .at(world, point)
            .unwrap_or_else(|| panic!("nobody at {point:?}, which a person was drawn in"));
        assert!(world.objects[&id].person.is_some(), "object {id}");
        found.insert(id);
    }
    assert!(!found.is_empty(), "no drawn person is found by a click");
    let far = (0..bare.len())
        .find(|&i| {
            let [x, y] = [(i % 800) as i32, (i / 800) as i32];
            changed.iter().all(|&j| {
                let [cx, cy] = [(j % 800) as i32, (j / 800) as i32];
                (cx - x).abs() > 20 || (cy - y).abs() > 20
            })
        })
        .expect("a pixel far from every person");
    assert_eq!(
        people.at(world, [(far % 800) as i32, (far / 800) as i32]),
        None
    );
    // Behind a lot nearer than everyone, nobody is drawn, and a click
    // where they stand finds nobody.
    let mut covered = Canvas::new(800, 600, [0, 0, 0]);
    covered.depth.fill(i32::MIN);
    people.draw(world, &view.camera, &mut covered);
    for &i in changed.iter().step_by(7) {
        let point = [(i % 800) as i32, (i / 800) as i32];
        assert_eq!(people.at(world, point), None, "{point:?}");
    }
}

#[test]
fn play_mode_runs_the_goths_past_the_hour() {
    let Some(install) = install() else {
        return;
    };
    let options = Options {
        play: true,
        ..options(install)
    };
    let mut game = Game::open(&options).expect("the Goth house opens");
    // Past the hour turning at tick 878, through the errors play mode
    // catches.
    assert_eq!(game.run(3000), 3000);
    assert_eq!(game.stopped(), None);
    assert!(game.errors().len() as u64 <= game.caught());
}

#[test]
fn the_fridge_and_the_bookshelf_offer_their_interactions() {
    let Some(install) = install() else {
        return;
    };
    let house = goth_house(&install);
    let mut lot =
        sims_vm::boot::Lot::load(&install, &house, &[3], sims_vm::boot::Frames::default())
            .expect("the Goth house boots");
    let assets = open_house(&install, &house)
        .expect("the house opens")
        .assets;
    let person = lot.world().selected().expect("a Sim is selected");
    let before = lot.world().hash();
    // Every object's menu, or why it is not listed.
    let menus: Vec<(u16, Result<sims_game::menu::Listing, String>)> = lot
        .world()
        .objects
        .keys()
        .map(|&id| (id, sims_game::menu::entries(&lot, &assets, person, id)))
        .collect();
    // Listing them all changed nothing in the lot.
    assert_eq!(lot.world().hash(), before);
    let labels = |id: u16| -> Vec<String> {
        menus
            .iter()
            .find(|(i, _)| *i == id)
            .and_then(|(_, m)| m.as_ref().ok())
            .map(|m| m.entries.iter().map(|e| e.label.clone()).collect())
            .unwrap_or_default()
    };
    let find = |label: &str| {
        menus
            .iter()
            .find(|(_, m)| {
                m.as_ref()
                    .is_ok_and(|m| m.entries.iter().any(|e| e.label == label))
            })
            .map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("nothing offers {label}"))
    };
    let fridge = find("Have A Snack");
    let shelf = find("Read a Book");
    let fridge_menu = labels(fridge);
    assert!(
        fridge_menu.iter().any(|l| l == "Have A Meal"),
        "{fridge_menu:?}"
    );
    // The simulation lists many of them itself, naming an entry with
    // flag 0x80 with a "*" before it.
    let checked: Vec<&sims_game::menu::Listing> = menus
        .iter()
        .filter_map(|(_, m)| m.as_ref().ok())
        .filter(|m| m.unchecked.is_none() && !m.entries.is_empty())
        .collect();
    assert!(checked.len() >= 10, "{}", checked.len());
    assert!(
        checked
            .iter()
            .any(|m| m.entries.iter().any(|e| e.label.starts_with('*')))
    );
    let entry = sims_game::menu::entries(&lot, &assets, person, shelf)
        .expect("the menu is listed")
        .entries
        .into_iter()
        .find(|e| e.label == "Read a Book")
        .expect("Read a Book");
    assert!(matches!(
        lot.queue_interaction(person, shelf, entry.menu),
        Ok(Some(_))
    ));
    let queue = &lot.world().objects[&person].person.as_ref().unwrap().queue;
    assert!(
        queue
            .iter()
            .any(|a| a.target == shelf as i16 && a.table_index == entry.menu as i32),
        "{queue:?}"
    );
}

#[test]
fn a_tile_offers_the_menu_of_the_tile_its_orders_go_to() {
    let Some(install) = install() else {
        return;
    };
    let house = goth_house(&install);
    let mut lot =
        sims_vm::boot::Lot::load(&install, &house, &[3], sims_vm::boot::Frames::default())
            .expect("the Goth house boots");
    let assets = open_house(&install, &house)
        .expect("the house opens")
        .assets;
    let person = lot.world().selected().expect("a Sim is selected");
    let mut redirected = Vec::new();
    for &id in lot.world().objects.keys() {
        let target = sims_game::menu::target(&lot, id).expect("a target");
        let listing = sims_game::menu::entries(&lot, &assets, person, id);
        assert_eq!(
            listing,
            sims_game::menu::entries(&lot, &assets, person, target),
            "object {id} and its target {target}"
        );
        if target != id
            && let Ok(listing) = listing
            && let Some(entry) = listing.entries.first()
        {
            redirected.push((id, target, entry.clone()));
        }
    }
    // The beds' and the guitar's other tiles send their orders on.
    assert!(redirected.len() >= 8, "{}", redirected.len());
    // An order on such a tile goes where the menu said it would.
    let (tile, target, entry) = redirected[0].clone();
    assert!(matches!(
        lot.queue_interaction(person, tile, entry.menu),
        Ok(Some(_))
    ));
    let queue = &lot.world().objects[&person].person.as_ref().unwrap().queue;
    assert!(
        queue
            .iter()
            .any(|a| a.target == target as i16 && a.table_index == entry.menu as i32),
        "tile {tile}, target {target}: {queue:?}"
    );
}

#[test]
fn a_menu_for_someone_who_is_not_a_person_is_an_error() {
    let Some(install) = install() else {
        return;
    };
    let house = goth_house(&install);
    let lot = sims_vm::boot::Lot::load(&install, &house, &[3], sims_vm::boot::Frames::default())
        .expect("the Goth house boots");
    let assets = open_house(&install, &house)
        .expect("the house opens")
        .assets;
    let person = lot.world().selected().expect("a Sim is selected");
    let thing = *lot
        .world()
        .objects
        .iter()
        .find(|(_, o)| o.person.is_none())
        .expect("an object that is not a person")
        .0;
    // Not something unported: no fallback menu is shown.
    let listing = sims_game::menu::entries(&lot, &assets, thing, person);
    assert!(listing.is_err(), "{listing:?}");
}
