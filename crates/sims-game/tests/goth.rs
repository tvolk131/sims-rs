//! The Goths' house played headless: it boots, ticks, draws its people
//! over the lot and the panel over both. Needs `SIMS_INSTALL`; skipped
//! when it is unset. Nothing from the game is kept.

use sims_game::game::goth_house;
use sims_game::people::People;
use sims_game::pick;
use sims_game::view::View;
use sims_game::{Game, Options};
use sims_render::{Canvas, open_house};
use sims_ui::frame::Rect;
use sims_ui::household::Snapshot;
use sims_ui::hud::Kind;
use sims_ui::queue::Picture;
use sims_vm::SoundEvent;
use sims_vm::boot::{Frames, Lot};
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
        frames: Frames::default(),
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
    // from every person finds none. The plumb bob's pixels name the bob
    // itself, not the person holding it.
    let bob = world
        .selected()
        .and_then(|person| sims_people::bob::held_by(world, person));
    let (on_bob, of_people): (Vec<[i32; 2]>, Vec<[i32; 2]>) = changed
        .iter()
        .partition(|&&[x, y]| bob.is_some() && canvas.id_at(x, y) == bob);
    assert!(bob.is_none() || !on_bob.is_empty(), "the bob is not drawn");
    // A click on the bob's pixels finds the bob, not its holder.
    let camera = &view.camera;
    let people_ids = canvas.ids();
    assert!(
        bob.is_none()
            || on_bob
                .iter()
                .all(|&p| pick::noted(world, camera, &people_ids, p) == bob),
        "a click on the bob finds something else"
    );
    // A click on a pixel a person was drawn in finds that person, the
    // pixel itself and no reach around it.
    let mut found = std::collections::BTreeSet::new();
    for &[x, y] in &of_people {
        let Some(shown) = canvas.id_at(x, y) else {
            continue;
        };
        let id = pick::noted(world, camera, &people_ids, [x, y]);
        assert_eq!(id, Some(shown), "at {:?}", [x, y]);
        assert!(world.objects[&shown].person.is_some(), "object {shown}");
        found.insert(shown);
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
    assert_eq!(pick::noted(world, camera, &people_ids, far_point), None);
    // Far from everyone, a click finds an object by the sprite pick, from
    // the lot's depth alone: never a person, and mostly the object the
    // pixel shows (a rectangle nearer the viewer may hold the point too).
    let mut objects = std::collections::BTreeSet::new();
    let (mut shown_objects, mut agree) = (0, 0);
    for point in points.iter().filter(|p| far(p)) {
        let shown = bare.id_at(point[0], point[1]);
        assert_eq!(canvas.id_at(point[0], point[1]), shown, "{point:?}");
        let picked = pick::at(&lot, camera, &canvas, &bare.depth, &people_ids, *point);
        if shown.is_some() {
            shown_objects += 1;
            agree += usize::from(picked == shown);
        }
        if let Some(id) = picked {
            assert!(world.objects[&id].person.is_none(), "object {id}");
            objects.insert(id);
        }
    }
    // At 100 ticks, 30689 of the 38365 pixels that show an object.
    assert!(
        agree * 4 > shown_objects * 3,
        "the pick finds the shown object at only {agree} of {shown_objects} pixels"
    );
    assert!(objects.len() > 20, "only {} objects found", objects.len());
    // Behind a lot nearer than everyone, nobody is drawn, and a click
    // where they stand finds nobody.
    let mut covered = Canvas::new(800, 600, [0, 0, 0]);
    covered.depth.fill(i32::MIN);
    people.draw(world, camera, &mut covered);
    let covered = covered.ids();
    for &point in changed.iter().step_by(7) {
        assert_eq!(
            pick::noted(world, camera, &covered, point),
            None,
            "{point:?}"
        );
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
    assert_eq!(game.lot().world().globals[12], 17);
    assert_eq!(game.advance(1), 0);
    // From there time is owed as before: with that 1 µs, a tick.
    assert_eq!(game.advance(49_999), 1);
    assert_eq!(game.lot().world().globals[12], 0);
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
    assert!(!game.spin_due());
    let standing = game.lot().world().hash();
    // Short of a tick each time, so the lot stands still while half a
    // second passes: a quarter turn of the bob (half a turn a second).
    for _ in 0..12 {
        game.pace.restart();
        assert_eq!(game.advance(40_000), 0);
    }
    assert_eq!(game.lot().world().hash(), standing);
    // No tick ran, but the bob is due a new frame, so the window draws.
    assert!(game.spin_due());
    let after = game.render().rgb565();
    assert!(!game.spin_due());
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
fn the_pointer_reaching_an_object_lists_its_menu_and_a_click_lists_it_again() {
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
        // A click lists the menu again (0x48cbec) and opens that
        // listing, when it offers anything.
        game.press(point);
        assert_eq!(game.listed(), None);
        if game.close_menu() {
            opened = true;
            break;
        }
    }
    assert!(opened, "no object's menu opened");
}

/// The plumb bob is picked as itself, not as the person under it: the
/// original's bob draw makes the bob the graphic under the pointer when
/// it changes the depth there (0x586455), and the pick then lists the
/// bob's pie (0x48ca2c → 0x48c834). With the pointer on a pixel of the
/// bob, the selected Sim's pie is listed on object 383. As in the snack order's
/// recording at update 622, of Mortimer's object words only data word 50
/// changes, to 1 (his running checks' word, set by the bob's check
/// trees); his frames (tree, node, locals) and his last result stay as
/// they were.
#[test]
fn the_pointer_on_the_plumb_bob_lists_the_bobs_menu() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(100), 100);
    game.resize([800, 600]);
    game.render();
    let world = game.lot().world();
    assert_eq!(world.selected(), Some(21));
    let bob = sims_people::bob::held_by(world, 21).expect("Mortimer holds the bob");
    assert_eq!(bob, 383);
    let point = (0..600)
        .flat_map(|y| (0..800).map(move |x| [x, y]))
        .find(|&p| !game.over_panel(p) && game.under(p) == Some(bob))
        .expect("the bob is shown");
    let mortimer = |game: &Game| {
        let o = &game.lot().world().objects[&21];
        let stack: Vec<_> = o
            .stack
            .iter()
            .map(|f| (f.tree, f.node, f.locals.clone()))
            .collect();
        (o.data[50], o.last_result, stack)
    };
    let (word, last, stack) = mortimer(&game);
    assert_eq!(word, 0);
    game.hover(point);
    assert_eq!(game.listed(), Some((21, bob)));
    assert_eq!(mortimer(&game), (1, last, stack));
}

/// The sprite pick works on a turned view as on the first: in each of
/// the four rotations, most pixels that show an object pick it. A turn
/// takes effect on the pick with the frame drawn through it, not before:
/// until then the pointer picks from the frame on screen.
#[test]
fn the_pointer_picks_what_each_turned_view_shows() {
    use sims_game::view::Move;
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(100), 100);
    game.resize([800, 600]);
    let points: Vec<[i32; 2]> = (0..600)
        .step_by(3)
        .flat_map(|y| (0..800).step_by(3).map(move |x| [x, y]))
        .collect();
    for turn in 0..4 {
        game.render();
        let picks: Vec<Option<u16>> = points.iter().map(|&p| game.under(p)).collect();
        let world = game.lot().world();
        let (mut shown, mut agree) = (0, 0);
        for (&[x, y], &picked) in points.iter().zip(&picks) {
            let Some(id) = game.canvas().id_at(x, y) else {
                continue;
            };
            if game.over_panel([x, y]) || world.objects[&id].person.is_some() {
                continue;
            }
            shown += 1;
            agree += usize::from(picked == Some(id));
        }
        assert!(
            shown > 1000 && agree * 4 > shown * 3,
            "turn {turn}: the pick finds the shown object at {agree} of {shown} pixels"
        );
        // Turned but not drawn yet: the picks are the frame's on screen.
        game.apply(Move::Rotate(true));
        let after: Vec<Option<u16>> = points.iter().map(|&p| game.under(p)).collect();
        assert!(
            after == picks,
            "turn {turn}: the pick moved before the frame"
        );
    }
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
    let (fridge, snack, number) = snack_pie(&mut game);
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
    assert!(
        game.menu().is_some(),
        "pressing on an entry does not close the pie"
    );
    game.release(snack);
    assert!(
        game.menu().is_none(),
        "releasing on an entry closes the pie"
    );
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
    // He eats: the fridge gave him the action, whose icon is the snack
    // (object 263), yellow now that it is under way (person data word 39,
    // set at tick 837).
    let snack_guid = game.lot().world().objects[&263].guid;
    let running = &game.queue().icons[0];
    assert_eq!(
        running.uid,
        game.lot().world().objects[&person]
            .person
            .as_ref()
            .unwrap()
            .active
            .uid
    );
    assert_eq!(running.picture, Picture::Object(snack_guid));
    assert!(running.yellow);
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
/// Clicks each object on the lot until one opens a pie with Have A Snack
/// (the fridge), and leaves that pie open. Returns the object clicked,
/// where Have A Snack is, and its menu number.
fn snack_pie(game: &mut Game) -> (u16, [i32; 2], u32) {
    let points: Vec<([i32; 2], u16)> = (0..600)
        .step_by(6)
        .flat_map(|y| (0..800).step_by(6).map(move |x| [x, y]))
        .filter(|&p| !game.over_panel(p))
        .filter_map(|p| game.under(p).map(|id| (p, id)))
        .collect();
    let mut tried = std::collections::BTreeSet::new();
    for (point, id) in points {
        if !tried.insert(id) {
            continue;
        }
        game.hover(point);
        game.press(point);
        game.release(point);
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
            return (id, at, number);
        }
        game.close_menu();
    }
    panic!("no pie offers Have A Snack");
}

/// The snack queued from the fridge's pie shows as an icon in the top
/// left; a press on it cancels it, and so does a press on the icon of the
/// snack Mortimer is doing (`Lot::cancel`, 0x4cb126).
#[test]
fn a_press_on_an_actions_icon_cancels_it() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(50), 50);
    game.resize([800, 600]);
    game.render();
    let person = game.lot().world().selected().expect("a Sim is selected");
    let mortimer = |game: &Game| game.lot().world().objects[&person].person.clone().unwrap();
    // He is doing nothing, so nothing shows.
    assert!(game.queue().icons.is_empty());
    let (_, snack, _) = snack_pie(&mut game);
    game.press(snack);
    game.release(snack);
    game.render();
    let [queued] = &mortimer(&game).queue[..] else {
        panic!("{:?}", mortimer(&game).queue);
    };
    // The icon is the fridge's (object 118), grey, at (32, 32).
    let fridge = game.lot().world().objects[&118].guid;
    let [icon] = game
        .queue()
        .icons
        .clone()
        .try_into()
        .unwrap_or_else(|icons| {
            panic!("{icons:?}");
        });
    assert_eq!(icon.uid, queued.uid);
    assert_eq!(icon.picture, Picture::Object(fridge));
    assert!(!icon.yellow && !icon.cancelled);
    assert_eq!(icon.rect, Rect::new(32, 32, 45, 45));
    let centre = [icon.rect.x + 22, icon.rect.y + 22];
    // In a narrow window the note the order left across the top wraps
    // down over the icon; a press there is the band's, and cancels
    // nothing. The press clears the note.
    game.resize([80, 600]);
    game.render();
    game.take_sounds();
    let under_band = [icon.rect.x + 22, icon.rect.y + 8];
    assert_eq!(game.queue().at(under_band).map(|i| i.uid), Some(icon.uid));
    game.press(under_band);
    assert_eq!(mortimer(&game).queue.len(), 1);
    assert!(game.take_sounds().is_empty());
    game.resize([800, 600]);
    game.render();
    // A press on it takes it off the queue, with the interface's sound.
    assert!(game.over_panel(centre));
    game.press(centre);
    assert!(mortimer(&game).queue.is_empty());
    assert_eq!(game.take_sounds(), [delete()]);
    game.render();
    assert!(game.queue().icons.is_empty());
    // Queued again, he takes it 100 ticks later.
    let (_, snack, _) = snack_pie(&mut game);
    game.press(snack);
    game.release(snack);
    let uid = mortimer(&game).queue[0].uid;
    for _ in 0..100 {
        assert_eq!(game.run(1), 1);
    }
    assert_eq!(mortimer(&game).active.uid, uid);
    game.render();
    let icon = game.queue().icons[0].clone();
    assert_eq!((icon.uid, icon.yellow, icon.cancelled), (uid, false, false));
    // A press on the running snack cancels it: the cross shows, and a
    // second press does nothing.
    game.take_sounds();
    game.press([icon.rect.x + 22, icon.rect.y + 22]);
    assert_ne!(mortimer(&game).active.flags & 0x100, 0);
    assert_eq!(game.take_sounds(), [delete()]);
    game.render();
    assert!(game.queue().icons[0].cancelled);
    let before = mortimer(&game);
    game.press([icon.rect.x + 22, icon.rect.y + 22]);
    assert!(game.take_sounds().is_empty());
    assert_eq!(mortimer(&game).queue.len(), before.queue.len());
}

