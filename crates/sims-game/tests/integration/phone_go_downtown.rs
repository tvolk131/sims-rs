//! Going Downtown from the phone, through the game's own loop.
//!
//! A Sim picks the phone's Go Downtown (the phone's Call Plugin, tree
//! 0x2075). The plugin pays the fare, shows its dialogs, asks for a lot on
//! the graphical Downtown map, and runs Generic Sims Call 17, which writes
//! the traveler's saved state and asks for the trip
//! (`World::trip_request`). The original's main loop finds the request
//! between ticks and leaves the house; here [`Game`] stops ticking and
//! [`NeighborhoodShell::pump_trip_request`] departs with
//! `travel::depart_from_request_with`. The trip must depart once: no second fare
//! and no second set of saved-state tokens, which the shell's own departure
//! for the lot pick (`depart_to_destination`) would add.
//!
//! Needs an install (`SIMS_INSTALL`); skipped otherwise.

use crate::common::install_root;
use sims_formats::saved_objects::Interaction;
use sims_game::game::goth_house;
use sims_game::neighborhood::NeighborhoodShell;
use sims_game::{Frames, Game, Options};
use sims_sim::neighbors::Item;
use sims_vm::SpecialPanelResponse;
use std::path::PathBuf;

const HOME: u32 = 5;
const DOWNTOWN_LOT: u16 = 21;
/// The phone's Call Plugin, which dispatches to the plugin the entry names.
const CALL_PLUGIN: u16 = 0x2075;
/// The token kind of a traveler's saved state.
const SAVED_STATE_TOKEN: i32 = 2;
/// Frames the call may take: the cab comes, the dialogs, the lot picker.
const CALL_FRAMES: u32 = 5000;
/// Frames a phone may take to start the call before the next is tried.
const PHONE_FRAMES: u32 = 1500;
const SIZE: [i32; 2] = [1024, 768];

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The saved-state tokens in the inventory of the neighbor `guid` is.
fn saved_state(game: &Game, guid: u32) -> Vec<Item> {
    let world = game.lot().world();
    let Some(record) = world.neighbors.first_of(guid) else {
        return Vec::new();
    };
    world
        .neighbors
        .inventories
        .iter()
        .filter(|i| i.neighbor == record.id)
        .flat_map(|i| i.items.iter().cloned())
        .filter(|t| t.kind == SAVED_STATE_TOKEN)
        .collect()
}

/// The saved-state tokens of `guid`, as (GUID, count) in GUID order.
fn tokens_of(game: &Game, guid: u32) -> Vec<(u32, u16)> {
    let mut tokens: Vec<(u32, u16)> = saved_state(game, guid)
        .iter()
        .map(|t| (t.guid, t.count))
        .collect();
    tokens.sort_unstable();
    tokens
}

/// The objects (in id order) whose pie menu offers `person` Go Downtown
/// through the phone's Call Plugin, with that entry's action.
fn go_downtown(game: &mut Game, person: u16) -> Vec<(u16, Interaction)> {
    let objects: Vec<u16> = game.lot().world().objects.keys().copied().collect();
    let mut offers = Vec::new();
    for object in objects {
        let Ok(menu) = game.lot_mut().pie_menu(person, object) else {
            continue;
        };
        if let Some(entry) = menu.entries.iter().find(|e| {
            e.action.action == CALL_PLUGIN
                && e.name.as_deref().is_some_and(|n| n.contains("Downtown"))
        }) {
            offers.push((object, entry.action.clone()));
        }
    }
    offers
}

