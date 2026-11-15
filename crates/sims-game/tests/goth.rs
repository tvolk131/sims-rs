//! The Goths' house played headless: it boots, ticks, draws its people
//! over the lot and the panel over both. Needs `SIMS_INSTALL`; skipped
//! when it is unset. Nothing from the game is kept.

use sims_game::game::goth_house;
use sims_game::people::People;
use sims_game::pick;
use sims_game::view::View;
use sims_game::{Game, Options};
use sims_render::{Canvas, open_house};
use sims_ui::household::Snapshot;
use sims_ui::hud::Kind;
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
    // A tick every 42 ms at normal speed.
    assert_eq!(game.advance(100_000), 2);
    // Paused, the lot goes on ticking, but its clock stands.
    let clock = |game: &Game| {
        let world = game.lot().world();
        (world.ticks, world.globals[5], world.globals[6])
    };
    let before = clock(&game);
    game.toggle_pause();
    assert!(game.paused());
    assert_eq!(game.advance(420_000), 10);
    assert_eq!(clock(&game), before);
    game.toggle_pause();
    assert!(!game.paused());
    // Ultra speed is a tick every 2 ms, and the simulation's speed is 994.
    game.set_speed(sims_ui::command::Speed::Ultra);
    assert_eq!(game.lot().world().speed(), 994);
    assert_eq!(game.advance(20_000), 10);
    assert_eq!(game.lot().world().ticks, before.0.wrapping_add(10));
    // A speed button runs a paused game again.
    game.toggle_pause();
    game.set_speed(sims_ui::command::Speed::Fast);
    assert!(!game.paused());
    assert_eq!(game.lot().world().speed(), 924);
    // The button of the speed the game has does not start the time again.
    assert_eq!(game.advance(13_000), 0);
    game.set_speed(sims_ui::command::Speed::Fast);
    assert_eq!(game.advance(1_000), 1);
    assert_eq!(game.stopped(), None);
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
    let bare = canvas.clone();
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
    let points: Vec<[i32; 2]> = (0..600)
        .flat_map(|y| (0..800).map(move |x| [x, y]))
        .collect();
    let changed: Vec<[i32; 2]> = points
        .iter()
        .copied()
        .filter(|&[x, y]| {
            let i = y as usize * 800 + x as usize;
            bare.color[i] != canvas.color[i]
        })
        .collect();
    assert!(
        changed.len() > 200,
        "only {} pixels of people drawn",
        changed.len()
    );
    // A click on a pixel a person was drawn in finds a person; one far
    // from every person finds none.
    let mut found = std::collections::BTreeSet::new();
    for &point in &changed {
        let id = pick::person(world, &canvas, point)
            .unwrap_or_else(|| panic!("nobody at {point:?}, which a person was drawn in"));
        assert!(world.objects[&id].person.is_some(), "object {id}");
        found.insert(id);
    }
    assert!(!found.is_empty(), "no drawn person is found by a click");
    let far = |&[x, y]: &[i32; 2]| {
        changed
            .iter()
            .all(|&[cx, cy]| (cx - x).abs() > 20 || (cy - y).abs() > 20)
    };
    let far_point = *points
        .iter()
        .find(|p| far(p))
        .expect("a pixel far from every person");
    assert_eq!(pick::person(world, &canvas, far_point), None);
    // Far from everyone, a click finds the object shown at the pixel, as
    // the lot alone shows it: each object on the lot that shows a pixel
    // there is found by a click on it.
    let mut objects = std::collections::BTreeSet::new();
    for point in points.iter().filter(|p| far(p)) {
        let shown = bare.id_at(point[0], point[1]);
        assert_eq!(canvas.id_at(point[0], point[1]), shown, "{point:?}");
        let picked = pick::at(world, &canvas, *point);
        assert_eq!(picked, shown, "{point:?}");
        if let Some(id) = picked {
            assert!(world.objects[&id].person.is_none(), "object {id}");
            objects.insert(id);
        }
    }
    assert!(objects.len() > 20, "only {} objects found", objects.len());
    // Behind a lot nearer than everyone, nobody is drawn, and a click
    // where they stand finds nobody.
    let mut covered = Canvas::new(800, 600, [0, 0, 0]);
    covered.depth.fill(i32::MIN);
    people.draw(world, &view.camera, &mut covered);
    for &point in changed.iter().step_by(7) {
        assert_eq!(pick::at(world, &covered, point), None, "{point:?}");
    }
}