/// The fridge's icon, queued, and the snack's, running, show their
/// types' catalog pictures: the left cell of each one's own 74 by 37
/// thumbnail (`BMP_` of its catalog strings ID), centred in the icon
/// (0x4cbdc3, 0x4ccaa0).
#[test]
fn the_queues_icons_show_their_objects_catalog_pictures() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install.clone())).expect("the Goth house opens");
    assert_eq!(game.run(50), 50);
    game.resize([800, 600]);
    game.render();
    let (_, snack, _) = snack_pie(&mut game);
    game.press(snack);
    game.release(snack);
    game.render();
    let thumbnails = Thumbnails::new(&install);
    let fridge = game.lot().world().objects[&118].guid;
    assert_eq!(game.queue().icons[0].picture, Picture::Object(fridge));
    thumbnails.check(&game, fridge);
    // He eats the snack (object 263), its icon now the running one.
    assert_eq!(game.run(850), 850);
    game.render();
    let snack = game.lot().world().objects[&263].guid;
    assert_eq!(game.queue().icons[0].picture, Picture::Object(snack));
    assert_ne!(snack, fridge);
    thumbnails.check(&game, snack);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

#[test]
fn a_type_without_graphics_or_interactions_has_no_catalog_item() {
    let Some(install) = install() else {
        return;
    };
    let own = Thumbnails::new(&install);
    let mut thumbnails = sims_ui::thumbnail::Thumbnails::default();
    // ReserveTile: no graphics, and no interaction table (0x4575a1).
    let reserve = 0x3929_AADC;
    thumbnails.read(&own.index, &own.catalog, reserve).unwrap();
    assert!(own.catalog.object(reserve).is_some());
    assert_eq!(thumbnails.item(reserve), None);
    // The fridge has graphics and its thumbnail.
    let fridge = 0x675C_18AF;
    thumbnails.read(&own.index, &own.catalog, fridge).unwrap();
    let item = thumbnails.item(fridge).expect("the fridge's item");
    assert_eq!(item.thumbnail.as_ref(), Some(&own.thumbnail(fridge)));
}

/// Object types' own thumbnails, read from the install apart from the
/// game's reading.
struct Thumbnails {
    index: sims_content::Index,
    catalog: sims_content::Catalog,
}

impl Thumbnails {
    fn new(install: &std::path::Path) -> Self {
        let install = sims_content::Install::detect(install).unwrap();
        let index = sims_content::Index::build(&install).unwrap();
        let catalog = sims_content::Catalog::build(&index).unwrap();
        Self { index, catalog }
    }

    /// The `BMP_` of `guid`'s catalog strings ID, for a single-tile type.
    fn thumbnail(&self, guid: u32) -> sims_formats::bitmap::Image {
        use sims_formats::behavior::ObjectDefinition;
        use sims_formats::iff::Iff;
        let object = self.catalog.object(guid).expect("a known type");
        let iff = Iff::parse(self.index.resources[object.resource].read().unwrap()).unwrap();
        let definition =
            ObjectDefinition::parse(iff.data(iff.get(b"OBJD", object.chunk).unwrap())).unwrap();
        assert_eq!(definition.master_id, 0);
        let chunk = iff.get(b"BMP_", definition.catalog).expect("a thumbnail");
        let image = sims_formats::bitmap::bmp(iff.data(chunk)).unwrap();
        assert_eq!([image.width, image.height], [74, 37]);
        image
    }

    /// The first icon shows `guid`'s thumbnail's left cell, 4 pixels in.
    fn check(&self, game: &Game, guid: u32) {
        let thumbnail = self.thumbnail(guid);
        let icon = &game.queue().icons[0];
        let canvas = game.canvas();
        for [dx, dy] in [[12, 12], [22, 22], [30, 18], [18, 33], [33, 33], [8, 25]] {
            let x = usize::try_from(icon.rect.x + dx).unwrap();
            let y = usize::try_from(icon.rect.y + dy).unwrap();
            let [r, g, b, _] = thumbnail
                .pixel(
                    u32::try_from(dx - 4).unwrap(),
                    u32::try_from(dy - 4).unwrap(),
                )
                .unwrap();
            assert_eq!(
                canvas.color[y * canvas.width + x],
                [r, g, b],
                "type 0x{guid:08X} at ({dx}, {dy})"
            );
        }
    }
}

/// Play mode, through the window: the player orders a snack from the
/// fridge's pie and then the plant's Water from its pie. The water waits
/// behind the snack, its icon second. A press on the queued water's icon
/// takes it off; a press on the snack's icon while Mortimer eats it
/// cancels it, he puts the snack down (Drop, 0x454430) and goes to water
/// the plant. No error is caught on the way.
#[test]
fn a_second_order_waits_behind_the_first_and_a_press_cancels_either() {
    let Some(install) = install() else {
        return;
    };
    let open = || {
        let mut game = Game::open(&Options {
            play: true,
            ..options(install.clone())
        })
        .expect("the Goth house opens");
        assert_eq!(game.run(50), 50);
        game.resize([800, 600]);
        game.render();
        game
    };
    let (fridge, plant) = (118, 162);
    let mut game = open();
    let sim = game.lot().world().selected().expect("a Sim is selected");
    order(&mut game, fridge, "Have A Snack");
    order(&mut game, plant, "Water");
    let mortimer = |game: &Game| game.lot().world().objects[&sim].person.clone().unwrap();
    let orders: Vec<(i32, i16)> = mortimer(&game)
        .queue
        .iter()
        .map(|a| (a.uid, a.target))
        .collect();
    assert_eq!(orders, [(1, fridge as i16), (2, plant as i16)]);
    let guid = |game: &Game, id: u16| game.lot().world().objects[&id].guid;
    let icons = |game: &Game| -> Vec<(i32, Picture, i32)> {
        game.queue()
            .icons
            .iter()
            .map(|i| (i.uid, i.picture, i.rect.x))
            .collect()
    };
    assert_eq!(
        icons(&game),
        [
            (1, Picture::Object(guid(&game, fridge)), 32),
            (2, Picture::Object(guid(&game, plant)), 82)
        ]
    );

    // The queued water, taken off: the snack goes on to its end at step
    // 1136, and nothing follows it.
    let mut taken_off = open();
    order(&mut taken_off, fridge, "Have A Snack");
    order(&mut taken_off, plant, "Water");
    taken_off.take_sounds();
    taken_off.press([82 + 22, 32 + 22]);
    assert_eq!(taken_off.take_sounds(), [delete()]);
    assert_eq!(mortimer(&taken_off).queue.len(), 1);
    assert_eq!(taken_off.run(1085), 1085);
    assert_ne!(mortimer(&taken_off).active.uid, 0);
    assert_eq!(taken_off.run(1), 1);
    assert_eq!(mortimer(&taken_off).active.uid, 0);
    assert!(mortimer(&taken_off).queue.is_empty());
    assert_eq!(taken_off.caught(), 0, "{:?}", taken_off.errors());

    // Step 899: he eats the snack (object 263), his sixth action, as the
    // water took the second number; the water waits.
    assert_eq!(game.run(849), 849);
    game.render();
    let snack = 263;
    let eating = mortimer(&game).active.uid;
    assert_eq!(eating, 6);
    assert_eq!(
        icons(&game),
        [
            (6, Picture::Object(guid(&game, snack)), 32),
            (2, Picture::Object(guid(&game, plant)), 82)
        ]
    );
    assert!(game.queue().icons[0].yellow);
    game.take_sounds();
    game.press([32 + 22, 32 + 22]);
    assert_eq!(game.take_sounds(), [delete()]);
    assert_ne!(mortimer(&game).active.flags & 0x100, 0);
    // With the water queued, the cancel queues nothing of its own
    // (0x45d7c0).
    assert_eq!(mortimer(&game).queue.len(), 1);
    // He puts the snack down in front of him at step 1009, and at step
    // 1028 the water starts.
    assert_eq!(game.run(128), 128);
    assert_eq!(mortimer(&game).active.uid, eating);
    assert_eq!(game.lot().world().objects[&snack].position, [280, 456, 1]);
    assert_eq!(game.run(1), 1);
    let watering = mortimer(&game).active;
    assert_eq!((watering.uid, watering.target), (2, plant as i16));
    game.render();
    assert_eq!(icons(&game)[0].0, 2);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Moves the pointer straight onto `object` where the frame shows it
/// (listing its pie), presses it, and presses `label` on the pie.
fn order(game: &mut Game, object: u16, label: &str) {
    let at = (0..600)
        .step_by(4)
        .flat_map(|y| (0..800).step_by(4).map(move |x| [x, y]))
        .find(|&p| !game.over_panel(p) && game.under(p) == Some(object))
        .unwrap_or_else(|| panic!("object {object} is not on screen"));
    game.hover(at);
    game.press(at);
    game.release(at);
    game.render();
    let entry = {
        let menu = game
            .menu()
            .unwrap_or_else(|| panic!("object {object}'s pie"));
        let (_, r) = menu
            .labels()
            .into_iter()
            .find(|(text, _)| *text == label)
            .unwrap_or_else(|| panic!("{label} on object {object}'s pie"));
        [r.x + r.width / 2, r.y + r.height / 2]
    };
    game.press(entry);
    game.release(entry);
    assert!(game.menu().is_none());
    game.render();
}

fn delete() -> SoundEvent {
    SoundEvent::Play {
        object: -4,
        name: "UI_queue_delete".into(),
    }
}

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

/// Play mode, as the window runs with `--play`: the snack ordered through
/// the game's input layer is the scripted snack order (sims-vm's
/// `the_snack_order_pays_for_the_snack`, `Lot::queue_interaction` after 50
/// steps). The pointer moves onto the fridge, which lists its pie; a
/// press opens it, and a press on Have A Snack queues the action the
/// listing offered (`Lot::pick`). The action queued is the scripted one.
/// The lots differ until step 205: the listing's check trees and the
/// entry's check when it is chosen ran as him, and left his last result
/// and words in his frame storage, which the snack's frames start with
/// from step 150. From then on they are the same lot, step for step,
/// through the snack's end at step 1136, and play mode catches no error
/// on the way.
#[test]
fn the_snack_ordered_in_the_window_is_the_scripted_order() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        play: true,
        ..options(install.clone())
    })
    .expect("the Goth house opens");
    assert_eq!(game.run(50), 50);
    game.resize([800, 600]);
    game.render();
    let sim = game.lot().world().selected().expect("a Sim is selected");
    let fridge = 118;
    // Where the frame shows the fridge, found without moving the pointer,
    // whose every new object lists a menu that changes the lot.
    let at = (0..600)
        .step_by(4)
        .flat_map(|y| (0..800).step_by(4).map(move |x| [x, y]))
        .find(|&p| !game.over_panel(p) && game.under(p) == Some(fridge))
        .expect("the fridge is on screen");
    game.hover(at);
    assert_eq!(game.listed(), Some((sim, fridge)));
    game.press(at);
    game.release(at);
    game.render();
    let snack = {
        let menu = game.menu().expect("the fridge's pie opens");
        // The entry is one the fridge's check trees listed, not the
        // fallback's, so choosing it goes through `Lot::pick`.
        let entry = menu
            .entries
            .iter()
            .find(|e| e.label == "Have A Snack")
            .expect("Have A Snack listed");
        assert!(entry.action.is_some(), "{entry:?}");
        let (_, r) = menu
            .labels()
            .into_iter()
            .find(|(text, _)| *text == "Have A Snack")
            .expect("Have A Snack on the pie");
        [r.x + r.width / 2, r.y + r.height / 2]
    };
    game.press(snack);
    game.release(snack);
    assert!(game.menu().is_none());

    let mut scripted = Lot::load(&install, &goth_house(&install), &[3], Frames::default()).unwrap();
    for _ in 0..50 {
        assert!(scripted.step_play().unwrap().errors.is_empty());
    }
    let uid = scripted.queue_interaction(sim, fridge, 1).unwrap();
    assert_eq!(uid, Some(1));
    let queue = |lot: &Lot| {
        format!(
            "{:?}",
            lot.world().objects[&sim].person.as_ref().unwrap().queue
        )
    };
    assert_eq!(queue(game.lot()), queue(&scripted));
    // The listing and the entry's check ran the check trees on the
    // window's lot, which left Mortimer's last result and frame storage
    // changed until step 205.
    assert_ne!(game.lot().world().hash(), scripted.world().hash());
    let doing = |lot: &Lot| {
        lot.world().objects[&sim]
            .person
            .as_ref()
            .unwrap()
            .active
            .uid
    };
    for step in 51..=1136 {
        assert_eq!(game.run(1), 1, "step {step}");
        let update = scripted.step_play().unwrap();
        assert!(update.errors.is_empty(), "step {step}: {:?}", update.errors);
        let same = game.lot().world().hash() == scripted.world().hash();
        assert_eq!(same, step >= 205, "step {step}");
        match step {
            150 => assert_eq!(doing(game.lot()), 1),
            // He pays for the snack, whose action is his fifth.
            820 => {
                assert_eq!(game.lot().world().budget.funds, 2537);
                assert_eq!(doing(game.lot()), 5);
            }
            1135 => assert_eq!(doing(game.lot()), 5),
            1136 => assert_ne!(doing(game.lot()), 5),
            _ => {}
        }
    }
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
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
    assert_eq!(snapshot.lot(), i32::from(world.globals[10]));
    assert_eq!(snapshot.lot(), 5);
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