/// Answers the dialog the lot waits on with its first button by mouse,
/// through the button's release latch (`cTSWinBtn`, 0x5e4563..0x5e4601). A
/// button made by 0x5e3844 fires on release (+0xf4 is 0), and only while the
/// pointer is over it (+0x169, kept by the move handler at 0x5e4644): then
/// it is published as the button released (0x6fe9a4) and fires
/// (0x5e45e1..0x5e45f3). A press alone does nothing, and a press dragged off
/// the button is let go.
fn answer_by_mouse(game: &mut Game) {
    game.render();
    let layout = game.dialog().expect("the call's dialog is drawn").clone();
    let button = layout.buttons[0];
    let inside = [button.x + button.width / 2, button.y + button.height / 2];
    let outside = [2, 2];
    assert!(!button.contains(outside[0], outside[1]));
    let tick = game.lot().ticks();

    let _ = game.press(inside);
    let _ = game.hover(outside);
    let _ = game.release(outside);
    assert!(
        game.asking().is_some(),
        "a press dragged off the button answered"
    );
    assert_eq!(game.lot().ticks(), tick);

    let _ = game.press(inside);
    assert!(
        game.asking().is_some(),
        "the press answered before the release"
    );
    let _ = game.release(inside);
    assert!(
        game.lot().ticks() > tick,
        "the release did not answer: the tick that asked never ran"
    );
}

/// Picks Go Downtown for `caller` on each phone in turn until one asks the
/// call's questions (the Sim cannot route to every phone in the house), and
/// answers each dialog with its first button until the Downtown map is up.
fn call_downtown(game: &mut Game, caller: u16) {
    let offers = go_downtown(game, caller);
    assert!(!offers.is_empty(), "no phone offers Go Downtown");
    let mut dialogs = 0;
    for (_phone, action) in offers {
        game.lot_mut()
            .pick(caller, action)
            .expect("pick Go Downtown");
        for frame in 0..CALL_FRAMES {
            if dialogs == 0 && frame == PHONE_FRAMES {
                break;
            }
            game.run(1);
            assert!(game.stopped().is_none(), "lot halted: {:?}", game.stopped());
            if let Some(asking) = game.asking() {
                let object = asking.shown.object;
                if dialogs == 0 {
                    answer_by_mouse(game);
                } else {
                    game.answer(object, 0);
                }
                dialogs += 1;
                continue;
            }
            if game.asking_special_panel().is_some() {
                assert_eq!(dialogs, 2, "the call asks twice before the map");
                return;
            }
        }
        assert_eq!(dialogs, 0, "the call stopped between its dialogs");
    }
    panic!("the call never asked for a lot");
}

/// The household head's object on the lot.
fn head(game: &Game, guid: u32) -> u16 {
    let world = game.lot().world();
    world
        .people
        .iter()
        .copied()
        .find(|id| world.objects[id].guid == guid)
        .expect("the head is on the lot")
}

