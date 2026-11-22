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
    // object's place on the lot's terrain, which is the drawing's at
    // that terrain elevation or a pixel off it on flat tiles.
    let lot_shape = sims_render::Lot::from_world(world).expect("lot shape");
    for &(id, object) in &objects {
        let [x, y, level] = object.position.map(i32::from);
        let view = snapshot.projection(i32::from(id)).unwrap();
        assert_eq!(
            view.at,
            sims_game::sound::object_point_on_lot(&camera, &lot_shape, [x, y, level]),
            "object {id}"
        );
        let corners = sims_game::sound::tile_corners(&lot_shape, &camera, [x >> 4, y >> 4]);
        if corners.iter().all(|&h| h == corners[0]) {
            let elevation = corners[1] * 16;
            let drawn = camera.project_fine([x, y], level, elevation);
            assert!(
                (0..2).all(|i| (view.at[i] - drawn[i]).abs() <= 1),
                "object {id}: {:?} {drawn:?}",
                view.at
            );
        }
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
    let lot_shape = sims_render::Lot::from_world(game.lot().world()).expect("lot shape");
    assert_eq!(position[2], camera.level);
    let at = sims_game::sound::object_point_on_lot(&camera, &lot_shape, position);
    let rise = [19, 39, 78][camera.zoom.level() as usize - 1];
    // The house view sits above the 100-pixel live-mode strip (0x49ae5e,
    // middle [400, 250]), and 0x4a81cc aligns origin.x via `(x + 1) & !3`.
    assert!(
        (at[0] - 400).abs() <= 2 && at[1] - rise == 250,
        "centred at {at:?} (rise {rise})"
    );
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
    let head_idle = menu.head.as_ref().expect("the head is drawn").clone();
    assert!(
        head_idle.width > 0
            && head_idle.height > 0
            && head_idle.width <= 800
            && head_idle.height <= 500,
        "head bounds {}x{}",
        head_idle.width,
        head_idle.height
    );
    // Native pie head size (~1,750–2,000 opaque pixels in the house viewport),
    // clear around the bounding-box corners.
    let opaque = head_idle.rgba.chunks(4).filter(|p| p[3] == 255).count();
    assert!((1000..=4000).contains(&opaque), "opaque={opaque}");
    assert_eq!(head_idle.rgba[3], 0);
    // Moving the pointer onto a pie label turns and brightens the head.
    let labels = menu.labels();
    let first = labels[0].1;
    let on_label = [first.x + first.width / 2, first.y + first.height / 2];
    assert!(game.hover(on_label));
    game.render();
    let head_lit = game
        .menu()
        .expect("menu still open")
        .head
        .as_ref()
        .expect("head re-rendered")
        .clone();
    assert_ne!(head_lit.rgba, head_idle.rgba);
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
            lot.residential(),
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
        let lot_shape = sims_render::Lot::from_world(game.lot().world()).expect("lot shape");
        let rotated = camera.rotate_fine([position[0], position[1]]);
        let corners =
            sims_game::sound::tile_corners(&lot_shape, &camera, [rotated[0] >> 4, rotated[1] >> 4]);
        let drawn = camera.project_fine(rotated, position[2], corners[1] * 16);
        let rise = [19, 39, 78][camera.zoom.level() as usize - 1];
        assert!(
            (drawn[0] - 400).abs() <= 2 && (drawn[1] - rise - 250).abs() <= 1,
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
    // recording's (384, 250). With the sprite pick unprojecting and
    // widening rectangles against the original's origin column
    // (`camera.origin[0] + 1`), our pick finds the bookcase at that exact
    // point too.
    run_to(&mut game, &schedule, 93);
    let on_bookcase = recorded([384, 250]);
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

/// Control-panel fidelity integration test against the Goth house:
/// 1. Live-mode pause starts unlit (cyan row 0, `0x53b707..0x53b74d`) and
///    toggles lit (red row 1) every 500 ms (`0x53b500..0x53b528`), setting
///    `blink_due()`.
/// 2. Pressing `?` (`id::HELP`) hovers and dispatches `Command::OpenHelp`.
/// 3. Budget mode (`globals[32] == 1`) hides both money and family friends.
/// 4. Selecting a pet (`person.data[KIND] > 1`) switches open Interests/Gifts
///    to Needs (playing `UI_whoosh` when a balloon closes) and disables those
///    buttons.
#[test]
fn the_control_panel_pause_blink_help_budget_mode_and_pet_refresh() {
    use sims_game::hud::{Hud, INTERFACE, WHOOSH};
    use sims_game::pace::Pace;
    use sims_ui::balloon::Help;
    use sims_ui::command::{Command, Panel};
    use sims_ui::hud::{self, Kind, id};
    let Some(install) = install() else {
        return;
    };
    let house = goth_house(&install);
    let mut game = Game::open(&Options {
        install: install.clone(),
        house: house.clone(),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("the Goth house opens");
    game.render();

    let pause_kind = |game: &Game| {
        game.panel()
            .iter()
            .find_map(|i| match i.kind {
                Kind::Pause { paused, lit } => Some((paused, lit)),
                _ => None,
            })
            .expect("pause button")
    };
    assert_eq!(pause_kind(&game), (false, false));

    // Pause in Live mode: starts selected unlit (cyan row 0), blinks every 500 ms.
    game.toggle_pause();
    game.render();
    assert_eq!(pause_kind(&game), (true, false));
    assert!(!game.blink_due());
    game.advance(499_999);
    assert!(!game.blink_due());
    game.advance(1);
    assert!(game.blink_due());
    game.render();
    assert!(!game.blink_due());
    assert_eq!(pause_kind(&game), (true, true));
    game.advance(hud::PAUSE_BLINK_US);
    assert!(game.blink_due());
    game.render();
    assert_eq!(pause_kind(&game), (true, false));
    game.toggle_pause();
    game.render();
    assert_eq!(pause_kind(&game), (false, false));

    // Help button hovers and accepts clicks (`Command::OpenHelp`).
    let help = game
        .panel()
        .iter()
        .find(|i| i.command == Some(Command::OpenHelp))
        .expect("help button")
        .rect;
    let help_center = [help.x + help.width / 2, help.y + help.height / 2];
    assert!(game.hover(help_center));
    assert!(game.press(help_center).redraw);

    // Budget mode (`globals[32] == 1`) and pet selection (`person.data[KIND] > 1`)
    // exercised directly on `Hud`.
    let opened = open_house(&install, &house).expect("the house opens");
    let lot = game.lot();
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
    hud.draw(
        lot.world(),
        lot.residential(),
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert_eq!(
        hud.press(help_center, lot.world(), lot.residential()),
        Some(Command::OpenHelp)
    );

    let mut budget = lot.world().clone();
    if budget.globals.len() <= 32 {
        budget.globals.resize(33, 0);
    }
    budget.globals[32] = 1;
    hud.draw(
        &budget,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert!(
        !hud.items()
            .iter()
            .any(|i| matches!(i.kind, Kind::Friends(_)))
    );
    assert!(
        !hud.items()
            .iter()
            .any(|i| matches!(&i.kind, Kind::Text { text, .. } if text.starts_with('$')))
    );

    let mut pet_world = lot.world().clone();
    let selected_id = pet_world.selected().expect("Mortimer selected");
    pet_world
        .objects
        .get_mut(&selected_id)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[sims_sim::neighbors::KIND] = 2;
    hud.controls.panel = Some(Panel::Interests);
    hud.controls.help = Some(Help::Sign(2));
    hud.draw(
        &pet_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert_eq!(hud.controls.panel, Some(Panel::Needs));
    assert_eq!(hud.controls.help, None);
    assert_eq!(
        hud.take_sounds(),
        [SoundEvent::Play {
            object: INTERFACE,
            name: WHOOSH.into(),
        }]
    );
    for id in [id::INTERESTS, id::GIFTS] {
        let btn = hud
            .items()
            .iter()
            .find(|i| matches!(i.kind, Kind::Button { id: bid, .. } if bid == id))
            .expect("gadget button");
        assert_eq!(btn.command, None);
    }

    // Drawn while a pet is selected (Interests disabled), then switching
    // selection back to human before the press still opens Interests.
    let interests = hud
        .items()
        .iter()
        .find(|i| {
            matches!(
                i.kind,
                Kind::Button {
                    id: id::INTERESTS,
                    ..
                }
            )
        })
        .expect("interests button")
        .rect;
    let interests_center = [
        interests.x + interests.width / 2,
        interests.y + interests.height / 2,
    ];
    assert_eq!(hud.press(interests_center, &pet_world, true), None);
    assert_eq!(
        hud.press(interests_center, lot.world(), true),
        Some(Command::OpenPanel(Panel::Interests))
    );
    assert_eq!(hud.controls.panel, Some(Panel::Interests));

    // Removing the selected Sim from the live family's roster (`world.families`,
    // 0x51c5ef -> 0x4e97dc) disables Interests on redraw and refuses clicks.
    let mut removed = lot.world().clone();
    let mortimer_guid = removed.objects[&selected_id].guid;
    let family_id = i32::from(removed.globals[9]);
    removed
        .families
        .iter_mut()
        .find(|f| f.id == family_id)
        .expect("Goth family")
        .members
        .retain(|&g| g != mortimer_guid);
    hud.draw(
        &removed,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    let interests_btn = hud
        .items()
        .iter()
        .find(|i| {
            matches!(
                i.kind,
                Kind::Button {
                    id: id::INTERESTS,
                    ..
                }
            )
        })
        .expect("interests button");
    assert_eq!(interests_btn.command, None);
    assert_eq!(hud.press(interests_center, &removed, true), None);
}

/// Install-backed verification of pet Personality/Skills subpanels, state-3
/// ultra-speed blinking, duplicate live household instance selection, and
/// non-residential House panel suppression on the Complete Collection install.
#[test]
fn pet_subpanels_ultra_speed_blink_and_duplicate_instance_selection() {
    use sims_game::hud::Hud;
    use sims_game::pace::Pace;
    use sims_ui::command::{Command, Panel};
    use sims_ui::hud::{Kind, id};
    let Some(install) = install() else {
        return;
    };
    let house = goth_house(&install);
    let mut game = Game::open(&Options {
        install: install.clone(),
        house: house.clone(),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("the Goth house opens");
    assert_eq!(game.run(2), 2);
    let opened = open_house(&install, &house).expect("the Goth house opens");
    let lot = game.lot();
    let mut hud = Hud::open(
        &install,
        &house,
        &opened.assets,
        lot.world(),
        lot.program().autonomy_constants().friendship_threshold,
    )
    .expect("HUD opens");
    let view = View::new([800, 600], [0, 0], opened.rotation);
    let mut canvas = Canvas::new(800, 600, [0; 3]);

    // 1. State-3 ultra-speed blink (`0x53b4c9..0x53b7e7`): put the lot into
    //    idle speed (`return_speed = NORMAL`, `speed = IDLE`), verify Normal
    //    stays selected while Ultra blinks unselected -> selected at 500 ms
    //    and back at 1000 ms, and stays steady when return_speed == ULTRA.
    let mut idle_world = lot.world().clone();
    idle_world.globals[16] = sims_sim::speed::IDLE;
    idle_world.globals[18] = sims_sim::speed::NORMAL;
    let speed_btn = |hud: &Hud, target: i32| {
        hud.items()
            .iter()
            .find_map(|i| match i.kind {
                Kind::Button { id, selected, .. } if id == target => Some(selected),
                _ => None,
            })
            .expect("speed button")
    };
    assert!(!hud.sync_clock_blink(&idle_world, 0));
    hud.draw(
        &idle_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert!(speed_btn(&hud, id::NORMAL));
    assert!(!speed_btn(&hud, id::ULTRA));
    assert!(!hud.sync_clock_blink(&idle_world, 499_999));
    assert!(hud.sync_clock_blink(&idle_world, 500_000));
    hud.draw(
        &idle_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert!(speed_btn(&hud, id::NORMAL));
    assert!(speed_btn(&hud, id::ULTRA));
    assert!(hud.sync_clock_blink(&idle_world, 1_000_000));
    hud.draw(
        &idle_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert!(!speed_btn(&hud, id::ULTRA));
    // When return_speed is already ULTRA, Ultra stays steady selected (no blink).
    let mut ultra_idle = idle_world.clone();
    ultra_idle.globals[18] = sims_sim::speed::ULTRA;
    assert!(!hud.sync_clock_blink(&ultra_idle, 1_500_000));
    assert!(!hud.sync_clock_blink(&ultra_idle, 2_000_000));
    hud.draw(
        &ultra_idle,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert!(speed_btn(&hud, id::ULTRA));

    // Scripted Special Effect `0x10c` speed reset (`0x4f6856..0x4f687a`):
    // changes speed & return_speed from (IDLE, FAST) -> (NORMAL, NORMAL)
    // WITHOUT clearing `[control_panel + 0x1ec]`: `0x53b0d3` preserves state 3
    // and its running 500 ms timer (`since = 2_600_000`), `0x53b121..0x53b128`
    // immediately clears Ultra's selected bit, and the next scheduled callback
    // at 3_100_000 turns Ultra back on. A manual Live-mode speed click
    // (`0x53a5e3`) clears `[+0x1ec]`, stopping the blink even though the speed
    // globals are identical.
    let mut scripted_world = idle_world.clone();
    scripted_world.globals[18] = sims_sim::speed::FAST;
    assert!(!hud.sync_clock_blink(&scripted_world, 2_100_000));
    assert!(hud.sync_clock_blink(&scripted_world, 2_600_000));
    assert!(hud.controls.ultra_lit);
    scripted_world.set_speed(i32::from(sims_sim::speed::NORMAL));
    assert_eq!(
        (scripted_world.speed(), scripted_world.return_speed()),
        (sims_sim::speed::NORMAL, sims_sim::speed::NORMAL)
    );
    // Speed refresh clears Ultra's selected bit (`0x53b121..0x53b128`) while
    // keeping state 3's scheduled 500 ms timer (`since = 2_600_000`).
    assert!(hud.sync_clock_blink(&scripted_world, 2_800_000));
    assert!(!hud.controls.ultra_lit);
    // Next scheduled callback at 3_100_000 turns Ultra back on.
    assert!(hud.sync_clock_blink(&scripted_world, 3_100_000));
    assert!(hud.controls.ultra_lit);
    hud.draw(
        &scripted_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert!(speed_btn(&hud, id::NORMAL));
    assert!(speed_btn(&hud, id::ULTRA));
    // Manual Live-mode speed click clears `[control_panel + 0x1ec]`.
    hud.set_idle_speed(false);
    assert!(hud.sync_clock_blink(&scripted_world, 3_200_000));
    assert!(!hud.controls.ultra_lit);
    assert!(!hud.sync_clock_blink(&scripted_world, 3_700_000));
    assert!(!hud.controls.ultra_lit);

    // 2. Dog (`KIND = 2`) and Cat (`KIND = 4`) Personality and Pet Skills panels
    //    drawn with real UIGraphics art and fonts.
    let mut dog_world = lot.world().clone();
    let selected_id = dog_world.selected().expect("Mortimer selected");
    {
        let p = dog_world
            .objects
            .get_mut(&selected_id)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        p.data[sims_sim::neighbors::KIND] = 2;
        p.data[10] = 500; // House Breaking / Hunting
        p.data[12] = 800; // Tricks
        p.data[11] = 300; // Obedience
    }
    let mut cat_world = dog_world.clone();
    cat_world
        .objects
        .get_mut(&selected_id)
        .unwrap()
        .person
        .as_mut()
        .unwrap()
        .data[sims_sim::neighbors::KIND] = 4;

    // Personality panel: pet traits differ from human traits, and no zodiac sign.
    hud.controls.panel = Some(Panel::Personality);
    let mut human_canvas = Canvas::new(800, 600, [0; 3]);
    hud.draw(
        lot.world(),
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut human_canvas,
    );
    let mut dog_pers_canvas = Canvas::new(800, 600, [0; 3]);
    hud.draw(
        &dog_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut dog_pers_canvas,
    );
    assert!(hud.items().iter().any(|i| matches!(
        i.kind,
        Kind::Personality {
            pet: true,
            zodiac: None,
            ..
        }
    )));
    assert!(
        !hud.items()
            .iter()
            .any(|i| matches!(i.kind, Kind::Sign { .. }))
    );
    assert_ne!(human_canvas.color, dog_pers_canvas.color);

    // Job panel: Dog shows Dog Pet Skills, Cat shows Cat Pet Skills, distinct pixels.
    hud.controls.panel = Some(Panel::Job);
    let mut dog_skills_canvas = Canvas::new(800, 600, [0; 3]);
    hud.draw(
        &dog_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut dog_skills_canvas,
    );
    assert!(hud.items().iter().any(|i| matches!(
        i.kind,
        Kind::PetSkills {
            kind: sims_ui::household::PetKind::Dog,
            skills: [500, 800, 300],
        }
    )));
    assert!(
        !hud.items()
            .iter()
            .any(|i| matches!(i.kind, Kind::Skills { .. } | Kind::Work { .. }))
    );

    let mut cat_skills_canvas = Canvas::new(800, 600, [0; 3]);
    hud.draw(
        &cat_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut cat_skills_canvas,
    );
    assert!(hud.items().iter().any(|i| matches!(
        i.kind,
        Kind::PetSkills {
            kind: sims_ui::household::PetKind::Cat,
            skills: [500, 800, 300],
        }
    )));
    assert_ne!(dog_skills_canvas.color, cat_skills_canvas.color);

    // 3. Stale Sign press guard (`0x51cd17 -> 0x51ce05`, `0x51d1cc..0x51d1ee`):
    //    draw human Personality so `Kind::Sign` is in `hud.items()`, then click
    //    that stale Sign rect against `dog_world` before redraw — must return
    //    None and emit no `UI_whoosh`.
    hud.controls.panel = Some(Panel::Personality);
    hud.draw(
        lot.world(),
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    let _ = hud.take_sounds();
    let sign_rect = hud
        .items()
        .iter()
        .find(|i| matches!(i.kind, Kind::Sign { .. }))
        .expect("human zodiac sign")
        .rect;
    let sign_center = [
        sign_rect.x + sign_rect.width / 2,
        sign_rect.y + sign_rect.height / 2,
    ];
    assert_eq!(hud.press(sign_center, &dog_world, true), None);
    assert!(hud.controls.help.is_none());
    assert!(hud.take_sounds().is_empty());

    // 4. Duplicate live household Sim instance (`0x51c5ef -> 0x4e97dc`,
    //    `0x4e9608`): a clone with a LOWER object ID appended later in
    //    `world.people` still resolves Mortimer's resident slot to the first
    //    person in `world.people`, highlights their portrait, and keeps
    //    Interests enabled.
    let mut dup_world = lot.world().clone();
    let clone_id = (1..selected_id)
        .find(|id| dup_world.objects.get(id).is_none_or(|o| o.person.is_none()))
        .expect("lower non-person object ID");
    let mut clone = dup_world.objects[&selected_id].clone();
    clone.id = clone_id;
    clone.selected = true;
    dup_world.objects.get_mut(&selected_id).unwrap().selected = false;
    dup_world.objects.insert(clone_id, clone);
    dup_world.people.push(clone_id);
    hud.controls.panel = Some(Panel::Needs);
    hud.draw(
        &dup_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    let interests = hud
        .items()
        .iter()
        .find(|i| {
            matches!(
                i.kind,
                Kind::Button {
                    id: id::INTERESTS,
                    ..
                }
            )
        })
        .expect("interests button");
    assert_eq!(
        interests.command,
        Some(Command::OpenPanel(Panel::Interests))
    );
    assert!(
        hud.items()
            .iter()
            .any(|i| matches!(i.kind, Kind::Portrait { selected: true, .. })
                && i.command == Some(Command::Select { sim: selected_id })),
        "duplicate household instance must highlight the resident's portrait"
    );

    // 5. Non-residential House panel suppression (`0x51d3f9..0x51d412`): clears
    //    stored `controls.panel` to `None` so reusing the HUD on a residential
    //    lot does not reopen House.
    hud.controls.panel = Some(Panel::House);
    hud.draw(
        lot.world(),
        false,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert!(
        !hud.items()
            .iter()
            .any(|i| matches!(i.kind, Kind::House { .. }))
    );
    assert_eq!(hud.controls.panel, None);
    hud.draw(
        lot.world(),
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert!(
        !hud.items()
            .iter()
            .any(|i| matches!(i.kind, Kind::House { .. }))
    );
}

/// Install-backed verification of Child Report Card (`[esi + 0x124]`),
/// Superstar / Fame (`[esi + 0x11c]`), and Needs / Pet Needs / Pet Skills /
/// Report Card / Fame help balloons on the Complete Collection install.
#[test]
fn report_card_superstar_panel_and_help_balloons() {
    use sims_game::hud::{Hud, INTERFACE, WHOOSH};
    use sims_game::pace::Pace;
    use sims_ui::balloon::Help;
    use sims_ui::command::{Command, Panel};
    use sims_ui::hud::Kind;
    use sims_vm::SoundEvent;

    let Some(install) = install() else {
        return;
    };
    let house = goth_house(&install);
    let mut game = Game::open(&Options {
        install: install.clone(),
        house: house.clone(),
        answers: vec![3],
        play: false,
        frames: Frames::default(),
    })
    .expect("the Goth house opens");
    assert_eq!(game.run(2), 2);
    let opened = open_house(&install, &house).expect("the Goth house opens");
    let lot = game.lot();
    let mut hud = Hud::open(
        &install,
        &house,
        &opened.assets,
        lot.world(),
        lot.program().autonomy_constants().friendship_threshold,
    )
    .expect("HUD opens");
    let view = View::new([800, 600], [0, 0], opened.rotation);
    let mut canvas = Canvas::new(800, 600, [0; 3]);

    // 1. Human Needs help balloons: clicking Hunger opens balloon (+ UI_whoosh),
    //    clicking Social replaces in-place (no extra UI_whoosh, 0x50fbdf), and
    //    clicking Social again closes it (+ UI_whoosh, 0x50fb7f).
    hud.controls.panel = Some(Panel::Needs);
    hud.draw(
        lot.world(),
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    let _ = hud.take_sounds();
    let hunger_rect = hud
        .items()
        .iter()
        .find(|i| {
            i.command
                == Some(Command::Help(Help::Need {
                    pet: false,
                    index: 0,
                }))
        })
        .expect("Hunger hitbox")
        .rect;
    let social_rect = hud
        .items()
        .iter()
        .find(|i| {
            i.command
                == Some(Command::Help(Help::Need {
                    pet: false,
                    index: 5,
                }))
        })
        .expect("Social hitbox")
        .rect;
    let center = |r: sims_ui::frame::Rect| [r.x + r.width / 2, r.y + r.height / 2];

    assert_eq!(
        hud.press(center(hunger_rect), lot.world(), true),
        Some(Command::Help(Help::Need {
            pet: false,
            index: 0
        }))
    );
    assert_eq!(
        hud.take_sounds(),
        [SoundEvent::Play {
            object: INTERFACE,
            name: WHOOSH.into(),
        }]
    );
    assert_eq!(
        hud.press(center(social_rect), lot.world(), true),
        Some(Command::Help(Help::Need {
            pet: false,
            index: 5
        }))
    );
    assert!(
        hud.take_sounds().is_empty(),
        "switching between open balloons replaces in place without UI_whoosh"
    );
    assert_eq!(
        hud.press(center(social_rect), lot.world(), true),
        Some(Command::Help(Help::Need {
            pet: false,
            index: 5
        }))
    );
    assert_eq!(hud.controls.help, None);
    assert_eq!(
        hud.take_sounds(),
        [SoundEvent::Play {
            object: INTERFACE,
            name: WHOOSH.into(),
        }]
    );

    // 2. Child Report Card on Cassandra (object 16): opens grade button ->
    //    Report Card balloon; switching selection to Mortimer closes the
    //    balloon on redraw and emits UI_whoosh; stale click rejected.
    let mut cass_world = lot.world().clone();
    let mortimer = cass_world.selected().expect("Mortimer selected");
    cass_world.objects.get_mut(&mortimer).unwrap().selected = false;
    cass_world.objects.get_mut(&16).unwrap().selected = true;
    hud.controls.panel = Some(Panel::Job);
    hud.draw(
        &cass_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    let _ = hud.take_sounds();
    let rc_btn = hud
        .items()
        .iter()
        .find(|i| matches!(i.kind, Kind::ReportCardButton { .. }))
        .expect("Report Card grade button")
        .rect;
    // Stale press against Mortimer (Adult Job) must be ignored without WHOOSH.
    assert_eq!(hud.press(center(rc_btn), lot.world(), true), None);
    assert!(hud.take_sounds().is_empty());
    // Press against Cassandra opens Report Card balloon + WHOOSH.
    assert_eq!(
        hud.press(center(rc_btn), &cass_world, true),
        Some(Command::Help(Help::ReportCard))
    );
    assert_eq!(hud.controls.help, Some(Help::ReportCard));
    assert_eq!(hud.take_sounds().len(), 1);
    // Drawing with Mortimer selected closes Report Card balloon + WHOOSH.
    hud.draw(
        lot.world(),
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert_eq!(hud.controls.help, None);
    assert_eq!(hud.take_sounds().len(), 1);

    // 3. Superstar / Fame panel on Bella (object 18, unemployed): setting
    //    person.data[80] > 0 routes Job tab to Fame and opens Fame balloons.
    let mut fame_world = lot.world().clone();
    fame_world.objects.get_mut(&mortimer).unwrap().selected = false;
    let bella = fame_world.objects.get_mut(&18).unwrap();
    bella.selected = true;
    let bp = bella.person.as_mut().unwrap();
    bp.data[80] = 50; // FameScore > 0
    bp.data[81] = 4; // 2.0 stars ("Studio Fly")
    hud.controls.panel = Some(Panel::Job);
    hud.draw(
        &fame_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert!(hud.items().iter().any(|i| matches!(
        &i.kind,
        Kind::Fame {
            fame_level: 4,
            fame_title,
            ..
        } if fame_title == "Studio Fly"
    )));
    let fame_lvl_rect = hud
        .items()
        .iter()
        .find(|i| i.command == Some(Command::Help(Help::FameLevel(4))))
        .expect("Fame level button")
        .rect;
    let _ = hud.take_sounds();
    assert_eq!(
        hud.press(center(fame_lvl_rect), &fame_world, true),
        Some(Command::Help(Help::FameLevel(4)))
    );
    assert_eq!(hud.take_sounds().len(), 1);
    hud.draw(
        &fame_world,
        true,
        &opened.assets,
        &view,
        &Pace::default(),
        None,
        &mut canvas,
    );
    assert!(hud.items().iter().any(|i| matches!(
        &i.kind,
        Kind::Balloon(b) if b.title.starts_with("Studio Fly")
    )));
    // Pressing on the open balloon itself (`0x66bd68 -> 0x5dd85f`) keeps it
    // open without `UI_whoosh`, while hovering onto the balloon itself
    // (`0x51a0a2..0x51a0b5`, `0x51d312`) or outside the subpanel
    // (`0x51f1f2..0x51f3e5`, `0x51d2ee..0x51d329`) dismisses it with `UI_whoosh`.
    let balloon_rect = hud
        .items()
        .iter()
        .find(|i| matches!(i.kind, Kind::Balloon(_)))
        .expect("Fame balloon")
        .rect;
    let _ = hud.take_sounds();
    assert_eq!(hud.press(center(balloon_rect), &fame_world, true), None);
    assert_eq!(hud.controls.help, Some(Help::FameLevel(4)));
    assert!(hud.take_sounds().is_empty());
    assert!(hud.hover(center(balloon_rect)));
    assert_eq!(hud.controls.help, None);
    assert_eq!(
        hud.take_sounds(),
        [SoundEvent::Play {
            object: INTERFACE,
            name: WHOOSH.into(),
        }]
    );

    // 4. Moving the pointer onto the lot (`Game::hover`), leaving the window
    //    (`Game::leave`), or clicking directly on the lot (`Game::press`, both
    //    empty ground and an object that opens a pie menu) dismisses an open
    //    help balloon with `UI_whoosh`. At 1024x768 (`0x518f48`, `0x51f260`),
    //    the western subpanel spans x=521..1024 so hovering (650, 700) keeps
    //    Hunger open.
    game.resize([1024, 768]);
    game.render();
    let hunger_1024 = game
        .panel()
        .iter()
        .find_map(|i| {
            (i.command
                == Some(Command::Help(Help::Need {
                    pet: false,
                    index: 0,
                })))
            .then_some(i.rect)
        })
        .expect("Hunger hitbox at 1024x768");
    assert_eq!(hunger_1024.x, 521);
    game.press(center(hunger_1024));
    game.render();
    let _ = game.take_sounds();
    let _ = game.hover([650, 700]);
    assert!(game.take_sounds().is_empty());
    game.render();
    assert!(
        game.panel()
            .iter()
            .any(|i| matches!(&i.kind, Kind::Balloon(b) if b.title == "Hunger"))
    );
    // Moving onto the balloon itself dismisses it + WHOOSH.
    let balloon_1024 = game
        .panel()
        .iter()
        .find(|i| matches!(i.kind, Kind::Balloon(_)))
        .expect("Hunger balloon at 1024x768")
        .rect;
    assert!(game.hover(center(balloon_1024)));
    assert!(game.take_sounds().contains(&SoundEvent::Play {
        object: INTERFACE,
        name: WHOOSH.into(),
    }));

    game.resize([800, 600]);
    game.render();
    let find_hunger = |game: &Game| {
        game.panel().iter().find_map(|i| {
            (i.command
                == Some(Command::Help(Help::Need {
                    pet: false,
                    index: 0,
                })))
            .then_some(i.rect)
        })
    };
    let need_rect = match find_hunger(&game) {
        Some(rect) => rect,
        None => {
            let toggle_rect = game
                .panel()
                .iter()
                .find(|i| i.command == Some(Command::TogglePanel))
                .expect("TogglePanel button")
                .rect;
            game.press(center(toggle_rect));
            game.render();
            find_hunger(&game).expect("Hunger hitbox on live Game")
        }
    };
    game.press(center(need_rect));
    game.render();
    assert!(
        game.panel()
            .iter()
            .any(|i| matches!(i.kind, Kind::Balloon(_)))
    );
    let _ = game.take_sounds();
    // Moving the pointer from the help control onto the lot dismisses it + WHOOSH.
    assert!(game.hover([10, 10]));
    assert!(game.take_sounds().contains(&SoundEvent::Play {
        object: INTERFACE,
        name: WHOOSH.into(),
    }));
    game.render();
    assert!(
        !game
            .panel()
            .iter()
            .any(|i| matches!(i.kind, Kind::Balloon(_)))
    );
    // Reopen balloon and leave the window: dismisses + WHOOSH.
    game.press(center(need_rect));
    game.render();
    let _ = game.take_sounds();
    assert!(game.leave());
    assert!(game.take_sounds().contains(&SoundEvent::Play {
        object: INTERFACE,
        name: WHOOSH.into(),
    }));
    game.render();
    assert!(
        !game
            .panel()
            .iter()
            .any(|i| matches!(i.kind, Kind::Balloon(_)))
    );
    // Reopen balloon and press empty lot directly: balloon closes + WHOOSH + redraw.
    game.press(center(need_rect));
    game.render();
    let _ = game.take_sounds();
    let outcome = game.press([10, 10]);
    assert!(outcome.redraw);
    assert!(game.take_sounds().contains(&SoundEvent::Play {
        object: INTERFACE,
        name: WHOOSH.into(),
    }));
    game.render();
    assert!(
        !game
            .panel()
            .iter()
            .any(|i| matches!(i.kind, Kind::Balloon(_)))
    );
    // Reopen balloon and press a clickable lot object: balloon dismisses + WHOOSH
    // and the object's pie menu opens.
    game.press(center(need_rect));
    game.render();
    assert!(
        game.panel()
            .iter()
            .any(|i| matches!(i.kind, Kind::Balloon(_)))
    );
    let _ = game.take_sounds();
    let object_pt = (100..700)
        .step_by(12)
        .flat_map(|x| (60..340).step_by(12).map(move |y| [x, y]))
        .find(|&pt| game.under(pt).is_some())
        .expect("a clickable object on the Goth lot");
    let outcome = game.press(object_pt);
    assert!(outcome.redraw);
    assert!(game.take_sounds().contains(&SoundEvent::Play {
        object: INTERFACE,
        name: WHOOSH.into(),
    }));
    game.render();
    assert!(
        !game
            .panel()
            .iter()
            .any(|i| matches!(i.kind, Kind::Balloon(_)))
    );
}

/// Track 2's switch-and-order recording (`goth-switch-cancel-order-20261115`)
/// opens the live-mode panel, clicks Bella's portrait to select her (taking
/// effect between updates 423 and 424), clicks her portrait again to centre
/// the view on her (after update 543), orders `Have A Snack` from the fridge
/// using the pointer, then opens the fridge's pie and hovers `Have A Snack`
/// a second time before update 900.
///
/// Replayed offline in strict mode on the recording's frame schedule with
/// the raw, unshifted recording coordinates, an unmasked comparison of
/// globals, all three RNGs, scheduler cursor, and every object's selected
/// flag, data, temps, attributes, `last_result`, frames, person data,
/// motives, active, previous, and queue (377 objects at update 1 down to
/// 347 at update 900) matched updates 1..=900 except for:
/// - UI global 12 (updates 214..=900);
/// - routing `person.data[37]` marked unknown in `routing.unknown_words`
///   for objects 16, 18, 21, and 259;
/// - frame locals marked unknown in `frame_storage` for object 21
///   (frame 8, tree 8301, local 0 at updates 675..=715) and object 259
///   (frame 2, tree 4099, local 7 at updates 14..=596);
/// - unmarked uninitialized frame locals for object 18 (frame 6, tree
///   8292, local 0 at updates 203..=227; frame 9, tree 8229, locals 2–3
///   at updates 368..=408).
///
/// This test pins selected checkpoints through update 900.
#[test]
fn the_recorded_switch_order_replayed_in_the_window_matches_the_recording() {
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse(
        "0,1,2,3,33,63,93,123,153,183,213,243,273,303,333,363,393,423,453,483,513,543,573,603,633,663,693,715,738,768,798,828,858,883",
    )
    .expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    game.render();

    let mortimer = 21u16;
    let bella = 18u16;
    let fridge = 118u16;
    let counter = 119u16;
    let bob = 383u16;
    let portrait = [296, 527];
    let last = |game: &Game| game.lot().world().objects[&bella].last_result;
    let person = |game: &Game| {
        game.lot().world().objects[&bella]
            .person
            .as_ref()
            .expect("Bella is a person")
            .clone()
    };

    // First press on Bella's portrait switches selection from Mortimer to
    // Bella, refreshes Mortimer's animator, and moves the plumb bob at 424.
    run_to(&mut game, &schedule, 423);
    assert_eq!(game.lot().world().selected(), Some(mortimer));
    game.press(portrait);
    game.release(portrait);
    let world = game.lot().world();
    assert_eq!(world.selected(), Some(bella));
    assert_eq!(world.globals[3], bella as i16);
    assert!(!world.objects[&mortimer].selected);
    let anim = &world.animators[&mortimer];
    assert_eq!(anim.clock, anim.clock_at(world.ticks as i32));
    run_to(&mut game, &schedule, 424);
    assert_eq!(game.lot().world().objects[&bob].data[2], bella as i16);

    // Second press on Bella's portrait centres the 800x500 house view on
    // Bella (standing on House 05's height-6 plateau at [374, 446, 1]),
    // landing on the original's origin [472, -117] (`camera.origin`
    // [471, -117] in our tile-art convention) so the raw recording
    // coordinates match to the pixel.
    run_to(&mut game, &schedule, 543);
    game.press(portrait);
    game.release(portrait);
    assert_eq!(game.view.camera.origin, [471, -117]);
    game.render();

    // First approach to the fridge: raw recording hover coordinates match
    // the original's exact picks (None, None, fridge 118).
    run_to(&mut game, &schedule, 603);
    game.hover([280, 235]);
    assert_eq!(game.under([280, 235]), None);
    assert_eq!(game.listed(), None);
    run_to(&mut game, &schedule, 633);
    game.hover([280, 223]);
    assert_eq!(game.under([280, 223]), None);
    assert_eq!(game.listed(), None);
    run_to(&mut game, &schedule, 663);
    game.hover([280, 211]);
    assert_eq!(game.under([280, 211]), Some(fridge));
    assert_eq!(game.listed(), Some((bella, fridge)));

    // Pressing the fridge opens the pie with the recording's exact label
    // rectangles; hovering and choosing Have A Snack at [280, 136] checks
    // it (last_result true at 716), queues uid 6 at 739, and starts it at 742.
    run_to(&mut game, &schedule, 693);
    game.press([280, 211]);
    game.release([280, 211]);
    game.render();
    let rects: Vec<(&str, [i32; 4])> = game
        .menu()
        .expect("the fridge's pie is open")
        .labels()
        .into_iter()
        .map(|(text, r)| (text, [r.x, r.y, r.width, r.height]))
        .collect();
    assert_eq!(
        rects,
        [
            ("Have A Snack", [224, 122, 113, 29]),
            ("Have Quick Breakfast", [340, 197, 180, 29]),
            ("Have Breakfast", [215, 271, 130, 29]),
            ("Serve Breakfast", [84, 197, 136, 29]),
        ]
    );
    run_to(&mut game, &schedule, 715);
    assert!(!last(&game));
    game.hover([280, 136]);
    run_to(&mut game, &schedule, 716);
    assert!(last(&game));

    run_to(&mut game, &schedule, 738);
    game.press([280, 136]);
    game.release([280, 136]);
    assert!(game.menu().is_none());
    run_to(&mut game, &schedule, 739);
    let queued = person(&game).queue;
    assert_eq!(queued.len(), 1);
    assert_eq!(
        (
            queued[0].uid,
            queued[0].target,
            queued[0].table_index,
            queued[0].action,
            queued[0].priority,
            queued[0].flags,
        ),
        (6, fridge as i16, 1, 4103, 50, 12)
    );
    run_to(&mut game, &schedule, 742);
    assert_eq!(
        (person(&game).active.uid, person(&game).active.target),
        (6, fridge as i16)
    );

    // Second approach to the fridge: with Bella in the kitchen cutting away
    // the front wall, [280, 235] picks counter 119 (last_result false at
    // 769), [280, 223] picks None, and [280, 211] re-lists fridge 118
    // (last_result false at 829).
    run_to(&mut game, &schedule, 768);
    game.hover([280, 235]);
    assert_eq!(game.under([280, 235]), Some(counter));
    assert_eq!(game.listed(), Some((bella, counter)));
    run_to(&mut game, &schedule, 769);
    assert!(!last(&game));

    run_to(&mut game, &schedule, 798);
    game.hover([280, 223]);
    assert_eq!(game.under([280, 223]), None);
    run_to(&mut game, &schedule, 828);
    assert!(last(&game));
    game.hover([280, 211]);
    assert_eq!(game.under([280, 211]), Some(fridge));
    assert_eq!(game.listed(), Some((bella, fridge)));
    run_to(&mut game, &schedule, 829);
    assert!(!last(&game));

    run_to(&mut game, &schedule, 858);
    game.press([280, 211]);
    game.release([280, 211]);
    game.render();
    assert!(game.menu().is_some());
    run_to(&mut game, &schedule, 883);
    game.hover([280, 136]);
    run_to(&mut game, &schedule, 884);
    assert!(last(&game));

    run_to(&mut game, &schedule, 900);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Centring on any tile of a multi-tile object averages its linked parts'
/// positions (`0x4071a3`) before rotating and projecting (`0x434ced` →
/// `0x4a7996`). On the Goth lot, object 25 is a 4-tile object whose parts
/// average to `[400, 432, 1]`, yielding native origin `[432, -142]`
/// (`camera.origin` `[431, -142]` in our tile-art convention, instead of
/// `[447, -142]` from part 25 alone).
#[test]
fn centring_a_multi_tile_object_averages_its_parts() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    game.resize([800, 600]);
    let lead = 25u16;
    let mut parts = vec![lead];
    while let Some(&next) = parts
        .last()
        .and_then(|id| game.lot().world().next_parts.get(id))
    {
        parts.push(next);
    }
    assert_eq!(parts.len(), 4, "object 25 is a 4-tile object");
    for part in parts {
        game.view.camera.origin = [0, 0];
        game.centre_on(part);
        assert_eq!(
            game.view.camera.origin,
            [431, -142],
            "centring part {part} of object 25"
        );
    }
}

/// Track 2's Read A Book pick-and-memory diagnostic recording
/// (`goth-read-pick-20261121`, 900 updates) replayed through the window's
/// input layer. Its `frame1`/`frame2`/`frame3` watches (updates 1–84,
/// 101–719, 731–900) together with its diagnostic pick/tool watches in
/// updates 85–100 and 720–730 pin the drawn-frame schedule
/// (`0,1,2,3,33,63,93,123,145,165,183+30`) and confirm that at update 93
/// the original picked bookcase 132 purely through the 2D sprite pick
/// (`pick85 = 0 -> 0`, `tool85 = 0 -> 132`) at the exact recorded point
/// `(384, 250)`. With `pick::sprite` unprojecting and widening rectangles
/// against the original's origin column (`camera.origin[0] + 1`), our pick
/// finds bookcase 132 at `recorded([384, 250])` directly.
///
/// An unmasked offline comparison across updates 1–900 (all three RNGs,
/// tick, action serial, scheduler cursor, paused, mode, budget, globals,
/// and every object's selected flag, position, registry word, data, temps,
/// attributes, last_result, frames, person data, motives, room, last_decay,
/// active, previous, and queue) found exactly eight raw differences: four
/// `person.data[37]` routing words marked unknown in
/// `routing.unknown_words` (objects 16, 18, 21, 259), two stale frame
/// locals marked unknown in `frame_storage` (object 21 tree 8301 local 0
/// at 668–678; object 259 tree 4099 local 7 at 14–596), and Mortimer's
/// `data[50]` (724–900) and `last_result` (724–868) from the recording's
/// Wine 8.0 `GetClipStatus` stub listing salesman 259 under the still
/// pointer at 723 (`pick720 = 0 -> 0`, `tool720 = 0 -> 259`). See
/// `docs/decisions/20261121-sprite-pick-origin-column-and-sit-read-pick-replays.md`.
#[test]
fn the_recorded_read_order_with_the_pick_watched_replayed_in_the_window_matches_the_recording() {
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse("0,1,2,3,33,63,93,123,145,165,183+30").expect("a schedule");
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
    let person = |game: &Game, id: u16| game.lot().world().objects[&id].person.clone().unwrap();

    run_to(&mut game, &schedule, 89);
    game.hover(recorded([384, 262]));
    assert_eq!(game.under(recorded([384, 262])), None);
    assert_eq!(game.listed(), None);

    // At update 93 the original's `tool85` wrote 0 -> 132 with `pick85`
    // staying 0 at (384, 250). Ours picks bookcase 132 at the exact point.
    run_to(&mut game, &schedule, 93);
    assert_eq!(game.under(recorded([384, 250])), Some(bookcase));
    assert!(last_result(&game));
    game.hover(recorded([384, 250]));
    assert_eq!(game.listed(), Some((mortimer, bookcase)));
    assert!(last_result(&game));

    // Press at 123 opens the held pie; release at 124 settles it.
    run_to(&mut game, &schedule, 123);
    game.press(recorded([384, 250]));
    game.render();
    assert!(game.menu().is_some_and(|m| m.held()));
    let pie: Vec<String> = game
        .menu()
        .expect("the bookcase's pie is open")
        .labels()
        .into_iter()
        .map(|(text, _)| text.to_string())
        .collect();
    assert_eq!(pie, ["Study Cooking", "Study Mechanical", "Read a Book"]);
    run_to(&mut game, &schedule, 124);
    game.release(recorded([384, 250]));
    assert!(game.menu().is_some_and(|m| !m.held()));

    // Hover Read a Book at 145 checks it; press at 165 holds without
    // ordering; release at 183 re-checks and orders it (active at 184).
    run_to(&mut game, &schedule, 145);
    let read = pie_entry(&game, "Read a Book");
    let before = game.lot().world().hash();
    game.hover(read);
    assert_ne!(game.lot().world().hash(), before);
    run_to(&mut game, &schedule, 165);
    game.press(read);
    assert!(game.menu().is_some_and(|m| m.held()));
    assert_eq!(person(&game, mortimer).active.uid, 0);
    assert!(person(&game, mortimer).queue.is_empty());
    run_to(&mut game, &schedule, 183);
    game.release(read);
    assert!(game.menu().is_none());
    game.hover(recorded([384, 324]));
    run_to(&mut game, &schedule, 184);
    let p = person(&game, mortimer);
    assert_eq!(action(&p.active), (1, bookcase as i16, 4110, 6, 50, 12));
    assert!(p.queue.is_empty());
    assert!(!last_result(&game));
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

    // Everyone's room is counted again in the frame after 603.
    run_to(&mut game, &schedule, 603);
    for (id, before) in [
        (alexander, 0x429a_aaab),
        (bella, 0x4225_ac1d),
        (mortimer, 0x429a_aaab),
        (salesman, 0x429a_aaab),
    ] {
        assert_eq!(room(&game, id), before, "{id}");
    }
    run_to(&mut game, &schedule, 604);
    for (id, after) in [
        (alexander, 0x4235_5555),
        (bella, 0x4277_073c),
        (mortimer, 0x4235_5555),
        (salesman, 0x4235_5555),
    ] {
        assert_eq!(room(&game, id), after, "{id}");
    }

    // At 723 the still pointer rests on empty lawn: ours picks nothing,
    // whereas the Wine capture's stubbed GetClipStatus gave salesman 259
    // a full-screen +0x170 rectangle so `tool720` wrote 0 -> 259.
    run_to(&mut game, &schedule, 723);
    assert_eq!(game.under(recorded([384, 324])), None);
    run_to(&mut game, &schedule, 900);
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Track 2's Sit + Read screenshot-and-pick recording
/// (`goth-sit-shots-20261121`, 900 updates) replayed through the window's
/// input layer: Sit on the porch bench (173, active at 216), then Read a
/// Book on the bookcase (132, queued behind Mortimer's active action at
/// 789 and active by 900). Its `frame` watch gives all 35 drawn frames,
/// `pick` stays 0 throughout, and `tool` pins every hover/press listing
/// (`173` at 123 and 153, `132` at 695 and 725) plus the Wine 8.0
/// `GetClipStatus` stub listings under the still pointer (`16` Alexander
/// at 275 and `383` plumb bob at 305).
///
/// An unmasked offline comparison across updates 1–900 (the same full
/// field set as the Read pick replay above) found exactly eight raw
/// differences: four `person.data[37]` routing words marked unknown in
/// `routing.unknown_words` (objects 16, 18, 21, 259), two stale frame
/// locals marked unknown in `frame_storage` (object 21 tree 8228 local 2
/// at 478–498; object 259 tree 4099 local 7 at 14–596), and Mortimer's
/// `last_result` (276–305) and `data[50]` (306–695) from those two Wine
/// `GetClipStatus` still-pointer listings. See
/// `docs/decisions/20261121-sprite-pick-origin-column-and-sit-read-pick-replays.md`.
#[test]
fn the_recorded_sit_and_read_orders_with_shots_replayed_in_the_window_matches_the_recording() {
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse(
        "0,1,2,3,33,63,93,123,153,171,192,215,245,275,305,335,365,395,425,455,485,515,545,575,605,635,665,695,725,754,773,788+30",
    )
    .expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    let (alexander, mortimer, bookcase, bench) = (16, 21, 132, 173);
    assert_eq!(game.lot().world().selected(), Some(mortimer));
    let last_result = |game: &Game| game.lot().world().objects[&mortimer].last_result;
    let person = |game: &Game, id: u16| game.lot().world().objects[&id].person.clone().unwrap();

    // Order 1: Sit on porch bench 173.
    run_to(&mut game, &schedule, 85);
    game.hover(recorded([432, 254]));
    assert_eq!(game.listed(), None);
    run_to(&mut game, &schedule, 92);
    game.hover(recorded([432, 242]));
    assert_eq!(game.listed(), None);
    run_to(&mut game, &schedule, 123);
    assert_eq!(game.under(recorded([432, 230])), Some(bench));
    game.hover(recorded([432, 230]));
    assert_eq!(game.listed(), Some((mortimer, bench)));
    run_to(&mut game, &schedule, 124);
    assert!(!last_result(&game));

    run_to(&mut game, &schedule, 153);
    assert!(last_result(&game));
    game.press(recorded([432, 230]));
    game.render();
    assert!(game.menu().is_some_and(|m| m.held()));
    let sit_pie: Vec<String> = game
        .menu()
        .expect("the bench's pie is open")
        .labels()
        .into_iter()
        .map(|(text, _)| text.to_string())
        .collect();
    assert_eq!(sit_pie, ["Sit"]);
    run_to(&mut game, &schedule, 154);
    assert!(!last_result(&game));
    run_to(&mut game, &schedule, 155);
    game.release(recorded([432, 230]));
    assert!(game.menu().is_some_and(|m| !m.held()));

    run_to(&mut game, &schedule, 171);
    let sit = pie_entry(&game, "Sit");
    game.hover(sit);
    run_to(&mut game, &schedule, 172);
    assert!(last_result(&game));
    run_to(&mut game, &schedule, 192);
    game.press(sit);
    assert!(game.menu().is_some_and(|m| m.held()));
    assert_eq!(person(&game, mortimer).active.uid, 0);
    run_to(&mut game, &schedule, 215);
    game.release(sit);
    assert!(game.menu().is_none());
    game.hover(recorded([432, 155]));
    run_to(&mut game, &schedule, 216);
    let p = person(&game, mortimer);
    assert_eq!(action(&p.active), (1, bench as i16, 8213, 0, 50, 12));
    assert!(p.queue.is_empty());
    assert!(!last_result(&game));

    // Under the still pointer at (432, 155) (lawn behind the bedroom),
    // ours picks nothing at 275 and 305; the Wine capture's stubbed
    // GetClipStatus listed Alexander (16) at 275 and the plumb bob (383)
    // at 305 via their full-screen +0x170 rectangles.
    run_to(&mut game, &schedule, 275);
    assert_eq!(game.under(recorded([432, 155])), None);
    run_to(&mut game, &schedule, 305);
    assert_eq!(game.under(recorded([432, 155])), None);

    // Room update after frame 605.
    run_to(&mut game, &schedule, 605);
    assert_eq!(room(&game, alexander), 0x429a_aaab);
    assert_eq!(room(&game, mortimer), 0x429a_aaab);
    run_to(&mut game, &schedule, 606);
    assert_eq!(room(&game, alexander), 0x4231_5555);
    assert_eq!(room(&game, mortimer), 0x4231_5555);

    // Order 2: Read a Book on bookcase 132 at the exact recorded point
    // (384, 250).
    run_to(&mut game, &schedule, 665);
    game.hover(recorded([384, 262]));
    assert_eq!(game.under(recorded([384, 262])), None);
    assert_eq!(game.listed(), None);
    run_to(&mut game, &schedule, 695);
    assert_eq!(game.under(recorded([384, 250])), Some(bookcase));
    game.hover(recorded([384, 250]));
    assert_eq!(game.listed(), Some((mortimer, bookcase)));
    run_to(&mut game, &schedule, 696);
    assert!(last_result(&game));

    run_to(&mut game, &schedule, 725);
    game.press(recorded([384, 250]));
    game.render();
    assert!(game.menu().is_some_and(|m| m.held()));
    let read_pie: Vec<String> = game
        .menu()
        .expect("the bookcase's pie is open")
        .labels()
        .into_iter()
        .map(|(text, _)| text.to_string())
        .collect();
    assert_eq!(
        read_pie,
        ["Study Cooking", "Study Mechanical", "Read a Book"]
    );
    run_to(&mut game, &schedule, 728);
    game.release(recorded([384, 250]));
    assert!(game.menu().is_some_and(|m| !m.held()));

    run_to(&mut game, &schedule, 754);
    let read = pie_entry(&game, "Read a Book");
    game.hover(read);
    run_to(&mut game, &schedule, 773);
    game.press(read);
    assert!(game.menu().is_some_and(|m| m.held()));
    run_to(&mut game, &schedule, 788);
    game.release(read);
    assert!(game.menu().is_none());
    game.hover(recorded([384, 324]));

    // Queued behind Mortimer's active autonomous action (uid 7 on object
    // 217) at 789, followed by uid 9 at 818, and active by 900.
    run_to(&mut game, &schedule, 789);
    let p = person(&game, mortimer);
    assert_eq!(action(&p.active), (7, 217, 4102, 1, 2, 0));
    assert_eq!(
        p.queue.iter().map(action).collect::<Vec<_>>(),
        [(8, bookcase as i16, 4110, 6, 50, 12)]
    );
    run_to(&mut game, &schedule, 818);
    let p = person(&game, mortimer);
    assert_eq!(action(&p.active), (7, 217, 4102, 1, 2, 0));
    assert_eq!(
        p.queue.iter().map(action).collect::<Vec<_>>(),
        [
            (8, bookcase as i16, 4110, 6, 50, 12),
            (9, 259, 4107, 3, 2, 0),
        ]
    );
    run_to(&mut game, &schedule, 900);
    let p = person(&game, mortimer);
    assert_eq!(action(&p.active), (8, bookcase as i16, 4110, 6, 50, 12));
    assert_eq!(
        p.queue.iter().map(action).collect::<Vec<_>>(),
        [(9, 259, 4107, 3, 2, 0)]
    );
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

#[test]
fn camera_moves_sync_the_lots_view_centre_pick() {
    use sims_game::view::Move;
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    let expected = |game: &Game, expanded: bool| {
        let shape = sims_render::Lot::from_world(game.lot().world()).unwrap();
        let strip = if expanded { sims_ui::hud::STRIP } else { 0 };
        let house_size = [game.view.size[0], (game.view.size[1] - strip).max(1)];
        let mut camera = game.view.camera;
        camera.origin[0] = camera.origin[0].saturating_add(1);
        sims_render::screen_centre_pick(&shape, &camera, house_size)
            .map(|[x, y]| [(x >> 4) as i8, (y >> 4) as i8])
    };
    let initial = game.lot().view_centre();
    assert!(initial.is_some(), "opening the house sets view_centre");
    assert_eq!(initial, expected(&game, true));

    game.apply(Move::Scroll([160, 80]));
    assert_eq!(game.lot().view_centre(), expected(&game, true));
    assert_ne!(game.lot().view_centre(), initial);

    game.apply(Move::Rotate(true));
    assert_eq!(game.lot().view_rotation(), game.view.camera.rotation);
    assert_eq!(game.lot().view_centre(), expected(&game, true));

    game.apply(Move::ZoomIn);
    assert_eq!(game.lot().view_centre(), expected(&game, true));

    game.apply(Move::LevelUp);
    assert_eq!(game.lot().view_centre(), expected(&game, true));

    game.resize([1024, 768]);
    assert_eq!(game.lot().view_centre(), expected(&game, true));

    let mortimer = game.lot().world().selected().expect("a Sim is selected");
    game.centre_on(mortimer);
    assert_eq!(game.lot().view_centre(), expected(&game, true));

    // Collapsing and re-expanding the control panel changes the active house
    // viewport height (0x4a7b58) and must re-sync `view_centre`; `centre_on`
    // also uses the active viewport, shifting `origin[1]` down 50 pixels when
    // re-centring the same Sim with the panel collapsed.
    game.apply(Move::ZoomIn); // Medium -> Near on 1024x768 so origin isn't clamped
    game.centre_on(mortimer);
    let expanded_origin = game.view.camera.origin;
    let expanded_centre = game.lot().view_centre();
    assert_eq!(expanded_centre, expected(&game, true));

    game.render();
    let toggle_point = |game: &Game| {
        game.panel()
            .iter()
            .find(|item| item.command == Some(sims_ui::command::Command::TogglePanel))
            .map(|item| {
                [
                    item.rect.x + item.rect.width / 2,
                    item.rect.y + item.rect.height / 2,
                ]
            })
            .expect("TogglePanel button is laid out")
    };
    let before_collapse = game.lot().view_centre();
    let p = toggle_point(&game);
    game.press(p);
    game.release(p);
    assert_eq!(game.lot().view_centre(), expected(&game, false));
    assert_ne!(game.lot().view_centre(), before_collapse);

    game.centre_on(mortimer);
    assert_eq!(game.view.camera.origin[1], expanded_origin[1] + 50);
    assert_eq!(game.lot().view_centre(), expected(&game, false));
    assert_eq!(game.lot().view_centre(), expanded_centre);

    game.render();
    let p = toggle_point(&game);
    game.press(p);
    game.release(p);
    assert_eq!(game.lot().view_centre(), expected(&game, true));
    game.centre_on(mortimer);
    assert_eq!(game.view.camera.origin, expanded_origin);
    assert_eq!(game.lot().view_centre(), expanded_centre);

    // Restoring a saved game keeps its saved view_centre; no-op and wall-mode
    // camera commands do not overwrite it until camera geometry actually moves.
    let saved_centre = game.lot().view_centre();
    let saved = game.save().unwrap();
    game.apply(Move::Scroll([320, 160]));
    assert_ne!(game.lot().view_centre(), saved_centre);
    game.restore(saved).unwrap();
    assert_eq!(game.lot().view_centre(), saved_centre);
    assert!(!game.apply(Move::Scroll([0, 0])));
    assert_eq!(game.lot().view_centre(), saved_centre);
    game.apply(Move::Walls(sims_render::Walls::Down));
    assert_eq!(game.lot().view_centre(), saved_centre);
    assert!(game.apply(Move::Scroll([160, 80])));
    assert_eq!(game.lot().view_centre(), expected(&game, true));
}

/// Track 2's Buy Mode recording (`goth-buy-chair-order-20261116`) runs in Live
/// mode through update 183, clicks the Buy Mode button `(93, 458)` so update
/// 184 enters mode 2 (partial pass), clicks Seating `(258, 524)`, clicks slot
/// 0 `(437, 523)` (Werkbunnst All Purpose Chair, `0x26bfbb29`, `$80`), and
/// moves the pointer onto the house view at `recorded([480, 380])` before
/// update 1110, creating the chair (object 260) and its `"Tile Cursor"`
/// (object 261, `0x00000437`) at `[584, 472, 1]` without yet committing the
/// placement before update 1200.
///
/// Replayed offline in strict mode on the recording's frame schedule, an
/// unmasked comparison of tick, scheduler cursor, lot/global RNGs, all 64
/// globals, and every object's position, `last_result`, `data`, `temps`,
/// `attributes`, frames, person `data`, `motives_bits`, `active`, and `queue`
/// (377 objects at update 1, 345 at 6..=1109, 347 at 1110..=1200) matched
/// updates 1..=1200 with only three known differences:
/// - UI global 12 (updates 184..=1200: `2008` Buy -> `150` Seating -> `1500`
///   Dining Chairs -> `4300` thumbnail template);
/// - routing `person.data[37]` marked unknown in `routing.unknown_words` for
///   object 259 (updates 14..=1200: `0` vs `3287`);
/// - uninitialized frame local marked unknown in `frame_storage` for object
///   259 (frame 2, tree 4099, local 7 at updates 14..=1200: `0` vs `2623`).
///
/// This test pins selected checkpoints through update 1200.
#[test]
fn the_recorded_buy_chair_order_replayed_in_the_window_matches_the_recording() {
    use sims_ui::command::Mode;
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse(
        "0,1,2,3,33,63,93,123,153,183,213,243,273,303,330,347,367,397,419,446,467,491,518,539,569,599,627,657,687,713,731,757,787,812,842,872,893,923,953,983,1013,1042,1069,1087,1109,1137,1160,1179",
    )
    .expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    game.render();

    // Run in Live mode through update 183, then click Buy Mode at (93, 458).
    run_to(&mut game, &schedule, 183);
    assert_eq!(game.mode(), Mode::Live);
    assert_eq!(game.lot().world().globals[19], 0);
    assert_eq!(game.lot().world().ticks, 88_627);
    assert_eq!(game.lot().world().budget.funds, 2542);
    assert_eq!(game.lot().world().objects.len(), 345);

    game.press([93, 458]);
    game.release([93, 458]);
    assert_eq!(game.mode(), Mode::Buy);
    assert_eq!(game.lot().world().globals[19], 2);
    game.render();

    // Update 184 is a partial pass: world.ticks stays at 88627.
    run_to(&mut game, &schedule, 184);
    assert_eq!(game.lot().world().ticks, 88_627);
    assert_eq!(game.lot().world().globals[19], 2);

    // Click Seating at (258, 524), then slot 0 (Werkbunnst Chair, $80) at (437, 523).
    run_to(&mut game, &schedule, 518);
    game.press([258, 524]);
    game.release([258, 524]);
    game.render();

    run_to(&mut game, &schedule, 872);
    game.press([437, 523]);
    game.release([437, 523]);
    assert_eq!(game.buy_selected(), Some(0x26bf_bb29));
    // No objects are created while the pointer remains over the HUD.
    assert!(game.buy_cursor().is_none());
    assert_eq!(game.lot().world().objects.len(), 345);
    game.render();

    // Moving into the house view at recorded([480, 380]) before update 1110
    // creates the chair (260) and its Tile Cursor (261) at [584, 472, 1].
    run_to(&mut game, &schedule, 1109);
    assert_eq!(game.lot().world().objects.len(), 345);
    let lot_point = recorded([480, 380]);
    assert!(!game.over_panel(lot_point));
    game.hover(lot_point);
    let cursor = game.buy_cursor().expect("Buy cursor active on house view");
    assert_eq!(cursor.held, 260);
    assert_eq!(cursor.cursor, 261);
    assert_eq!(cursor.guid, 0x26bf_bb29);
    assert_eq!(cursor.price, 80);
    assert!(cursor.valid);
    assert_eq!(cursor.position, [584, 472, 1]);
    game.render();

    // Run through update 1200 in strict mode (partial passes only).
    run_to(&mut game, &schedule, 1200);
    let world = game.lot().world();
    assert_eq!(world.ticks, 88_627);
    assert_eq!(world.globals[19], 2);
    assert_eq!(world.budget.funds, 2542);
    assert_eq!(world.lot_rng.state(), 890_318_272);
    assert_eq!(world.global_rng.state(), 270_289_136);
    assert_eq!(world.objects.len(), 347);

    let chair = &world.objects[&260];
    assert_eq!(chair.guid, 0x26bf_bb29);
    assert_eq!(chair.position, [584, 472, 1]);
    assert_eq!(chair.flags(), 9);
    assert_eq!(chair.countdown(), Some(19_999));
    assert!(chair.last_result);
    assert_eq!(chair.data[1], 2);
    assert_eq!(chair.data[26], 261);
    assert_eq!(chair.data[41], 80);
    assert_eq!(chair.data[45], 1997);
    assert_eq!(chair.data[46], 6);
    assert_eq!(chair.data[47], 16);
    assert_eq!(
        chair
            .stack
            .iter()
            .map(|f| (f.tree, f.node, f.parameters.clone()))
            .collect::<Vec<_>>(),
        vec![(4096, 0, vec![0, 0, 0, 0]), (280, 0, vec![20000])]
    );

    let tile_cursor = &world.objects[&261];
    assert_eq!(tile_cursor.guid, sims_vm::TILE_CURSOR_GUID);
    assert_eq!(tile_cursor.position, [584, 472, 1]);
    assert_eq!(tile_cursor.flags(), 0);
    assert_eq!(tile_cursor.countdown(), Some(-1));
    assert!(tile_cursor.last_result);
    assert_eq!(tile_cursor.data[1], 0);
    assert_eq!(tile_cursor.data[18], 1997);
    assert_eq!(tile_cursor.data[23], 2);
    assert_eq!(tile_cursor.data[48], 23970);
    assert_eq!(
        tile_cursor
            .stack
            .iter()
            .map(|f| (f.tree, f.node, f.parameters.clone()))
            .collect::<Vec<_>>(),
        vec![(4096, 0, vec![0, 0, 0, 0])]
    );

    assert!(game.problems().is_empty(), "{:?}", game.problems());
    assert!(game.stopped().is_none(), "{:?}", game.stopped());
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// End-to-end Buy Mode placement, rotation (keyboard and mouse drag),
/// cancellation, funds deduction, and return to Live mode through the window
/// input layer.
#[test]
fn buy_mode_places_rotates_cancels_and_returns_to_live_mode() {
    use sims_ui::command::Mode;
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    game.resize([800, 600]);
    game.render();
    assert_eq!(game.run(60), 60);

    // Enter Buy Mode, open Seating, select Werkbunnst All Purpose Chair ($80).
    game.press([93, 458]);
    game.release([93, 458]);
    assert_eq!(game.mode(), Mode::Buy);
    assert_eq!(game.lot().world().globals[19], 2);

    game.press([258, 524]);
    game.release([258, 524]);
    game.press([437, 523]);
    game.release([437, 523]);
    assert_eq!(game.buy_selected(), Some(0x26bf_bb29));

    // Hover onto the lawn, move onto the HUD to hide the cursor (verifying
    // keyboard rotation while hidden does not bring it back onto the lot),
    // save & restore mid-preview (verifying the save strips the unpaid preview
    // and Tile Cursor), then cancel: both objects are deleted and funds stay 2542.
    let spot = recorded([480, 380]);
    let count_before = game.lot().world().objects.len();
    game.hover(spot);
    let held_first = game.buy_cursor().expect("cursor active").held;
    let cursor_first = game.buy_cursor().expect("cursor active").cursor;
    assert_eq!(game.lot().world().objects.len(), count_before + 2);

    // Save while the preview stands on the lawn, then restore into a fresh
    // game check later: the saved world must not contain the unpaid preview.
    let saved_mid_preview = game.save().expect("saving during Buy preview succeeds");
    assert!(!saved_mid_preview.world().objects.contains_key(&held_first));
    assert!(
        !saved_mid_preview
            .world()
            .objects
            .contains_key(&cursor_first)
    );
    assert_eq!(saved_mid_preview.world().objects.len(), count_before);

    game.hover([300, 520]);
    assert!(!game.buy_cursor().expect("cursor kept while hidden").valid);
    assert_eq!(game.buy_cursor().unwrap().position, [584, 472, 1]);
    assert_eq!(game.lot().world().objects[&held_first].position[0], -16);
    assert_eq!(game.lot().world().objects[&cursor_first].position[0], -16);
    // Native `0x487d4e` preserves the saved coordinates on the hidden cursor
    // and `0x486b9e` consumes `.`/`,` whenever the cursor exists, turning at
    // those saved coordinates (`0x421be4`).
    assert!(game.rotate_held(true));
    assert_eq!(game.lot().world().objects[&held_first].data[1], 4);
    assert_eq!(
        game.lot().world().objects[&held_first].position,
        [584, 472, 1]
    );
    assert_eq!(
        game.lot().world().objects[&cursor_first].position,
        [584, 472, 1]
    );

    assert!(game.cancel_buy());
    assert!(game.buy_cursor().is_none());
    assert!(game.buy_selected().is_none());
    assert!(!game.lot().world().objects.contains_key(&held_first));
    assert!(!game.lot().world().objects.contains_key(&cursor_first));
    assert_eq!(game.lot().world().budget.funds, 2542);

    // Re-select the chair, hover onto the lawn, rotate clockwise by keyboard,
    // then press and drag through all four quadrants (`0x485d5d..0x485dad`:
    // upper-right -> 4, lower-left -> 0, lower-right -> 2, upper-left -> 6 at
    // rotation 0), releasing 20 px upper-left without an extra hover to commit
    // facing 6 (`0x4864f0`).
    let spend_before = game.lot().world().budget.days[0].expect("today")[6];
    game.press([437, 523]);
    game.release([437, 523]);
    game.hover(spot);
    assert_eq!(game.lot().world().objects[&held_first].data[1], 2);
    assert!(game.rotate_held(true));
    assert_eq!(game.lot().world().objects[&held_first].data[1], 4);
    game.press(spot);
    game.hover([spot[0] + 20, spot[1] - 20]);
    assert_eq!(game.lot().world().objects[&held_first].data[1], 4);
    game.hover([spot[0] - 20, spot[1] + 20]);
    assert_eq!(game.lot().world().objects[&held_first].data[1], 0);
    game.hover([spot[0] + 20, spot[1] + 20]);
    assert_eq!(game.lot().world().objects[&held_first].data[1], 2);
    game.release([spot[0] - 20, spot[1] - 20]);

    // Werkbunnst Chair has OBJD word 96 == 1 (`keep_buying`), so committing it
    // automatically arms a second copy (`0x4864d0` -> `0x486f5a`) with the
    // placed chair's facing (`6`), tracking the release tile (empty lawn, so
    // valid there); moving back onto the occupied `spot` moves the new chair
    // off-lot while its Tile Cursor marks the occupied tile — matching
    // `goth-buy-chair-order-20261118` u=880.
    assert_eq!(game.buy_selected(), Some(0x26bf_bb29));
    let next_cursor = game
        .buy_cursor()
        .expect("repeat purchase arms a second chair")
        .clone();
    assert_ne!(next_cursor.held, held_first);
    assert!(next_cursor.valid);
    assert_ne!(next_cursor.position, [584, 472, 1]);
    game.hover(spot);
    let next_on_occupied = game.buy_cursor().expect("cursor over occupied tile");
    assert!(!next_on_occupied.valid);
    let world = game.lot().world();
    let chair = &world.objects[&held_first];
    assert_eq!(chair.guid, 0x26bf_bb29);
    assert_eq!(chair.position, [584, 472, 1]);
    assert_eq!(chair.data[1], 6);
    assert_eq!(world.objects[&next_cursor.held].position[0], -16);
    assert_eq!(world.objects[&next_cursor.held].data[1], 6);
    assert_eq!(world.budget.funds, 2462);
    assert_eq!(world.globals[13], 2462);
    assert_eq!(world.globals[14], 0);
    assert_eq!(world.budget.days[0].expect("today")[6], spend_before + 80);

    // Switch back to Live mode by clicking the Live button `(35, 448)`:
    // cancels the armed second preview, returns global[19] to 0 (`go_live`),
    // spawns the native `"comeandsee"` (`0xf3aaa62f`, `0x40726a`) marker on
    // the newly bought chair, and resumes simulation ticks.
    let ticks_before = game.lot().world().ticks;
    game.press([35, 448]);
    game.release([35, 448]);
    assert_eq!(game.mode(), Mode::Live);
    assert!(game.buy_cursor().is_none());
    assert!(game.buy_selected().is_none());
    assert_eq!(game.lot().world().objects.len(), count_before + 2);
    let marker = game.lot().world().objects[&held_first].data[26] as u16;
    assert_eq!(
        game.lot().world().objects[&marker].guid,
        sims_vm::COMEANDSEE_GUID
    );
    assert_eq!(game.lot().world().globals[19], 0);
    game.render();
    assert_eq!(game.run(30), 30);
    assert_eq!(game.lot().world().ticks, ticks_before + 30);

    // Restoring the mid-preview save and returning to Live leaves no leaked
    // chair or Tile Cursor and keeps funds at 2542.
    game.restore(saved_mid_preview)
        .expect("mid-preview save restores");
    assert_eq!(game.mode(), Mode::Buy);
    assert!(game.buy_cursor().is_none());
    assert_eq!(game.lot().world().objects.len(), count_before);
    assert_eq!(game.lot().world().budget.funds, 2542);
    game.set_mode(Mode::Live);
    assert_eq!(game.lot().world().objects.len(), count_before);
    assert_eq!(game.lot().world().budget.funds, 2542);

    assert!(game.problems().is_empty(), "{:?}", game.problems());
    assert!(game.stopped().is_none(), "{:?}", game.stopped());
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Buy Mode surface placement (`0x422050` using `data[4]` allowed heights and
/// `0x4221e4..0x422280` surface alignment), OBJf function 11 (`USER_PLACEMENT`,
/// `0x43562f` / `0x422e99`), and preserving the cursor handle when a commit is
/// refused.
#[test]
fn buy_mode_places_on_surfaces_runs_user_placement_and_keeps_cursor_on_failed_commit() {
    use sims_ui::buy::BuyCatalog;
    use sims_ui::command::Mode;
    let Some(install) = install() else {
        return;
    };
    let opened = sims_render::open_house(&install, &goth_house(&install)).expect("open Goth house");
    let catalog = BuyCatalog::load(opened.assets.index(), opened.assets.catalog());

    let mut lot = Lot::load(&install, &goth_house(&install), &[3], Frames::default()).unwrap();
    for _ in 0..60 {
        lot.step().unwrap();
    }
    lot.set_mode(sims_vm::PanelMode::Buy).unwrap();

    // Find every empty surface slot on level 1 in the Goth house (such as the
    // Mesquite Desk/Table) and verify all three stock desk items (Moneywell
    // Computer, SCTC BR-8 Standard Telephone, Gloominator Desk Lamp) snap into
    // a compatible empty surface slot with `data[4] != 0` and commit cleanly.
    let empty_surfaces: Vec<(u16, [i32; 3])> = lot
        .world()
        .objects
        .iter()
        .filter_map(|(&id, obj)| {
            (obj.person.is_none()
                && obj.position[0] >= 0
                && obj.position[2] == 1
                && obj.slots.len() > 1
                && obj.slots[1].object == 0)
                .then_some((id, obj.position))
        })
        .collect();
    assert!(!empty_surfaces.is_empty(), "Goth house has empty surfaces");

    for needle in [
        "Moneywell Computer",
        "SCTC BR-8 Standard Telephone",
        "Gloominator Desk Lamp",
    ] {
        let item = catalog
            .all()
            .iter()
            .find(|i| i.name.contains(needle))
            .unwrap_or_else(|| panic!("catalog item matching {needle:?}"));
        let mut cursor = lot
            .buy_start(item.guid, 1, 2)
            .unwrap()
            .unwrap_or_else(|| panic!("affordable {needle}"));
        let mut snapped = None;
        for &(surface_id, pos) in &empty_surfaces {
            let valid = lot.buy_move(&mut cursor, [pos[0], pos[1]], pos[2]).unwrap();
            if valid && cursor.container.is_some_and(|(c, _)| c == surface_id) {
                snapped = Some(surface_id);
                break;
            }
        }
        let surface_id = snapped.unwrap_or_else(|| panic!("{needle} did not snap to any surface"));
        // Manual rotation on the surface must keep the surface container and
        // the user's chosen facing even when `buy_move` runs again at the same
        // tile (e.g., on mouse press/release, `0x421e72` / `0x422050`).
        let before_dir = lot.world().objects[&cursor.held].data[1];
        let step = lot.world().objects[&cursor.held].data[23];
        if step > 0 {
            let next_dir = (before_dir + step).rem_euclid(8);
            assert!(lot.buy_rotate(&mut cursor, next_dir).unwrap());
            assert_eq!(cursor.container.map(|(c, _)| c), Some(surface_id));
            assert_eq!(lot.world().objects[&cursor.held].data[1], next_dir);
            let pos = cursor.position;
            assert!(lot.buy_move(&mut cursor, [pos[0], pos[1]], pos[2]).unwrap());
            assert_eq!(cursor.container.map(|(c, _)| c), Some(surface_id));
            assert_eq!(lot.world().objects[&cursor.held].data[1], next_dir);
            // Temporarily unaffordable preview must still retain its surface
            // container so manual rotation survives when funds recover again.
            let orig_price = cursor.price;
            cursor.price = lot.world().budget.funds + 10;
            assert!(!lot.buy_move(&mut cursor, [pos[0], pos[1]], pos[2]).unwrap());
            assert!(!cursor.valid);
            assert_eq!(cursor.container.map(|(c, _)| c), Some(surface_id));
            assert_eq!(lot.world().objects[&cursor.held].data[1], next_dir);
            cursor.price = orig_price;
            assert!(lot.buy_move(&mut cursor, [pos[0], pos[1]], pos[2]).unwrap());
            assert!(cursor.valid);
            assert_eq!(cursor.container.map(|(c, _)| c), Some(surface_id));
            assert_eq!(lot.world().objects[&cursor.held].data[1], next_dir);
        }
        // Cancel before testing the next item so the same surface slot stays free.
        lot.buy_cancel(&cursor).unwrap();
    }

    // Commit the Gloominator Desk Lamp onto a surface and verify container link
    // (`data[2]`), funds deduction, and cleanup of its Tile Cursor.
    let lamp = catalog
        .all()
        .iter()
        .find(|i| i.name.contains("Gloominator Desk Lamp"))
        .expect("Gloominator Desk Lamp");
    let mut cursor = lot.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
    let surface_id = empty_surfaces
        .iter()
        .find_map(|&(id, pos)| {
            let valid = lot.buy_move(&mut cursor, [pos[0], pos[1]], pos[2]).unwrap();
            (valid && cursor.container.is_some_and(|(c, _)| c == id)).then_some(id)
        })
        .expect("surface for lamp");
    let funds_before = lot.world().budget.funds;
    let placed_lamp = lot.buy_commit(&cursor).unwrap();
    assert!(!lot.world().objects.contains_key(&cursor.cursor));
    assert_eq!(lot.world().objects[&placed_lamp].data[2], surface_id as i16);
    assert_eq!(lot.world().budget.funds, funds_before - lamp.price);

    // OBJf function 11 (`USER_PLACEMENT`, `0x43562f` / `0x422e99`): committing
    // the Jack-O'-Lantern runs its user-placement callback cleanly.
    if let Some(pumpkin) = catalog
        .all()
        .iter()
        .find(|i| i.name.contains("Jack-O") && i.price <= lot.world().budget.funds)
    {
        let mut cursor = lot.buy_start(pumpkin.guid, 1, 2).unwrap().unwrap();
        assert!(lot.buy_move(&mut cursor, [584, 472], 1).unwrap());
        let placed = lot.buy_commit(&cursor).unwrap();
        assert!(lot.world().objects.contains_key(&placed));
        assert!(!lot.world().objects.contains_key(&cursor.cursor));
    }

    // Unsnapped fallback (`0x485f23` -> `0x421e72` -> `0x4355bc..0x4355cd`):
    // Werkbunnst Chair collides with object 121 at snapped `[296, 424]`. Native
    // `UserTestPlace` requires tile-centred coordinates (`low nibbles == 8`),
    // so retrying at unsnapped `[292, 431]` leaves the held chair off-lot,
    // places only `"Tile Cursor"` at `[292, 431, 1]`, and refuses commit.
    let mut unsnapped = lot.buy_start(0x26bf_bb29, 1, 2).unwrap().unwrap();
    assert!(!lot.buy_move(&mut unsnapped, [292, 431], 1).unwrap());
    assert!(!unsnapped.valid);
    assert_eq!(unsnapped.position, [292, 431, 1]);
    assert!(lot.world().objects[&unsnapped.held].position[0] < 0);
    assert_eq!(
        lot.world().objects[&unsnapped.cursor].position,
        [292, 431, 1]
    );
    assert!(lot.buy_commit(&unsnapped).is_err());
    lot.buy_cancel(&unsnapped).unwrap();

    // Initial off-lot rotation runs `LEAVING` (`0x42271d` / `0x421bff` ->
    // `0x433148`): starting Scrimpon Crimson Awnings (`0xefba77ca`) at facing 2
    // runs its OBJf 10 `LEAVING` callback while off-lot, setting `data[13] = 4`
    // so open-lawn placement at `[584, 472]` is refused immediately.
    if catalog.get(0xefba_77ca).is_some() {
        let mut awn = lot.buy_start(0xefba_77ca, 1, 2).unwrap().unwrap();
        assert_eq!(lot.world().objects[&awn.held].data[13], 4);
        assert!(!lot.buy_move(&mut awn, [584, 472], 1).unwrap());
        assert!(!awn.valid);
        assert!(lot.buy_commit(&awn).is_err());
        lot.buy_cancel(&awn).unwrap();
    }

    // Ground flag `0x20` (`UserTestPlace`, `0x435522`): road/curb tile
    // `[24, 328, 1]` has ground flags `0x27` (`& 0x20 != 0`) and must be
    // refused by Buy placement.
    let mut blocked_cur = lot.buy_start(0x26bf_bb29, 1, 2).unwrap().unwrap();
    assert!(!lot.buy_move(&mut blocked_cur, [24, 328], 1).unwrap());
    assert!(!blocked_cur.valid);
    lot.buy_cancel(&blocked_cur).unwrap();

    // Wall-mounted auto-rotation (`0x48736b` + `0x43583e`): Narcisco Wall
    // Mirror fails at facing 2 at `[264, 504]` and auto-rotates to facing 4.
    // Manual rotation (`0x4226bc` / `0x421be4`) back to facing 2 must NOT
    // re-run pointer-move auto-rotation: it returns `Ok(false)`, keeps facing
    // 2 with `cursor.valid == false`, and refuses commit until rotated back.
    let mirror = catalog
        .all()
        .iter()
        .find(|i| i.name.contains("Narcisco"))
        .expect("Narcisco Wall Mirror");
    let mut mirror_cur = lot.buy_start(mirror.guid, 1, 2).unwrap().unwrap();
    assert!(lot.buy_move(&mut mirror_cur, [264, 504], 1).unwrap());
    assert_eq!(lot.world().objects[&mirror_cur.held].data[1], 4);
    assert!(!lot.buy_rotate(&mut mirror_cur, 2).unwrap());
    assert!(!mirror_cur.valid);
    assert_eq!(lot.world().objects[&mirror_cur.held].data[1], 2);
    assert!(lot.buy_commit(&mirror_cur).is_err());
    assert!(lot.buy_rotate(&mut mirror_cur, 4).unwrap());
    assert!(mirror_cur.valid);
    assert_eq!(lot.world().objects[&mirror_cur.held].data[1], 4);
    // Failed unsnapped fallback (`0x421fcf..0x421fed`) saves the cursor's
    // actual coordinates so manual rotation (`0x422705..0x422716`) keeps the
    // cursor at `[581, 475, 1]` instead of jumping to tile centre `[584, 472, 1]`.
    assert!(!lot.buy_move(&mut mirror_cur, [581, 475], 1).unwrap());
    assert_eq!(mirror_cur.position, [581, 475, 1]);
    assert_eq!(
        lot.world().objects[&mirror_cur.cursor].position,
        [581, 475, 1]
    );
    assert!(!lot.buy_rotate(&mut mirror_cur, 2).unwrap());
    assert_eq!(mirror_cur.position, [581, 475, 1]);
    assert_eq!(
        lot.world().objects[&mirror_cur.cursor].position,
        [581, 475, 1]
    );
    lot.buy_cancel(&mirror_cur).unwrap();

    // Undo / Redo for surface purchase (`0x48ba19`, `0x435751`, `0x48c07b`,
    // `0x48c20e`): undo runs OBJf 12 (`USER_PICKUP`) and detaches the lamp
    // off-lot while keeping it registered; redo reuses the exact same
    // registered instance and restores its desk slot and charge.
    assert!(lot.can_buy_undo());
    let lamp_reg = lot.world().objects[&placed_lamp].registry_word;
    let mut funds_before_lamp_undo = lot.world().budget.funds;
    while lot.world().objects[&placed_lamp].position[0] >= 0 {
        funds_before_lamp_undo = lot.world().budget.funds;
        assert!(lot.buy_undo().unwrap());
    }
    assert!(lot.world().objects.contains_key(&placed_lamp));
    assert!(lot.world().objects[&placed_lamp].position[0] < 0);
    assert_eq!(lot.world().objects[&placed_lamp].data[2], 0);
    assert_eq!(
        lot.world().budget.funds,
        funds_before_lamp_undo + lamp.price
    );
    assert!(lot.can_buy_redo());
    assert!(lot.buy_redo().unwrap());
    assert!(lot.world().objects[&placed_lamp].position[0] >= 0);
    assert_eq!(lot.world().objects[&placed_lamp].registry_word, lamp_reg);
    assert_eq!(lot.world().objects[&placed_lamp].data[2], surface_id as i16);
    assert_eq!(lot.world().budget.funds, funds_before_lamp_undo);

    // Refused Redo transfer (`0x48bdfe` line `0x48be72` -> `0x493353`):
    // buying two Werkbunnst Chairs at `[552, 472]` and `[520, 472]`, undoing
    // both, blocking the first chair's tile `[552, 472]` with a preview chair,
    // and redoing must transfer the refused first chair to Undo without
    // charging while keeping the second chair on Redo so the next Redo restores
    // it and re-charges `$80`. Undoing `ch2` then leaves the refused `ch1`
    // record on top of Undo, whose Undo must return `false` without
    // double-refunding `ch1`.
    {
        let mut ch1 = lot.buy_start(0x26bf_bb29, 1, 2).unwrap().unwrap();
        assert!(lot.buy_move(&mut ch1, [552, 472], 1).unwrap());
        let ch1_id = lot.buy_commit(&ch1).unwrap();
        let mut ch2 = lot.buy_start(0x26bf_bb29, 1, 2).unwrap().unwrap();
        assert!(lot.buy_move(&mut ch2, [520, 472], 1).unwrap());
        let ch2_id = lot.buy_commit(&ch2).unwrap();
        assert!(lot.buy_undo().unwrap());
        assert!(lot.buy_undo().unwrap());
        let funds_after_undos = lot.world().budget.funds;
        let mut blocker = lot.buy_start(0x26bf_bb29, 1, 2).unwrap().unwrap();
        assert!(lot.buy_move(&mut blocker, [552, 472], 1).unwrap());
        // First redo pops `ch1`: blocked, not charged, transferred to Undo,
        // leaving `ch2` on Redo.
        assert!(!lot.buy_redo().unwrap());
        assert_eq!(lot.world().budget.funds, funds_after_undos);
        assert!(lot.world().objects[&ch1_id].position[0] < 0);
        assert!(lot.can_buy_redo());
        // Second redo restores `ch2` and re-charges `$80`.
        assert!(lot.buy_redo().unwrap());
        assert_eq!(lot.world().budget.funds, funds_after_undos - 80);
        assert!(lot.world().objects[&ch2_id].position[0] >= 0);
        lot.buy_cancel(&blocker).unwrap();
        assert!(lot.buy_undo().unwrap());
        assert_eq!(lot.world().budget.funds, funds_after_undos);
        // Next undo hits the refused `ch1` record and must NOT double-refund.
        assert!(!lot.buy_undo().unwrap());
        assert_eq!(lot.world().budget.funds, funds_after_undos);
    }

    // Blocked container Redo: buying a Mesquite Desk on the lawn and placing a
    // Gloominator Desk Lamp on it, undoing both, blocking the desk's tile with
    // a previewed chair, then redoing both must refuse BOTH the desk and the
    // lamp (since the desk is still detached off-lot) without charging for the
    // lamp, and undoing the refused records must NOT double-refund.
    if let Some(desk) = catalog
        .all()
        .iter()
        .find(|i| i.name.contains("Mesquite") && i.name.contains("Desk"))
    {
        let mut desk_cur = lot.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot.buy_move(&mut desk_cur, [520, 472], 1).unwrap());
        let desk_id = lot.buy_commit(&desk_cur).unwrap();
        let mut lamp_cur = lot.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
        assert!(lot.buy_move(&mut lamp_cur, [520, 472], 1).unwrap());
        assert_eq!(lamp_cur.container.map(|(c, _)| c), Some(desk_id));
        let desk_lamp_id = lot.buy_commit(&lamp_cur).unwrap();
        assert!(lot.buy_undo().unwrap());
        assert!(lot.buy_undo().unwrap());
        assert!(lot.world().objects[&desk_id].position[0] < 0);
        assert!(lot.world().objects[&desk_lamp_id].position[0] < 0);
        let funds_before_blocked_redo = lot.world().budget.funds;
        let mut blocker = lot.buy_start(0x26bf_bb29, 1, 2).unwrap().unwrap();
        assert!(lot.buy_move(&mut blocker, [520, 472], 1).unwrap());
        // Desk redo is refused because the preview chair occupies `[520, 472]`.
        assert!(!lot.buy_redo().unwrap());
        assert!(lot.world().objects[&desk_id].position[0] < 0);
        assert_eq!(lot.world().budget.funds, funds_before_blocked_redo);
        // Lamp redo must also be refused because its container desk is off-lot.
        assert!(!lot.buy_redo().unwrap());
        assert!(lot.world().objects[&desk_lamp_id].position[0] < 0);
        assert_eq!(lot.world().budget.funds, funds_before_blocked_redo);
        lot.buy_cancel(&blocker).unwrap();
        // Undoing the refused Buy Redo record must NOT double-refund; it
        // returns `false` and clears the undo stack (`0x48bd21` / `0x4932fa`).
        assert!(!lot.buy_undo().unwrap());
        assert_eq!(lot.world().budget.funds, funds_before_blocked_redo);

        // Saving after undoing a desk beneath an unpaid preview lamp: Undo
        // takes the desk (and unpaid preview lamp in its slot) off-lot;
        // `save_without_buy_cursor` must strip the preview BEFORE purging
        // history so it does not double-delete the lamp.
        let mut desk_cur2 = lot.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot.buy_move(&mut desk_cur2, [520, 472], 1).unwrap());
        let desk_id2 = lot.buy_commit(&desk_cur2).unwrap();
        let mut lamp_preview = lot.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
        assert!(lot.buy_move(&mut lamp_preview, [520, 472], 1).unwrap());
        assert_eq!(lamp_preview.container.map(|(c, _)| c), Some(desk_id2));
        assert!(lot.buy_undo().unwrap());
        assert!(lot.world().objects[&desk_id2].position[0] < 0);
        let saved_under_preview = lot
            .save_without_buy_cursor(&lamp_preview)
            .expect("save strips unpaid preview before purging undone desk");
        assert!(!saved_under_preview.world().objects.contains_key(&desk_id2));
        assert!(
            !saved_under_preview
                .world()
                .objects
                .contains_key(&lamp_preview.held)
        );
        assert!(
            !saved_under_preview
                .world()
                .objects
                .contains_key(&lamp_preview.cursor)
        );
        lot.buy_cancel(&lamp_preview).unwrap();
    }

    // Missing CTSS fallback (`0x457d3b`): lamps without CTSS 2008 take their
    // OBJD chunk label instead of sorting with an empty name.
    for guid in [0x3db3_2c6cu32, 0x4eac_803au32] {
        if let Some(item) = catalog.get(guid) {
            assert!(
                !item.name.is_empty(),
                "0x{guid:08X} should fall back to OBJD chunk label"
            );
        }
    }

    // Drag rotation facing-offset timing (`0x485da9`): Spartan Special bed has
    // a nonzero OBJD `+0x7a` facing offset, so dragging lower-right (`desired
    // == current == 2`) at rotation 0 skips rotation and keeps facing 2.
    let spartan = catalog
        .all()
        .iter()
        .find(|i| i.name.contains("Spartan Special"))
        .expect("Spartan Special bed");
    let mut bed_game = Game::open(&options(install.clone())).expect("open Goth house");
    bed_game.resize([800, 600]);
    bed_game.render();
    assert_eq!(bed_game.run(60), 60);
    bed_game.set_mode(Mode::Buy);
    // Open Seating -> Beds (subcat 3 at [383, 528]) and select slot 0 (Spartan
    // Special bed, cheapest bed at $300).
    bed_game.press([258, 524]);
    bed_game.release([258, 524]);
    bed_game.press([383, 528]);
    bed_game.release([383, 528]);
    bed_game.press([437, 523]);
    bed_game.release([437, 523]);
    assert_eq!(bed_game.buy_selected(), Some(spartan.guid));
    let lawn = recorded([480, 380]);
    bed_game.hover(lawn);
    let bed_cur = bed_game.buy_cursor().expect("bed cursor active").clone();
    assert_ne!(bed_cur.facing_offset, 0);
    assert_eq!(bed_game.lot().world().objects[&bed_cur.held].data[1], 2);
    // Lower-right drag (`desired == 2 == current`) must keep facing 2.
    bed_game.press(lawn);
    bed_game.hover([lawn[0] + 20, lawn[1] + 20]);
    assert_eq!(bed_game.lot().world().objects[&bed_cur.held].data[1], 2);
    bed_game.cancel_buy();

    // Refused commit on an invalid tile retains `buy_cursor` in `Game` so
    // cancellation still deletes both preview objects cleanly.
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    game.resize([800, 600]);
    game.render();
    assert_eq!(game.run(60), 60);
    game.render();
    let live_top350: Vec<[u8; 3]> = game.canvas().color[..800 * 350].to_vec();
    // Clicking the lot in Build or Options mode (`0x4cea5d`) must NOT open a
    // Live Mode pie menu or mutate simulation state.
    for non_live in [Mode::Build, Mode::Options] {
        game.set_mode(non_live);
        let hash_before = game.lot().world().hash();
        game.press([266, 190]);
        game.release([266, 190]);
        assert!(game.menu().is_none(), "{non_live:?} opened a Live pie menu");
        assert_eq!(
            game.lot().world().hash(),
            hash_before,
            "{non_live:?} click mutated world state"
        );
    }
    game.set_mode(Mode::Buy);
    // Entering Buy Mode must draw the dotted black lot placement grid on
    // unfloored buildable terrain immediately (`0x4ce44f` -> `0x4c4222`,
    // `0x4c42f0`, `0x4c37f2`), even before selecting an item.
    game.render();
    let buy_top350: Vec<[u8; 3]> = game.canvas().color[..800 * 350].to_vec();
    let added_grid_pixels = live_top350
        .iter()
        .zip(buy_top350.iter())
        .filter(|&(before, after)| *before != [0, 0, 0] && *after == [0, 0, 0])
        .count();
    assert!(
        added_grid_pixels > 500,
        "expected Buy Mode lot placement grid in top 350 rows, got {added_grid_pixels} new black pixels"
    );
    // Hovering and rendering in Buy Mode with no item armed must NOT run Live
    // Mode interaction checks (`Game::follow`) or mutate simulation state.
    let hash_before_buy_hover = game.lot().world().hash();
    game.hover([266, 190]);
    game.render();
    assert_eq!(game.listed(), None);
    assert_eq!(game.lot().world().hash(), hash_before_buy_hover);
    let objs_before = game.lot().world().objects.len();
    game.press([258, 524]);
    game.release([258, 524]);
    game.press([437, 523]);
    game.release([437, 523]);
    // Place one chair on the lawn, leaving the repeat-purchase chair armed and
    // invalid over the now-occupied tile; releasing there plays UI_error and
    // keeps `buy_cursor` intact until `cancel_buy` deletes both objects.
    let spot = recorded([480, 380]);
    game.hover(spot);
    let first_chair = game.buy_cursor().expect("first chair cursor").held;
    let first_reg = game.lot().world().objects[&first_chair].registry_word;
    // Unavailable Undo / Redo must not cancel the active preview or selection
    // before any purchase has been committed.
    assert!(!game.can_undo());
    assert!(!game.can_redo());
    assert!(!game.undo_buy());
    assert!(!game.redo_buy());
    assert!(game.buy_cursor().is_some());
    assert!(game.buy_selected().is_some());
    game.press(spot);
    game.release(spot);
    let second = game.buy_cursor().expect("repeat purchase armed").clone();
    assert!(!second.valid);
    game.take_sounds();
    game.press(spot);
    game.release(spot);
    assert!(game.buy_cursor().is_some());
    assert!(game.lot().world().objects.contains_key(&second.held));
    assert!(game.lot().world().objects.contains_key(&second.cursor));
    assert!(
        game.take_sounds()
            .iter()
            .any(|s| matches!(s, SoundEvent::Play { name, .. } if name == "UI_error"))
    );
    // Stationary Undo / Redo (`0x4a8545`, `0x4a856a`): Undo keeps the repeat
    // preview armed and off-lot without moving it onto the freed tile, so an
    // immediate Redo without moving the pointer restores `first_chair` and
    // re-charges `$80`. Explicitly hovering the repeat preview onto the freed
    // tile after Undo places it on-lot and causes Redo to be refused via
    // `0x4337f6` without charging `$80`.
    assert!(game.can_undo());
    assert!(game.undo_buy());
    assert_eq!(game.buy_selected(), Some(0x26bf_bb29));
    let armed_after_undo = game
        .buy_cursor()
        .expect("repeat preview stays armed after Undo")
        .clone();
    assert_eq!(armed_after_undo.held, second.held);
    assert_eq!(armed_after_undo.cursor, second.cursor);
    assert!(!armed_after_undo.valid);
    assert!(game.lot().world().objects[&second.held].position[0] < 0);
    assert!(game.lot().world().objects[&first_chair].position[0] < 0);
    assert_eq!(game.lot().world().budget.funds, 2542);
    // Stationary Redo succeeds immediately because `second.held` stayed off-lot.
    assert!(game.redo_buy());
    assert!(game.lot().world().objects[&first_chair].position[0] >= 0);
    assert_eq!(
        game.lot().world().objects[&first_chair].registry_word,
        first_reg
    );
    assert_eq!(game.lot().world().budget.funds, 2542 - 80);
    // Now Undo again and explicitly hover `spot`, moving `second.held` onto the
    // freed tile so Redo collides (`0x4337f6`) and is refused without charging.
    assert!(game.undo_buy());
    assert_eq!(game.lot().world().budget.funds, 2542);
    game.hover(spot);
    let hovered_preview = game.buy_cursor().expect("preview moved onto tile").clone();
    assert!(hovered_preview.valid);
    assert!(game.lot().world().objects[&second.held].position[0] >= 0);
    assert!(!game.redo_buy());
    assert_eq!(game.buy_selected(), Some(0x26bf_bb29));
    assert!(game.lot().world().objects[&first_chair].position[0] < 0);
    assert_eq!(game.lot().world().budget.funds, 2542);
    // Undoing a refused `Buy` Redo record must NOT double-refund `$80`: it
    // returns `false`, clears Undo (`0x4932fa` -> `0x493376`), purges the
    // detached `first_chair`, and leaves funds at `$2542`.
    assert!(!game.undo_buy());
    assert_eq!(game.lot().world().budget.funds, 2542);
    assert!(!game.lot().world().objects.contains_key(&first_chair));
    // Commit the armed `second` preview at `spot` (`$2542 -> $2462`) and
    // cancel its repeat preview so the remaining assertions test `first_chair`
    // Undo/Redo with exact accounting.
    game.press(spot);
    game.release(spot);
    let first_chair = second.held;
    let first_reg = game.lot().world().objects[&first_chair].registry_word;
    assert!(game.cancel_buy());
    assert!(game.buy_cursor().is_none());
    assert!(game.lot().world().objects[&first_chair].position[0] >= 0);
    assert_eq!(game.lot().world().objects.len(), objs_before + 1);
    let funds_after_buy = game.lot().world().budget.funds;
    assert_eq!(funds_after_buy, 2542 - 80);

    // Unrelated Undo while holding a manually rotated surface item (`0x421e72`):
    // click Back (`[234, 550]`), Lighting (`[337, 570]`), and slot 0
    // (Gloominator Desk Lamp, $85), hover over the surface at `[381, 190]`,
    // rotate manually, undo the lawn chair, and verify the lamp stays armed on
    // the surface with its manual facing intact.
    game.press([234, 550]); // Back from Seating to top-level categories
    game.release([234, 550]);
    game.press([337, 570]); // Lighting (cat 6 -> sub 0 Table Lamps)
    game.release([337, 570]);
    game.press([437, 523]); // Slot 0: Gloominator Desk Lamp ($85)
    game.release([437, 523]);
    assert_eq!(game.buy_selected(), Some(lamp.guid));
    game.hover([381, 190]);
    let lamp_cur = game.buy_cursor().expect("lamp cursor active").clone();
    assert!(lamp_cur.valid && lamp_cur.container.is_some());
    let before_dir = game.lot().world().objects[&lamp_cur.held].data[1];
    assert!(game.rotate_held(true));
    let manual_dir = game.lot().world().objects[&lamp_cur.held].data[1];
    assert_ne!(manual_dir, before_dir);
    // Undo the unrelated lawn chair purchase: lamp stays armed on the surface
    // and keeps `manual_dir`.
    assert!(game.undo_buy());
    let after_undo = game.buy_cursor().expect("lamp stays armed").clone();
    assert!(after_undo.valid);
    assert_eq!(after_undo.container, lamp_cur.container);
    assert_eq!(
        game.lot().world().objects[&after_undo.held].data[1],
        manual_dir
    );
    // Redo the lawn chair purchase: lamp still keeps `manual_dir`.
    assert!(game.redo_buy());
    let after_redo = game.buy_cursor().expect("lamp stays armed").clone();
    assert!(after_redo.valid);
    assert_eq!(after_redo.container, lamp_cur.container);
    assert_eq!(
        game.lot().world().objects[&after_redo.held].data[1],
        manual_dir
    );
    game.cancel_buy();

    // Undo / Redo of a surface beneath a manually rotated preview lamp must
    // preserve `cursor.container` so the next hover at the same spot keeps the
    // lamp's manual facing (`0x421e72`).
    game.press([234, 550]); // Back from Lighting to top-level categories
    game.release([234, 550]);
    game.press([295, 524]); // Surfaces (cat 1 -> sub 0 End Tables)
    game.release([295, 524]);
    game.press([437, 523]); // Slot 0 surface item
    game.release([437, 523]);
    let surface_spot = recorded([440, 380]);
    game.hover(surface_spot);
    game.press(surface_spot);
    game.release(surface_spot);
    game.cancel_buy();
    // Arm Gloominator Desk Lamp over the newly placed lawn surface, rotate it,
    // undo and redo the surface without moving the pointer, then hover again.
    game.press([234, 550]); // Back from Surfaces to top-level categories
    game.release([234, 550]);
    game.press([337, 570]); // Lighting -> Table Lamps
    game.release([337, 570]);
    game.press([437, 523]); // Slot 0: Gloominator Desk Lamp
    game.release([437, 523]);
    game.hover(surface_spot);
    let surf_lamp = game.buy_cursor().expect("lamp over new surface").clone();
    assert!(surf_lamp.valid && surf_lamp.container.is_some());
    let surf_before_dir = game.lot().world().objects[&surf_lamp.held].data[1];
    assert!(game.rotate_held(true));
    let surf_manual_dir = game.lot().world().objects[&surf_lamp.held].data[1];
    assert_ne!(surf_manual_dir, surf_before_dir);
    assert!(game.undo_buy());
    assert!(game.redo_buy());
    game.hover(surface_spot);
    let surf_lamp_after = game.buy_cursor().expect("lamp still armed").clone();
    assert!(surf_lamp_after.valid);
    assert_eq!(surf_lamp_after.container, surf_lamp.container);
    assert_eq!(
        game.lot().world().objects[&surf_lamp_after.held].data[1],
        surf_manual_dir
    );
    game.cancel_buy();
    // Undo the extra lawn surface so the remaining assertions check `first_chair`.
    assert!(game.undo_buy());

    // Undo / Redo after cancelling the repeat preview, plus leaving Buy Mode
    // deleting the detached instance (`0x4cea5d` -> `0x4933dc` -> `0x48c20e`).
    assert!(game.can_undo());
    assert!(game.undo_buy());
    assert!(game.lot().world().objects.contains_key(&first_chair));
    assert!(game.lot().world().objects[&first_chair].position[0] < 0);
    assert_eq!(game.lot().world().budget.funds, funds_after_buy + 80);
    assert!(!game.can_undo());
    assert!(game.can_redo());
    assert!(game.redo_buy());
    assert!(game.lot().world().objects[&first_chair].position[0] >= 0);
    assert_eq!(
        game.lot().world().objects[&first_chair].registry_word,
        first_reg
    );
    assert_eq!(game.lot().world().budget.funds, funds_after_buy);
    assert!(game.can_undo());
    assert!(game.undo_buy());
    assert!(game.lot().world().objects[&first_chair].position[0] < 0);
    game.set_mode(Mode::Live);
    assert!(!game.lot().world().objects.contains_key(&first_chair));
    assert_eq!(game.lot().world().objects.len(), objs_before);
    assert!(!game.lot().can_buy_undo());
    assert!(!game.lot().can_buy_redo());
    // Returning to Live Mode removes the Buy Mode lot placement grid.
    game.render();
    assert_eq!(&game.canvas().color[..800 * 350], live_top350.as_slice());

    // Mid-drag Undo (`0x4a8545` + `0x485bda`): place a chair at tile-centred
    // screen point `[577, 432]`, press again on the occupied tile with the
    // repeat preview, undo the first chair while holding the mouse button,
    // drag in the same facing quadrant to `[589, 444]`, and release. Native
    // re-evaluates placement at the saved tile so the second chair commits.
    game.set_mode(Mode::Buy);
    game.press([258, 524]);
    game.release([258, 524]);
    game.press([437, 523]);
    game.release([437, 523]);
    let centred_spot = [577, 432];
    game.hover(centred_spot);
    game.press(centred_spot);
    game.release(centred_spot);
    let drag_funds_after_first = game.lot().world().budget.funds;
    let repeat_chair = game.buy_cursor().expect("repeat preview armed").held;
    game.press(centred_spot);
    assert!(game.undo_buy());
    assert_eq!(game.lot().world().budget.funds, drag_funds_after_first + 80);
    game.hover([589, 444]);
    let drag_cur = game.buy_cursor().expect("drag preview revalidated").clone();
    assert!(drag_cur.valid);
    assert_eq!(drag_cur.held, repeat_chair);
    game.release([589, 444]);
    assert!(game.lot().world().objects[&repeat_chair].position[0] >= 0);
    assert_eq!(game.lot().world().budget.funds, drag_funds_after_first);
    game.cancel_buy();
    assert!(game.undo_buy());

    // Mid-drag Redo of a surface (`0x485bda` before `0x485da9`): buy an End
    // Table at `[577, 432]`, cancel its repeat preview, Undo, select Table
    // Lamps, press at `[577, 432]`, drag to `[589, 420]` (facing 4), Redo the
    // table while holding the mouse button, and release at `[589, 420]`. The
    // lamp snaps onto the restored table in `0x485bda` and then rotates back
    // to facing 4 in `0x485da9` before committing.
    game.press([234, 550]); // Back from Seating to top-level categories
    game.release([234, 550]);
    game.press([295, 524]); // Surfaces -> End Tables
    game.release([295, 524]);
    game.press([437, 523]); // Slot 0 End Table
    game.release([437, 523]);
    game.hover(centred_spot);
    game.press(centred_spot);
    game.release(centred_spot);
    game.cancel_buy();
    assert!(game.undo_buy());
    game.press([234, 550]); // Back from Surfaces to top-level categories
    game.release([234, 550]);
    game.press([337, 570]); // Lighting -> Table Lamps
    game.release([337, 570]);
    game.press([437, 523]); // Slot 0 Table Lamp
    game.release([437, 523]);
    game.hover(centred_spot);
    let drag_lamp = game.buy_cursor().expect("lamp armed").held;
    game.press(centred_spot);
    game.hover([589, 420]);
    assert_eq!(game.lot().world().objects[&drag_lamp].data[1], 4);
    assert!(game.redo_buy());
    game.release([589, 420]);
    assert!(game.lot().world().objects[&drag_lamp].position[0] >= 0);
    assert_ne!(game.lot().world().objects[&drag_lamp].data[2], 0);
    assert_eq!(game.lot().world().objects[&drag_lamp].data[1], 4);

    // Changing floors during a drag (`0x485ba2` -> `0x485bda` / `0x485e72`):
    // start dragging a Werkbunnst Chair on the ground-floor lawn, move up to
    // floor 2, drag >6 px, and release. Placement re-evaluates on floor 2
    // (no floor above the lawn) and must NOT buy the chair downstairs.
    game.press([234, 550]); // Back to top-level categories
    game.release([234, 550]);
    game.press([258, 524]); // Seating -> Chairs
    game.release([258, 524]);
    game.press([437, 523]); // Werkbunnst Chair
    game.release([437, 523]);
    let floor_spot = [577, 432];
    game.hover(floor_spot);
    let funds_before_floor_drag = game.lot().world().budget.funds;
    game.press(floor_spot);
    game.view.camera.level = 2;
    game.hover([589, 444]);
    game.release([589, 444]);
    assert_eq!(game.lot().world().budget.funds, funds_before_floor_drag);
    assert!(game.buy_cursor().is_some_and(|c| !c.valid));

    // Leaving the house view onto the catalog and re-entering recreates the
    // preview at the tool's stored facing (`0x487d3d` -> `0x486f5a`), discarding
    // any uncommitted `.`/`,` rotation on the hidden preview.
    game.view.camera.level = 1;
    game.cancel_buy();
    game.press([437, 523]); // Werkbunnst Chair
    game.release([437, 523]);
    game.hover(floor_spot);
    assert_eq!(
        game.lot().world().objects[&game.buy_cursor().unwrap().held].data[1],
        2
    );
    assert!(game.rotate_held(false)); // `,` single counterclockwise turn -> 0
    assert_eq!(
        game.lot().world().objects[&game.buy_cursor().unwrap().held].data[1],
        0
    );
    assert!(game.rotate_held(true)); // `.` -> 2
    assert!(game.rotate_held(true)); // `.` -> 4
    assert_eq!(
        game.lot().world().objects[&game.buy_cursor().unwrap().held].data[1],
        4
    );
    // Move onto the catalog panel (`0x487d4e`): saved coordinates remain intact
    // for `rotate_held` (`0x486b9e`), then returning to the lawn
    // (`0x487d3d` -> `0x486f5a`) recreates the preview at stored facing `2`.
    let saved_pos = game.buy_cursor().unwrap().position;
    game.hover([300, 520]);
    assert_eq!(game.buy_cursor().unwrap().position, saved_pos);
    assert!(game.rotate_held(true));
    game.hover(floor_spot);
    let reentered_chair = game.buy_cursor().unwrap().held;
    assert_eq!(game.lot().world().objects[&reentered_chair].data[1], 2);
    game.press(floor_spot);
    game.release(floor_spot);
    assert_eq!(game.lot().world().objects[&reentered_chair].data[1], 2);

    // Committed facing persists across switching to Live and back to Buy
    // (`0x487191` retains `tool[0x12]` and `tool[0x10]` for `0x486f5a`).
    game.press([437, 523]);
    game.release([437, 523]);
    game.hover([555, 420]);
    assert!(game.rotate_held(true)); // 2 -> 4
    let persisted_chair = game.buy_cursor().unwrap().held;
    assert_eq!(game.lot().world().objects[&persisted_chair].data[1], 4);
    game.press([555, 420]);
    game.release([555, 420]);
    assert_eq!(game.lot().world().objects[&persisted_chair].data[1], 4);
    game.set_mode(Mode::Live);
    game.set_mode(Mode::Buy);
    game.press([258, 524]); // Seating
    game.release([258, 524]);
    game.press([437, 523]); // Werkbunnst Chair
    game.release([437, 523]);
    game.hover([535, 410]);
    assert_eq!(
        game.lot().world().objects[&game.buy_cursor().unwrap().held].data[1],
        4
    );

    // Blocked placement preview (`0x42242f`): hovering the chair onto a
    // blocked spot leaves the chair off-lot and the Tile Cursor on-lot, yet
    // rendering still draws the chair's sprites at the cursor's position.
    let blocked_spot = [338, 276];
    game.hover(blocked_spot);
    let blocked_cur = game.buy_cursor().unwrap().clone();
    assert!(!blocked_cur.valid);
    assert_eq!(
        game.lot().world().objects[&blocked_cur.held].position[0],
        -16
    );
    assert!(game.lot().world().objects[&blocked_cur.cursor].position[0] >= 0);
    let canvas = game.render();
    let mut drew_held = 0usize;
    let mut exact_red_gray = 0usize;
    let mut red_dominant = 0usize;
    for y in 0..canvas.height as i32 {
        for x in 0..canvas.width as i32 {
            if canvas.id_at(x, y) == Some(blocked_cur.held) {
                drew_held += 1;
                let [r, g, b] = canvas.color[y as usize * canvas.width + x as usize];
                if r > g + 20 && r > b + 20 && (i32::from(g) - i32::from(b)).abs() <= 4 {
                    exact_red_gray += 1;
                }
                if r > b {
                    red_dominant += 1;
                }
            }
        }
    }
    assert!(
        drew_held > 0,
        "blocked held chair must still draw at the on-lot Tile Cursor position (0x42242f)"
    );
    assert!(
        exact_red_gray > 50 && red_dominant * 10 >= drew_held * 9,
        "blocked held chair must use red-tinted 0x49bd78 palette (exact_red_gray={exact_red_gray}, red_dominant={red_dominant}/{drew_held})"
    );

    // Entering Buy Mode from a collapsed Live panel refreshes `view_centre`,
    // and returning to Live restores the collapsed Live panel (`0x4cea5d`).
    game.set_mode(Mode::Live);
    game.render();
    let toggle = game
        .panel()
        .iter()
        .find(|item| item.command == Some(sims_ui::command::Command::TogglePanel))
        .map(|item| {
            [
                item.rect.x + item.rect.width / 2,
                item.rect.y + item.rect.height / 2,
            ]
        })
        .expect("TogglePanel button");
    game.press(toggle);
    game.release(toggle);
    let collapsed_centre = game.lot().view_centre();
    game.set_mode(Mode::Buy);
    assert_ne!(game.lot().view_centre(), collapsed_centre);
    game.set_mode(Mode::Live);
    assert_eq!(game.lot().view_centre(), collapsed_centre);
}

/// Strict 1,200-update replay of Track 2's Complete Collection 1.91
/// `goth-buy-move-sell-20261121` recording on its exact drawn-frame schedule
/// (`0,1,2,3+30`, verified across all 432 `watch.jsonl` rows):
/// - Live updates `1..=93` (`tick 88445..=88537`, `funds=2542`, `n_obj=345`);
/// - `F2` -> Buy Mode (`g[19]=2`) at `u=94` (partial passes `94..=903`);
/// - Seating `(258, 524)` -> Werkbunnst Chair `(437, 523)` -> hover `(480, 380)`
///   creating chair `260` + cursor `261` at `[584, 472, 1]` (`dir=2`) at `u=304`;
/// - Buy chair `260` (`$80`, `funds=2462`) at `u=454` with repeat `261`/`263`;
/// - `Escape` cancels repeat preview at `u=484` (`n_obj=346`, `260.data[26]=0`);
/// - Click `(480, 365)` picks up chair `260` at `u=544` (`cursor 261.dir=2`);
/// - Move `(550, 380)` with native grab offset -> `[632, 440, 1]` at `u=574`;
/// - `.` rotates chair `260` to `dir=4` (`cursor 261.dir=2`) at `u=604`;
/// - Click `(550, 380)` drops chair `260` at `[632, 440, 1]` `dir=4` at `u=694`;
/// - Click `(432, 230)` picks up 2-tile porch bench `173`/`174` at `u=784`
///   (showing the STR# 136 Sell Prompt for `"Parque Fresco del Aire Bench"` /
///   `"$250"`);
/// - `Delete` sells bench `173`/`174` at `u=844` (`funds=2712`, `+250`);
/// - `F1` -> Live Mode (`g[19]=0`) at `u=904` (`tick=88538`, purging `173`/`174`
///   and spawning chair helper `261`, `n_obj=345`);
/// - Live updates `904..=1200` (`tick=88834`, `lot_rng=1_819_397_574`,
///   `global_rng=270_289_136`, Mortimer `21` autonomously sitting in chair
///   `260`'s helper `261` with action `(1, 261, 4102)` at `u=1144` and `u=1200`).
#[test]
fn buy_mode_move_and_sell_replays_recorded_goth_session() {
    use sims_ui::command::Mode;
    let Some(install) = install() else {
        return;
    };
    let frames = Frames::parse("0,1,2,3+30").expect("a schedule");
    let schedule = frames.clone();
    let mut game = Game::open(&Options {
        frames,
        ..options(install)
    })
    .expect("the Goth house opens");
    game.resize([800, 600]);
    game.render();

    // Live Mode updates 1..=93 (`tick=88537`, `funds=2542`, `n_obj=345`).
    run_to(&mut game, &schedule, 93);
    assert_eq!(game.lot().world().ticks, 88_537);
    assert_eq!(game.lot().world().globals[19], 0);
    assert_eq!(game.lot().world().budget.funds, 2542);
    assert_eq!(game.lot().world().lot_rng.state(), 2_189_872_521);
    assert_eq!(game.lot().world().global_rng.state(), 270_289_136);
    assert_eq!(game.lot().world().objects.len(), 345);
    assert!(game.lot().world().objects.contains_key(&173));
    assert!(game.lot().world().objects.contains_key(&174));

    // F2 -> Buy Mode before u=94 (partial pass: tick stays 88537, g[19]=2).
    game.set_mode(Mode::Buy);
    assert_eq!(game.lot().world().globals[19], 2);
    run_to(&mut game, &schedule, 94);
    assert_eq!(game.lot().world().ticks, 88_537);
    assert_eq!(game.lot().world().globals[19], 2);

    // Click Seating (258, 524) and Werkbunnst Chair (437, 523).
    run_to(&mut game, &schedule, 184);
    game.press([258, 524]);
    game.release([258, 524]);
    run_to(&mut game, &schedule, 274);
    game.press([437, 523]);
    game.release([437, 523]);
    assert_eq!(game.buy_selected(), Some(0x26bf_bb29));

    // Popup card carries formatted STR# 160 ratings (`Comfort: 2`), dynamically
    // expands past 127 px (`0x50845d`), and renders the rating pixels visibly.
    let popup = game
        .buy_layout()
        .popup
        .as_ref()
        .expect("selected chair popup")
        .clone();
    assert!(
        popup.ratings.iter().any(|r| r == "Comfort: 2"),
        "expected Comfort: 2 on Werkbunnst Chair popup, got {:?}",
        popup.ratings
    );
    assert!(
        popup.rect.height > 127,
        "popup card must grow past 127 px to fit description + Comfort: 2, got height {}",
        popup.rect.height
    );
    let canvas = game.render();
    let rating_band_top = (popup.rect.y + popup.rect.height - 28).max(popup.rect.y);
    let rating_band_bottom = popup.rect.y + popup.rect.height;
    let rating_pixels = (rating_band_top..rating_band_bottom)
        .flat_map(|y| (popup.rect.x + 178..popup.rect.x + popup.rect.width).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let [r, g, b] = canvas.color[y as usize * canvas.width + x as usize];
            r > 120 && g > 110 && r > b
        })
        .count();
    assert!(
        rating_pixels > 20,
        "expected visible Comfort: 2 rating pixels in bottom band of popup, got {rating_pixels}"
    );

    // Move onto lawn at recorded (480, 380) before u=304 -> creates chair 260
    // + cursor 261 at [584, 472, 1], facing 2 (`n_obj=347`).
    run_to(&mut game, &schedule, 303);
    let buy_spot = recorded([480, 380]);
    game.hover(buy_spot);
    run_to(&mut game, &schedule, 304);
    let chair_id = game.buy_cursor().expect("chair cursor").held;
    assert_eq!(chair_id, 260);
    assert_eq!(game.lot().world().objects.len(), 347);
    assert_eq!(
        game.lot().world().objects[&chair_id].position,
        [584, 472, 1]
    );
    assert_eq!(game.lot().world().objects[&chair_id].data[1], 2);

    // Place chair 260 at recorded (480, 380) before u=454 (`funds=2462`,
    // `n_obj=348` with repeat chair 261 + cursor 263).
    run_to(&mut game, &schedule, 453);
    game.press(buy_spot);
    game.release(buy_spot);
    run_to(&mut game, &schedule, 454);
    assert_eq!(game.lot().world().budget.funds, 2462);
    assert_eq!(game.lot().world().globals[13], 2462);
    assert_eq!(game.lot().world().objects.len(), 348);

    // Escape cancels repeat preview before u=484 (`n_obj=346`, `260.data[26]=0`).
    run_to(&mut game, &schedule, 483);
    assert!(game.cancel_buy());
    run_to(&mut game, &schedule, 484);
    assert!(game.buy_cursor().is_none());
    assert_eq!(game.lot().world().objects.len(), 346);
    assert_eq!(game.lot().world().objects[&chair_id].data[26], 0);
    game.render();

    // Click (480, 365) (backrest of chair 260) before u=544: Hand Tool picks up
    // chair 260 with click-to-pick-up (`n_obj=347`, cursor 261 dir=2).
    run_to(&mut game, &schedule, 543);
    let pickup_chair = recorded([480, 365]);
    assert_eq!(game.under(pickup_chair), Some(chair_id));
    game.press(pickup_chair);
    game.release(pickup_chair);
    run_to(&mut game, &schedule, 544);
    let picked = game
        .buy_cursor()
        .expect("chair stays attached after click-to-pick-up")
        .clone();
    assert!(picked.is_existing());
    assert_eq!(picked.held, 260);
    assert_eq!(picked.cursor, 261);
    assert_eq!(game.lot().world().objects.len(), 347);
    assert_eq!(
        game.lot().world().objects[&chair_id].position,
        [584, 472, 1]
    );
    assert_eq!(
        game.lot().world().objects[&picked.cursor].position,
        [584, 472, 1]
    );
    assert_eq!(game.lot().world().objects[&picked.cursor].data[1], 2);
    assert_eq!(
        game.lot().world().objects[&chair_id].data[26],
        picked.cursor as i16
    );
    assert_eq!(game.lot().world().budget.funds, 2462);

    // Move to recorded (550, 380) before u=574: grab offset places chair 260 &
    // cursor 261 at exact recorded tile [632, 440, 1].
    run_to(&mut game, &schedule, 573);
    let drop_chair = recorded([550, 380]);
    game.hover(drop_chair);
    run_to(&mut game, &schedule, 574);
    assert_eq!(
        game.lot().world().objects[&chair_id].position,
        [632, 440, 1]
    );
    assert_eq!(
        game.lot().world().objects[&picked.cursor].position,
        [632, 440, 1]
    );

    // `.` rotates held chair to facing 4 while Tile Cursor keeps facing 2 at u=604.
    run_to(&mut game, &schedule, 603);
    assert!(game.rotate_held(true));
    run_to(&mut game, &schedule, 604);
    assert_eq!(game.lot().world().objects[&chair_id].data[1], 4);
    assert_eq!(game.lot().world().objects[&picked.cursor].data[1], 2);

    // Click (550, 380) drops chair 260 at [632, 440, 1] facing 4 before u=694.
    run_to(&mut game, &schedule, 693);
    game.press(drop_chair);
    game.release(drop_chair);
    run_to(&mut game, &schedule, 694);
    assert!(game.buy_cursor().is_none());
    assert!(!game.lot().world().objects.contains_key(&picked.cursor));
    assert_eq!(game.lot().world().objects.len(), 346);
    assert_eq!(
        game.lot().world().objects[&chair_id].position,
        [632, 440, 1]
    );
    assert_eq!(game.lot().world().objects[&chair_id].data[1], 4);
    assert_eq!(game.lot().world().objects[&chair_id].data[26], 0);
    assert_eq!(game.lot().world().budget.funds, 2462);
    game.render();

    // Click (432, 230) picks up 2-tile porch bench 173/174 before u=784 (`n_obj=348`).
    run_to(&mut game, &schedule, 783);
    let pickup_bench = recorded([432, 230]);
    let bench_hit = game.under(pickup_bench);
    assert!(
        matches!(bench_hit, Some(173 | 174)),
        "expected porch bench 173/174 at {pickup_bench:?}, got {bench_hit:?}"
    );
    game.press(pickup_bench);
    game.release(pickup_bench);
    run_to(&mut game, &schedule, 784);
    let bench_cur = game
        .buy_cursor()
        .expect("bench stays attached after click-to-pick-up")
        .clone();
    assert!(bench_cur.is_existing());
    assert_eq!(Some(bench_cur.held), bench_hit);
    assert_eq!(game.lot().world().objects[&173].position, [424, 360, 1]);
    assert_eq!(game.lot().world().objects[&174].position, [424, 376, 1]);
    assert_eq!(game.lot().world().objects[&bench_cur.cursor].data[1], 2);
    assert_eq!(game.lot().world().objects[&173].data[41], 250);
    assert_eq!(game.lot().world().objects.len(), 348);
    // Holding an existing object replaces the Buy catalog strip with the
    // native STR# 136 Sell Prompt (`0x487d7a` -> `0x4cf90b` -> `0x50a343`).
    assert!(game.buy_layout().widgets.is_empty());
    let prompt = game
        .buy_layout()
        .sell_prompt
        .as_ref()
        .expect("sell prompt shown while holding porch bench")
        .clone();
    assert_eq!(prompt.name, "Parque Fresco del Aire Bench");
    assert_eq!(prompt.refund, 250);
    assert!(prompt.can_delete);
    let prompt_text = prompt.lines.join("\n");
    assert!(
        prompt_text.contains("Parque Fresco del Aire Bench") && prompt_text.contains("$250"),
        "unexpected sell prompt lines: {:?}",
        prompt.lines
    );
    let canvas = game.render();
    // Native `after-sell.png` draws the bench isometric preview (`0x4384c8`)
    // at x=264..343, y=509..590 (centered at 304, 550), line 1 starting at
    // (399, 517), and line 2 starting at y=536.
    let mut portrait_min_x = i32::MAX;
    let mut portrait_max_x = 0;
    let mut portrait_min_y = i32::MAX;
    let mut portrait_max_y = 0;
    let mut portrait_pixels = 0usize;
    for y in 505..597 {
        let bg = canvas.color[y as usize * canvas.width + 245];
        for x in 252..390 {
            let [r, g, b] = canvas.color[y as usize * canvas.width + x as usize];
            let diff =
                r.abs_diff(bg[0]) as u32 + g.abs_diff(bg[1]) as u32 + b.abs_diff(bg[2]) as u32;
            if diff > 45 {
                portrait_pixels += 1;
                portrait_min_x = portrait_min_x.min(x);
                portrait_max_x = portrait_max_x.max(x);
                portrait_min_y = portrait_min_y.min(y);
                portrait_max_y = portrait_max_y.max(y);
            }
        }
    }
    assert!(
        portrait_pixels > 1500
            && (260..=268).contains(&portrait_min_x)
            && (339..=347).contains(&portrait_max_x)
            && (506..=513).contains(&portrait_min_y)
            && (586..=594).contains(&portrait_max_y),
        "expected bench portrait near x=264..343 y=509..590, got {portrait_pixels} px at x={portrait_min_x}..{portrait_max_x} y={portrait_min_y}..{portrait_max_y}"
    );
    let mut text_min_x = i32::MAX;
    let mut text_min_y = i32::MAX;
    let mut line1_max_y = 0;
    let mut line2_min_y = i32::MAX;
    let mut line2_max_y = 0;
    let mut prompt_text_pixels = 0usize;
    for y in 505..595 {
        for x in 395..780 {
            let [r, g, b] = canvas.color[y as usize * canvas.width + x as usize];
            if r > 180 && g > 180 && b > 200 {
                prompt_text_pixels += 1;
                text_min_x = text_min_x.min(x);
                text_min_y = text_min_y.min(y);
                if y < 532 {
                    line1_max_y = line1_max_y.max(y);
                } else {
                    line2_min_y = line2_min_y.min(y);
                    line2_max_y = line2_max_y.max(y);
                }
            }
        }
    }
    assert!(
        prompt_text_pixels > 80
            && (397..=402).contains(&text_min_x)
            && (515..=520).contains(&text_min_y)
            && (524..=527).contains(&line1_max_y)
            && (534..=539).contains(&line2_min_y)
            && (546..=550).contains(&line2_max_y),
        "expected 10pt sell prompt text line 1 near (399, 517..526) and line 2 near y=536..548, got {prompt_text_pixels} bright px at x={text_min_x}, line1={text_min_y}..{line1_max_y}, line2={line2_min_y}..{line2_max_y}"
    );

    // Delete sells porch bench 173/174 before u=844: refunds $250 (2462 -> 2712),
    // deletes 2-tile Tile Cursor, detaches 173/174 to [-16, -16, 1] (`n_obj=346`),
    // and plays `UI_moneyback` (`0x487e18` -> `0x492ad4`).
    run_to(&mut game, &schedule, 843);
    let _ = game.take_sounds();
    assert!(game.sell_held());
    let sell_sounds = game.take_sounds();
    assert!(
        sell_sounds.contains(&SoundEvent::Play {
            object: -4,
            name: "UI_moneyback".into(),
        }),
        "expected UI_moneyback on nonzero sale, got {sell_sounds:?}"
    );
    run_to(&mut game, &schedule, 844);
    assert!(game.buy_cursor().is_none());
    assert!(!game.lot().world().objects.contains_key(&bench_cur.cursor));
    assert_eq!(game.lot().world().budget.funds, 2712);
    assert_eq!(game.lot().world().globals[13], 2712);
    assert_eq!(game.lot().world().objects[&173].position, [-16, -16, 1]);
    assert_eq!(game.lot().world().objects[&174].position, [-16, -16, 1]);
    assert_eq!(game.lot().world().objects.len(), 346);

    // F1 -> Live Mode before u=904: purges detached 173 & 174 (`0x48c20e`),
    // sets g[19]=0, spawns chair 260 helper 261 (`n_obj=345`, `tick=88538`),
    // and replays Live updates 904..=1200 to exact oracle parity.
    run_to(&mut game, &schedule, 903);
    assert_eq!(game.lot().world().ticks, 88_537);
    game.set_mode(Mode::Live);
    assert_eq!(game.lot().world().globals[19], 0);
    assert!(!game.lot().world().objects.contains_key(&173));
    assert!(!game.lot().world().objects.contains_key(&174));
    run_to(&mut game, &schedule, 904);
    assert_eq!(game.lot().world().ticks, 88_538);
    assert_eq!(game.lot().world().objects.len(), 345);
    assert_eq!(game.lot().world().objects[&chair_id].data[26], 261);
    assert!(game.lot().world().objects.contains_key(&261));
    assert_eq!(
        game.lot().world().objects[&chair_id].position,
        [632, 440, 1]
    );
    assert_eq!(game.lot().world().objects[&chair_id].data[1], 4);
    assert_eq!(game.lot().world().budget.funds, 2712);

    // Update 1144 (scenario `after` shot): Mortimer autonomously sitting in
    // chair 260's helper 261 (`uid=1, target=261, action=4102`), exact RNGs.
    run_to(&mut game, &schedule, 1144);
    assert_eq!(game.lot().world().ticks, 88_778);
    assert_eq!(game.lot().world().objects.len(), 345);
    assert_eq!(game.lot().world().lot_rng.state(), 4_116_324_892);
    assert_eq!(game.lot().world().global_rng.state(), 270_289_136);
    let mortimer_1144 = game.lot().world().objects[&21].person.as_ref().unwrap();
    assert_eq!(
        (
            mortimer_1144.active.uid,
            mortimer_1144.active.target,
            mortimer_1144.active.action
        ),
        (1, 261, 4102)
    );

    // Final update 1200: exact tick 88834, funds 2712, object count 345,
    // exact lot & global RNGs, and Mortimer still active on chair 260 helper 261.
    run_to(&mut game, &schedule, 1200);
    assert_eq!(game.lot().world().ticks, 88_834);
    assert_eq!(game.lot().world().objects.len(), 345);
    assert_eq!(game.lot().world().budget.funds, 2712);
    assert_eq!(game.lot().world().lot_rng.state(), 1_819_397_574);
    assert_eq!(game.lot().world().global_rng.state(), 270_289_136);
    let mortimer_1200 = game.lot().world().objects[&21].person.as_ref().unwrap();
    assert_eq!(
        (
            mortimer_1200.active.uid,
            mortimer_1200.active.target,
            mortimer_1200.active.action
        ),
        (1, 261, 4102)
    );
    assert_eq!(
        game.lot().world().objects[&chair_id].position,
        [632, 440, 1]
    );
    assert_eq!(game.lot().world().objects[&chair_id].data[1], 4);
    assert!(game.problems().is_empty(), "{:?}", game.problems());
    assert!(game.stopped().is_none(), "{:?}", game.stopped());
    assert_eq!(game.caught(), 0, "{:?}", game.errors());
}

/// Hand Tool guards (`CanPickUp`, `CanDelete`), cancel-move snapback & save,
/// surface-container move/Undo/Redo with children, depreciation (`0x43219a`),
/// catalog-strip sell, drag-to-drop vs click-to-pick-up, Shift no-duplicate on
/// existing move, multipart `0x485f49` saved coordinates, Buy sort toggle, and
/// `x`/`z` catalog paging.
#[test]
fn buy_mode_hand_tool_guards_depreciation_undo_redo_and_followups() {
    use sims_ui::buy::{BuyCatalog, BuySort};
    use sims_ui::command::Mode;
    let Some(install) = install() else {
        return;
    };
    let opened = sims_render::open_house(&install, &goth_house(&install)).expect("open Goth house");
    let catalog = BuyCatalog::load(opened.assets.index(), opened.assets.catalog());

    // --- 1. VM-level guards, depreciation, container+child move & Undo/Redo ---
    let mut lot = Lot::load(&install, &goth_house(&install), &[3], Frames::default()).unwrap();
    for _ in 0..60 {
        lot.step().unwrap();
    }
    lot.set_mode(sims_vm::PanelMode::Buy).unwrap();

    // Persons cannot be picked up or deleted.
    let mortimer = lot
        .world()
        .objects
        .values()
        .find(|o| o.person.is_some())
        .expect("Mortimer on lot")
        .id;
    assert!(!lot.can_pick_up(mortimer).unwrap());
    assert!(!lot.can_delete(mortimer).unwrap());
    assert!(lot.buy_pick_up(mortimer).unwrap().is_none());

    // Place a Mesquite Desk on the lawn and a Gloominator Desk Lamp on top of it.
    let desk = catalog
        .all()
        .iter()
        .find(|i| i.name.contains("Mesquite") && i.name.contains("Desk"))
        .expect("Mesquite Desk");
    let lamp = catalog
        .all()
        .iter()
        .find(|i| i.name.contains("Gloominator Desk Lamp"))
        .expect("Gloominator Desk Lamp");
    let mut desk_cur = lot.buy_start(desk.guid, 1, 2).unwrap().unwrap();
    assert!(lot.buy_move(&mut desk_cur, [584, 472], 1).unwrap());
    let desk_id = lot.buy_commit(&desk_cur).unwrap();
    let mut lamp_cur = lot.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
    assert!(lot.buy_move(&mut lamp_cur, [584, 472], 1).unwrap());
    assert_eq!(lamp_cur.container.map(|(c, _)| c), Some(desk_id));
    let lamp_id = lot.buy_commit(&lamp_cur).unwrap();

    // Desk with a lamp on top CAN be picked up, CANNOT be deleted (`0x435440`).
    assert!(lot.can_pick_up(desk_id).unwrap());
    assert!(!lot.can_delete(desk_id).unwrap());

    // Pick up the desk (with lamp), move it to [600, 440], rotate to facing 4,
    // then cancel (`0x487191`): desk and lamp must snap back to [584, 472] dir 2!
    let mut held_desk = lot.buy_pick_up(desk_id).unwrap().expect("desk picked up");
    assert!(held_desk.is_existing());
    assert!(!lot.buy_sell(&held_desk).unwrap()); // refused while lamp is on top
    assert!(lot.buy_move(&mut held_desk, [600, 440], 1).unwrap());
    assert!(lot.buy_rotate(&mut held_desk, 4).unwrap());
    assert_eq!(lot.world().objects[&desk_id].position, [600, 440, 1]);
    assert_eq!(lot.world().objects[&lamp_id].position, [600, 440, 1]);
    // Mid-pickup save (`save_without_buy_cursor`) must snap desk & lamp back to
    // origin [584, 472] without deleting them or leaking Tile Cursor.
    let saved_mid_move = lot
        .save_without_buy_cursor(&held_desk)
        .expect("save during move");
    assert!(saved_mid_move.world().objects.contains_key(&desk_id));
    assert!(
        !saved_mid_move
            .world()
            .objects
            .contains_key(&held_desk.cursor)
    );
    assert_eq!(
        saved_mid_move.world().objects[&desk_id].position,
        [584, 472, 1]
    );
    assert_eq!(saved_mid_move.world().objects[&desk_id].data[1], 2);
    assert_eq!(
        saved_mid_move.world().objects[&lamp_id].position,
        [584, 472, 1]
    );
    // Cancel snapback restores desk & lamp to [584, 472] dir 2 with 0 funds change.
    let funds_before_cancel = lot.world().budget.funds;
    lot.buy_cancel(&held_desk).unwrap();
    assert!(!lot.world().objects.contains_key(&held_desk.cursor));
    assert_eq!(lot.world().objects[&desk_id].position, [584, 472, 1]);
    assert_eq!(lot.world().objects[&desk_id].data[1], 2);
    assert_eq!(lot.world().objects[&lamp_id].position, [584, 472, 1]);
    assert_eq!(lot.world().objects[&lamp_id].data[2], desk_id as i16);
    assert_eq!(lot.world().budget.funds, funds_before_cancel);

    // Commit moving the desk + lamp to [600, 440] facing 4, then Undo & Redo Move!
    let mut held_desk2 = lot.buy_pick_up(desk_id).unwrap().unwrap();
    assert!(lot.buy_move(&mut held_desk2, [600, 440], 1).unwrap());
    assert!(lot.buy_rotate(&mut held_desk2, 4).unwrap());
    assert_eq!(lot.buy_commit(&held_desk2).unwrap(), desk_id);
    assert_eq!(lot.world().budget.funds, funds_before_cancel);
    assert_eq!(lot.world().objects[&desk_id].position, [600, 440, 1]);
    assert_eq!(lot.world().objects[&desk_id].data[1], 4);
    assert_eq!(lot.world().objects[&lamp_id].position, [600, 440, 1]);
    assert_eq!(lot.world().objects[&lamp_id].data[2], desk_id as i16);
    assert!(lot.buy_undo().unwrap());
    assert_eq!(lot.world().budget.funds, funds_before_cancel);
    assert_eq!(lot.world().objects[&desk_id].position, [584, 472, 1]);
    assert_eq!(lot.world().objects[&desk_id].data[1], 2);
    assert_eq!(lot.world().objects[&lamp_id].position, [584, 472, 1]);
    assert_eq!(lot.world().objects[&lamp_id].data[2], desk_id as i16);
    assert!(lot.buy_redo().unwrap());
    assert_eq!(lot.world().budget.funds, funds_before_cancel);
    assert_eq!(lot.world().objects[&desk_id].position, [600, 440, 1]);
    assert_eq!(lot.world().objects[&desk_id].data[1], 4);
    assert_eq!(lot.world().objects[&lamp_id].position, [600, 440, 1]);

    // Pick up the lamp off the desk, sell it, then Undo & Redo Sell!
    let lamp_val = lot.resale_value(lamp_id).unwrap();
    assert_eq!(lamp_val, lamp.price);
    let funds_before_sell = lot.world().budget.funds;
    let held_lamp = lot.buy_pick_up(lamp_id).unwrap().unwrap();
    assert!(lot.buy_sell(&held_lamp).unwrap());
    assert_eq!(lot.world().budget.funds, funds_before_sell + lamp_val);
    assert!(lot.world().objects[&lamp_id].position[0] < 0);
    // Undo Sell: restores lamp onto desk slot and re-deducts `lamp_val`.
    assert!(lot.buy_undo().unwrap());
    assert_eq!(lot.world().budget.funds, funds_before_sell);
    assert_eq!(lot.world().objects[&lamp_id].position, [600, 440, 1]);
    assert_eq!(lot.world().objects[&lamp_id].data[2], desk_id as i16);
    // Redo Sell: detaches lamp again and re-credits `lamp_val`.
    assert!(lot.buy_redo().unwrap());
    assert_eq!(lot.world().budget.funds, funds_before_sell + lamp_val);
    assert!(lot.world().objects[&lamp_id].position[0] < 0);

    // Depreciation (`0x43219a`) + CanPickUp / CanDelete flag guards on a
    // synthetic world mutation via `Interpreter` directly.
    {
        let mut world = lot.world().clone();
        let interp =
            sims_vm::Interpreter::new(lot.program(), lot.catalog(), sims_vm::Stream::Global);
        // Porch bench 173/174: intact value == 250; broken part 174 (`data[15]=1`)
        // halves resale value to 125 (`0x43219a`).
        assert_eq!(interp.resale_value(&world, 173).unwrap(), 250);
        world.objects.get_mut(&174).unwrap().data[15] = 1;
        assert_eq!(interp.resale_value(&world, 173).unwrap(), 125);
        world.objects.get_mut(&174).unwrap().data[15] = 0;

        // `data[43] & 0x02 == 0` refuses CanPickUp and CanDelete.
        let orig_flags = world.objects[&desk_id].data[43];
        world.objects.get_mut(&desk_id).unwrap().data[43] = orig_flags & !0x02;
        assert!(!interp.can_pick_up(&world, desk_id).unwrap());
        assert!(!interp.can_delete(&world, desk_id).unwrap());
        // `data[43] & 0x08 == 0` allows CanPickUp, refuses CanDelete.
        world.objects.get_mut(&desk_id).unwrap().data[43] = (orig_flags | 0x02) & !0x08;
        assert!(interp.can_pick_up(&world, desk_id).unwrap());
        assert!(!interp.can_delete(&world, desk_id).unwrap());
        world.objects.get_mut(&desk_id).unwrap().data[43] = orig_flags;

        // In-use guards (`data[62] > 0` or `data[8] & 0x20 != 0`) refuse CanPickUp.
        world.objects.get_mut(&desk_id).unwrap().data[62] = 1;
        assert!(!interp.can_pick_up(&world, desk_id).unwrap());
        world.objects.get_mut(&desk_id).unwrap().data[62] = 0;
        let orig_d8 = world.objects[&desk_id].data[8];
        world.objects.get_mut(&desk_id).unwrap().data[8] = orig_d8 | 0x20;
        assert!(!interp.can_pick_up(&world, desk_id).unwrap());
        world.objects.get_mut(&desk_id).unwrap().data[8] = orig_d8;
    }

    // Multipart `0x485f49` dual fit refusal preserves the last saved
    // `cursor.position` written by `0x421e72`: place a 2-tile bed at a valid
    // lawn tile `[584, 440, 1]`, then move onto the last interior corner
    // `[edge, edge]` where both snapped and fallback `fits` fail. Physical
    // cursor goes off-lot `[-16, -16]`, while saved `cursor.position` stays at
    // `[584, 440, 1]` so manual `buy_turn` retries at `[584, 440, 1]`.
    let spartan = catalog
        .all()
        .iter()
        .find(|i| i.name.contains("Spartan Special"))
        .expect("Spartan Special bed");
    let mut bed_cur = lot.buy_start(spartan.guid, 1, 2).unwrap().unwrap();
    assert!(lot.buy_move(&mut bed_cur, [632, 440], 1).unwrap());
    assert_eq!(bed_cur.position, [632, 440, 1]);
    let edge = (i32::from(lot.world().size) - 2) * 16 + 8;
    assert!(!lot.buy_move(&mut bed_cur, [edge, edge], 1).unwrap());
    assert!(lot.world().objects[&bed_cur.cursor].position[0] < 0);
    assert_eq!(bed_cur.position, [632, 440, 1]);
    // Manual `buy_turn` retries at the preserved `[632, 440, 1]` (never `[edge, edge]`):
    let _ = lot.buy_turn(&mut bed_cur, 1).unwrap();
    assert_eq!(lot.world().objects[&bed_cur.cursor].position, [632, 440, 1]);
    assert_eq!(lot.world().objects[&bed_cur.held].data[1], 4);
    assert!(lot.buy_turn(&mut bed_cur, -1).unwrap());
    assert_eq!(lot.world().objects[&bed_cur.held].position, [632, 440, 1]);
    assert_eq!(lot.world().objects[&bed_cur.held].data[1], 2);
    lot.buy_cancel(&bed_cur).unwrap();

    // Non-lead multi-tile pickup pivots around the clicked part (`0x4215e6` /
    // `0x421be4`): picking up porch bench tile `174` keeps `held == 174`,
    // anchors `174` at its moved tile during rotation, and pivots `173` around it.
    let mut non_lead = lot
        .buy_pick_up(174)
        .unwrap()
        .expect("bench tile 174 picked up");
    assert_eq!(non_lead.held, 174);
    assert_eq!(non_lead.position, [424, 376, 1]);
    let mut pivoted = false;
    'search: for tx in 32..=39 {
        for ty in 24..=30 {
            let spot = [tx * 16 + 8, ty * 16 + 8];
            if !lot.buy_move(&mut non_lead, spot, 1).unwrap() {
                continue;
            }
            assert_eq!(lot.world().objects[&174].position, [spot[0], spot[1], 1]);
            let lead_before_turn = lot.world().objects[&173].position;
            assert_ne!(lead_before_turn, [spot[0], spot[1], 1]);
            if lot.buy_turn(&mut non_lead, 1).unwrap() {
                assert_eq!(lot.world().objects[&174].position, [spot[0], spot[1], 1]);
                assert_ne!(lot.world().objects[&173].position, lead_before_turn);
                pivoted = true;
                break 'search;
            }
            let _ = lot.buy_turn(&mut non_lead, -1).unwrap();
        }
    }
    assert!(
        pivoted,
        "found open lawn tile to pivot bench around tile 174"
    );
    lot.buy_cancel(&non_lead).unwrap();
    assert_eq!(lot.world().objects[&173].position, [424, 360, 1]);
    assert_eq!(lot.world().objects[&174].position, [424, 376, 1]);

    // --- 2. Game-level drag-to-drop, Shift no-duplicate, catalog-drop sell,
    //        second Buy click sort toggle, and `x`/`z` catalog paging ---
    let mut game = Game::open(&options(install.clone())).expect("open Goth house");
    game.resize([800, 600]);
    game.render();
    assert_eq!(game.run(60), 60);
    game.set_mode(Mode::Buy);
    assert_eq!(game.buy_state().sort, BuySort::ByFunction);

    // Second Buy button click (`[93, 458]`) toggles sort ByFunction <-> ByRoom
    // instead of collapsing the panel (`0x53a2c3` / `0x4cf3af`).
    game.press([93, 458]);
    game.release([93, 458]);
    assert_eq!(game.mode(), Mode::Buy);
    assert_eq!(game.buy_state().sort, BuySort::ByRoom);
    game.press([93, 458]);
    game.release([93, 458]);
    assert_eq!(game.buy_state().sort, BuySort::ByFunction);

    // Open Seating -> All (subcat 7 at `[383, 576]`): test `x`/`z` paging.
    game.press([258, 524]);
    game.release([258, 524]);
    game.press([383, 576]);
    game.release([383, 576]);
    assert_eq!(game.buy_state().page, 0);
    assert!(game.buy_layout().total_pages > 1);
    assert!(game.page_buy(1)); // `x`
    assert_eq!(game.buy_state().page, 1);
    assert!(game.page_buy(-1)); // `z`
    assert_eq!(game.buy_state().page, 0);
    assert!(!game.page_buy(-1)); // clamped at 0

    // Buy a chair on the lawn, cancel repeat preview, render so `under` sees it.
    game.press([269, 528]); // Dining Chairs subcat 0
    game.release([269, 528]);
    game.press([437, 523]); // Werkbunnst Chair ($80)
    game.release([437, 523]);
    let lawn = recorded([480, 380]);
    game.hover(lawn);
    let chair = game.buy_cursor().unwrap().held;
    game.press(lawn);
    game.release(lawn);
    game.cancel_buy();
    game.render();

    // Press-and-drag (> 6 px) directly on an existing object with Shift held:
    // native `0x485f98` sets `+0x7b = 1` so dragging > 6 px rotates in place
    // (`0x485a2e`), `0x4864d0` commits on release (`dx^2 + dy^2 >= 4`), and
    // Shift does NOT buy a duplicate copy (`0x4864d0` checks `+0x75 != 0`).
    let funds_after_chair = game.lot().world().budget.funds;
    let obj_count = game.lot().world().objects.len();
    game.set_shift(true);
    let grab = recorded([480, 365]);
    let drag_rotate_target = [grab[0] + 20, grab[1] - 20]; // upper-right -> facing 4
    game.press(grab);
    game.hover(drag_rotate_target);
    game.release(drag_rotate_target);
    game.set_shift(false);
    assert!(game.buy_cursor().is_none());
    assert!(game.buy_selected().is_none());
    assert_eq!(game.lot().world().budget.funds, funds_after_chair);
    assert_eq!(game.lot().world().objects.len(), obj_count);
    assert_eq!(game.lot().world().objects[&chair].position, [584, 472, 1]);
    assert_eq!(game.lot().world().objects[&chair].data[1], 4);
    game.render();

    // Selling by clicking the Buy catalog strip (`0x50d04d` -> `0x4cf97a` ->
    // `0x487e18`): click-to-pick-up the chair, then click on the catalog strip (300, 520).
    let chair_grab = grab;
    assert_eq!(game.under(chair_grab), Some(chair));
    game.press(chair_grab);
    game.release(chair_grab);
    assert!(
        game.buy_cursor()
            .is_some_and(sims_vm::BuyCursor::is_existing)
    );
    game.press([300, 520]);
    game.release([300, 520]);
    assert!(game.buy_cursor().is_none());
    assert_eq!(game.lot().world().budget.funds, funds_after_chair + 80);
    assert!(game.lot().world().objects[&chair].position[0] < 0);

    // Undo Sell, then test dragging & dropping onto the catalog strip (`release`).
    assert!(game.undo_buy());
    assert_eq!(game.lot().world().budget.funds, funds_after_chair);
    assert_eq!(game.lot().world().objects[&chair].position, [584, 472, 1]);
    game.render();
    game.press(chair_grab);
    game.hover([350, 540]);
    game.release([350, 540]);
    assert!(game.buy_cursor().is_none());
    assert_eq!(game.lot().world().budget.funds, funds_after_chair + 80);
    assert!(game.lot().world().objects[&chair].position[0] < 0);

    // Selling an unpaid new catalog preview cancels it with 0 refund and no
    // Undo entry.
    let funds_before_unpaid_sell = game.lot().world().budget.funds;
    game.press([437, 523]);
    game.release([437, 523]);
    game.hover(lawn);
    let unpaid = game.buy_cursor().unwrap().held;
    assert!(game.sell_held());
    assert!(game.buy_cursor().is_none());
    assert!(!game.lot().world().objects.contains_key(&unpaid));
    assert_eq!(game.lot().world().budget.funds, funds_before_unpaid_sell);

    // Pressing F2 (`set_mode(Mode::Buy)`) while already in Buy Mode is a
    // no-op (`0x4cea5d`): open category and active Hand Tool pickup persist.
    assert!(game.undo_buy()); // restore chair on lawn
    game.render();
    let state_before_f2 = game.buy_state();
    game.set_mode(Mode::Buy);
    assert_eq!(game.buy_state(), state_before_f2);
    game.press(chair_grab);
    game.release(chair_grab);
    let held_before_f2 = game.buy_cursor().expect("chair held").clone();
    game.set_mode(Mode::Buy);
    assert_eq!(game.buy_cursor(), Some(&held_before_f2));
    game.cancel_buy();

    // Undo while holding a picked-up existing object releases Hand Tool
    // ownership first: buy a fresh chair, cancel repeat, pick it up, then
    // press Undo. Cursor clears, chair detaches, funds refund ONCE, and
    // Delete / Escape / Save cannot double-refund or resurrect it.
    game.set_mode(Mode::Live);
    game.set_mode(Mode::Buy);
    game.press([258, 524]); // Seating -> Dining Chairs
    game.release([258, 524]);
    game.press([437, 523]); // Werkbunnst Chair ($80)
    game.release([437, 523]);
    let undo_spot = [555, 420];
    let funds_before_undo_test = game.lot().world().budget.funds;
    game.hover(undo_spot);
    let undo_chair = game.buy_cursor().unwrap().held;
    assert!(game.buy_cursor().unwrap().valid);
    game.press(undo_spot);
    game.release(undo_spot);
    assert_eq!(game.lot().world().budget.funds, funds_before_undo_test - 80);
    game.cancel_buy();
    game.render();
    let undo_grab = (undo_spot[0] - 25..=undo_spot[0] + 25)
        .step_by(2)
        .flat_map(|x| {
            (undo_spot[1] - 35..=undo_spot[1] + 15)
                .step_by(2)
                .map(move |y| [x, y])
        })
        .find(|&pt| game.under(pt) == Some(undo_chair))
        .expect("clickable undo_chair pixel");
    game.press(undo_grab);
    game.release(undo_grab);
    assert!(
        game.buy_cursor()
            .is_some_and(|c| c.is_existing() && c.held == undo_chair)
    );
    assert!(game.undo_buy());
    assert!(game.buy_cursor().is_none());
    assert!(game.buy_layout().sell_prompt.is_none());
    assert!(game.lot().world().objects[&undo_chair].position[0] < 0);
    assert_eq!(game.lot().world().budget.funds, funds_before_undo_test);
    assert!(!game.sell_held());
    assert!(!game.cancel_buy());
    assert_eq!(game.lot().world().budget.funds, funds_before_undo_test);
    let saved_after_undo = game.save().unwrap();
    assert!(!saved_after_undo.world().objects.contains_key(&undo_chair));

    // Refused Move Redo (destination blocked by an unpaid preview) keeps the
    // paid object on-lot at `before`, and returning to Live Mode does NOT
    // delete it (`0x48c20e` never deletes `Move` records).
    assert!(game.redo_buy());
    assert_eq!(game.lot().world().budget.funds, funds_before_undo_test - 80);
    let orig_pos = game.lot().world().objects[&undo_chair].position;
    game.render();
    game.press(undo_grab);
    game.release(undo_grab);
    let dest_spot = [577, 432];
    game.hover(dest_spot);
    let moved_pos = game.lot().world().objects[&undo_chair].position;
    assert_ne!(moved_pos, orig_pos);
    game.press(dest_spot);
    game.release(dest_spot);
    assert_eq!(game.lot().world().objects[&undo_chair].position, moved_pos);
    // Undo the move -> chair returns to `orig_pos`.
    assert!(game.undo_buy());
    assert_eq!(game.lot().world().objects[&undo_chair].position, orig_pos);
    // Arm a new chair preview directly on `moved_pos` so Move Redo is blocked.
    game.press([437, 523]);
    game.release([437, 523]);
    let blocker_spot = [
        dest_spot[0] - (undo_grab[0] - undo_spot[0]),
        dest_spot[1] - (undo_grab[1] - undo_spot[1]),
    ];
    game.hover(blocker_spot);
    let blocker = game.buy_cursor().unwrap().held;
    assert_eq!(game.lot().world().objects[&blocker].position, moved_pos);
    assert!(!game.redo_buy());
    assert_eq!(game.lot().world().objects[&undo_chair].position, orig_pos);
    game.cancel_buy();
    game.set_mode(Mode::Live);
    assert!(game.lot().world().objects.contains_key(&undo_chair));
    assert_eq!(game.lot().world().objects[&undo_chair].position, orig_pos);

    // Non-deletable Sell Prompt (`STR# 136` entry 2): holding an end table with
    // a lamp on top shows `"<name> cannot be deleted."` and refuses `sell_held`.
    game.set_mode(Mode::Buy);
    game.press([295, 524]); // Surfaces -> End Tables
    game.release([295, 524]);
    game.press([437, 523]); // Slot 0 End Table
    game.release([437, 523]);
    let table_spot = recorded([440, 380]);
    game.hover(table_spot);
    let table_id = game.buy_cursor().unwrap().held;
    game.press(table_spot);
    game.release(table_spot);
    game.cancel_buy();
    game.press([234, 550]); // Back
    game.release([234, 550]);
    game.press([337, 570]); // Lighting -> Table Lamps
    game.release([337, 570]);
    game.press([437, 523]); // Slot 0 Gloominator Desk Lamp
    game.release([437, 523]);
    game.hover(table_spot);
    assert_eq!(
        game.buy_cursor().unwrap().container.map(|(c, _)| c),
        Some(table_id)
    );
    game.press(table_spot);
    game.release(table_spot);
    game.cancel_buy();
    game.render();
    let table_pt = (table_spot[0] - 25..=table_spot[0] + 25)
        .step_by(2)
        .flat_map(|x| {
            (table_spot[1] - 25..=table_spot[1] + 25)
                .step_by(2)
                .map(move |y| [x, y])
        })
        .find(|&pt| game.under(pt) == Some(table_id))
        .expect("clickable end table pixel");
    game.press(table_pt);
    game.release(table_pt);
    let cant_del = game
        .buy_layout()
        .sell_prompt
        .as_ref()
        .expect("sell prompt for table with lamp")
        .clone();
    assert!(!cant_del.can_delete);
    assert!(
        cant_del.lines.join(" ").ends_with("cannot be deleted."),
        "unexpected non-deletable prompt: {:?}",
        cant_del.lines
    );
    assert!(game.sell_held()); // handled, refused with UI_error
    assert!(game.buy_cursor().is_some()); // still held
    game.cancel_buy();

    // --- 3. Sol Round 2 regressions: refused snapback rollback, refused Buy
    //        Redo no double-refund, same-minute comeandsee registration order,
    //        and F1 preserving Redo until after update_comeandsee ---
    {
        let mut lot2 = Lot::load(&install, &goth_house(&install), &[3], Frames::default()).unwrap();
        for _ in 0..60 {
            lot2.step().unwrap();
        }
        lot2.set_mode(sims_vm::PanelMode::Buy).unwrap();
        let chair_guid = 0x26bf_bb29u32;

        // Single-cursor ownership invariant: while a BuyCursor is active,
        // `buy_start` and `buy_pick_up` refuse opening a second simultaneous
        // cursor (`Ok(None)`), and `buy_snap_back` cleanly restores the chair.
        let mut c = lot2.buy_start(chair_guid, 1, 2).unwrap().unwrap();
        assert!(lot2.buy_move(&mut c, [584, 472], 1).unwrap());
        let chair_id = lot2.buy_commit(&c).unwrap();
        let mut held = lot2.buy_pick_up(chair_id).unwrap().unwrap();
        assert!(lot2.buy_move(&mut held, [632, 440], 1).unwrap());
        assert!(
            lot2.buy_start(chair_guid, 1, 2).unwrap().is_none(),
            "buy_start must refuse a second simultaneous BuyCursor"
        );
        assert!(
            lot2.buy_pick_up(chair_id).unwrap().is_none(),
            "buy_pick_up must refuse while another BuyCursor is active"
        );
        lot2.buy_snap_back(&held).unwrap();
        assert_eq!(lot2.world().objects[&chair_id].position, [584, 472, 1]);
        assert!(!lot2.world().objects.contains_key(&held.cursor));

        // P2 #4: refused Buy Redo transferred to Undo must NOT double-refund.
        let funds_before_undo = lot2.world().budget.funds;
        assert!(lot2.buy_undo().unwrap());
        assert_eq!(lot2.world().budget.funds, funds_before_undo + 80);
        let mut blocker2 = lot2.buy_start(chair_guid, 1, 2).unwrap().unwrap();
        assert!(lot2.buy_move(&mut blocker2, [584, 472], 1).unwrap());
        assert!(!lot2.buy_redo().unwrap());
        assert_eq!(lot2.world().budget.funds, funds_before_undo + 80);
        assert!(!lot2.buy_undo().unwrap());
        assert_eq!(lot2.world().budget.funds, funds_before_undo + 80);
        lot2.buy_cancel(&blocker2).unwrap();

        // P2 #3: two same-minute purchases receive `comeandsee` markers in
        // `world.order` (newest-first) order (`0x40726a`).
        let mut a = lot2.buy_start(chair_guid, 1, 2).unwrap().unwrap();
        assert!(lot2.buy_move(&mut a, [584, 472], 1).unwrap());
        let id_a = lot2.buy_commit(&a).unwrap();
        let mut b = lot2.buy_start(chair_guid, 1, 2).unwrap().unwrap();
        assert!(lot2.buy_move(&mut b, [632, 440], 1).unwrap());
        let id_b = lot2.buy_commit(&b).unwrap();
        assert!(id_b > id_a);
        lot2.set_mode(sims_vm::PanelMode::Live).unwrap();
        let marker_of = |id: u16| -> u16 {
            lot2.world().objects[&id]
                .relationships
                .iter()
                .find_map(|r| {
                    let target = u16::try_from(r.target).ok()?;
                    lot2.world()
                        .objects
                        .get(&target)
                        .filter(|o| o.guid == sims_vm::COMEANDSEE_GUID)
                        .map(|_| target)
                })
                .expect("comeandsee marker attached")
        };
        let marker_a = marker_of(id_a);
        let marker_b = marker_of(id_b);
        assert!(
            marker_b < marker_a,
            "newer chair {id_b} must receive lower comeandsee ID than older chair {id_a} (got {marker_b} vs {marker_a})"
        );
    }

    // P2 #2: F1 while holding an existing object with an undone chair in Redo
    // snaps the held object back WITHOUT purging Redo before `update_comeandsee`
    // (`0x4cea5d` -> `0x40726a` -> `0x4933dc`), so the surviving chair's marker
    // does NOT reuse the undone chair's ID.
    {
        let mut g2 = Game::open(&options(install.clone())).expect("open Goth house");
        g2.resize([800, 600]);
        g2.render();
        assert_eq!(g2.run(60), 60);
        g2.set_mode(Mode::Buy);
        g2.press([258, 524]); // Seating
        g2.release([258, 524]);
        g2.press([437, 523]); // Werkbunnst Chair
        g2.release([437, 523]);
        g2.hover([577, 432]);
        let chair1 = g2.buy_cursor().unwrap().held;
        g2.press([577, 432]);
        g2.release([577, 432]);
        g2.hover([555, 420]);
        let chair2 = g2.buy_cursor().unwrap().held;
        g2.press([555, 420]);
        g2.release([555, 420]);
        g2.cancel_buy();
        assert!(g2.undo_buy()); // chair2 detached in Redo
        assert!(g2.lot().world().objects[&chair2].position[0] < 0);
        g2.render();
        let bench_pt = recorded([432, 230]);
        g2.press(bench_pt);
        g2.release(bench_pt);
        assert!(g2.buy_cursor().is_some_and(sims_vm::BuyCursor::is_existing));
        g2.set_mode(Mode::Live);
        assert!(!g2.lot().world().objects.contains_key(&chair2));
        let marker = g2.lot().world().objects[&chair1]
            .relationships
            .iter()
            .find_map(|r| {
                let target = u16::try_from(r.target).ok()?;
                g2.lot()
                    .world()
                    .objects
                    .get(&target)
                    .filter(|o| o.guid == sims_vm::COMEANDSEE_GUID)
                    .map(|_| target)
            })
            .expect("comeandsee marker on chair1");
        assert_ne!(
            marker, chair2,
            "comeandsee marker must not reuse undone chair {chair2}'s ID before 0x4933dc purge"
        );
    }

    // --- 4. Sol Round 3 regressions: refused Buy->Move->Sell Redo chain,
    //        child-only snapback rollback + save refusal, and F1 unpaid preview
    //        teardown AFTER `update_comeandsee` ---
    {
        // P2 #1: Buy -> Move -> Sell -> Undo x3 -> block origin with unpaid
        // preview -> Redo x3 must refuse Buy, Move, AND Sell without creating
        // free furniture or issuing an extra refund.
        let mut g3 = Game::open(&options(install.clone())).expect("open Goth house");
        g3.resize([800, 600]);
        g3.render();
        assert_eq!(g3.run(60), 60);
        g3.set_mode(Mode::Buy);
        let start_funds = g3.lot().world().budget.funds;
        g3.press([258, 524]); // Seating
        g3.release([258, 524]);
        g3.press([437, 523]); // Werkbunnst Chair ($80)
        g3.release([437, 523]);
        g3.hover([577, 432]);
        let chair = g3.buy_cursor().unwrap().held;
        g3.press([577, 432]);
        g3.release([577, 432]);
        assert_eq!(g3.lot().world().budget.funds, start_funds - 80);
        g3.cancel_buy();
        g3.render();
        // Pick up chair and move it to [555, 420].
        g3.press([577, 432]);
        g3.release([577, 432]);
        assert_eq!(g3.buy_cursor().unwrap().held, chair);
        g3.hover([555, 420]);
        g3.press([555, 420]);
        g3.release([555, 420]);
        assert!(g3.buy_cursor().is_none());
        g3.render();
        // Pick up chair and sell it.
        g3.press([555, 420]);
        g3.release([555, 420]);
        assert_eq!(g3.buy_cursor().unwrap().held, chair);
        assert!(g3.sell_held());
        assert_eq!(g3.lot().world().budget.funds, start_funds);
        // Undo Sell, Undo Move, Undo Buy.
        assert!(g3.undo_buy());
        assert_eq!(g3.lot().world().budget.funds, start_funds - 80);
        assert!(g3.undo_buy());
        assert!(g3.undo_buy());
        assert_eq!(g3.lot().world().budget.funds, start_funds);
        assert!(g3.lot().world().objects[&chair].position[0] < 0);
        // Hover unpaid chair preview over original tile [577, 432], then Redo x3.
        g3.press([437, 523]);
        g3.release([437, 523]);
        g3.hover([577, 432]);
        assert!(g3.buy_cursor().unwrap().valid);
        assert!(!g3.redo_buy()); // Buy Redo refused
        assert!(g3.lot().world().objects[&chair].position[0] < 0);
        assert_eq!(g3.lot().world().budget.funds, start_funds);
        assert!(!g3.redo_buy()); // Move Redo must NOT resurrect detached chair
        assert!(g3.lot().world().objects[&chair].position[0] < 0);
        assert_eq!(g3.lot().world().budget.funds, start_funds);
        assert!(!g3.redo_buy()); // Sell Redo must NOT credit extra $80
        assert!(g3.lot().world().objects[&chair].position[0] < 0);
        assert_eq!(g3.lot().world().budget.funds, start_funds);
        g3.cancel_buy();
        assert!(!g3.undo_buy());
        assert_eq!(g3.lot().world().budget.funds, start_funds);
    }

    {
        // P2 #2: Child-only refusal during snapback must fail atomically and
        // leave the paid surface child on-lot (never detached off-lot or
        // omitted on save).
        let mut lot3 = Lot::load(&install, &goth_house(&install), &[3], Frames::default()).unwrap();
        for _ in 0..60 {
            lot3.step().unwrap();
        }
        lot3.set_mode(sims_vm::PanelMode::Buy).unwrap();
        let desk = catalog
            .all()
            .iter()
            .find(|i| i.name.contains("Mesquite") && i.name.contains("Desk"))
            .expect("Mesquite Desk");
        let lamp = catalog
            .all()
            .iter()
            .find(|i| i.name.contains("Gloominator"))
            .expect("Gloominator Desk Lamp");
        let mut d1 = lot3.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot3.buy_move(&mut d1, [584, 472], 1).unwrap());
        let desk1 = lot3.buy_commit(&d1).unwrap();
        let mut d2 = lot3.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot3.buy_move(&mut d2, [552, 472], 1).unwrap());
        let desk2 = lot3.buy_commit(&d2).unwrap();
        let mut l1 = lot3.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
        assert!(lot3.buy_move(&mut l1, [584, 472], 1).unwrap());
        assert_eq!(l1.container.map(|(c, _)| c), Some(desk1));
        let paid_lamp = lot3.buy_commit(&l1).unwrap();
        // Single-cursor surface ownership: holding `desk1` (with `paid_lamp`
        // in its slot) prevents opening a second cursor on `paid_lamp` or
        // starting an unpaid preview; snapping `desk1` back restores both.
        let mut held_desk = lot3.buy_pick_up(desk1).unwrap().unwrap();
        assert!(lot3.buy_move(&mut held_desk, [600, 440], 1).unwrap());
        assert!(lot3.buy_pick_up(paid_lamp).unwrap().is_none());
        assert!(lot3.buy_start(lamp.guid, 1, 2).unwrap().is_none());
        lot3.buy_snap_back(&held_desk).unwrap();
        assert_eq!(lot3.world().objects[&desk1].position, [584, 472, 1]);
        assert_eq!(lot3.world().objects[&paid_lamp].position, [584, 472, 1]);

        // Unpaid preview sitting on a surface whose Buy is then undone:
        // `buy_commit` MUST refuse (neither charging funds nor deleting the
        // object) because the container desk is now off-lot, and `buy_pick_up`
        // on the desk while the preview is active is also refused.
        let mut l2 = lot3.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
        assert!(lot3.buy_move(&mut l2, [552, 472], 1).unwrap());
        assert_eq!(l2.container.map(|(c, _)| c), Some(desk2));
        assert!(lot3.buy_pick_up(desk2).unwrap().is_none());
        assert!(lot3.buy_undo().unwrap()); // undoes paid_lamp
        assert!(lot3.buy_undo().unwrap()); // undoes desk2 (detaching desk2 + preview l2)
        assert!(lot3.world().objects[&desk2].position[0] < 0);
        let funds_before_offlot_commit = lot3.world().budget.funds;
        assert!(
            lot3.buy_commit(&l2).is_err(),
            "buy_commit must refuse when surface container was detached by Undo"
        );
        assert_eq!(lot3.world().budget.funds, funds_before_offlot_commit);
        lot3.buy_cancel(&l2).unwrap();
    }

    {
        // P2 #3: F1 while an unpaid repeat preview is hovered over an undone
        // chair's tile runs `go_live` -> `update_comeandsee` -> `0x4933dc`
        // BEFORE tearing down the Buy tool (`0x4cea5d`), so chair1's marker
        // does NOT reuse either the undone chair's ID or the preview's IDs.
        let mut g4 = Game::open(&options(install.clone())).expect("open Goth house");
        g4.resize([800, 600]);
        g4.render();
        assert_eq!(g4.run(60), 60);
        g4.set_mode(Mode::Buy);
        g4.press([258, 524]); // Seating
        g4.release([258, 524]);
        g4.press([437, 523]); // Werkbunnst Chair
        g4.release([437, 523]);
        g4.hover([577, 432]);
        let chair1 = g4.buy_cursor().unwrap().held;
        g4.press([577, 432]);
        g4.release([577, 432]);
        g4.hover([555, 420]);
        let chair2 = g4.buy_cursor().unwrap().held;
        g4.press([555, 420]);
        g4.release([555, 420]);
        assert!(g4.undo_buy()); // chair2 detached in Redo; repeat preview stays armed
        g4.hover([555, 420]);
        let preview = g4
            .buy_cursor()
            .expect("unpaid repeat preview active")
            .clone();
        assert!(!preview.is_existing());
        assert!(preview.valid);
        g4.set_mode(Mode::Live);
        assert!(g4.buy_cursor().is_none());
        assert!(!g4.lot().world().objects.contains_key(&chair2));
        assert!(!g4.lot().world().objects.contains_key(&preview.held));
        assert!(!g4.lot().world().objects.contains_key(&preview.cursor));
        let marker = g4.lot().world().objects[&chair1]
            .relationships
            .iter()
            .find_map(|r| {
                let target = u16::try_from(r.target).ok()?;
                g4.lot()
                    .world()
                    .objects
                    .get(&target)
                    .filter(|o| o.guid == sims_vm::COMEANDSEE_GUID)
                    .map(|_| target)
            })
            .expect("comeandsee marker on chair1");
        assert_ne!(marker, chair2);
        assert_ne!(marker, preview.held);
        assert_ne!(marker, preview.cursor);
    }

    // --- 5. Sol Round 4 regressions: Redo Move child resurrection, Redo Sell
    //        non-empty surface guard, F1 unpaid preview on undone surface, and
    //        refused Buy Redo atomic parent/child rollback ---
    {
        let desk = catalog
            .all()
            .iter()
            .find(|i| i.name.contains("Mesquite") && i.name.contains("Desk"))
            .expect("Mesquite Desk");
        let lamp = catalog
            .all()
            .iter()
            .find(|i| i.name.contains("Gloominator"))
            .expect("Gloominator Desk Lamp");

        // P1 #1: Redo Move of a desk must NOT resurrect a refunded lamp child.
        let mut lot_p1a =
            Lot::load(&install, &goth_house(&install), &[3], Frames::default()).unwrap();
        for _ in 0..60 {
            lot_p1a.step().unwrap();
        }
        lot_p1a.set_mode(sims_vm::PanelMode::Buy).unwrap();
        let mut d1 = lot_p1a.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p1a.buy_move(&mut d1, [584, 472], 1).unwrap());
        let desk1 = lot_p1a.buy_commit(&d1).unwrap();
        let mut d2 = lot_p1a.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p1a.buy_move(&mut d2, [552, 472], 1).unwrap());
        let desk2 = lot_p1a.buy_commit(&d2).unwrap();
        let mut l1 = lot_p1a.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p1a.buy_move(&mut l1, [584, 472], 1).unwrap());
        assert_eq!(l1.container.map(|(c, _)| c), Some(desk1));
        let lamp_id = lot_p1a.buy_commit(&l1).unwrap();
        let mut lm = lot_p1a.buy_pick_up(lamp_id).unwrap().unwrap();
        assert!(lot_p1a.buy_move(&mut lm, [552, 472], 1).unwrap());
        assert_eq!(lm.container.map(|(c, _)| c), Some(desk2));
        lot_p1a.buy_commit(&lm).unwrap();
        let mut dm = lot_p1a.buy_pick_up(desk2).unwrap().unwrap();
        assert!(lot_p1a.buy_move(&mut dm, [600, 440], 1).unwrap());
        lot_p1a.buy_commit(&dm).unwrap();
        // Undo desk2 move, lamp move, and lamp buy -> lamp refunded & off-lot.
        assert!(lot_p1a.buy_undo().unwrap());
        assert!(lot_p1a.buy_undo().unwrap());
        assert!(lot_p1a.buy_undo().unwrap());
        let funds_after_refund = lot_p1a.world().budget.funds;
        assert!(lot_p1a.world().objects[&lamp_id].position[0] < 0);
        // Block (desk1, 0) with unpaid preview lamp, then Redo x3.
        let mut blocker = lot_p1a.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p1a.buy_move(&mut blocker, [584, 472], 1).unwrap());
        assert!(!lot_p1a.buy_redo().unwrap()); // lamp buy refused
        assert!(!lot_p1a.buy_redo().unwrap()); // lamp move refused
        assert!(!lot_p1a.buy_redo().unwrap()); // desk2 move MUST refuse (child lamp is detached!)
        assert!(lot_p1a.world().objects[&lamp_id].position[0] < 0);
        assert_eq!(lot_p1a.world().objects[&desk2].position, [552, 472, 1]);
        assert_eq!(lot_p1a.world().budget.funds, funds_after_refund);
        lot_p1a.buy_cancel(&blocker).unwrap();

        // P1 #2: Redo Sell of a desk that still holds a paid lamp (after a
        // refused Move Redo) must refuse via `can_delete` instead of detaching
        // and permanently deleting the paid lamp.
        let mut lot_p1b =
            Lot::load(&install, &goth_house(&install), &[3], Frames::default()).unwrap();
        for _ in 0..60 {
            lot_p1b.step().unwrap();
        }
        lot_p1b.set_mode(sims_vm::PanelMode::Buy).unwrap();
        let mut d1 = lot_p1b.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p1b.buy_move(&mut d1, [584, 472], 1).unwrap());
        let desk1 = lot_p1b.buy_commit(&d1).unwrap();
        let mut d2 = lot_p1b.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p1b.buy_move(&mut d2, [552, 472], 1).unwrap());
        let desk2 = lot_p1b.buy_commit(&d2).unwrap();
        let mut l1 = lot_p1b.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p1b.buy_move(&mut l1, [584, 472], 1).unwrap());
        let lamp_id = lot_p1b.buy_commit(&l1).unwrap();
        let mut lm = lot_p1b.buy_pick_up(lamp_id).unwrap().unwrap();
        assert!(lot_p1b.buy_move(&mut lm, [552, 472], 1).unwrap());
        assert_eq!(lm.container.map(|(c, _)| c), Some(desk2));
        lot_p1b.buy_commit(&lm).unwrap();
        let d1_pick = lot_p1b.buy_pick_up(desk1).unwrap().unwrap();
        assert!(lot_p1b.buy_sell(&d1_pick).unwrap());
        // Undo Sell (restores desk1) and Undo Move (puts lamp back on desk1).
        assert!(lot_p1b.buy_undo().unwrap());
        assert!(lot_p1b.buy_undo().unwrap());
        let funds_before_redo = lot_p1b.world().budget.funds;
        assert_eq!(lot_p1b.world().objects[&lamp_id].data[2], desk1 as i16);
        let mut blocker = lot_p1b.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p1b.buy_move(&mut blocker, [552, 472], 1).unwrap());
        assert!(!lot_p1b.buy_redo().unwrap()); // lamp move refused; lamp stays on desk1
        assert!(!lot_p1b.buy_redo().unwrap()); // desk1 sell MUST refuse (`can_delete` false)
        assert!(lot_p1b.world().objects[&desk1].position[0] >= 0);
        assert!(lot_p1b.world().objects[&lamp_id].position[0] >= 0);
        assert_eq!(lot_p1b.world().budget.funds, funds_before_redo);
        lot_p1b.buy_cancel(&blocker).unwrap();
        lot_p1b.set_mode(sims_vm::PanelMode::Live).unwrap();
        assert!(lot_p1b.world().objects.contains_key(&desk1));
        assert!(lot_p1b.world().objects.contains_key(&lamp_id));

        // Single-cursor invariant prevents committing a lamp onto an uncommitted
        // desk preview or opening a second blocker cursor while moving a lamp.
        let mut lot_p2b =
            Lot::load(&install, &goth_house(&install), &[3], Frames::default()).unwrap();
        for _ in 0..60 {
            lot_p2b.step().unwrap();
        }
        lot_p2b.set_mode(sims_vm::PanelMode::Buy).unwrap();
        let mut d2 = lot_p2b.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p2b.buy_move(&mut d2, [552, 472], 1).unwrap());
        let desk2 = lot_p2b.buy_commit(&d2).unwrap();
        let mut d1 = lot_p2b.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p2b.buy_move(&mut d1, [584, 472], 1).unwrap());
        assert!(
            lot_p2b.buy_start(lamp.guid, 1, 2).unwrap().is_none(),
            "cannot start a lamp preview while a desk preview is still active"
        );
        let desk1 = lot_p2b.buy_commit(&d1).unwrap();
        let mut l1 = lot_p2b.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
        assert!(lot_p2b.buy_move(&mut l1, [584, 472], 1).unwrap());
        let paid_lamp = lot_p2b.buy_commit(&l1).unwrap();
        let mut moved_lamp = lot_p2b.buy_pick_up(paid_lamp).unwrap().unwrap();
        assert!(lot_p2b.buy_move(&mut moved_lamp, [552, 472], 1).unwrap());
        assert_eq!(moved_lamp.container.map(|(c, _)| c), Some(desk2));
        assert!(
            lot_p2b.buy_start(lamp.guid, 1, 2).unwrap().is_none(),
            "cannot start a second preview cursor while holding a picked-up lamp"
        );
        lot_p2b.buy_snap_back(&moved_lamp).unwrap();
        assert_eq!(lot_p2b.world().objects[&desk1].position, [584, 472, 1]);
        assert_eq!(lot_p2b.world().objects[&paid_lamp].position, [584, 472, 1]);

        // P2 #1: F1 with an unpaid lamp preview sitting on an undone end table
        // must switch to Live Mode cleanly without failing `missing object`.
        let mut g5 = Game::open(&options(install.clone())).expect("open Goth house");
        g5.resize([800, 600]);
        g5.render();
        assert_eq!(g5.run(60), 60);
        g5.set_mode(Mode::Buy);
        g5.press([295, 524]); // Surfaces -> End Tables
        g5.release([295, 524]);
        g5.press([437, 523]); // Slot 0 End Table
        g5.release([437, 523]);
        g5.hover([577, 432]);
        let table_id = g5.buy_cursor().unwrap().held;
        g5.press([577, 432]);
        g5.release([577, 432]);
        g5.cancel_buy();
        g5.press([234, 550]); // Back
        g5.release([234, 550]);
        g5.press([337, 570]); // Lighting -> Table Lamps
        g5.release([337, 570]);
        g5.press([437, 523]); // Slot 0 Desk Lamp
        g5.release([437, 523]);
        g5.hover([577, 432]);
        let lamp_prev = g5.buy_cursor().expect("lamp preview on table").clone();
        assert_eq!(lamp_prev.container.map(|(c, _)| c), Some(table_id));
        assert!(g5.undo_buy()); // table detached off-lot with lamp_prev in slot 0
        assert!(g5.lot().world().objects[&table_id].position[0] < 0);
        // Press F1 without moving the pointer.
        g5.set_mode(Mode::Live);
        assert_eq!(g5.mode(), Mode::Live);
        assert!(g5.buy_cursor().is_none());
        assert!(!g5.lot().world().objects.contains_key(&table_id));
        assert!(!g5.lot().world().objects.contains_key(&lamp_prev.held));
        assert!(!g5.lot().world().objects.contains_key(&lamp_prev.cursor));
    }

    // --- 6. Sol Round 5 regressions: drag release above 100px Buy strip must
    //        not sell, Buy Redo affordability guard, and off-lot buy_pick_up
    //        refusal ---
    {
        // P1: dragging the Goth porch bench and releasing at (350, 470) in the
        // house viewport (above the 100px Buy strip at y=500..600) must NOT
        // sell it.
        let mut g6 = Game::open(&options(install.clone())).expect("open Goth house");
        g6.resize([800, 600]);
        g6.render();
        assert_eq!(g6.run(60), 60);
        g6.set_mode(Mode::Buy);
        g6.render();
        let funds_before_drag = g6.lot().world().budget.funds;
        let bench_pt = recorded([432, 230]);
        g6.press(bench_pt);
        assert!(g6.buy_cursor().is_some_and(sims_vm::BuyCursor::is_existing));
        g6.hover([350, 470]);
        g6.release([350, 470]);
        assert_eq!(
            g6.lot().world().budget.funds,
            funds_before_drag,
            "releasing inside house viewport at y=470 must not sell bench"
        );
        g6.set_mode(Mode::Live);
        assert!(g6.lot().world().objects.contains_key(&173));
        assert!(g6.lot().world().objects.contains_key(&174));

        // P2 #1 & P2 #2: Buy Redo must refuse rather than overdraw household
        // funds when an earlier Sell Redo was refused, and `Lot::buy_pick_up`
        // must reject detached off-lot history instances.
        let mut lot5 = Lot::load(&install, &goth_house(&install), &[3], Frames::default()).unwrap();
        for _ in 0..60 {
            lot5.step().unwrap();
        }
        lot5.set_mode(sims_vm::PanelMode::Buy).unwrap();
        let chair_guid = 0x26bf_bb29u32;
        let desk = catalog
            .all()
            .iter()
            .find(|i| i.name.contains("Mesquite") && i.name.contains("Desk"))
            .expect("Mesquite Desk");
        let lamp = catalog
            .all()
            .iter()
            .find(|i| i.name.contains("Gloominator"))
            .expect("Gloominator Desk Lamp");

        // P2 #2: selling a chair detaches it; `buy_pick_up` on that ID must
        // return `Ok(None)` instead of allowing a second sale.
        let mut ch = lot5.buy_start(chair_guid, 1, 2).unwrap().unwrap();
        assert!(lot5.buy_move(&mut ch, [600, 440], 1).unwrap());
        let ch_id = lot5.buy_commit(&ch).unwrap();
        let ch_pick = lot5.buy_pick_up(ch_id).unwrap().unwrap();
        assert!(lot5.buy_sell(&ch_pick).unwrap());
        let funds_after_first_sell = lot5.world().budget.funds;
        assert!(lot5.world().objects[&ch_id].position[0] < 0);
        assert!(
            lot5.buy_pick_up(ch_id).unwrap().is_none(),
            "detached sold chair must not be pickable again"
        );
        assert_eq!(lot5.world().budget.funds, funds_after_first_sell);

        // Retained `BuyCursor` across `buy_undo` at the public `Lot` API must
        // refuse double-selling or free resurrection, while `buy_cancel` still
        // cleans up the stale `"Tile Cursor"` without placing the chair back.
        let mut ch_stale_src = lot5.buy_start(chair_guid, 1, 2).unwrap().unwrap();
        assert!(lot5.buy_move(&mut ch_stale_src, [600, 440], 1).unwrap());
        let stale_chair = lot5.buy_commit(&ch_stale_src).unwrap();
        let mut stale_cur = lot5.buy_pick_up(stale_chair).unwrap().unwrap();
        assert!(lot5.buy_undo().unwrap()); // refunds $80, detaches stale_chair
        let funds_after_stale_undo = lot5.world().budget.funds;
        assert!(lot5.world().objects[&stale_chair].position[0] < 0);
        assert!(!lot5.buy_sell(&stale_cur).unwrap());
        assert!(!lot5.buy_move(&mut stale_cur, [600, 440], 1).unwrap());
        assert!(lot5.buy_commit(&stale_cur).is_err());
        assert_eq!(lot5.world().budget.funds, funds_after_stale_undo);
        lot5.buy_cancel(&stale_cur).unwrap();
        assert!(!lot5.world().objects.contains_key(&stale_cur.cursor));
        assert!(
            lot5.world()
                .objects
                .get(&stale_chair)
                .is_none_or(|o| o.position[0] < 0)
        );
        assert_eq!(lot5.world().budget.funds, funds_after_stale_undo);

        // P2 #1: Buy Redo affordability check after a refused Sell Redo.
        let mut ch_keep = lot5.buy_start(chair_guid, 1, 2).unwrap().unwrap();
        assert!(lot5.buy_move(&mut ch_keep, [600, 440], 1).unwrap());
        lot5.buy_commit(&ch_keep).unwrap();
        let mut d1 = lot5.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot5.buy_move(&mut d1, [584, 472], 1).unwrap());
        let desk1 = lot5.buy_commit(&d1).unwrap();
        let mut d2 = lot5.buy_start(desk.guid, 1, 2).unwrap().unwrap();
        assert!(lot5.buy_move(&mut d2, [552, 472], 1).unwrap());
        let desk2 = lot5.buy_commit(&d2).unwrap();
        let mut l1 = lot5.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
        assert!(lot5.buy_move(&mut l1, [584, 472], 1).unwrap());
        let lamp_id = lot5.buy_commit(&l1).unwrap();
        let mut lm = lot5.buy_pick_up(lamp_id).unwrap().unwrap();
        assert!(lot5.buy_move(&mut lm, [552, 472], 1).unwrap());
        assert_eq!(lm.container.map(|(c, _)| c), Some(desk2));
        lot5.buy_commit(&lm).unwrap();
        let funds_before_desk_sell = lot5.world().budget.funds;
        let d1_pick = lot5.buy_pick_up(desk1).unwrap().unwrap();
        assert!(lot5.buy_sell(&d1_pick).unwrap());
        let funds_after_desk_sell = lot5.world().budget.funds;
        assert!(funds_after_desk_sell > funds_before_desk_sell);
        // Find a floor item priced strictly between `funds_before_desk_sell`
        // and `funds_after_desk_sell` so it is affordable only if desk1 was sold.
        let mut tested_overdraw = false;
        for item in catalog
            .all()
            .iter()
            .filter(|i| i.price > funds_before_desk_sell && i.price <= funds_after_desk_sell)
        {
            let Some(mut exp_cur) = lot5.buy_start(item.guid, 1, 2).unwrap() else {
                continue;
            };
            if !lot5.buy_move(&mut exp_cur, [584, 424], 1).unwrap() {
                lot5.buy_cancel(&exp_cur).unwrap();
                continue;
            }
            let exp_id = lot5.buy_commit(&exp_cur).unwrap();
            assert!(lot5.buy_undo().unwrap()); // undo expensive item
            assert!(lot5.buy_undo().unwrap()); // undo desk1 sell
            assert!(lot5.buy_undo().unwrap()); // undo lamp move
            assert_eq!(lot5.world().budget.funds, funds_before_desk_sell);
            let mut blocker = lot5.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
            assert!(lot5.buy_move(&mut blocker, [552, 472], 1).unwrap());
            assert!(!lot5.buy_redo().unwrap()); // lamp move refused
            assert!(!lot5.buy_redo().unwrap()); // desk1 sell refused
            // Expensive item Buy Redo must refuse instead of overdrawing funds!
            assert!(!lot5.buy_redo().unwrap());
            assert!(lot5.world().objects[&exp_id].position[0] < 0);
            assert_eq!(lot5.world().budget.funds, funds_before_desk_sell);
            assert!(lot5.world().budget.funds >= 0);
            lot5.buy_cancel(&blocker).unwrap();
            tested_overdraw = true;
            break;
        }
        assert!(
            tested_overdraw,
            "expected an item priced between {funds_before_desk_sell} and {funds_after_desk_sell}"
        );

        // --- 7. Sol Round 6b regressions: ID-reuse retired cursor protection,
        //        desk sale after lamp sale does not restore refunded lamp, and
        //        unpaid preview cannot be picked up & sold for cash ---
        {
            let mut lot6 =
                Lot::load(&install, &goth_house(&install), &[3], Frames::default()).unwrap();
            for _ in 0..60 {
                lot6.step().unwrap();
            }
            lot6.set_mode(sims_vm::PanelMode::Buy).unwrap();

            // P2 #3: unpaid preview must NOT be pickable as existing furniture
            // and sold for cash.
            let funds_start = lot6.world().budget.funds;
            let mut prev = lot6.buy_start(chair_guid, 1, 2).unwrap().unwrap();
            assert!(lot6.buy_move(&mut prev, [600, 440], 1).unwrap());
            assert!(
                lot6.buy_pick_up(prev.held).unwrap().is_none(),
                "unpaid preview must not be pickable as existing furniture"
            );
            lot6.buy_cancel(&prev).unwrap();
            assert_eq!(lot6.world().budget.funds, funds_start);

            // P2 #1: retired cursor handle after ID reuse must NOT delete a
            // newly bought chair via buy_cancel, buy_sell, save, or set_mode.
            let mut c1 = lot6.buy_start(chair_guid, 1, 2).unwrap().unwrap();
            assert!(lot6.buy_move(&mut c1, [600, 440], 1).unwrap());
            let chair1 = lot6.buy_commit(&c1).unwrap();
            let old_pick = lot6.buy_pick_up(chair1).unwrap().unwrap();
            let old_cursor_id = old_pick.cursor;
            assert!(lot6.buy_sell(&old_pick).unwrap());
            let mut c2 = lot6.buy_start(chair_guid, 1, 2).unwrap().unwrap();
            assert!(lot6.buy_move(&mut c2, [584, 424], 1).unwrap());
            let chair2 = lot6.buy_commit(&c2).unwrap();
            assert_eq!(
                chair2, old_cursor_id,
                "verify ID reuse of old Tile Cursor ID"
            );
            // None of these may delete chair2!
            assert!(!lot6.buy_sell(&old_pick).unwrap());
            lot6.buy_cancel(&old_pick).unwrap();
            assert!(lot6.world().objects.contains_key(&chair2));
            let saved = lot6.save_without_buy_cursor(&old_pick).unwrap();
            assert!(saved.world().objects.contains_key(&chair2));

            // Single-cursor surface ownership + Sol Round 8b P2 #3:
            // picking up a desk prevents picking up its lamp at the same time;
            // moving the desk (uncommitted) and then undoing the lamp's Buy
            // snaps the desk back to its pre-pickup tile BEFORE undoing the
            // lamp so the uncommitted surface move is never made permanent.
            let mut dk = lot6.buy_start(desk.guid, 1, 2).unwrap().unwrap();
            assert!(lot6.buy_move(&mut dk, [584, 472], 1).unwrap());
            let desk_id = lot6.buy_commit(&dk).unwrap();
            let mut lp = lot6.buy_start(lamp.guid, 1, 2).unwrap().unwrap();
            assert!(lot6.buy_move(&mut lp, [584, 472], 1).unwrap());
            let lamp_id = lot6.buy_commit(&lp).unwrap();
            let mut desk_pick = lot6.buy_pick_up(desk_id).unwrap().unwrap();
            assert!(lot6.buy_move(&mut desk_pick, [600, 440], 1).unwrap());
            assert_eq!(lot6.world().objects[&desk_id].position, [600, 440, 1]);
            assert!(
                lot6.buy_pick_up(lamp_id).unwrap().is_none(),
                "cannot pick up lamp while desk cursor is active"
            );
            assert!(lot6.buy_undo().unwrap()); // snaps desk back to [584, 472], undoes lamp_id
            assert!(!lot6.world().objects.contains_key(&desk_pick.cursor));
            assert_eq!(
                lot6.world().objects[&desk_id].position,
                [584, 472, 1],
                "undoing child purchase must snap uncommitted desk move back to origin"
            );
            assert!(lot6.world().objects[&lamp_id].position[0] < 0);
            let funds_after_lamp_undo = lot6.world().budget.funds;
            assert!(!lot6.buy_sell(&desk_pick).unwrap());
            lot6.buy_cancel(&desk_pick).unwrap();
            assert_eq!(lot6.world().budget.funds, funds_after_lamp_undo);
            assert_eq!(lot6.world().objects[&desk_id].position, [584, 472, 1]);
            assert!(
                lot6.world().objects[&lamp_id].position[0] < 0,
                "cancelling retired desk handle must not restore already-refunded lamp"
            );

            // Also verify set_mode_releasing_buy_cursor with retired old_pick
            // does NOT delete chair2.
            lot6.set_mode_releasing_buy_cursor(sims_vm::PanelMode::Live, Some(&old_pick))
                .unwrap();
            assert!(lot6.world().objects.contains_key(&chair2));

            // Sol Round 7 regression: Undo while retaining a pickup handle
            // retires it immediately, so even after a later commit purges the
            // detached object and an unpaid preview or paid chair reuses its
            // ID, the old pickup handle CANNOT sell, mutate, or delete the new
            // object via buy_sell, buy_cancel, save_without_buy_cursor, or
            // set_mode_releasing_buy_cursor.
            lot6.set_mode(sims_vm::PanelMode::Buy).unwrap();
            let mut u1 = lot6.buy_start(chair_guid, 1, 2).unwrap().unwrap();
            assert!(lot6.buy_move(&mut u1, [600, 440], 1).unwrap());
            let undone_chair = lot6.buy_commit(&u1).unwrap();
            let stale_after_undo = lot6.buy_pick_up(undone_chair).unwrap().unwrap();
            assert!(lot6.buy_undo().unwrap()); // detaches undone_chair, retires stale_after_undo & removes Tile Cursor
            assert!(!lot6.world().objects.contains_key(&stale_after_undo.cursor));
            // Commit another chair to purge buy_redo and free `undone_chair`'s ID.
            let mut other = lot6.buy_start(chair_guid, 1, 2).unwrap().unwrap();
            assert!(lot6.buy_move(&mut other, [632, 440], 1).unwrap());
            let other_id = lot6.buy_commit(&other).unwrap();
            assert!(!lot6.world().objects.contains_key(&undone_chair));
            // Start a new unpaid preview that reuses `undone_chair`'s ID.
            let mut reused_prev = lot6.buy_start(chair_guid, 1, 2).unwrap().unwrap();
            assert_eq!(
                reused_prev.held, undone_chair,
                "expected new preview to reuse purged undone chair ID"
            );
            assert!(lot6.buy_move(&mut reused_prev, [600, 440], 1).unwrap());
            let funds_before_stale_sell = lot6.world().budget.funds;
            assert!(
                !lot6.buy_sell(&stale_after_undo).unwrap(),
                "retired pickup handle must not sell reused unpaid preview"
            );
            assert_eq!(lot6.world().budget.funds, funds_before_stale_sell);
            assert!(lot6.world().objects.contains_key(&reused_prev.held));
            assert!(lot6.world().objects.contains_key(&other_id));
            // `save()` / `save_without_buy_cursor` always strips the active
            // unpaid preview (`reused_prev`) from the saved world while keeping
            // committed furniture (`other_id`) intact.
            let saved_unpaid = lot6.save_without_buy_cursor(&stale_after_undo).unwrap();
            assert!(saved_unpaid.world().objects.contains_key(&other_id));
            assert!(!saved_unpaid.world().objects.contains_key(&reused_prev.held));
            assert!(
                !saved_unpaid
                    .world()
                    .objects
                    .contains_key(&reused_prev.cursor)
            );
            // Commit `reused_prev` into a paid chair and verify save + mode
            // switch with `stale_after_undo` keep both `other_id` and `paid_reused`.
            let paid_reused = lot6.buy_commit(&reused_prev).unwrap();
            let saved_paid = lot6.save_without_buy_cursor(&stale_after_undo).unwrap();
            assert!(saved_paid.world().objects.contains_key(&other_id));
            assert!(saved_paid.world().objects.contains_key(&paid_reused));
            lot6.set_mode_releasing_buy_cursor(sims_vm::PanelMode::Live, Some(&stale_after_undo))
                .unwrap();
            assert!(lot6.world().objects.contains_key(&other_id));
            assert!(lot6.world().objects.contains_key(&paid_reused));

            // Undo of a `Move` while holding a pickup handle for the same chair
            // snaps it back and applies Undo without corrupting redo.
            lot6.set_mode(sims_vm::PanelMode::Buy).unwrap();
            let mut m1 = lot6.buy_pick_up(paid_reused).unwrap().unwrap();
            assert!(lot6.buy_move(&mut m1, [552, 472], 1).unwrap());
            assert_eq!(lot6.buy_commit(&m1).unwrap(), paid_reused);
            let stale_after_move_undo = lot6.buy_pick_up(paid_reused).unwrap().unwrap();
            assert!(lot6.buy_undo().unwrap());
            assert!(lot6.can_buy_redo());
            assert!(!lot6.buy_sell(&stale_after_move_undo).unwrap());
            lot6.buy_cancel(&stale_after_move_undo).unwrap();
            assert!(
                lot6.can_buy_redo(),
                "cancelling a retired pickup handle must not clear unrelated redo history"
            );
            assert!(
                !lot6
                    .world()
                    .objects
                    .contains_key(&stale_after_move_undo.cursor)
            );
            assert_eq!(lot6.world().objects[&paid_reused].position, [600, 440, 1]);

            // Sol Round 8b P2 #1: cross-lot cursor tokens are globally unique,
            // so a cursor handle from `lot5` cannot affect `lot6`.
            let lot5_prev = lot5.buy_start(chair_guid, 1, 2).unwrap().unwrap();
            let mut lot6_pick = lot6.buy_pick_up(paid_reused).unwrap().unwrap();
            assert!(!lot6.buy_sell(&lot5_prev).unwrap());
            lot6.buy_cancel(&lot5_prev).unwrap();
            assert!(lot6.world().objects.contains_key(&paid_reused));
            assert!(lot6.world().objects.contains_key(&lot6_pick.cursor));
            lot5.buy_cancel(&lot5_prev).unwrap();

            // Sol Round 8b P2 #2: `Lot::save()` while a paid chair is held and
            // hidden off-lot snaps it back on the saved copy so `restore()`
            // never strands paid furniture off-lot or leaves a Tile Cursor.
            lot6.buy_hide(&mut lot6_pick).unwrap();
            assert!(lot6.world().objects[&paid_reused].position[0] < 0);
            let saved_hidden = lot6.save().unwrap();
            assert_eq!(
                saved_hidden.world().objects[&paid_reused].position,
                [600, 440, 1],
                "Lot::save() must snap back a held/hidden paid chair"
            );
            assert!(!saved_hidden.world().objects.contains_key(&lot6_pick.cursor));
            lot6.restore(saved_hidden).unwrap();
            assert_eq!(lot6.world().objects[&paid_reused].position, [600, 440, 1]);
            assert!(!lot6.world().objects.contains_key(&lot6_pick.cursor));
            // Restored chair is pickable again immediately.
            let repick = lot6.buy_pick_up(paid_reused).unwrap().unwrap();
            lot6.buy_cancel(&repick).unwrap();

            // Sol Round 8b P2 #4: plain `Lot::set_mode` Buy -> Live -> Buy
            // releases any active cursor instead of blocking Buy Mode forever.
            let abandoned = lot6.buy_start(chair_guid, 1, 2).unwrap().unwrap();
            lot6.set_mode(sims_vm::PanelMode::Live).unwrap();
            assert!(!lot6.world().objects.contains_key(&abandoned.held));
            assert!(!lot6.world().objects.contains_key(&abandoned.cursor));
            lot6.set_mode(sims_vm::PanelMode::Buy).unwrap();
            let fresh = lot6
                .buy_start(chair_guid, 1, 2)
                .unwrap()
                .expect("Buy Mode must not stay blocked after plain set_mode Live -> Buy");
            lot6.buy_cancel(&fresh).unwrap();
        }
    }
}

/// Opening a pie menu near the top edge of the expanded bottom control strip
/// (starting at `y = 500` in an 800×600 window) clips the 3D pie head to the
/// 800×500 house viewport so zero opaque head pixels draw over the bottom
/// strip, while the upper half of the head still draws inside the house view.
#[test]
fn pie_head_is_clipped_to_house_viewport_above_the_bottom_strip() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(100), 100);
    game.resize([800, 600]);
    game.render();

    // Pan the camera so the porch bench (173, single-entry "Sit" pie with no
    // bottom label) sits right at y = 487, near the top of the expanded bottom
    // control strip (y = 500), so its 3D pie head extends well across y = 500.
    let bench_at = recorded([432, 230]);
    assert_eq!(game.under(bench_at), Some(173));
    let target_y = 487;
    game.view.camera.origin[1] += target_y - bench_at[1];
    let baseline = game.render().color.clone();
    let point = [bench_at[0], target_y];
    assert_eq!(game.under(point), Some(173));
    assert!(!game.over_panel(point));
    game.leave();
    game.hover(point);
    game.press(point);
    game.render();
    let menu = game.menu().expect("bench pie opened near bottom strip");
    let origin = menu.head_origin;
    let head = menu.head.as_ref().expect("head rendered");
    // The head is clipped right at the bottom edge of the 800×500 house
    // viewport (y = 500), so its bottom edge stops at 500 and its opaque count
    // is strictly smaller than Mortimer's unclipped idle head (1,940 px).
    assert_eq!(
        origin[1] + head.height as i32,
        500,
        "expected head bottom to be clipped at y=500, origin={origin:?}, height={}",
        head.height
    );
    let clipped_opaque = head.rgba.chunks(4).filter(|p| p[3] == 255).count();
    assert!(
        clipped_opaque > 500 && clipped_opaque < 1800,
        "expected partial head clipped at y=500 (unclipped idle is 1940), got {clipped_opaque}"
    );
    let frame = game.canvas().color.clone();
    let label_rects: Vec<Rect> = menu.labels().into_iter().map(|(_, r)| r).collect();
    let in_label = |x: i32, y: i32| label_rects.iter().any(|r| r.contains(x, y));

    // Zero pixels anywhere on the bottom control strip (y >= 500, outside pie
    // labels) are modified by the pie head or circular backdrop.
    for dy in 500..600 {
        for dx in 0..800 {
            if in_label(dx, dy) {
                continue;
            }
            let idx = dy as usize * 800 + dx as usize;
            assert_eq!(
                frame[idx], baseline[idx],
                "pie opened at {point:?} bled onto bottom strip at ({dx}, {dy})"
            );
        }
    }

    // The upper portion of the head still draws inside the 800×500 house view.
    let mut drawn_above = 0usize;
    for sy in 0..head.height {
        let dy = origin[1] + sy as i32;
        assert!(
            (0..500).contains(&dy),
            "head pixel outside viewport: y={dy}"
        );
        for sx in 0..head.width {
            let dx = origin[0] + sx as i32;
            let p = (sy * head.width + sx) as usize * 4;
            if head.rgba[p + 3] != 255 || in_label(dx, dy) {
                continue;
            }
            let idx = dy as usize * 800 + dx as usize;
            let expected_head = [head.rgba[p], head.rgba[p + 1], head.rgba[p + 2]];
            if frame[idx] == expected_head {
                drawn_above += 1;
            }
        }
    }
    assert!(
        drawn_above > 100,
        "expected >100 head pixels drawn inside house viewport above y=500, got {drawn_above}"
    );
}

/// Opening a pie menu seeds the pointer from the press (or the on-screen clamp
/// warp when the menu shifts to stay on screen), matching native `0x5ebd05` /
/// `0x5ebd29`, so the opening frame does not aim the head at a stale hover.
#[test]
fn pie_opening_frame_aims_head_at_press_not_stale_hover() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(100), 100);
    game.resize([800, 600]);
    game.render();

    // Hover above the porch bench (where its "Sit" label will appear), then
    // press the bench center directly without sending another hover event.
    let bench = recorded([432, 230]);
    assert_eq!(game.under(bench), Some(173));
    game.hover([bench[0], bench[1] - 75]);
    game.press(bench);
    game.render();
    let opening_head = game
        .menu()
        .expect("bench pie opened")
        .head
        .as_ref()
        .expect("opening head rendered")
        .clone();
    let opening_origin = game.menu().unwrap().head_origin;

    // Hovering at the press coordinate must produce the exact same idle pose.
    game.hover(bench);
    game.render();
    let settled_head = game
        .menu()
        .expect("bench pie still open")
        .head
        .as_ref()
        .expect("settled head rendered")
        .clone();
    assert_eq!(opening_origin, game.menu().unwrap().head_origin);
    assert_eq!(
        opening_head.rgba, settled_head.rgba,
        "opening frame aimed at stale hover instead of press coordinate"
    );

    // When a pie menu moves on screen to stay inside the window, the opening
    // frame before `take_pointer()` must aim at the warped menu center rather
    // than the pre-warp press point.
    game.close_menu();
    let edge_y = 25;
    game.view.camera.origin[1] += edge_y - bench[1];
    game.render();
    let edge_bench = [bench[0], edge_y];
    assert_eq!(game.under(edge_bench), Some(173));
    game.hover([edge_bench[0], edge_bench[1] + 90]);
    game.press(edge_bench);
    let warped = game
        .menu()
        .expect("edge pie opened")
        .pending_pointer()
        .expect("edge pie moved on screen and scheduled cursor warp");
    assert_ne!(warped, edge_bench);
    // Render BEFORE calling take_pointer().
    game.render();
    let warped_opening = game
        .menu()
        .unwrap()
        .head
        .as_ref()
        .expect("warped opening head")
        .clone();
    // Hovering onto "Sit" and releasing (settling the edge-clamped menu)
    // WITHOUT consuming the pending opening warp must keep the head aimed at
    // "Sit" rather than snapping back to `warped` (`0x5eb53c`).
    let sit = pie_entry(&game, "Sit");
    game.hover(sit);
    game.render();
    let sit_hover_head = game
        .menu()
        .unwrap()
        .head
        .as_ref()
        .expect("Sit hover head")
        .clone();
    assert_ne!(sit_hover_head.rgba, warped_opening.rgba);
    game.release(sit);
    game.render();
    let sit_settled_head = game
        .menu()
        .expect("edge pie settled and stays open")
        .head
        .as_ref()
        .expect("Sit settled head")
        .clone();
    assert_eq!(
        sit_settled_head.rgba, sit_hover_head.rgba,
        "settling an edge-clamped menu snapped the head back to a stale opening warp"
    );

    assert_eq!(game.take_pointer(), Some(warped));
    game.hover(warped);
    game.render();
    let warped_after = game
        .menu()
        .unwrap()
        .head
        .as_ref()
        .expect("warped settled head")
        .clone();
    assert_eq!(
        warped_opening.rgba, warped_after.rgba,
        "edge-clamped pie aimed at pre-warp cursor instead of warped center"
    );
}

/// Composed submenu rendering matches Complete Collection (`social-queued.png`):
/// opening Bella's `Talk...` submenu keeps Mortimer's 3D head frozen at the
/// parent menu's center looking down toward `Talk...`, draws the submenu's top
/// label (`About Interests`) over the upper head while leaving the lower face
/// visible below the label, and centers the dark circular backdrop on the
/// submenu.
#[test]
fn pie_submenu_keeps_parent_head_frozen_and_draws_submenu_labels_above_it() {
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
    let bella = 18u16;

    run_to(&mut game, &schedule, 93);
    let on_bella = match game.under(recorded([400, 274])) {
        Some(id) if id == bella => recorded([400, 274]),
        _ => [499, 317],
    };
    game.hover(on_bella);
    run_to(&mut game, &schedule, 123);
    let bare_lot = game.render().color.clone();

    game.press(on_bella);
    game.release(on_bella);
    game.render();
    let talk = pie_entry(&game, "Talk...");
    game.hover(talk);
    game.render();

    let (parent_center, parent_origin, parent_head) = {
        let menu = game.menu().expect("parent menu open");
        (
            menu.active_center().expect("parent center"),
            menu.head_origin,
            menu.head.as_ref().expect("parent head").clone(),
        )
    };

    // Open the Talk... submenu.
    game.press(talk);
    game.release(talk);
    let sub_frame = game.render().color.clone();
    let menu = game.menu().expect("submenu open");
    let sub_center = menu.active_center().expect("submenu center");
    let sub_origin = menu.head_origin;
    let sub_head = menu.head.as_ref().expect("submenu head");

    // 1. Head stays frozen at the parent menu's origin and pose.
    assert_eq!(sub_origin, parent_origin);
    assert_eq!(sub_head.rgba, parent_head.rgba);
    // 2. Active backdrop center moves down to the submenu.
    assert_ne!(sub_center, parent_center);
    assert!(
        sub_center[1] > parent_center[1],
        "Talk... submenu center {sub_center:?} should be below parent {parent_center:?}"
    );

    // 3. The submenu's top label ("About Interests") overlaps the upper head
    //    and draws on top of it, while the lower face remains visible outside
    //    label rects.
    let label_rects: Vec<Rect> = menu.labels().into_iter().map(|(_, r)| r).collect();
    let about = menu
        .labels()
        .into_iter()
        .find(|(text, _)| *text == "About Interests")
        .expect("About Interests label")
        .1;
    let in_any_label = |x: i32, y: i32| label_rects.iter().any(|r| r.contains(x, y));

    let mut head_under_about = 0usize;
    let mut head_occluded_by_about = 0usize;
    let mut head_visible_outside_labels = 0usize;
    for sy in 0..sub_head.height {
        let dy = sub_origin[1] + sy as i32;
        if !(0..500).contains(&dy) {
            continue;
        }
        for sx in 0..sub_head.width {
            let dx = sub_origin[0] + sx as i32;
            if !(0..800).contains(&dx) {
                continue;
            }
            let p = (sy * sub_head.width + sx) as usize * 4;
            if sub_head.rgba[p + 3] != 255 {
                continue;
            }
            let head_rgb = [sub_head.rgba[p], sub_head.rgba[p + 1], sub_head.rgba[p + 2]];
            let actual = sub_frame[dy as usize * 800 + dx as usize];
            if about.contains(dx, dy) {
                head_under_about += 1;
                if actual != head_rgb {
                    head_occluded_by_about += 1;
                }
            } else if !in_any_label(dx, dy) && actual == head_rgb {
                head_visible_outside_labels += 1;
            }
        }
    }
    assert!(
        head_under_about > 50 && head_occluded_by_about * 2 > head_under_about,
        "expected About Interests to draw over the upper head: under={head_under_about}, occluded={head_occluded_by_about}"
    );
    assert!(
        head_visible_outside_labels > 400,
        "expected lower frozen head to stay visible outside labels, got {head_visible_outside_labels}"
    );

    // 4. Verify the radial backdrop moved from `parent_center` to `sub_center`
    //    (`0x462325` / `0x462342`):
    //    - Pixels strictly inside the submenu's inner disk (r < 67) AND
    //      strictly outside the parent's full disk (r >= 100) are shaded.
    //    - Pixels strictly inside the parent's upper inner disk (r < 67) AND
    //      strictly outside the submenu's full disk (r >= 100), outside labels
    //      and the frozen head, remain unshaded (identical to `bare_lot`).
    let dist2 = |a: [i32; 2], b: [i32; 2]| {
        let dx = a[0] - b[0];
        let dy = a[1] - b[1];
        dx * dx + dy * dy
    };
    let in_head = |x: i32, y: i32| {
        let sx = x - sub_origin[0];
        let sy = y - sub_origin[1];
        if sx < 0 || sy < 0 || sx >= sub_head.width as i32 || sy >= sub_head.height as i32 {
            return false;
        }
        sub_head.rgba[(sy as usize * sub_head.width as usize + sx as usize) * 4 + 3] != 0
    };

    let mut sub_only_checked = 0usize;
    let mut sub_only_shaded = 0usize;
    let mut parent_only_checked = 0usize;
    let mut parent_only_unshaded = 0usize;
    for dy in 0..500 {
        for dx in 0..800 {
            if in_any_label(dx, dy) || in_head(dx, dy) {
                continue;
            }
            let d_sub = dist2([dx, dy], sub_center);
            let d_parent = dist2([dx, dy], parent_center);
            let idx = dy as usize * 800 + dx as usize;
            if d_sub < 67 * 67 && d_parent >= 100 * 100 {
                sub_only_checked += 1;
                if sub_frame[idx] != bare_lot[idx] {
                    sub_only_shaded += 1;
                }
            } else if d_parent < 67 * 67 && d_sub >= 100 * 100 {
                parent_only_checked += 1;
                if sub_frame[idx] == bare_lot[idx] {
                    parent_only_unshaded += 1;
                }
            }
        }
    }
    assert!(
        sub_only_checked > 500 && sub_only_shaded * 10 > sub_only_checked * 9,
        "expected submenu-only inner disk outside parent disk to be shaded: {sub_only_shaded}/{sub_only_checked}"
    );
    assert!(
        parent_only_checked > 500 && parent_only_unshaded == parent_only_checked,
        "expected parent-only inner disk outside submenu disk to be unshaded: {parent_only_unshaded}/{parent_only_checked}"
    );
}

/// The pie head submits into the house's final 3D pass with `D3DCMP_LESSEQUAL`
/// depth testing (`0x4a9500`, `0x5ce959`–`0x5ce977`, `0x4623f5`–`0x462414`
/// placing the head origin at `z_screen = 0.5 * ((3000 + 10) << zoom)`). At
/// Near zoom, Mortimer's neutral head's visible front-facing pixels span depths
/// ~11,833–12,049, so nearer foreground scene geometry (e.g. grass at depth
/// 8,703) occludes the head, the midpoint of those visible bounds clips only
/// the farther half, and deeper/empty geometry lets the entire head draw and
/// write its depths.
#[test]
fn pie_head_respects_house_depth_buffer_lessequal() {
    let Some(install) = install() else {
        return;
    };
    let mut game = Game::open(&options(install)).expect("the Goth house opens");
    assert_eq!(game.run(100), 100);
    game.resize([800, 600]);
    game.render();

    let bench = recorded([432, 230]);
    assert_eq!(game.under(bench), Some(173));
    game.press(bench);
    game.view.camera.zoom = sims_render::Zoom::Near;
    game.render();

    let menu = game.menu().expect("bench pie open");
    let head = menu.head.as_ref().expect("Near-zoom head rendered");
    let origin = menu.head_origin;
    let head_depth = &menu.head_depth;
    let opaque_depths: Vec<i32> = head
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(head_depth.iter().copied())
        .filter(|(p, _)| p[3] == 255)
        .map(|(_, d)| d)
        .collect();
    assert_eq!(opaque_depths.len(), 1_940);
    let min_d = *opaque_depths.iter().min().unwrap();
    let max_d = *opaque_depths.iter().max().unwrap();
    assert!(
        (11_800..=11_860).contains(&min_d) && (12_020..=12_080).contains(&max_d),
        "expected Near-zoom Mortimer visible head depth ~11,833..12,049, got {min_d}..{max_d}"
    );

    // 1. Foreground scene nearer than every head vertex rejects all 1,940 pixels.
    let fg_depth = sims_render::depth::Depth::new(&game.view.camera).grass([62, 62], 200);
    assert!(fg_depth < min_d, "fg_depth={fg_depth} min_d={min_d}");
    let mut fg_canvas = Canvas::new(800, 500, [1, 2, 3]);
    fg_canvas.depth.fill(fg_depth);
    sims_game::people::composite_pie_head(&mut fg_canvas, origin, head, head_depth, (800, 500));
    let fg_written = fg_canvas.color.iter().filter(|p| **p != [1, 2, 3]).count();
    assert_eq!(fg_written, 0, "nearer foreground must occlude the pie head");

    // 2. Mid-head depth draws only the nearer half of visible pixels (z <= mid_d).
    let mid_d = (min_d + max_d) / 2;
    let mut mid_canvas = Canvas::new(800, 500, [1, 2, 3]);
    mid_canvas.depth.fill(mid_d);
    sims_game::people::composite_pie_head(&mut mid_canvas, origin, head, head_depth, (800, 500));
    let mid_written = mid_canvas.color.iter().filter(|p| **p != [1, 2, 3]).count();
    let expected_mid = opaque_depths.iter().filter(|&&d| d <= mid_d).count();
    assert!(expected_mid > 500 && expected_mid < 1_940);
    assert_eq!(mid_written, expected_mid);

    // 3. Empty/far scene depth draws all 1,940 pixels and updates canvas.depth.
    let mut far_canvas = Canvas::new(800, 500, [1, 2, 3]);
    sims_game::people::composite_pie_head(&mut far_canvas, origin, head, head_depth, (800, 500));
    let far_written = far_canvas.color.iter().filter(|p| **p != [1, 2, 3]).count();
    assert_eq!(far_written, 1_940);
    let updated_depths = far_canvas
        .depth
        .iter()
        .filter(|&&d| d != sims_render::depth::EMPTY)
        .count();
    assert_eq!(updated_depths, 1_940);
}