/// Strict mode, through the window: a press on another Goth's portrait
/// selects her (0x4cf9c4, 0x445449), refreshing Mortimer's animator as he
/// is deselected (0x58da12 with 0), and plays `UI_click`. A second press
/// on her portrait centres the view on her (0x434ced). An order from the
/// fridge's pie then goes to her, not to him, and she takes it.
#[test]
fn a_press_on_a_portrait_selects_that_goth_who_then_takes_the_order() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(50), 50);
    game.resize([800, 600]);
    game.render();
    let mortimer = game.lot().world().selected().expect("a Sim is selected");
    let portrait = |game: &Game, sim: u16| {
        let r = game
            .panel()
            .iter()
            .find(|item| item.command == Some(sims_ui::command::Command::Select { sim }))
            .unwrap_or_else(|| panic!("person {sim}'s portrait"))
            .rect;
        [r.x + r.width / 2, r.y + r.height / 2]
    };
    let other = game
        .panel()
        .iter()
        .find_map(|item| match item.command {
            Some(sims_ui::command::Command::Select { sim }) if sim != mortimer => Some(sim),
            _ => None,
        })
        .expect("another Goth's portrait");
    let click = SoundEvent::Play {
        object: -4,
        name: "UI_click".into(),
    };
    game.take_sounds();
    let camera = game.view.camera;
    game.press(portrait(&game, other));
    let world = game.lot().world();
    assert_eq!(world.selected(), Some(other));
    assert_eq!(world.globals[3], other as i16);
    assert!(!world.objects[&mortimer].selected);
    // His animator's clock was read again at the tick count.
    let animator = &world.animators[&mortimer];
    assert_eq!(animator.clock, animator.clock_at(world.ticks as i32));
    assert_eq!(game.take_sounds().last(), Some(&click));
    assert_eq!(
        game.view.camera, camera,
        "a first press does not move the view"
    );
    // The panel and the queue follow her.
    game.render();
    assert_eq!(game.queue().person, Some(other));
    // A second press centres the view on her, a little above her feet,
    // and changes nothing on the lot.
    let hash = game.lot().world().hash();
    game.press(portrait(&game, other));
    assert_eq!(game.lot().world().hash(), hash);
    assert_eq!(game.take_sounds(), [click]);
    let camera = game.view.camera;
    let position = game.lot().world().objects[&other].position;
    assert_eq!(position[2], camera.level);
    let at = sims_game::sound::object_point(&camera, position);
    let rise = [19, 39, 78][camera.zoom.level() as usize - 1];
    assert_eq!([at[0], at[1] - rise], [400, 300]);
    // An order from the fridge's pie goes to her.
    let fridge = 118;
    let before = game.lot().world().objects[&mortimer]
        .person
        .as_ref()
        .unwrap()
        .queue
        .len();
    order(&mut game, fridge, "Have A Snack");
    let queued = |game: &Game, sim: u16| {
        let me = game.lot().world().objects[&sim].person.as_ref().unwrap();
        std::iter::once(&me.active)
            .chain(&me.queue)
            .any(|action| action.target == fridge as i16 && action.caller == sim as i16)
    };
    assert!(queued(&game, other));
    assert_eq!(
        game.lot().world().objects[&mortimer]
            .person
            .as_ref()
            .unwrap()
            .queue
            .len(),
        before
    );
    // She takes it: it becomes her running action, in strict mode.
    let mut taken = None;
    for step in 0..400 {
        assert_eq!(game.run(1), 1, "{:?}", game.stopped());
        let me = game.lot().world().objects[&other].person.as_ref().unwrap();
        if me.active.target == fridge as i16 {
            taken = Some(step);
            break;
        }
    }
    assert!(taken.is_some(), "she never took the order");
    // At tick 455 Mortimer (object 21), now left to himself, routes to
    // a door one of his routes goes through, and the route fails
    // (0x45b176). The salesman (object 259) leaves at tick 510 while his
    // animator still plays a walk; the reset removes that layer at once
    // (0x586b63). The lot runs on past it (checked here to tick 540; it
    // was run to 3540 without a stop). Not compared with a recording.
    while game.lot().ticks() < 509 {
        assert_eq!(game.run(1), 1, "{:?}", game.stopped());
    }
    assert_eq!(mortimer, 21);
    assert!(game.lot().world().animators[&259].foreground.is_some());
    assert_eq!(game.run(1), 1, "{:?}", game.stopped());
    assert!(!game.lot().world().objects.contains_key(&259));
    assert_eq!(game.run(30), 30, "{:?}", game.stopped());
    assert_eq!(game.lot().ticks(), 540);
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

#[test]
fn the_job_panel_shows_what_the_next_level_needs() {
    let Some(install) = install() else {
        return;
    };
    let game = Game::open(&options(install)).expect("the Goth house opens");
    let mut world = game.lot().world().clone();
    let mortimer = world.selected().expect("a Sim is selected");
    let needs = |world: &sims_sim::World| {
        game.snapshot(world)
            .resident(mortimer)
            .expect("Mortimer lives here")
            .needs
    };
    // A Science "Test Subject" moves on to a level that needs nothing.
    assert_eq!(needs(&world), [0; 6]);
    // A "Field Researcher" with two points of Logic: "Science Teacher"
    // needs one of Charisma and three of Logic (work.iff).
    let data = &mut world
        .objects
        .get_mut(&mortimer)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data;
    data[57] = 2;
    data[18] = 200;
    assert_eq!(needs(&world), [0, 0, 100, 0, 300, 0]);
}

/// The Goth house is residential, so with Mortimer selected the House
/// button opens the house panel, silently (0x51bdf2 to 0x51be0c).
#[test]
fn the_house_button_opens_the_house_panel_on_the_goths_lot() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        play: true,
        ..options(install)
    })
    .expect("the Goth house opens");
    assert_eq!(game.run(20), 20);
    assert!(game.lot().residential());
    game.resize([800, 600]);
    game.render();
    assert!(game.lot().world().selected().is_some());
    let house = |game: &Game| {
        game.panel()
            .iter()
            .any(|item| matches!(item.kind, Kind::House { .. }))
    };
    assert!(!house(&game), "the needs show first");
    let button = game
        .panel()
        .iter()
        .find(|item| {
            item.command
                == Some(sims_ui::command::Command::OpenPanel(
                    sims_ui::command::Panel::House,
                ))
        })
        .expect("the House button opens its panel")
        .rect;
    game.take_sounds();
    game.press([button.x + button.width / 2, button.y + button.height / 2]);
    assert!(game.take_sounds().is_empty());
    game.render();
    assert!(house(&game));
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// The House button's press is checked as the lot is at the press, not
/// as the frame drew it (0x51bdf2, 0x51be03). With nobody selected, or on
/// a lot that is not residential, it is refused with `UI_error` for the
/// interface (0x51be1a) and the panel stays as it was. The panel follows
/// the selection at the next frame. A help balloon that closes then
/// whooshes once (0x50fb7f).
#[test]
fn the_house_button_is_checked_at_the_press() {
    use sims_game::hud::{ERROR, Hud, INTERFACE, WHOOSH};
    use sims_game::pace::Pace;
    use sims_ui::balloon::Help;
    use sims_ui::command::{Command, Panel};
    let Some(install) = install() else {
        return;
    };
    let house = goth_house(&install);
    let lot = Lot::load(&install, &house, &[3], Frames::default()).expect("the Goth house boots");
    let opened = open_house(&install, &house).expect("the house opens");
    let mut hud = Hud::open(
        &install,
        &house,
        &opened.assets,
        lot.world(),
        lot.program().autonomy_constants().friendship_threshold,
    )
    .expect("the panel opens");
    let view = View::new([800, 600], [0, 0], opened.rotation);
    let mut canvas = Canvas::new(800, 600, [0; 3]);
    let mut draw = |hud: &mut Hud, world: &sims_sim::World| {
        hud.draw(
            world,
            &opened.assets,
            &view,
            &Pace::default(),
            None,
            &mut canvas,
        );
        hud.take_sounds()
    };
    let selected = lot.world().clone();
    assert!(selected.selected().is_some());
    let mut nobody = selected.clone();
    for object in nobody.objects.values_mut() {
        object.selected = false;
    }
    let sound = |name: &str| SoundEvent::Play {
        object: INTERFACE,
        name: name.into(),
    };

    assert!(draw(&mut hud, &selected).is_empty());
    assert_eq!(hud.controls.panel, Some(Panel::Needs));
    let button = hud
        .items()
        .iter()
        .find(|i| i.command == Some(Command::OpenPanel(Panel::House)))
        .expect("the House button")
        .rect;
    let centre = [button.x + button.width / 2, button.y + button.height / 2];
    // Drawn with Mortimer selected; nobody is by the press.
    let refused = Some(Command::Refused { error: true });
    assert_eq!(hud.press(centre, &nobody, true), refused);
    assert_eq!(hud.take_sounds(), [sound(ERROR)]);
    assert_eq!(hud.controls.panel, Some(Panel::Needs));
    // Selected, on a lot that is not residential.
    assert_eq!(hud.press(centre, &selected, false), refused);
    assert_eq!(hud.take_sounds(), [sound(ERROR)]);
    assert_eq!(hud.controls.panel, Some(Panel::Needs));

    // A balloon open when the selection goes closes at the next frame,
    // with one whoosh, and no panel is open then.
    hud.controls.panel = Some(Panel::Personality);
    hud.controls.help = Some(Help::Sign(1));
    assert_eq!(draw(&mut hud, &nobody), [sound(WHOOSH)]);
    assert_eq!((hud.controls.panel, hud.controls.help), (None, None));
    assert!(draw(&mut hud, &nobody).is_empty());
    // Drawn with nobody selected; Mortimer is by the press, on the
    // residential Goth lot, so the house panel opens, silently.
    assert!(lot.residential());
    assert_eq!(
        hud.press(centre, &selected, lot.residential()),
        Some(Command::OpenPanel(Panel::House))
    );
    assert!(hud.take_sounds().is_empty());
    assert_eq!(hud.controls.panel, Some(Panel::House));
    assert!(draw(&mut hud, &selected).is_empty());
    assert!(
        hud.items()
            .iter()
            .any(|i| matches!(i.kind, Kind::House { .. }))
    );
}