#[test]
fn the_phones_go_downtown_leaves_the_lot_once_through_the_shell() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run the phone Go Downtown test");
        return;
    };
    let dir = std::env::temp_dir().join(format!("sims-game-phone-downtown-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let _dir = TempDir(dir.clone());
    let mut shell = NeighborhoodShell::open(
        root,
        dir.join("saves.redb"),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("open the neighborhood");
    shell.resize(SIZE);
    shell.enter_lot(HOME).expect("enter the house");

    // The household's head, who can reach a phone in the house.
    let leader_guid = shell
        .stored_neighborhood()
        .family_in(HOME)
        .expect("a family lives in the house")
        .1
        .members[0];
    let game = shell.game_mut().expect("the house");
    assert_eq!(game.run(5), 5);
    let caller = head(game, leader_guid);
    let funds_before = game.lot().world().budget.funds;
    call_downtown(game, caller);
    shell.render();
    assert!(shell.destination_picker_open());

    // Click lot 21 on the map. The tick the pick answers runs again at once,
    // and the shell carries out whatever trip it asks for.
    let point = shell
        .game()
        .expect("the house")
        .destination_picker()
        .expect("the Downtown map")
        .hit_point_for_lot(SIZE, DOWNTOWN_LOT)
        .expect("lot 21 on the map");
    let _ = shell.hover(point);
    let _ = shell.press(point);
    let _ = shell.release(point);

    // As the window does each frame: run what is owed, then let the shell
    // carry out a trip the lot asked for.
    let mut frames = 0;
    let mut asked = false;
    // The saved-state tokens routine 17 wrote, as the home lot held them
    // when the trip was asked for.
    let mut written: Option<Vec<(u32, u16)>> = None;
    while shell.active_lot_house() == Some(HOME) {
        frames += 1;
        assert!(frames < CALL_FRAMES, "the trip never departed");
        let game = shell.game_mut().expect("the house");
        assert!(game.stopped().is_none(), "lot halted: {:?}", game.stopped());
        if let Some(asking) = game.asking() {
            let object = asking.shown.object;
            game.answer(object, 0);
        } else {
            game.run(1);
        }
        if game.trip_request().is_some() {
            // A pending trip stops the ticks until it is carried out.
            assert_eq!(game.run(1), 0, "a tick ran with a trip pending");
            assert_eq!(game.advance(1_000_000), 0, "time ran with a trip pending");
            asked = true;
            written.get_or_insert_with(|| tokens_of(game, leader_guid));
        }
        shell.pump_trip_request();
    }
    assert!(asked, "the trip departed without the script asking for it");
    assert_eq!(shell.active_lot_house(), Some(u32::from(DOWNTOWN_LOT)));

    let trip = shell.active_trip().expect("an active trip").clone();
    assert_eq!(trip.leader_guid, leader_guid);
    assert_eq!(
        trip.travelers.len(),
        1,
        "Go Downtown takes the caller alone"
    );
    assert_eq!(trip.fare_paid, 0, "the plugin already paid the fare");
    assert!(
        trip.funds_at_arrival < funds_before,
        "the plugin's fare was paid once: {} from {funds_before}",
        trip.funds_at_arrival
    );

    let game = shell.game_mut().expect("the Downtown lot");
    assert!(game.trip_request().is_none(), "the request was carried out");
    assert!(
        game.take_pending_travel_lot().is_none(),
        "the lot pick departs once"
    );
    // Routine 17 wrote one token for each value it saves, and the trip
    // carries exactly those (GUIDs and counts) to the destination, where
    // the arrival takes the clock (GUIDs 7 and 8) out and sets the clock
    // to it (0x4f1ae9 → 0x46d86f).
    let written = written.expect("the trip was asked for");
    assert!(!written.is_empty(), "routine 17 saved the caller's state");
    // The arrival takes the hour token, then the minute token, and stops
    // at the first one missing (a clock token of 0 is never written).
    let count_of = |guid| written.iter().find(|&&(g, _)| g == guid).map(|&(_, c)| c);
    let (hour, minute) = (count_of(7), count_of(8));
    let arrived: Vec<(u32, u16)> = written
        .iter()
        .copied()
        .filter(|&(guid, _)| {
            !(hour.is_some() && guid == 7 || hour.is_some() && minute.is_some() && guid == 8)
        })
        .collect();
    assert_eq!(
        tokens_of(game, leader_guid),
        arrived,
        "the saved state traveled as routine 17 wrote it, less the clock"
    );
    let (Some(hour), Some(minute)) = (hour, minute) else {
        panic!("the capture asks at a clock with both tokens: {written:?}");
    };
    let globals = &game.lot().world().globals;
    assert_eq!(
        (globals[0], globals[5]),
        (hour as i16, minute as i16),
        "the arrival sets the clock from the tokens"
    );
    // The Downtown lot plays on.
    assert_eq!(game.run(10), 10);
}

/// A house opened without its neighborhood cannot travel: the host declines
/// the trip ([`Game::decline_trip`]) and the lot plays on. The lot drops the
/// request (`Lot::cancel_trip_request`) with the saved state routine 17
/// wrote for it, so the script's tail restores nothing from it.
#[test]
fn a_declined_trip_lets_the_lot_play_on() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run the phone Go Downtown test");
        return;
    };
    let mut game = Game::open(&Options {
        house: goth_house(&root),
        install: root,
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("open the house");
    game.resize(SIZE);
    assert_eq!(game.run(5), 5);
    let leader_guid = {
        let world = game.lot().world();
        let family = i32::from(world.globals[9]);
        world
            .families
            .iter()
            .find(|f| f.id == family)
            .map(|f| f.members[0])
            .expect("the household")
    };
    let caller = head(&game, leader_guid);
    call_downtown(&mut game, caller);
    game.render();
    assert!(game.answer_special_panel(SpecialPanelResponse::Lot(Some(DOWNTOWN_LOT as i16))));
    for _ in 0..10 {
        if game.trip_request().is_some() {
            break;
        }
        assert_eq!(game.run(1), 1);
    }
    let trip = game
        .trip_request()
        .expect("the script asked for the trip")
        .clone();
    assert_eq!(i32::from(trip.house), i32::from(DOWNTOWN_LOT));
    assert_eq!(trip.leader_guid, leader_guid);
    assert_eq!(game.run(1), 0, "a tick ran with a trip pending");

    assert!(
        !tokens_of(&game, leader_guid).is_empty(),
        "routine 17 saved the caller's state"
    );
    let dropped = game.decline_trip().expect("the request is dropped");
    assert_eq!(dropped, trip);
    assert!(game.trip_request().is_none());
    assert!(
        game.lot().world().trip_request.is_none(),
        "the lot dropped the request"
    );
    assert_eq!(
        tokens_of(&game, leader_guid),
        [],
        "the saved state routine 17 wrote was taken back"
    );
    assert_eq!(game.run(30), 30, "the lot plays on");
    assert!(
        game.trip_request().is_none(),
        "the dropped trip is not asked for again"
    );
}

/// In co-op the world changes only through the lockstep, so declining a
/// trip only marks it declined and the request stays in the lot. Once the
/// session ends and the lot ticks on its own, the request is dropped with
/// the saved state routine 17 wrote. Otherwise it would hide every later
/// trip: a later Generic Sims Call 17 only adds to a pending request.
#[test]
fn a_trip_declined_in_coop_is_dropped_once_the_lot_plays_alone() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run the phone Go Downtown test");
        return;
    };
    let mut game = Game::open(&Options {
        house: goth_house(&root),
        install: root,
        answers: vec![3],
        play: true,
        frames: Frames::default(),
    })
    .expect("open the house");
    game.resize(SIZE);
    assert_eq!(game.run(5), 5);
    let leader_guid = {
        let world = game.lot().world();
        let family = i32::from(world.globals[9]);
        world
            .families
            .iter()
            .find(|f| f.id == family)
            .map(|f| f.members[0])
            .expect("the household")
    };
    let caller = head(&game, leader_guid);
    let ask_for_downtown = |game: &mut Game| {
        call_downtown(game, caller);
        game.render();
        assert!(game.answer_special_panel(SpecialPanelResponse::Lot(Some(DOWNTOWN_LOT as i16))));
        for _ in 0..10 {
            if game.trip_request().is_some() {
                return game.trip_request().expect("asked").clone();
            }
            assert_eq!(game.run(1), 1);
        }
        panic!("the script never asked for the trip");
    };
    let trip = ask_for_downtown(&mut game);

    let mesh = sims_net::MemoryMesh::new(7);
    game.coop_start_host_with_transport(
        sims_game::coop::CoopTransport::memory(mesh.add_node("mem://host")),
        sims_net::ManifestDigest([0x5A; 32]),
        sims_game::coop::CoopStartOptions {
            player_name: "Host".into(),
            ..sims_game::coop::CoopStartOptions::default()
        },
    )
    .expect("host co-op");
    assert_eq!(game.decline_trip(), Some(trip.clone()));
    assert!(game.trip_request().is_none());
    assert!(
        game.lot().world().trip_request.is_some(),
        "in co-op the request stays in the lot"
    );

    game.leave_coop();
    assert_eq!(game.run(1), 1, "the lot plays on alone");
    assert!(
        game.lot().world().trip_request.is_none(),
        "the request declined in co-op was dropped"
    );
    assert_eq!(
        tokens_of(&game, leader_guid),
        [],
        "the saved state routine 17 wrote was taken back"
    );

    // The first call's tail finishes before the phone offers the trip again.
    let mut ticks = 0;
    while go_downtown(&mut game, caller).is_empty() {
        assert!(
            ticks < CALL_FRAMES,
            "the phone never offers Go Downtown again"
        );
        assert_eq!(game.run(1), 1);
        ticks += 1;
    }
    let again = ask_for_downtown(&mut game);
    assert_eq!(again.leader_guid, leader_guid);
    assert_eq!(i32::from(again.house), i32::from(DOWNTOWN_LOT));
}