#[test]
fn the_player_answers_the_help_dialog_from_the_window() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        answers: Vec::new(),
        ..options(install.clone())
    })
    .expect("the Goth house opens");
    // Tick 2 shows the Help System's dialog, Yes and No, and the lot
    // waits; what was owed beyond it is dropped.
    assert_eq!(game.advance(149_999), 1);
    assert_eq!(game.stopped(), None);
    let shown = game.asking().expect("a dialog waits").shown.clone();
    assert_eq!(shown.answers, &[2, 3]);
    let waiting = game.lot().world().hash();
    // The lot does not go on while it waits.
    assert_eq!(game.run(10), 0);
    assert_eq!(game.lot().world().hash(), waiting);
    game.resize([800, 600]);
    game.render();
    let layout = game.dialog().expect("the dialog is drawn");
    assert_eq!(layout.caller, shown.object);
    assert_eq!(layout.buttons.len(), 2);
    // No, the second button: as `--answers 3`.
    let no = layout.buttons[1];
    game.press([no.x + no.width / 2, no.y + no.height / 2]);
    assert!(game.asking().is_none(), "{:?}", game.stopped());
    // The answered tick ran at once, and the 49.999 ms owed when the
    // dialog came were dropped.
    assert_eq!(game.lot().ticks(), 2);
    assert_eq!(game.advance(1), 0);
    // From there time is owed as before: with that 1 µs, a tick.
    assert_eq!(game.advance(49_999), 1);
    assert_eq!(game.run(197), 197);
    assert_eq!(game.stopped(), None);
    let mut listed = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(listed.run(200), 200);
    assert_eq!(game.lot().world().hash(), listed.lot().world().hash());
    // The dialog is gone once answered.
    game.render();
    assert!(game.dialog().is_none());
}

#[test]
fn the_plumb_bob_spins_by_the_time_passed_in() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(100), 100);
    game.resize([800, 600]);
    let before = game.render().rgb565();
    let standing = game.lot().world().hash();
    // Short of a tick each time, so the lot stands still while half a
    // second passes: a quarter turn of the bob (half a turn a second).
    for _ in 0..12 {
        game.pace.restart();
        assert_eq!(game.advance(40_000), 0);
    }
    assert_eq!(game.lot().world().hash(), standing);
    let after = game.render().rgb565();
    let changed: Vec<usize> = (0..before.len())
        .filter(|&i| before[i] != after[i])
        .collect();
    assert!(!changed.is_empty(), "the bob did not spin");
    let xs = changed.iter().map(|i| i % 800);
    let ys = changed.iter().map(|i| i / 800);
    let (x0, x1) = (xs.clone().min().unwrap(), xs.max().unwrap());
    let (y0, y1) = (ys.clone().min().unwrap(), ys.max().unwrap());
    // A few pixels of the bob, over Mortimer's head, and nothing else.
    assert!(x1 - x0 < 16 && y1 - y0 < 30, "x {x0}..={x1}, y {y0}..={y1}");
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
    // Every object's menu, or why it is not listed. A listing the
    // simulation could not make leaves the lot as it was.
    let ids: Vec<u16> = lot.world().objects.keys().copied().collect();
    let mut menus: Vec<(u16, Result<sims_game::menu::Listing, String>)> = Vec::new();
    for id in ids {
        let before = lot.world().hash();
        let listing = sims_game::menu::entries(&mut lot, &assets, person, id);
        if !listing.as_ref().is_ok_and(|l| l.unchecked.is_none()) {
            assert_eq!(lot.world().hash(), before, "object {id}: {listing:?}");
        }
        menus.push((id, listing));
    }
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
    // The simulation lists the fridge itself; at the morning the house
    // loads in, its check tree names the meals breakfasts (0x458789).
    let fridge_menu = labels(fridge);
    assert_eq!(
        fridge_menu,
        [
            "Have A Snack",
            "Have Quick Breakfast",
            "Have Breakfast",
            "Serve Breakfast"
        ],
    );
    assert!(
        menus
            .iter()
            .any(|(id, m)| *id == fridge && m.as_ref().is_ok_and(|m| m.unchecked.is_none()))
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
    let entry = sims_game::menu::entries(&mut lot, &assets, person, shelf)
        .expect("the menu is listed")
        .entries
        .into_iter()
        .find(|e| e.label == "Read a Book")
        .expect("Read a Book");
    // Queued as choosing it does: the offered action, or by its number.
    let queued = match entry.action.clone() {
        Some(action) => lot.pick(person, action),
        None => lot.queue_interaction(person, shelf, entry.menu),
    };
    assert!(matches!(queued, Ok(Some(_))), "{queued:?}");
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
    let mut redirected = 0;
    // The redirected tiles that offer something, with the first entry.
    let mut offered = Vec::new();
    let ids: Vec<u16> = lot.world().objects.keys().copied().collect();
    // The menu numbers and labels a listing shows, or its error.
    let shown = |listing: &Result<sims_game::menu::Listing, String>| {
        listing.as_ref().map_err(String::clone).map(|l| {
            let entries: Vec<(u32, String)> = l
                .entries
                .iter()
                .map(|e| (e.menu, e.label.clone()))
                .collect();
            (entries, l.unchecked.clone())
        })
    };
    for id in ids {
        let target = sims_game::menu::target(&lot, id).expect("a target");
        let listing = sims_game::menu::entries(&mut lot, &assets, person, id);
        assert_eq!(
            shown(&listing),
            shown(&sims_game::menu::entries(&mut lot, &assets, person, target)),
            "object {id} and its target {target}"
        );
        if target != id {
            redirected += 1;
            if let Ok(listing) = listing
                && let Some(entry) = listing.entries.first()
            {
                offered.push((id, target, entry.clone()));
            }
        }
    }
    // The beds' and the guitar's other tiles send their orders on.
    assert!(redirected >= 8, "{redirected}");
    // An order on such a tile goes where the menu said it would.
    let (tile, target, entry) = offered
        .first()
        .expect("a tile that offers something")
        .clone();
    let queued = match entry.action.clone() {
        Some(action) => lot.pick(person, action),
        None => lot.queue_interaction(person, tile, entry.menu),
    };
    assert!(matches!(queued, Ok(Some(_))), "{queued:?}");
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
    let mut lot =
        sims_vm::boot::Lot::load(&install, &house, &[3], sims_vm::boot::Frames::default())
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
    let before = lot.world().hash();
    let listing = sims_game::menu::entries(&mut lot, &assets, thing, person);
    assert!(listing.is_err(), "{listing:?}");
    assert_eq!(lot.world().hash(), before);
}