/// The live panel's Personality button shows the selected Sim's
/// personality as the world holds it (person data words 7, 6, 3, 5 and
/// 2: Neat, Outgoing, Active, Playful, Nice), and follows a press on
/// another Goth's portrait.
#[test]
fn the_personality_panel_shows_the_selected_goths_traits() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        play: true,
        ..options(install)
    })
    .expect("the Goth house opens");
    assert_eq!(game.run(20), 20);
    game.resize([800, 600]);
    game.render();
    let centre = |r: Rect| [r.x + r.width / 2, r.y + r.height / 2];
    let button = game
        .panel()
        .iter()
        .find(|item| {
            item.command
                == Some(sims_ui::command::Command::OpenPanel(
                    sims_ui::command::Panel::Personality,
                ))
        })
        .expect("the Personality button")
        .rect;
    game.press(centre(button));
    let shown = |game: &mut Game| {
        game.render();
        let shown: Vec<[i16; 5]> = game
            .panel()
            .iter()
            .filter_map(|item| match item.kind {
                Kind::Personality { traits, .. } => Some(traits),
                _ => None,
            })
            .collect();
        assert_eq!(shown.len(), 1, "one personality panel");
        shown[0]
    };
    let traits = |game: &Game, sim: u16| {
        let data = &game.lot().world().objects[&sim]
            .person
            .as_ref()
            .unwrap()
            .data;
        [7, 6, 3, 5, 2].map(|word| data[word])
    };
    let mortimer = game.lot().world().selected().expect("a Sim is selected");
    assert_eq!(shown(&mut game), traits(&game, mortimer));
    // Another Goth's portrait: the panel stays open and shows hers.
    let (other, portrait) = game
        .panel()
        .iter()
        .find_map(|item| match item.command {
            Some(sims_ui::command::Command::Select { sim }) if sim != mortimer => {
                Some((sim, item.rect))
            }
            _ => None,
        })
        .expect("another Goth's portrait");
    game.press(centre(portrait));
    assert_eq!(game.lot().world().selected(), Some(other));
    let hers = shown(&mut game);
    assert_eq!(hers, traits(&game, other));
    assert_ne!(hers, traits(&game, mortimer), "the two Goths differ");
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// A press on Mortimer's sign of the zodiac opens its help balloon over
/// the lot, with the whoosh the original plays, and a second press
/// closes it with another (0x51bfb3, 0x50fb3f, 0x50fbc5).
#[test]
fn the_sign_opens_and_closes_its_help_balloon() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        play: true,
        ..options(install)
    })
    .expect("the Goth house opens");
    assert_eq!(game.run(20), 20);
    game.resize([800, 600]);
    game.render();
    let centre = |r: Rect| [r.x + r.width / 2, r.y + r.height / 2];
    let find = |game: &Game, wanted: fn(&Kind) -> bool| {
        game.panel()
            .iter()
            .find(|item| wanted(&item.kind))
            .map(|item| item.rect)
    };
    let button = game
        .panel()
        .iter()
        .find(|item| {
            item.command
                == Some(sims_ui::command::Command::OpenPanel(
                    sims_ui::command::Panel::Personality,
                ))
        })
        .expect("the Personality button")
        .rect;
    game.press(centre(button));
    game.render();
    game.take_sounds();
    let sign = find(&game, |k| matches!(k, Kind::Sign { .. })).expect("the sign");
    assert!(sign.width > 0, "the sign is sized to its name");
    let whoosh = SoundEvent::Play {
        object: -4,
        name: "UI_whoosh".into(),
    };
    let whooshes = |game: &mut Game| game.take_sounds().iter().filter(|s| **s == whoosh).count();
    game.press(centre(sign));
    assert_eq!(whooshes(&mut game), 1);
    game.render();
    // The press is carried out: no status band says otherwise.
    assert!(!game.over_panel([400, 2]), "a status band shows");
    assert_eq!(
        find(&game, |k| matches!(k, Kind::Balloon(_))),
        Some(Rect::new(241, 373, 559, 127))
    );
    // The balloon takes a press over the lot, and opens no menu.
    assert!(sims_ui::hud::covers(game.panel(), [300, 400]));
    game.press([300, 400]);
    assert!(game.menu().is_none());
    game.press(centre(sign));
    assert_eq!(whooshes(&mut game), 1);
    game.render();
    assert!(!game.over_panel([400, 2]), "a status band shows");
    assert_eq!(find(&game, |k| matches!(k, Kind::Balloon(_))), None);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// The live panel's Needs meters are the selected Sim's motives as the
/// world holds them, in the panel's order (Hunger, Energy, Comfort, Fun,
/// Hygiene, Social, Bladder, Room: motives 7, 5, 6, 15, 8, 14, 9 and 13).
/// They follow the ticks and a press on another Goth's portrait.
#[test]
fn the_needs_panel_follows_the_selected_goths_motives() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        play: true,
        ..options(install)
    })
    .expect("the Goth house opens");
    assert_eq!(game.run(20), 20);
    game.resize([800, 600]);
    let shown = |game: &mut Game| {
        game.render();
        game.panel()
            .iter()
            .filter_map(|item| match item.kind {
                Kind::Meter(value) => Some(value),
                _ => None,
            })
            .collect::<Vec<f32>>()
    };
    let motives = |game: &Game, sim: u16| {
        let motives = &game.lot().world().objects[&sim]
            .person
            .as_ref()
            .unwrap()
            .motives;
        [7, 5, 6, 15, 8, 14, 9, 13]
            .map(|motive| motives[motive])
            .to_vec()
    };
    let mortimer = game.lot().world().selected().expect("a Sim is selected");
    let before = shown(&mut game);
    assert_eq!(before, motives(&game, mortimer));
    assert_eq!(game.run(200), 200);
    let after = shown(&mut game);
    assert_eq!(after, motives(&game, mortimer));
    assert_ne!(after, before, "his needs moved in 200 ticks");
    let (other, portrait) = game
        .panel()
        .iter()
        .find_map(|item| match item.command {
            Some(sims_ui::command::Command::Select { sim }) if sim != mortimer => {
                Some((sim, item.rect))
            }
            _ => None,
        })
        .expect("another Goth's portrait");
    game.press([
        portrait.x + portrait.width / 2,
        portrait.y + portrait.height / 2,
    ]);
    assert_eq!(game.lot().world().selected(), Some(other));
    let hers = shown(&mut game);
    assert_eq!(hers, motives(&game, other));
    assert_ne!(hers, motives(&game, mortimer), "the two Goths differ");
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// The live panel's Job button shows the selected Sim's skills as the
/// world holds them (person data words 10, 12, 11, 17, 18 and 15:
/// Cooking, Mechanical, Charisma, Body, Logic, Creativity) and what the
/// next level of their career needs, and follows a press on another
/// Sim's portrait. The Goths have no skill points yet, so this is
/// checked in house 7, whose two Sims have different skills (Cooking 3
/// and Mechanical 2 against none);
/// `the_job_panel_shows_what_the_next_level_needs` checks the needs.
#[test]
fn the_job_panel_shows_the_selected_sims_skills() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        play: true,
        house: install.join("UserData/Houses/House07.iff"),
        ..options(install)
    })
    .expect("house 7 opens");
    assert_eq!(game.stopped(), None, "the load completes");
    assert_eq!(game.run(20), 20);
    game.resize([800, 600]);
    game.render();
    let centre = |r: Rect| [r.x + r.width / 2, r.y + r.height / 2];
    let button = game
        .panel()
        .iter()
        .find(|item| {
            item.command
                == Some(sims_ui::command::Command::OpenPanel(
                    sims_ui::command::Panel::Job,
                ))
        })
        .expect("the Job button")
        .rect;
    game.press(centre(button));
    let shown = |game: &mut Game| {
        game.render();
        let shown: Vec<([i16; 6], [i32; 6])> = game
            .panel()
            .iter()
            .filter_map(|item| match item.kind {
                Kind::Skills { skills, needs } => Some((skills, needs)),
                _ => None,
            })
            .collect();
        assert_eq!(shown.len(), 1, "one job panel");
        shown[0]
    };
    let held = |game: &Game, sim: u16| {
        let world = game.lot().world();
        let data = &world.objects[&sim].person.as_ref().unwrap().data;
        let needs = game
            .snapshot(world)
            .resident(sim)
            .expect("a Goth lives here")
            .needs;
        ([10, 12, 11, 17, 18, 15].map(|word| data[word]), needs)
    };
    let first = game.lot().world().selected().expect("a Sim is selected");
    let theirs = shown(&mut game);
    assert_eq!(theirs, held(&game, first));
    let (other, portrait) = game
        .panel()
        .iter()
        .find_map(|item| match item.command {
            Some(sims_ui::command::Command::Select { sim }) if sim != first => {
                Some((sim, item.rect))
            }
            _ => None,
        })
        .expect("the other Sim's portrait");
    game.press(centre(portrait));
    assert_eq!(game.lot().world().selected(), Some(other));
    let others = shown(&mut game);
    assert_eq!(others, held(&game, other));
    assert_ne!(others.0, theirs.0, "the two Sims' skills differ");
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// The live panel's Relationships button lists the people the selected
/// Sim knows, from the neighborhood records the booted lot holds: the
/// Goths know only each other, the married ones with the pink heart, and
/// each entry's numbers are the selected Sim's row toward that person
/// (values 0 and 2). The list follows a press on another Goth's
/// portrait.
#[test]
fn the_relationship_panel_lists_who_the_selected_goth_knows() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        play: true,
        ..options(install)
    })
    .expect("the Goth house opens");
    assert_eq!(game.run(20), 20);
    game.resize([800, 600]);
    game.render();
    let centre = |r: Rect| [r.x + r.width / 2, r.y + r.height / 2];
    let button = game
        .panel()
        .iter()
        .find(|item| {
            item.command
                == Some(sims_ui::command::Command::OpenPanel(
                    sims_ui::command::Panel::Relationships,
                ))
        })
        .expect("the Relationships button")
        .rect;
    game.press(centre(button));
    let shown = |game: &mut Game| {
        game.render();
        game.panel()
            .iter()
            .filter_map(|item| match &item.kind {
                Kind::Relation { relation, .. } => Some(relation.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // Each entry's numbers are the selected Sim's row toward the
    // person's record, as the world holds it.
    let check = |game: &Game, sim: u16, shown: &[sims_ui::household::Relation]| {
        let world = game.lot().world();
        let neighbors = &world.neighbors;
        let viewer = neighbors
            .first_of(world.objects[&sim].guid)
            .expect("the Sim's own record");
        for relation in shown {
            let target = neighbors.first_of(relation.guid).expect("a record");
            let row = viewer.row(i32::from(target.id)).expect("a row");
            assert_eq!(relation.daily, row.values[0].clamp(-100, 100));
            assert_eq!(relation.lifetime, row.values[2].clamp(-100, 100));
            assert_eq!(relation.pink_heart, row.values[1] != 0);
        }
    };
    // Who is listed, in order: the type's GUID, the record, the daily
    // and lifetime relationships, then the smiley, the pink and red
    // hearts and the family mark.
    let listed = |shown: &[sims_ui::household::Relation]| {
        shown
            .iter()
            .map(|r| {
                (
                    r.guid,
                    r.id,
                    r.daily,
                    r.lifetime,
                    [r.friends, r.pink_heart, r.red_heart, r.family],
                )
            })
            .collect::<Vec<_>>()
    };
    let (mortimer, bella, cassandra) = (0xc07f_6184, 0xc0c6_298e, 0xc120_7913);
    let selected = game.lot().world().selected().expect("a Sim is selected");
    assert_eq!(game.lot().world().objects[&selected].guid, mortimer);
    let his = shown(&mut game);
    check(&game, selected, &his);
    // His family only, his wife first with the pink heart.
    assert_eq!(
        listed(&his),
        [
            (bella, 33, 100, 100, [true, true, false, true]),
            (cassandra, 32, 75, 75, [true, false, false, true]),
        ]
    );
    let (other, portrait) = game
        .panel()
        .iter()
        .find_map(|item| match item.command {
            Some(sims_ui::command::Command::Select { sim }) if sim != selected => {
                Some((sim, item.rect))
            }
            _ => None,
        })
        .expect("another Goth's portrait");
    assert_eq!(game.lot().world().objects[&other].guid, bella);
    game.press(centre(portrait));
    assert_eq!(game.lot().world().selected(), Some(other));
    let hers = shown(&mut game);
    check(&game, other, &hers);
    assert_eq!(
        listed(&hers),
        [
            (mortimer, 34, 100, 100, [true, true, false, true]),
            (cassandra, 32, 85, 85, [true, false, false, true]),
        ]
    );
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// A press on the selected Sim's portrait centres the view on him even
/// after the view has been scrolled further than 16 bits of screen hold:
/// where the drawing puts him then is the middle of the screen, a little
/// above his feet.
#[test]
fn a_press_on_the_selected_goths_portrait_centres_him_after_a_long_scroll() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(50), 50);
    game.resize([800, 600]);
    game.render();
    let mortimer = game.lot().world().selected().expect("a Sim is selected");
    let portrait = game
        .panel()
        .iter()
        .find(|item| item.command == Some(sims_ui::command::Command::Select { sim: mortimer }))
        .expect("his portrait")
        .rect;
    let press = [
        portrait.x + portrait.width / 2,
        portrait.y + portrait.height / 2,
    ];
    for scroll in [[40_000, 0], [-40_000, 50_000], [0, -60_000]] {
        let origin = game.view.camera.origin;
        game.view.apply(sims_game::view::Move::Scroll(scroll));
        assert!(
            game.view
                .camera
                .origin
                .iter()
                .zip(origin)
                .any(|(n, o)| (n - o).abs() > 1 << 15),
            "{:?}",
            game.view.camera.origin
        );
        game.press(press);
        let camera = game.view.camera;
        let position = game.lot().world().objects[&mortimer].position;
        let drawn = camera.project_fine(
            camera.rotate_fine([position[0], position[1]]),
            position[2],
            0,
        );
        let rise = [19, 39, 78][camera.zoom.level() as usize - 1];
        assert!(
            (drawn[0] - 400).abs() <= 1 && (drawn[1] - rise - 300).abs() <= 1,
            "{scroll:?}: drawn at {drawn:?}"
        );
    }
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Track 2's recording of the original (the goth-snack scenario) ordered
/// Mortimer's snack with the pointer: it reached the fridge before update
/// 124, pressed before 154, reached "Have A Snack" on the pie before 176
/// and chose it before 202. The window, given the same input between the
/// same ticks in strict mode, and drawing the house view's frames when
/// the recording shows the original drew them, matched the recording's
/// full state after every update from 1 to 1000 but for a few things
/// (see docs/decisions/). This pins what the recording showed the input
/// doing: Mortimer's last result after each step, and what the fridge's
/// check trees left in his frames when the snack starts. Then what the
/// frames did: when Bella's Room motive took her room's new score, and
/// when the queue's listing cleared Mortimer's next action's flag 0x80.
#[test]
fn the_recorded_snack_order_replayed_in_the_window_matches_the_recording() {
    let Some(install) = install() else {
        return;
    };
    // The frames after these ticks, as the recording shows them: one
    // after every 30th update from 171, where the captures without input
    // have one after every 30th from 33. Which frames the original drew
    // before update 171 is not shown; these are the default's.
    let frames = Frames::parse("0,1,2,3,33,63,93,123,153,171+30").expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    let fridge = 118;
    let mortimer = game.lot().world().selected().expect("a Sim is selected");
    assert_eq!(mortimer, 21);
    let last = |game: &Game| game.lot().world().objects[&mortimer].last_result;
    let run_to = |game: &mut Game, tick: u32| run_to(game, &schedule, tick);
    run_to(&mut game, 123);
    let at = (0..600)
        .step_by(2)
        .flat_map(|y| (0..800).step_by(2).map(move |x| [x, y]))
        .find(|&p| !game.over_panel(p) && game.under(p) == Some(fridge))
        .expect("the fridge is on screen");
    game.hover(at);
    assert_eq!(game.listed(), Some((mortimer, fridge)));
    run_to(&mut game, 124);
    assert!(!last(&game), "the listing's last check failed");
    run_to(&mut game, 153);
    assert!(last(&game));
    // The press lists the menu again, whose last check fails again.
    game.press(at);
    game.release(at);
    game.render();
    run_to(&mut game, 154);
    assert!(!last(&game), "the press listed the menu again");
    run_to(&mut game, 175);
    let entry = {
        let menu = game.menu().expect("the fridge's pie is open");
        let (_, r) = menu
            .labels()
            .into_iter()
            .find(|(text, _)| *text == "Have A Snack")
            .expect("Have A Snack on the pie");
        [r.x + r.width / 2, r.y + r.height / 2]
    };
    // Reaching the entry checks its action again, which passes.
    game.hover(entry);
    run_to(&mut game, 176);
    assert!(last(&game), "reaching the entry checked it");
    run_to(&mut game, 201);
    game.press(entry);
    game.release(entry);
    assert!(game.menu().is_none());
    let queue = &game.lot().world().objects[&mortimer]
        .person
        .as_ref()
        .unwrap()
        .queue;
    assert_eq!(queue.len(), 1);
    let action = &queue[0];
    assert_eq!(
        (
            action.target,
            action.table_index,
            action.action,
            action.priority,
            action.flags
        ),
        (118, 1, 4103, 50, 12)
    );
    run_to(&mut game, 202);
    // The snack's frames hold what the checks left in his stack: the
    // last check tree run there was Have A Snack's (0x1008), when the
    // pointer reached its entry and again when it was chosen, not the
    // listing's last (0x1030).
    let stack = &game.lot().world().objects[&mortimer].stack;
    let locals = |tree: u16| {
        stack
            .iter()
            .find(|f| f.tree == tree)
            .map(|f| f.locals.clone())
            .unwrap_or_else(|| panic!("a frame of tree {tree:#06x}"))
    };
    assert_eq!(locals(4120), [4104, 254]);
    assert_eq!(locals(4142), [363]);
    // Bella's (person 18) Room motive takes her room's score each tick,
    // and the room is counted again only when a frame is drawn: the
    // daylight changed in update 453, the frame after 471 counted it,
    // and her motive took it in update 472, as in the recording.
    let room = |game: &Game| {
        game.lot().world().objects[&18]
            .person
            .as_ref()
            .unwrap()
            .motives[13]
            .to_bits()
    };
    run_to(&mut game, 471);
    assert_eq!(room(&game), 0x4225_ac1d);
    run_to(&mut game, 472);
    assert_eq!(room(&game), 0x4277_073c);
    // Mortimer chooses his next action in update 872; it is named, with
    // flag 0x80, until the frame after 891 lists the queue. The recording
    // shows it cleared after update 892, as it saves an update's state
    // before the frame after it; this step's state is after its frame.
    let running = |game: &Game| {
        let active = &game.lot().world().objects[&mortimer]
            .person
            .as_ref()
            .unwrap()
            .active;
        (active.target, active.action, active.flags)
    };
    run_to(&mut game, 890);
    assert_eq!(running(&game), (263, 4170, 0x9e));
    run_to(&mut game, 891);
    assert_eq!(running(&game), (263, 4170, 0x1e));
    run_to(&mut game, 1000);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Where a point of track 2's recordings of the Goth house is in the
/// window at 800 by 600: the original's view shows the lot 97 pixels
/// left of and 46 above ours (the screenshots correlate to the pixel,
/// people too; see docs/decisions/). The panel is not moved.
fn recorded([x, y]: [i32; 2]) -> [i32; 2] {
    [x + 97, y + 46]
}

/// The middle of `label` on the open pie menu.
fn pie_entry(game: &Game, label: &str) -> [i32; 2] {
    let menu = game.menu().expect("a pie menu is open");
    let (_, r) = menu
        .labels()
        .into_iter()
        .find(|(text, _)| *text == label)
        .unwrap_or_else(|| panic!("{label} on the pie"));
    [r.x + r.width / 2, r.y + r.height / 2]
}

/// Runs the lot to `tick`, drawing the house view after the updates
/// `frames` draws a frame after, as the original draws its view: the
/// pointer then picks from the frame the original's picked from (the
/// window draws more often). The lot must not be past `tick` already.
fn run_to(game: &mut Game, frames: &Frames, tick: u32) {
    assert!(game.lot().ticks() <= tick, "already past {tick}");
    while game.lot().ticks() < tick {
        assert_eq!(game.run(1), 1, "{:?}", game.stopped());
        if frames.after(game.lot().ticks()) {
            game.render();
        }
    }
}

/// A person's Room motive (13), as bits.
fn room(game: &Game, id: u16) -> u32 {
    game.lot().world().objects[&id]
        .person
        .as_ref()
        .unwrap()
        .motives[13]
        .to_bits()
}

/// (uid, target, routine, table index, priority, flags) of an action.
fn action(a: &sims_formats::saved_objects::Interaction) -> (i32, i16, u16, i32, i32, u32) {
    (
        a.uid,
        a.target,
        a.action,
        a.table_index,
        a.priority,
        a.flags,
    )
}

/// Track 2's recording of the original ordering Mortimer onto the porch
/// bench (goth-sit-read-order-20261115), replayed in the window: each
/// pointer move and press is given at the recording's point once the lot
/// has run the update before its command ended, as its effect shows in
/// the recording's state of the update the command ended. With frames after every 30th update from 170 (the recording's
/// Room motives show them after 470 and 620), the lot equals the
/// recording's after every update to 260 but for Bella's local from 203
/// to 227, as in the snack order. From 261 on, the original lists pies
/// under the still pointer that ours does not, and its pointer found the
/// bookcase at 590 where ours finds the salesman (see docs/decisions/).
#[test]
fn the_recorded_sit_order_replayed_in_the_window_matches_the_recording() {
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse("0,1,2,3,33,63,93,123,153,170+30").expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    let (mortimer, bench, salesman) = (21, 173, 259);
    assert_eq!(game.lot().world().selected(), Some(mortimer));
    for (tick, at) in [(91, [432, 254]), (117, [432, 242])] {
        run_to(&mut game, &schedule, tick);
        game.hover(recorded(at));
        assert_eq!(game.listed(), None, "{tick}");
    }
    run_to(&mut game, &schedule, 123);
    game.hover(recorded([432, 230]));
    assert_eq!(game.listed(), Some((mortimer, bench)));
    run_to(&mut game, &schedule, 153);
    game.press(recorded([432, 230]));
    game.release(recorded([432, 230]));
    game.render();
    let labels: Vec<String> = game
        .menu()
        .expect("the bench's pie")
        .labels()
        .into_iter()
        .map(|(text, _)| text.to_string())
        .collect();
    assert_eq!(labels, ["Sit"]);
    run_to(&mut game, &schedule, 173);
    let sit = pie_entry(&game, "Sit");
    game.hover(sit);
    run_to(&mut game, &schedule, 200);
    game.press(sit);
    game.release(sit);
    assert!(game.menu().is_none());
    // The recording pushed it before update 201 and had it running, with
    // an empty queue, at 203.
    run_to(&mut game, &schedule, 201);
    let person = game.lot().world().objects[&mortimer]
        .person
        .clone()
        .unwrap();
    assert_eq!(action(&person.active), (1, bench as i16, 8213, 0, 50, 12));
    assert!(person.queue.is_empty());
    // Bella's (18) room is counted again in the frame after 470, and the
    // others' after 620.
    run_to(&mut game, &schedule, 470);
    assert_eq!(room(&game, 18), 0x4225_ac1d);
    run_to(&mut game, &schedule, 471);
    assert_eq!(room(&game, 18), 0x4277_073c);
    run_to(&mut game, &schedule, 620);
    for id in [16, mortimer, salesman] {
        assert_eq!(room(&game, id), 0x429a_aaab, "{id}");
    }
    run_to(&mut game, &schedule, 621);
    for id in [16, mortimer, salesman] {
        assert_eq!(room(&game, id), 0x4232_aaab, "{id}");
    }
    run_to(&mut game, &schedule, 800);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Track 2's recording of the original ordering two snacks and
/// cancelling both (goth-cancel-order-20261116), replayed in the window
/// as the Sit order is. The second snack waits behind the first; a press
/// on its icon takes it off, and one on the running snack's icon cancels
/// it, which queues his own action 8318 and sets flag 0x100 on the
/// snack. The first cancel shows in update 505's state, the update its
/// release's command ended. The second shows in update 595's state,
/// which was saved before its release was sent, so which of its button
/// events acts is not known. The replay gives each press once the lot
/// has run the update before the one its effect shows in. With the frames
/// its Room motives show (after 316, 474, 648 and 1124), and the house
/// view drawn only after those updates, the lot equals the recording's
/// after every update to 1300 but for one listing ours does not make,
/// under the still pointer at 256 (see docs/decisions/).
#[test]
fn the_recorded_cancel_order_replayed_in_the_window_matches_the_recording() {
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse("0,1,2,3,33,63,93,123,153,171,201,231,261,291,316,474,648,1124")
        .expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    let (mortimer, fridge, bookcase, salesman) = (21, 118, 132, 259);
    assert_eq!(game.lot().world().selected(), Some(mortimer));
    let person = |game: &Game| {
        game.lot().world().objects[&mortimer]
            .person
            .clone()
            .unwrap()
    };
    let snack = |uid| (uid, fridge as i16, 4103, 1, 50, 12);
    let order = |game: &mut Game, hovers: &[(u32, [i32; 2])], press: u32, entry: (u32, u32)| {
        for &(tick, at) in hovers {
            run_to(game, &schedule, tick);
            game.hover(recorded(at));
        }
        assert_eq!(game.listed(), Some((mortimer, fridge)));
        run_to(game, &schedule, press);
        let on_fridge = recorded(hovers.last().unwrap().1);
        game.press(on_fridge);
        game.release(on_fridge);
        game.render();
        run_to(game, &schedule, entry.0);
        let at = pie_entry(game, "Have A Snack");
        game.hover(at);
        run_to(game, &schedule, entry.1);
        game.press(at);
        game.release(at);
        assert!(game.menu().is_none());
    };
    // The pointer reaches the kitchen counter's spot at 90 with the
    // kitchen wall shown in front of it: nothing is listed until the frame
    // after 93, which cuts the wall away and shows the counter (119).
    run_to(&mut game, &schedule, 90);
    game.hover(recorded([176, 222]));
    assert_eq!(game.listed(), None);
    run_to(&mut game, &schedule, 92);
    assert_eq!(game.listed(), None);
    run_to(&mut game, &schedule, 93);
    assert_eq!(game.listed(), Some((mortimer, 119)));
    order(
        &mut game,
        &[(123, [176, 210]), (153, [176, 198])],
        183,
        (201, 256),
    );
    // Queued behind his reading (the bookcase's 4110), as recorded at
    // 257.
    run_to(&mut game, &schedule, 257);
    let p = person(&game);
    assert_eq!(action(&p.active), (1, bookcase as i16, 4110, 6, 2, 0));
    assert_eq!(p.queue.iter().map(action).collect::<Vec<_>>(), [snack(2)]);
    // Choosing the entry checked it again (0x5eb433), which leaves his
    // word 50 at 0 and his last result 1, as recorded.
    let him = &game.lot().world().objects[&mortimer];
    assert_eq!((him.data[50], him.last_result), (0, true));
    // The second order's first two pointer moves; his room is counted
    // again in the frame after 316, the update the second one reached.
    run_to(&mut game, &schedule, 310);
    game.hover(recorded([176, 222]));
    run_to(&mut game, &schedule, 316);
    assert_eq!(room(&game, mortimer), 0x4225_ac1d);
    game.hover(recorded([176, 210]));
    run_to(&mut game, &schedule, 317);
    assert_eq!(room(&game, mortimer), 0x4277_073c);
    order(
        &mut game,
        &[(341, [176, 198]), (346, [176, 186])],
        376,
        (396, 444),
    );
    run_to(&mut game, &schedule, 445);
    let p = person(&game);
    assert_eq!(action(&p.active), snack(2));
    assert_eq!(p.queue.iter().map(action).collect::<Vec<_>>(), [snack(5)]);
    run_to(&mut game, &schedule, 474);
    assert_eq!(room(&game, mortimer), 0x415c_71c8);
    game.hover([104, 54]);
    run_to(&mut game, &schedule, 475);
    assert_eq!(room(&game, mortimer), 0x4253_a203);
    // The queued snack's icon, second in the queue.
    run_to(&mut game, &schedule, 504);
    assert_eq!(
        person(&game).queue.iter().map(action).collect::<Vec<_>>(),
        [snack(5)]
    );
    game.press([104, 54]);
    run_to(&mut game, &schedule, 505);
    let p = person(&game);
    assert_eq!(action(&p.active), snack(2));
    assert!(p.queue.is_empty());
    // The running snack's icon.
    run_to(&mut game, &schedule, 564);
    game.hover([54, 54]);
    run_to(&mut game, &schedule, 594);
    game.press([54, 54]);
    run_to(&mut game, &schedule, 595);
    let p = person(&game);
    assert_eq!(action(&p.active), (2, fridge as i16, 4103, 1, 50, 0x10c));
    assert_eq!(
        p.queue.iter().map(action).collect::<Vec<_>>(),
        [(7, mortimer as i16, 8318, 33, 50, 0)]
    );
    assert_eq!(p.data[33], 0);
    // The salesman is taken off the lot in update 597, his door knock
    // still playing, as in the recording.
    run_to(&mut game, &schedule, 596);
    assert!(game.lot().world().objects.contains_key(&salesman));
    run_to(&mut game, &schedule, 597);
    assert!(!game.lot().world().objects.contains_key(&salesman));
    run_to(&mut game, &schedule, 648);
    assert_eq!(room(&game, 16), 0x4277_073c);
    run_to(&mut game, &schedule, 649);
    assert_eq!(room(&game, 16), 0x422c_0000);
    run_to(&mut game, &schedule, 1124);
    assert_eq!(room(&game, mortimer), 0x4253_a203);
    run_to(&mut game, &schedule, 1125);
    assert_eq!(room(&game, mortimer), 0x4225_ac1d);
    run_to(&mut game, &schedule, 1300);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Track 2's recording of the original ordering Mortimer to talk to Bella
/// about interests (goth-talk-order-20261118), replayed in the window as
/// the Sit order is: a press on Bella opens her pie, Talk... opens its
/// submenu, and About Interests queues the talk. The original found Bella
/// under the pointer at the recording's (400, 274), three rows below the
/// lowest pixel our frame draws her at; while ours finds nothing there,
/// the replay points at that pixel (`docs/decisions/`). With the frames
/// its Room motives show (after every 30th update from 190), the lot
/// equals the recording's after every update to 1000, in every word our
/// state does not mark as unknown, but for the listings the original most
/// likely made under the still pointer, which ours does not make.
#[test]
fn the_recorded_talk_order_replayed_in_the_window_matches_the_recording() {
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse("0,1,2,3,33,63,93,123,153,190+30").expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    let (alexander, bella, mortimer, social, salesman) = (16, 18, 21, 260, 259);
    assert_eq!(game.lot().world().selected(), Some(mortimer));
    let labels = |game: &Game| -> Vec<String> {
        game.menu()
            .expect("a pie menu is open")
            .labels()
            .into_iter()
            .map(|(text, _)| text.to_string())
            .collect()
    };
    run_to(&mut game, &schedule, 85);
    game.hover(recorded([400, 286]));
    assert_eq!(game.listed(), None);
    // The original listed Bella's pie with the pointer at the recording's
    // (400, 274): Mortimer's last result goes from 1 to 0 at 94. Ours
    // finds nothing at that point; then the replay points at her lowest
    // drawn pixel.
    run_to(&mut game, &schedule, 93);
    let last_result = |game: &Game| game.lot().world().objects[&mortimer].last_result;
    assert!(last_result(&game));
    let on_bella = match game.under(recorded([400, 274])) {
        Some(id) if id == bella => recorded([400, 274]),
        _ => [499, 317],
    };
    game.hover(on_bella);
    assert_eq!(game.listed(), Some((mortimer, bella)));
    assert!(!last_result(&game));
    run_to(&mut game, &schedule, 94);
    assert!(!last_result(&game));
    run_to(&mut game, &schedule, 123);
    game.press(on_bella);
    game.release(on_bella);
    game.render();
    let pie = labels(&game);
    run_to(&mut game, &schedule, 138);
    let talk = pie_entry(&game, "Talk...");
    game.hover(talk);
    run_to(&mut game, &schedule, 176);
    game.press(talk);
    game.release(talk);
    game.render();
    let submenu = labels(&game);
    // The recording's screenshot at 177 shows the same six entries; the
    // harness chose the submenu's first entry.
    assert_eq!(
        pie,
        [
            "Dance...",
            "Hug...",
            "Kiss...",
            "Invite Downtown",
            "Talk...",
            "Invite to Old Town"
        ]
    );
    assert_eq!(submenu[0], "About Interests");
    run_to(&mut game, &schedule, 206);
    let interests = pie_entry(&game, "About Interests");
    game.hover(interests);
    run_to(&mut game, &schedule, 250);
    game.press(interests);
    game.release(interests);
    assert!(game.menu().is_none());
    game.hover(recorded([400, 273]));
    // Queued behind his reading (the bookcase's 4110) through the social
    // object, with Bella's answer running, as recorded at 251; running at
    // 252.
    run_to(&mut game, &schedule, 251);
    let person = |game: &Game, id: u16| game.lot().world().objects[&id].person.clone().unwrap();
    let talking = |uid, flags| (uid, social as i16, 4235, 21, 50, flags);
    let p = person(&game, mortimer);
    assert_eq!(action(&p.active), (1, 132, 4110, 6, 2, 0));
    assert_eq!(
        p.queue.iter().map(action).collect::<Vec<_>>(),
        [talking(2, 0)]
    );
    let p = person(&game, bella);
    assert_eq!(action(&p.active), (3, social as i16, 4236, 50, 50, 12));
    assert!(p.queue.is_empty());
    run_to(&mut game, &schedule, 252);
    let p = person(&game, mortimer);
    assert_eq!(action(&p.active), talking(2, 12));
    assert!(p.queue.is_empty());
    // Everyone's room is counted again in the frame after 520. There the
    // original's still pointer also lists a pie, most likely the
    // salesman's (unverified; Mortimer's word 50 goes to 1 at 521); ours
    // lists nothing.
    run_to(&mut game, &schedule, 520);
    for id in [alexander, bella, mortimer, salesman] {
        assert_eq!(room(&game, id), 0x429a_aaab, "{id}");
    }
    run_to(&mut game, &schedule, 521);
    for id in [alexander, bella, mortimer, salesman] {
        assert_eq!(room(&game, id), 0x4234_0000, "{id}");
    }
    run_to(&mut game, &schedule, 1000);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Track 2's Read A Book recording (`goth-read-order-20261118`) replayed
/// through the window's input layer: a press on the bookcase, then Read a
/// Book. Each input is given at the update before the recording's
/// command ended, except the press on the bookcase (below); frames are
/// drawn after the updates its Room motives show (every 30th from 186).
/// A scratch comparison found the lot equal to the recording's after
/// every update from 1 to 900 except for what
/// `docs/decisions/20261120-read-order-replayed-in-the-window.md` lists;
/// this test checks the states below.
#[test]
fn the_recorded_read_order_replayed_in_the_window_matches_the_recording() {
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse("0,1,2,3,33,63,93,123,153,186+30").expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    let (alexander, bella, mortimer, bookcase, salesman) = (16, 18, 21, 132, 259);
    assert_eq!(game.lot().world().selected(), Some(mortimer));
    let last_result = |game: &Game| game.lot().world().objects[&mortimer].last_result;
    run_to(&mut game, &schedule, 87);
    game.hover(recorded([384, 262]));
    assert_eq!(game.listed(), None);
    // The original listed the bookcase's pie with the pointer at the
    // recording's (384, 250), a wall pixel in our frame, where our pick
    // finds the door. While it does, the replay points 7 columns right on
    // the same row, where our pick finds the bookcase (the recording's
    // harness did not try that point).
    run_to(&mut game, &schedule, 93);
    let on_bookcase = match game.under(recorded([384, 250])) {
        Some(id) if id == bookcase => recorded([384, 250]),
        _ => [488, 296],
    };
    assert_eq!(game.under(on_bookcase), Some(bookcase));
    // His last result is 1 before and after, as recorded from 84 to 186.
    assert!(last_result(&game));
    game.hover(on_bookcase);
    assert_eq!(game.listed(), Some((mortimer, bookcase)));
    assert!(last_result(&game));
    // The harness's press on the bookcase ended at 122 and its release at
    // 144; it first reported the pie at 124, so the press is given after
    // 123, as for the Talk order's top pie.
    run_to(&mut game, &schedule, 123);
    game.press(on_bookcase);
    game.release(on_bookcase);
    game.render();
    let pie: Vec<String> = game
        .menu()
        .expect("the bookcase's pie is open")
        .labels()
        .into_iter()
        .map(|(text, _)| text.to_string())
        .collect();
    // The recording's harness saw the same three entries.
    assert_eq!(pie, ["Study Cooking", "Study Mechanical", "Read a Book"]);
    run_to(&mut game, &schedule, 143);
    let read = pie_entry(&game, "Read a Book");
    let hash = |game: &Game| game.lot().world().hash();
    let before = hash(&game);
    game.hover(read);
    assert_ne!(hash(&game), before, "reaching the entry checked it");
    run_to(&mut game, &schedule, 186);
    let person = |game: &Game, id: u16| game.lot().world().objects[&id].person.clone().unwrap();
    assert_eq!(person(&game, mortimer).active.uid, 0);
    // Choosing the entry checks it again (0x5eb433): the release ended
    // at 187.
    game.press(read);
    game.release(read);
    assert!(game.menu().is_none());
    game.hover(recorded([384, 324]));
    // Running at once, as recorded at 187, with his last result 0.
    run_to(&mut game, &schedule, 187);
    let p = person(&game, mortimer);
    assert_eq!(action(&p.active), (1, bookcase as i16, 4110, 6, 50, 12));
    assert!(p.queue.is_empty());
    assert!(!last_result(&game));
    // His new frames start with what that check left in his frame
    // storage, as recorded.
    let stack = &game.lot().world().objects[&mortimer].stack;
    let locals = |tree: u16| {
        stack
            .iter()
            .find(|f| f.tree == tree)
            .map(|f| f.locals.clone())
            .unwrap_or_else(|| panic!("a frame of tree {tree}"))
    };
    assert_eq!(locals(4116)[1], 254);
    assert_eq!(locals(4114)[0], 285);
    // Everyone's room is counted again in the frame after 606.
    run_to(&mut game, &schedule, 606);
    for (id, before) in [
        (alexander, 0x429a_aaab),
        (bella, 0x4225_ac1d),
        (mortimer, 0x429a_aaab),
        (salesman, 0x429a_aaab),
    ] {
        assert_eq!(room(&game, id), before, "{id}");
    }
    run_to(&mut game, &schedule, 607);
    for (id, after) in [
        (alexander, 0x4235_5555),
        (bella, 0x4277_073c),
        (mortimer, 0x4235_5555),
        (salesman, 0x4235_5555),
    ] {
        assert_eq!(room(&game, id), after, "{id}");
    }
    run_to(&mut game, &schedule, 900);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Track 2's snack recording with plumb-bob screenshots
/// (`goth-snack-bob-20261120`) replayed through the window's input layer:
/// a press on the fridge, then Have A Snack. Its frame watch gives the
/// frames the original drew, and its write watch on Mortimer's word 50
/// shows when the original listed a pie and checked an entry, so each
/// input is given at an update the watches bound (the second pointer
/// move's, 91, is a choice within its command's 90–93). A scratch
/// comparison found the lot equal to the recording's after every update
/// from 1 to 800 except for what
/// `docs/decisions/20261120-choosing-a-pie-entry-checks-it-again.md`
/// lists; this test checks the states below.
#[test]
fn the_recorded_snack_order_with_bob_shots_replayed_in_the_window_matches_the_recording() {
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse("0,1,2,3,32,62,92,122,152,172,201,223,253+30").expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    let (mortimer, fridge) = (21, 118);
    assert_eq!(game.lot().world().selected(), Some(mortimer));
    let last_result = |game: &Game| game.lot().world().objects[&mortimer].last_result;
    // The first two pointer moves list nothing. The second move's command
    // ran from 90 to 93, and the original listed nothing at the frame
    // after 92. The replay moves the pointer at 91, before that frame;
    // the recording does not say at which of those updates it moved.
    run_to(&mut game, &schedule, 85);
    game.hover(recorded([176, 222]));
    assert_eq!(game.listed(), None);
    run_to(&mut game, &schedule, 91);
    game.hover(recorded([176, 210]));
    run_to(&mut game, &schedule, 93);
    assert_eq!(game.listed(), None);
    assert!(last_result(&game));
    // The third move ended at 120; the original listed the fridge's pie
    // after 122, and Mortimer's last result goes to 0 at 123.
    run_to(&mut game, &schedule, 122);
    game.hover(recorded([176, 198]));
    assert_eq!(game.listed(), Some((mortimer, fridge)));
    run_to(&mut game, &schedule, 123);
    assert!(!last_result(&game));
    // The press listed the pie again after 152; the harness saw the same
    // four entries, and the opening release ran from update 154.
    run_to(&mut game, &schedule, 152);
    assert!(last_result(&game));
    game.press(recorded([176, 198]));
    game.render();
    assert!(game.menu().is_some_and(|m| m.held()));
    let pie: Vec<String> = game
        .menu()
        .expect("the fridge's pie is open")
        .labels()
        .into_iter()
        .map(|(text, _)| text.to_string())
        .collect();
    assert_eq!(
        pie,
        [
            "Have A Snack",
            "Have Quick Breakfast",
            "Have Breakfast",
            "Serve Breakfast"
        ]
    );
    run_to(&mut game, &schedule, 153);
    assert!(!last_result(&game));
    run_to(&mut game, &schedule, 154);
    game.release(recorded([176, 198]));
    assert!(game.menu().is_some_and(|m| !m.held()));
    // The original checked Have A Snack when the pointer reached it,
    // after 172, did nothing at the mousedown (ended 202), and checked
    // and chose it at the release, after 223.
    run_to(&mut game, &schedule, 172);
    let snack = pie_entry(&game, "Have A Snack");
    game.hover(snack);
    run_to(&mut game, &schedule, 173);
    assert!(last_result(&game));
    run_to(&mut game, &schedule, 201);
    game.press(snack);
    assert!(game.menu().is_some_and(|m| m.held()));
    assert!(
        game.lot().world().objects[&mortimer]
            .person
            .as_ref()
            .unwrap()
            .queue
            .is_empty(),
        "pressing an entry does not order it before release"
    );
    run_to(&mut game, &schedule, 223);
    // The check's effects here are not told apart from the order's; the
    // Read and cancel tests pin them.
    game.release(snack);
    assert!(game.menu().is_none());
    game.hover(recorded([200, 123]));
    // Taken at once, as recorded at 224.
    run_to(&mut game, &schedule, 224);
    let p = game.lot().world().objects[&mortimer]
        .person
        .clone()
        .unwrap();
    assert_eq!(action(&p.active), (1, fridge as i16, 4103, 1, 50, 12));
    assert!(p.queue.is_empty());
    // His room is counted again in the frame after 523.
    run_to(&mut game, &schedule, 523);
    assert_eq!(room(&game, mortimer), 0x429a_aaab);
    run_to(&mut game, &schedule, 524);
    assert_eq!(room(&game, mortimer), 0x4234_0000);
    // After 643 the original listed a pie under the still pointer, most
    // likely the plumb bob's (unverified): Mortimer's word 50 goes to 1
    // at 644. Ours lists nothing.
    run_to(&mut game, &schedule, 800);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Track 2's second recording of the two-snack cancel order
/// (`goth-cancel-pick-20261120`), replayed through the window's input
/// layer. Its watches give the frames the original drew and the object
/// its pointer listed, each write at a frame's update. The first two
/// pointer moves are given within their commands; each later move is
/// given at the update of the listing it made. Mortimer's state bounds
/// the rest: the entry is reached before his last result goes to 1, the
/// choice and the cancels are given at the update before their effect. A
/// scratch comparison found the lot equal to the recording's after every
/// update from 1 to 845, in every word it compares, except for Bella's
/// local from 203 to 227, as in the other replays
/// (`docs/decisions/20261120-cancel-order-replayed-with-the-pick-watched.md`);
/// this test checks the states below.
#[test]
fn the_recorded_cancel_order_with_the_pick_watched_replayed_in_the_window_matches_the_recording() {
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse(
        "0,1,2,3,33,63,93,123,153,166,188,210,240,270,300,330,360,390,411,431,446,476+30",
    )
    .expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    let (mortimer, bella, fridge, counter) = (21, 18, 118, 119);
    assert_eq!(game.lot().world().selected(), Some(mortimer));
    let last_result = |game: &Game| game.lot().world().objects[&mortimer].last_result;
    let person = |game: &Game| {
        game.lot().world().objects[&mortimer]
            .person
            .clone()
            .unwrap()
    };
    let snack = |uid| (uid, fridge as i16, 4103, 1, 50, 12);
    let choose = |game: &mut Game, reach: u32, choice: u32| {
        run_to(game, &schedule, reach);
        let at = pie_entry(game, "Have A Snack");
        game.hover(at);
        run_to(game, &schedule, reach + 1);
        assert!(last_result(game));
        run_to(game, &schedule, choice);
        game.press(at);
        game.release(at);
        assert!(game.menu().is_none());
        game.hover(recorded([200, 123]));
    };
    // The second move's command ran from 88 to 94, and the original wrote
    // no listing at 93, where our frame shows the kitchen counter at the
    // first move's point. Where its pointer was at that frame is not
    // known; the replay moves it at 91, before the frame.
    run_to(&mut game, &schedule, 84);
    game.hover(recorded([176, 222]));
    assert_eq!(game.listed(), None);
    run_to(&mut game, &schedule, 91);
    game.hover(recorded([176, 210]));
    run_to(&mut game, &schedule, 94);
    assert_eq!(game.listed(), None);
    assert!(last_result(&game));
    // The move onto the fridge ended at 122; it is given at 123, the
    // update of the original's listing, and his last result goes to 0 at
    // 124. The press lists the fridge again at 153.
    run_to(&mut game, &schedule, 123);
    game.hover(recorded([176, 198]));
    assert_eq!(game.listed(), Some((mortimer, fridge)));
    run_to(&mut game, &schedule, 124);
    assert!(!last_result(&game));
    run_to(&mut game, &schedule, 153);
    assert!(last_result(&game));
    game.press(recorded([176, 198]));
    game.release(recorded([176, 198]));
    game.render();
    assert!(game.menu().is_some());
    run_to(&mut game, &schedule, 154);
    assert!(!last_result(&game));
    // Have A Snack is reached after 166 and chosen after 210; taken at
    // once, as recorded at 211.
    choose(&mut game, 166, 210);
    run_to(&mut game, &schedule, 211);
    let p = person(&game);
    assert_eq!(action(&p.active), snack(1));
    assert!(p.queue.is_empty());
    // Under the still pointer the original listed nothing (240).
    run_to(&mut game, &schedule, 240);
    assert_eq!(game.listed(), None);
    // The second order's moves, each given at the update of the
    // original's listing: the counter at 300, nothing at 330, the fridge
    // at 360; the press lists it again at 390.
    run_to(&mut game, &schedule, 300);
    game.hover(recorded([176, 222]));
    assert_eq!(game.listed(), Some((mortimer, counter)));
    run_to(&mut game, &schedule, 330);
    game.hover(recorded([176, 210]));
    assert_eq!(game.listed(), None);
    run_to(&mut game, &schedule, 360);
    game.hover(recorded([176, 198]));
    assert_eq!(game.listed(), Some((mortimer, fridge)));
    run_to(&mut game, &schedule, 390);
    game.press(recorded([176, 198]));
    game.release(recorded([176, 198]));
    game.render();
    assert!(game.menu().is_some());
    choose(&mut game, 411, 446);
    run_to(&mut game, &schedule, 447);
    let p = person(&game);
    assert_eq!(action(&p.active), snack(1));
    assert_eq!(p.queue.iter().map(action).collect::<Vec<_>>(), [snack(5)]);
    run_to(&mut game, &schedule, 476);
    assert_eq!(game.listed(), None);
    assert_eq!(room(&game, bella), 0x4225_ac1d);
    run_to(&mut game, &schedule, 477);
    assert_eq!(room(&game, bella), 0x4277_073c);
    // Both presses on the icons act at their release's end: the queued
    // snack is taken off at 537, the running one cancelled at 627, and
    // neither earlier.
    run_to(&mut game, &schedule, 506);
    game.hover([104, 54]);
    run_to(&mut game, &schedule, 536);
    assert_eq!(
        person(&game).queue.iter().map(action).collect::<Vec<_>>(),
        [snack(5)]
    );
    game.press([104, 54]);
    run_to(&mut game, &schedule, 537);
    let p = person(&game);
    assert_eq!(action(&p.active), snack(1));
    assert!(p.queue.is_empty());
    run_to(&mut game, &schedule, 596);
    game.hover([54, 54]);
    run_to(&mut game, &schedule, 626);
    let p = person(&game);
    assert_eq!(action(&p.active), snack(1));
    assert!(p.queue.is_empty());
    assert_eq!(p.data[33], 50);
    game.press([54, 54]);
    run_to(&mut game, &schedule, 627);
    let p = person(&game);
    assert_eq!(action(&p.active), (1, fridge as i16, 4103, 1, 50, 0x10c));
    assert_eq!(
        p.queue.iter().map(action).collect::<Vec<_>>(),
        [(6, mortimer as i16, 8318, 33, 50, 0)]
    );
    assert_eq!(p.data[33], 0);
    run_to(&mut game, &schedule, 776);
    assert_eq!(room(&game, mortimer), 0x415c_71c8);
    run_to(&mut game, &schedule, 777);
    assert_eq!(room(&game, mortimer), 0x4253_a203);
    run_to(&mut game, &schedule, 845);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Reaching a pie entry checks its action, which changes the lot;
/// staying on it does not check it again, but leaving the window and
/// coming back onto it does, as passing over another point of the pie
/// would (`Game::leave` forgets the entry, as it forgets the pointer).
#[test]
fn coming_back_onto_a_pie_entry_checks_it_again() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    game.resize([800, 600]);
    assert_eq!(game.run(50), 50);
    game.render();
    let fridge = 118;
    let at = (0..600)
        .step_by(2)
        .flat_map(|y| (0..800).step_by(2).map(move |x| [x, y]))
        .find(|&p| !game.over_panel(p) && game.under(p) == Some(fridge))
        .expect("the fridge is on screen");
    game.hover(at);
    game.press(at);
    game.render();
    let entry = {
        let menu = game.menu().expect("the fridge's pie is open");
        let (_, r) = menu
            .labels()
            .into_iter()
            .find(|(text, _)| *text == "Have A Snack")
            .expect("Have A Snack on the pie");
        [r.x + r.width / 2, r.y + r.height / 2]
    };
    let hash = |game: &Game| game.lot().world().hash();
    let before = hash(&game);
    game.hover(entry);
    assert_ne!(hash(&game), before, "reaching the entry checked it");
    // Some ticks later the check would leave something else: his
    // motives, which it copies the advertisements against, have moved.
    assert_eq!(game.run(60), 60);
    game.render();
    let later = hash(&game);
    game.hover([entry[0] + 1, entry[1]]);
    assert_eq!(hash(&game), later, "still on the entry: not checked");
    game.leave();
    assert_eq!(hash(&game), later);
    game.hover(entry);
    assert_ne!(hash(&game), later, "coming back checked it again");
    assert!(game.menu().is_some());
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// The window's frame draws a bench in use after the Sim on it, as the
/// original does: at 7:12 AM (tick 1260), near zoom, rotation 2, camera
/// origin (591, −2018), walls down, the backrest's soft top edge blends
/// over Mortimer's coat, (32, 16, 8) and (65, 32, 8) in the original's
/// screenshot at the two pixels compared. Drawn into the lot under him,
/// the edge would blend over the floor instead (see sims-people's
/// `a_bench_in_use_draws_after_the_sim_on_it`).
#[test]
fn the_frame_draws_a_bench_in_use_after_the_sim_on_it() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        answers: vec![3, 3],
        ..options(install)
    })
    .expect("the Goth house opens");
    let left = 1260 - game.lot().ticks();
    assert_eq!(game.run(left), left);
    assert!(sims_render::holds_person(game.lot().world()).contains(&174));
    game.resize([800, 600]);
    game.view.camera.zoom = sims_render::Zoom::Near;
    game.view.camera.rotation = 2;
    game.view.camera.origin = [591, -2018];
    game.view.camera.walls = sims_render::Walls::Down;
    let frame = game.render().rgb565();
    for ([x, y], original) in [([385, 311], [32, 16, 8]), ([405, 305], [65, 32, 8])] {
        let at: [u8; 3] = frame[y * 800 + x];
        assert!(
            at.iter().zip(original).all(|(&a, b)| a.abs_diff(b) <= 12),
            "({x}, {y}): {at:?}"
        );
    }
}

/// The pointer on the in-use bench of
/// `the_frame_draws_a_bench_in_use_after_the_sim_on_it`. Where the
/// backrest, drawn after the people, covers Mortimer, the pick notes him:
/// in the original his draw changed the depth there before the bench drew
/// (0x590ebd to 0x590f84), so he is the noted graphic. Elsewhere on the
/// bench, the sprite pick finds it by the rectangle it was drawn into
/// after the people.
#[test]
fn the_pointer_notes_a_sim_behind_the_bench_they_sit_on() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&Options {
        answers: vec![3, 3],
        ..options(install)
    })
    .expect("the Goth house opens");
    let left = 1260 - game.lot().ticks();
    assert_eq!(game.run(left), left);
    let bench = 174;
    let mortimer = 21;
    assert!(sims_render::holds_person(game.lot().world()).contains(&bench));
    game.resize([800, 600]);
    game.view.camera.zoom = sims_render::Zoom::Near;
    game.view.camera.rotation = 2;
    game.view.camera.origin = [591, -2018];
    game.view.camera.walls = sims_render::Walls::Down;
    game.render();
    let on_bench: Vec<[i32; 2]> = (0..600)
        .flat_map(|y| (0..800).map(move |x| [x, y]))
        .filter(|&[x, y]| game.canvas().id_at(x, y) == Some(bench))
        .collect();
    let (behind, rest): (Vec<_>, Vec<_>) = on_bench
        .iter()
        .map(|&p| (p, game.under(p)))
        .partition(|&(_, picked)| picked == Some(mortimer));
    // 1134 of the bench's 3173 pixels cover him, and the other 2039 all
    // pick the bench.
    assert!(
        behind.len() > 500,
        "the bench covers {} of him",
        behind.len()
    );
    assert!(
        rest.len() > 500,
        "only {} of the bench's pixels",
        rest.len()
    );
    let missed: Vec<_> = rest.iter().filter(|&&(_, p)| p != Some(bench)).collect();
    assert!(
        missed.is_empty(),
        "{} of {}: {missed:?}",
        missed.len(),
        rest.len()
    );
}

/// Zoomed in, the rooms keep the cutaway masks built at far zoom until
/// a room update on the floor shown builds them again at the zoom shown
/// (0x4a7c9f; 0x466c3c at 0x466ef6): the lot's steps hand the game the
/// levels their room updates ran for.
#[test]
fn a_room_update_builds_the_masks_kept_from_another_zoom() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.view.camera.focus.masks_built_at, None);
    game.apply(sims_game::view::Move::ZoomIn);
    assert_eq!(
        game.view.camera.focus.masks_built_at,
        Some(sims_render::Zoom::Far)
    );
    let mut ticks = 0;
    while game.view.camera.focus.masks_built_at.is_some() && ticks < 2000 {
        assert_eq!(game.run(1), 1);
        ticks += 1;
    }
    assert_eq!(game.view.camera.focus.masks_built_at, None, "{ticks} ticks");
}

/// Pie entries are chosen on left release, not left press (0x5eb3d1,
/// 0x5eb433): pressing an entry and dragging off the pie before releasing
/// closes it without re-checking or ordering; dragging to another entry
/// orders that one; and dragging straight out from the opening press past
/// the dead zone orders in a single gesture.
#[test]
fn pressing_a_pie_entry_and_dragging_off_orders_nothing() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install.clone())).expect("the Goth house opens");
    assert_eq!(game.run(50), 50);
    game.resize([800, 600]);
    game.render();
    let person = game.lot().world().selected().expect("a Sim is selected");
    let fridge = 118;
    let on_fridge = (0..600)
        .step_by(4)
        .flat_map(|y| (0..800).step_by(4).map(move |x| [x, y]))
        .find(|&p| !game.over_panel(p) && game.under(p) == Some(fridge))
        .expect("the fridge is on screen");
    let queue_len = |game: &Game| {
        game.lot().world().objects[&person]
            .person
            .as_ref()
            .unwrap()
            .queue
            .len()
    };
    let label_center = |game: &Game, name: &str| {
        let menu = game.menu().expect("the fridge's pie is open");
        let (_, r) = menu
            .labels()
            .into_iter()
            .find(|(text, _)| *text == name)
            .unwrap_or_else(|| panic!("{name} on the fridge's pie"));
        [r.x + r.width / 2, r.y + r.height / 2]
    };

    let hash = |game: &Game| game.lot().world().hash();

    // 1. Open and settle, hover Have A Snack, advance 60 ticks so a repeat
    // check would change the lot (as `coming_back_onto_a_pie_entry_checks_it_again`
    // pins). Pressing Have A Snack runs no check and orders nothing; leaving
    // and re-hovering proves a check at this tick changes the world hash;
    // advancing again, pressing, dragging off the pie, and releasing closes
    // with no check and no order.
    game.hover(on_fridge);
    game.press(on_fridge);
    game.release(on_fridge);
    game.render();
    let snack = label_center(&game, "Have A Snack");
    game.hover(snack);
    assert_eq!(game.run(60), 60);
    game.render();
    let later = hash(&game);
    game.press(snack);
    assert!(game.menu().is_some_and(|m| m.held()));
    assert_eq!(hash(&game), later, "pressing a hovered entry runs no check");
    assert_eq!(queue_len(&game), 0);
    // Confirm the hash check above is sensitive to a real re-check here.
    game.leave();
    assert_eq!(hash(&game), later);
    game.hover(snack);
    assert_ne!(
        hash(&game),
        later,
        "a re-check at this tick mutates the lot"
    );
    assert_eq!(game.run(60), 60);
    game.render();
    let later = hash(&game);
    game.press(snack);
    assert_eq!(hash(&game), later);
    game.hover([5, 5]);
    assert_eq!(hash(&game), later);
    game.release([5, 5]);
    assert!(game.menu().is_none());
    assert_eq!(hash(&game), later);
    assert_eq!(queue_len(&game), 0);

    // 2. Pressing an entry, dragging outside the window (`leave`), and
    // releasing outside (`release_outside`, matching winit on macOS when no
    // `CursorMoved` follows `CursorLeft`) closes with no check and no order.
    game.hover(on_fridge);
    game.press(on_fridge);
    game.release(on_fridge);
    game.render();
    let snack = label_center(&game, "Have A Snack");
    game.hover(snack);
    assert_eq!(game.run(60), 60);
    game.render();
    let later = hash(&game);
    game.press(snack);
    assert!(game.menu().is_some_and(|m| m.held()));
    assert_eq!(hash(&game), later);
    game.leave();
    assert_eq!(hash(&game), later);
    game.release_outside();
    assert!(game.menu().is_none());
    assert_eq!(hash(&game), later);
    assert_eq!(queue_len(&game), 0);

    // 2b. Dragging out of the top of the window near a top-clamped pie
    // (where winit can send `CursorMoved` with `y < 0` after `CursorLeft`
    // while the point is still within pie `REACH` of the top label) closes
    // without checking or ordering.
    {
        let mut top_game = Game::open(&options(install.clone())).expect("the Goth house opens");
        top_game.resize([800, 600]);
        assert_eq!(top_game.run(50), 50);
        top_game.apply(sims_game::view::Move::Scroll([0, -150]));
        top_game.render();
        let top_fridge = (0..600)
            .step_by(2)
            .flat_map(|y| (0..800).step_by(2).map(move |x| [x, y]))
            .find(|&p| !top_game.over_panel(p) && top_game.under(p) == Some(fridge))
            .expect("the scrolled fridge is on screen");
        top_game.hover(top_fridge);
        top_game.press(top_fridge);
        top_game.release(top_fridge);
        top_game.render();
        let snack = label_center(&top_game, "Have A Snack");
        let above = [snack[0], -1];
        // Without client bounds clamping, the pie's own hit test would
        // still highlight Have A Snack at `y = -1`.
        assert!(
            top_game.menu().unwrap().highlighted(above).is_some(),
            "expected {above:?} to lie within pie reach above the clamped top entry"
        );
        top_game.hover(snack);
        assert_eq!(top_game.run(60), 60);
        top_game.render();
        let later = hash(&top_game);
        top_game.press(snack);
        assert_eq!(hash(&top_game), later);
        top_game.hover(above);
        assert_eq!(hash(&top_game), later);
        top_game.release(above);
        assert!(top_game.menu().is_none());
        assert_eq!(hash(&top_game), later);
        assert_eq!(queue_len(&top_game), 0);
    }

    // 2c. Shrinking the canvas during the opening press so the pie shifts
    // settles on release instead of ordering whatever entry slid under the
    // pointer; shrinking past the pointer treats the release as outside.
    {
        let mut shrink_game = Game::open(&options(install)).expect("the Goth house opens");
        shrink_game.resize([800, 600]);
        assert_eq!(shrink_game.run(50), 50);
        shrink_game.render();
        let fridge_at = (0..600)
            .step_by(2)
            .flat_map(|y| (0..800).step_by(2).map(move |x| [x, y]))
            .find(|&p| !shrink_game.over_panel(p) && shrink_game.under(p) == Some(fridge))
            .expect("the fridge is on screen");
        shrink_game.hover(fridge_at);
        shrink_game.press(fridge_at);
        // Shrink wide enough that `fridge_at` stays inside, but narrow enough
        // that the pie shifts horizontally before `render()`.
        let narrow = (fridge_at[0] + 40).max(300);
        shrink_game.resize([narrow, 600]);
        shrink_game.release(fridge_at);
        assert!(
            shrink_game.menu().is_some_and(|m| !m.held()),
            "resizing during the opening press settles the pie instead of choosing"
        );
        assert_eq!(queue_len(&shrink_game), 0);
        // Now shrink narrower than `fridge_at[0]` while settled: releasing at
        // the old coordinate is outside the shrunk canvas and closes without
        // ordering.
        let snack = label_center(&shrink_game, "Have A Snack");
        shrink_game.hover(snack);
        shrink_game.press(snack);
        shrink_game.resize([snack[0], 600]);
        shrink_game.release(snack);
        assert!(shrink_game.menu().is_none());
        assert_eq!(queue_len(&shrink_game), 0);
    }

    // 3. Press on Have A Snack, drag onto Have Quick Breakfast, release:
    // orders Have Quick Breakfast (not Have A Snack).
    game.hover(on_fridge);
    game.press(on_fridge);
    game.release(on_fridge);
    game.render();
    let snack = label_center(&game, "Have A Snack");
    let quick = label_center(&game, "Have Quick Breakfast");
    let quick_menu = game
        .menu()
        .unwrap()
        .entries
        .iter()
        .find(|e| e.label == "Have Quick Breakfast")
        .unwrap()
        .menu;
    game.hover(snack);
    game.press(snack);
    game.hover(quick);
    game.release(quick);
    assert!(game.menu().is_none());
    assert_eq!(queue_len(&game), 1);
    assert_eq!(
        game.lot().world().objects[&person]
            .person
            .as_ref()
            .unwrap()
            .queue[0]
            .table_index,
        quick_menu as i32
    );

    // 4. Single-gesture press-drag-release from an unmoved menu chooses
    // immediately on the opening press's release (0x5eb433).
    game.hover(on_fridge);
    game.press(on_fridge);
    game.render();
    let snack = label_center(&game, "Have A Snack");
    let snack_menu = game
        .menu()
        .unwrap()
        .entries
        .iter()
        .find(|e| e.label == "Have A Snack")
        .unwrap()
        .menu;
    assert!(game.menu().unwrap().will_choose(snack));
    game.hover(snack);
    game.release(snack);
    assert!(game.menu().is_none());
    assert_eq!(queue_len(&game), 2);
    assert_eq!(
        game.lot().world().objects[&person]
            .person
            .as_ref()
            .unwrap()
            .queue[1]
            .table_index,
        snack_menu as i32
    );
}