#[test]
fn the_pointer_reaching_an_object_lists_its_menu_once_and_a_click_opens_it() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(100), 100);
    game.resize([800, 600]);
    game.render();
    let person = game.lot().world().selected().expect("a Sim is selected");
    // Points over objects on the lot, not over the panel.
    let points: Vec<([i32; 2], u16)> = (0..600)
        .step_by(6)
        .flat_map(|y| (0..800).step_by(6).map(move |x| [x, y]))
        .filter(|&p| !game.over_panel(p))
        .filter_map(|p| game.under(p).map(|id| (p, id)))
        .collect();
    assert!(!points.is_empty());
    let mut opened = false;
    for (point, id) in points {
        // Off every object, then onto this one: it is listed.
        game.leave();
        assert_eq!(game.listed(), None);
        game.hover(point);
        assert_eq!(game.listed(), Some((person, id)), "{point:?}");
        // Moving on the same object lists nothing again.
        let before = game.lot().world().hash();
        game.hover(point);
        assert_eq!(game.lot().world().hash(), before);
        assert_eq!(game.listed(), Some((person, id)));
        // A click opens the listing, when it offers anything.
        game.press(point);
        assert_eq!(game.listed(), None);
        if game.close_menu() {
            opened = true;
            break;
        }
    }
    assert!(opened, "no object's menu opened");
}

#[test]
fn a_snack_is_chosen_from_the_fridges_pie() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(50), 50);
    game.resize([800, 600]);
    game.render();
    let person = game.lot().world().selected().expect("a Sim is selected");
    // A click on each object on the lot, until one opens a pie with Have A
    // Snack (the fridge).
    let points: Vec<([i32; 2], u16)> = (0..600)
        .step_by(6)
        .flat_map(|y| (0..800).step_by(6).map(move |x| [x, y]))
        .filter(|&p| !game.over_panel(p))
        .filter_map(|p| game.under(p).map(|id| (p, id)))
        .collect();
    let mut tried = std::collections::BTreeSet::new();
    let mut fridge = None;
    for (point, id) in points {
        if !tried.insert(id) {
            continue;
        }
        game.hover(point);
        game.press(point);
        // The pie is laid out when it is drawn.
        game.render();
        let snack = game.menu().and_then(|menu| {
            assert!(!menu.labels().is_empty(), "object {id}'s pie shows nothing");
            let at = menu
                .labels()
                .into_iter()
                .find(|(text, _)| *text == "Have A Snack")
                .map(|(_, r)| [r.x + r.width / 2, r.y + r.height / 2])?;
            let entry = menu.entries.iter().find(|e| e.label == "Have A Snack")?;
            Some((at, entry.menu))
        });
        if let Some((at, number)) = snack {
            fridge = Some((id, at, number));
            break;
        }
        game.close_menu();
    }
    let (fridge, snack, number) = fridge.expect("the fridge's pie offers Have A Snack");
    let queued = |game: &Game| {
        let queue = &game.lot().world().objects[&person]
            .person
            .as_ref()
            .unwrap()
            .queue;
        queue
            .iter()
            .filter(|a| a.target == fridge as i16 && a.table_index == number as i32)
            .count()
    };
    let before = queued(&game);
    game.press(snack);
    assert!(game.menu().is_none(), "choosing closes the pie");
    // Have A Snack, not another of the fridge's entries, is queued.
    assert_eq!(queued(&game), before + 1);
    // The panel follows the lot as it runs: Mortimer pays 5 for the snack
    // (at tick 820) and eats it, and the clock moves on.
    game.render();
    let start = panel(&game, person);
    assert_eq!(start.money, "$2,542");
    assert_eq!(game.run(850), 850);
    game.render();
    let fed = panel(&game, person);
    assert_eq!(fed.money, "$2,537");
    assert_ne!(fed.clock, start.clock);
    assert_ne!(fed.hunger, start.hunger);
    // The pointer on Mortimer's portrait shows his name over it.
    let (portrait, guid) = game
        .panel()
        .iter()
        .find_map(|i| match i.kind {
            Kind::Portrait {
                guid,
                selected: true,
            } => Some((i.rect, guid)),
            _ => None,
        })
        .expect("the selected Sim's portrait");
    assert_eq!(guid, game.lot().world().objects[&person].guid);
    assert_eq!(game.name(guid), Some("Mortimer"));
    game.hover([portrait.x + 20, portrait.y + 20]);
    game.render();
    assert!(
        game.panel().iter().any(|i| i.kind == Kind::Name { guid }),
        "no name over the portrait"
    );
    // The pointer leaving the window takes the name away.
    assert!(game.leave(), "leaving does not redraw");
    game.render();
    assert!(
        !game.panel().iter().any(|i| i.kind == Kind::Name { guid }),
        "the name stays after the pointer left"
    );
}

/// What the panel shows of the lot, as text, and the selected Sim's
/// hunger meter, which must be what the world holds.
struct Shown {
    clock: String,
    money: String,
    hunger: f32,
}

fn panel(game: &Game, person: u16) -> Shown {
    let world = game.lot().world();
    let texts: Vec<&str> = game
        .panel()
        .iter()
        .filter_map(|i| match &i.kind {
            Kind::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let snapshot = Snapshot::read(world, &[]);
    let (clock, money) = (snapshot.clock(), snapshot.money());
    assert!(texts.contains(&clock.as_str()), "{texts:?}");
    assert!(texts.contains(&money.as_str()), "{texts:?}");
    let hunger = world.objects[&person].person.as_ref().unwrap().motives[7];
    // Hunger is the needs panel's first meter.
    let meters: Vec<f32> = game
        .panel()
        .iter()
        .filter_map(|i| match i.kind {
            Kind::Meter(value) => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(meters.first(), Some(&hunger), "{meters:?}");
    Shown {
        clock,
        money,
        hunger,
    }
}

#[test]
fn the_ambience_follows_the_outdoor_share_of_the_screen() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    game.resize([800, 600]);
    let center = [400, 300];
    let camera = game.view.camera;
    let mut views = Vec::new();
    for zoom in sims_render::Zoom::ALL {
        for turns in 0..4 {
            game.view.camera = camera.zoomed(zoom, center).rotated(turns, center);
            views.push(game.ambience_view());
        }
    }
    // The original's values are **unverified**.
    // Zoomed in on the house, less of the screen is outdoors.
    assert!(views.iter().all(|v| (0..=100).contains(v)), "{views:?}");
    assert!(
        views[8..].iter().sum::<i32>() < views[..4].iter().sum::<i32>(),
        "{views:?}"
    );
    // Upstairs, where the house has fewer rooms, more of it is outdoors.
    game.view.camera = camera.zoomed(sims_render::Zoom::Near, center);
    let ground = game.ambience_view();
    game.view.camera.level = 2;
    let upstairs = game.ambience_view();
    assert!(upstairs > ground, "{ground} {upstairs}");
    // A camera far off the lot hears no outdoors, and does not panic.
    for origin in [[i32::MIN, 0], [i32::MAX, i32::MAX], [0, i32::MIN]] {
        game.view.camera.origin = origin;
        assert_eq!(game.ambience_view(), 0);
    }
}

#[test]
fn the_sound_scripts_learn_about_the_goths() {
    use sims_audio::player::Sources;
    let Some(install) = install() else {
        return;
    };
    let game = Game::open(&options(install)).expect("the Goth house opens");
    let world = game.lot().world();
    let snapshot = sims_game::sound::sources(world);
    let hour = i32::from(world.globals[0]);
    // Cassandra is a child (field 0 is 2 below age 12), Bella a woman
    // and Mortimer a man (person data word 65); field 1 is the age, and
    // 15 to 20 the personality over 10. Skills are 0 in the save.
    let people: Vec<(i32, [i32; 3], [i32; 6])> = world
        .objects
        .iter()
        .filter(|(_, o)| o.person.is_some())
        .map(|(&id, _)| {
            let a = snapshot.objects[&i32::from(id)];
            (a[0], [a[1], a[2], a[11]], a[15..].try_into().unwrap())
        })
        .collect();
    assert_eq!(
        people,
        [
            (2, [9, 0, 0], [40, 50, 0, 70, 20, 70]),
            (1, [27, 0, 0], [70, 50, 0, 30, 30, 70]),
            (0, [27, 0, 0], [40, 40, 0, 70, 60, 40]),
        ]
    );
    for (&id, object) in &world.objects {
        let answers = snapshot.objects[&i32::from(id)];
        // Any object hears the hour and its room's size in tiles.
        assert_eq!(answers[4], hour);
        let room = object.data.get(29).and_then(|r| world.rooms.rooms.get(r));
        assert_eq!(answers[13], room.map_or(100, |r| r.half_tiles / 2));
        match &object.person {
            // Motive 3 over 10, truncated.
            Some(person) => assert_eq!(answers[14], sims_game::sound::tenth(person.motives[3])),
            None => {
                let mut rest = answers;
                (rest[4], rest[13]) = (0, 0);
                assert_eq!(rest, [0; 21], "object {id}");
            }
        }
    }
    // An ID that names nothing is outside every room; field 21 on is
    // refused.
    assert_eq!(snapshot.field(0, 13), Some(100));
    assert_eq!(snapshot.field(-3, 4), Some(hour));
    assert_eq!(snapshot.field(70_000, 0), Some(0));
    assert_eq!(snapshot.field(16, 21), None);
    // Every object is where the lot has it, for its sounds' placement;
    // an ID that names nothing is nowhere.
    for (&id, object) in &world.objects {
        assert_eq!(snapshot.position(i32::from(id)), Some(object.position));
    }
    assert_eq!(snapshot.position(70_000), None);
}

#[test]
fn the_sound_scripts_see_the_goths_where_the_camera_shows_them() {
    use sims_audio::placement;
    use sims_audio::player::{SPL_NORMAL, Sources};
    use sims_game::view::Move;
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    game.resize([800, 600]);
    while game.view.camera.rotation != 0 {
        game.apply(Move::Rotate(true));
    }
    let snapshot = game.sound_sources();
    let camera = game.view.camera;
    let world = game.lot().world();
    let objects: Vec<(u16, &sims_sim::Object)> = world
        .objects
        .iter()
        .filter(|(id, _)| **id > 0)
        .map(|(&id, o)| (id, o))
        .collect();
    assert_eq!(snapshot.screen.len(), objects.len());
    // Unturned, the sound's projection is the original view's for the
    // object's place on the floor, which is the drawing's or a pixel off
    // it.
    for &(id, object) in &objects {
        let [x, y, level] = object.position.map(i32::from);
        let view = snapshot.projection(i32::from(id)).unwrap();
        assert_eq!(
            view.at,
            sims_game::sound::fine_point(&camera, [x, y], level),
            "object {id}"
        );
        let drawn = camera.project_fine([x, y], level, 0);
        assert!(
            (0..2).all(|i| (view.at[i] - drawn[i]).abs() <= 1),
            "object {id}: {:?} {drawn:?}",
            view.at
        );
        assert_eq!(view.screen, [800, 600]);
        assert_eq!(view.zoom, camera.zoom.level() as i32);
        assert_eq!(view.shown_level, camera.level);
    }
    // The Goths are on the screen at the far zoom the house opens at,
    // where a track's sounds lose 560 of their 1024; nearer, less.
    let people: Vec<u32> = objects
        .iter()
        .filter(|(_, o)| o.person.is_some())
        .map(|&(id, _)| u32::from(id))
        .collect();
    let far = placement::combine(&people, SPL_NORMAL, &snapshot).unwrap();
    assert!(far.0 <= 1024 - 560, "{far:?}");
    assert!((0..=1024).contains(&far.1), "{far:?}");
    game.apply(Move::ZoomIn);
    let medium = placement::combine(&people, SPL_NORMAL, &game.sound_sources()).unwrap();
    assert!(medium.0 > far.0, "{medium:?} {far:?}");
}

#[test]
fn a_press_on_a_portrait_selects_that_goth() {
    let Some(install) = install() else {
        return;
    };
    // Play mode: in strict mode deselecting Mortimer fails on his
    // animator's refresh, which the simulation has not ported.
    let mut game = Game::open(&Options {
        play: true,
        ..options(install)
    })
    .expect("the Goth house opens");
    assert_eq!(game.run(20), 20);
    game.resize([800, 600]);
    game.render();
    let world = game.lot().world();
    let mortimer = world.selected().expect("a Sim is selected");
    // The household: the people who live on the lot (person word 32 is
    // 0) and are of Mortimer's family (word 61).
    let family = world.objects[&mortimer].person.as_ref().unwrap().data[61];
    let household: Vec<&sims_sim::Object> = world
        .people
        .iter()
        .map(|id| &world.objects[id])
        .filter(|o| {
            let data = &o.person.as_ref().unwrap().data;
            data[32] == 0 && data[61] == family
        })
        .collect();
    let other = household
        .iter()
        .find(|o| o.id != mortimer)
        .expect("another Goth")
        .id;
    let members: Vec<u32> = household.iter().map(|o| o.guid).collect();
    // Where the panel shows the other Goth's portrait.
    let controls = sims_ui::hud::Controls {
        expanded: true,
        ..sims_ui::hud::Controls::default()
    };
    let snapshot = sims_ui::household::Snapshot::read(world, &members);
    let portrait = sims_ui::hud::layout([800, 600], &snapshot, &controls)
        .into_iter()
        .find(|item| item.command == Some(sims_ui::command::Command::Select { sim: other }))
        .expect("the other Goth's portrait")
        .rect;
    game.press([
        portrait.x + portrait.width / 2,
        portrait.y + portrait.height / 2,
    ]);
    assert_eq!(game.lot().world().selected(), Some(other));
    assert_eq!(game.lot().world().globals[3], other as i16);
    // The lot goes on with the new selection.
    assert_eq!(game.run(50), 50);
    assert_eq!(game.stopped(), None);
    game.select(mortimer);
    assert_eq!(game.lot().world().selected(), Some(mortimer));
}

#[test]
fn the_pie_shows_the_selected_goths_head() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(100), 100);
    game.resize([800, 600]);
    game.render();
    let selected = game.lot().world().selected().expect("a Sim is selected");
    let points: Vec<[i32; 2]> = (0..600)
        .step_by(6)
        .flat_map(|y| (0..800).step_by(6).map(move |x| [x, y]))
        .filter(|&p| !game.over_panel(p))
        .collect();
    for point in points {
        game.leave();
        game.hover(point);
        game.press(point);
        if game.menu().is_some() {
            break;
        }
    }
    game.render();
    let menu = game.menu().expect("the menu stays open");
    assert_eq!(menu.person, selected);
    let head = menu.head.as_ref().expect("the head is drawn");
    assert_eq!([head.width, head.height], menu.face_size());
    // The head fills much of the face, and is clear around it.
    let opaque = head.rgba.chunks(4).filter(|p| p[3] == 255).count();
    let area = (head.width * head.height) as usize;
    assert!(
        opaque > area / 5 && opaque < area * 4 / 5,
        "{opaque} of {area}"
    );
    assert_eq!(head.rgba[3], 0);
    assert!(game.problems().is_empty(), "{:?}", game.problems());
}
