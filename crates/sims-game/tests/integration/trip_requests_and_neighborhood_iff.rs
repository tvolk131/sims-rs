//! Trips a destination's scripts ask for, through the shell, and the
//! `Neighborhood.iff` the shell keeps beside its saves.
//!
//! On Downtown the cab sign offers Call Cab/Go Home and Go Somewhere
//! Else. Each runs Generic Sims Call 17. Go Home asks for house -1,
//! which 0x45a22d turns into the family's house, so
//! [`NeighborhoodShell::pump_trip_request`] returns the travelers with
//! `travel::return_from_request`. Go Somewhere Else asks for another
//! destination, which the port does not carry out: the shell shows a
//! notice and the lot drops the request (`Lot::cancel_trip_request`).
//!
//! The shell writes `Neighborhood.iff` (`NeighborhoodStore::
//! save_neighborhood_iff`) after each commit that changes the
//! neighborhood, as the original saves it (0x4252cc) after a lot save and
//! a trip. The file is read back by importing it into a fresh store.
//!
//! Needs an install (`SIMS_INSTALL`); skipped otherwise.

use crate::common::install_root;
use sims_game::Frames;
use sims_game::hud::DialogKey;
use sims_game::neighborhood::NeighborhoodShell;
use sims_sim::neighborhood_store::{NeighborhoodStore, StoredNeighborhood};
use sims_vm::travel::TravelRequest;
use std::path::{Path, PathBuf};

const HOME: u32 = 5;
const DOWNTOWN_LOT: u32 = 21;
/// Another Downtown lot, for Go Somewhere Else.
const OTHER_DOWNTOWN_LOT: u16 = 22;
/// The cab sign's Call Cab/Go Home.
const GO_HOME: u16 = 0x107b;
/// The cab sign's Call Cab/Go Somewhere Else.
const GO_SOMEWHERE_ELSE: u16 = 0x1004;
/// The token kind of a traveler's saved state.
const SAVED_STATE_TOKEN: i32 = 2;
/// Person data words 58 (age), 61 (family) and 65 (kind: 0 or 1 for a
/// human).
const AGE: usize = 58;
const FAMILY: usize = 61;
const KIND: usize = 65;
/// Frames a Sim may take to walk to the sign and call.
const CALL_FRAMES: u32 = 3000;
const SIZE: [i32; 2] = [1024, 768];

struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sims-game-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open_shell(root: PathBuf, dir: &Path) -> NeighborhoodShell {
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
    shell
}

/// The record a `Neighborhood.iff` holds, read back through an import
/// into a fresh store.
fn read_back(path: &Path, scratch: &Path) -> StoredNeighborhood {
    let folder = scratch.join("read-back");
    std::fs::create_dir_all(&folder).expect("read-back folder");
    std::fs::copy(path, folder.join("Neighborhood.iff")).expect("copy the file");
    let store = NeighborhoodStore::open(&scratch.join("read-back.redb")).expect("a fresh store");
    store
        .import_directory("ReadBack", &folder, 0xff)
        .expect("the written file imports");
    store
        .load_neighborhood("ReadBack")
        .expect("load")
        .expect("the imported neighborhood")
}

/// The tokens of `kind` in the inventory of the neighbor `guid` is, as
/// (GUID, count) in inventory order.
fn tokens_of(world: &sims_sim::World, guid: u32, kind: i32) -> Vec<(u32, u16)> {
    let Some(record) = world.neighbors.first_of(guid) else {
        return Vec::new();
    };
    world
        .neighbors
        .inventories
        .iter()
        .filter(|i| i.neighbor == record.id)
        .flat_map(|i| &i.items)
        .filter(|t| t.kind == kind)
        .map(|t| (t.guid, t.count))
        .collect()
}

/// The tokens of `kind` in neighbor `record`'s inventory in `stored`.
fn stored_tokens(stored: &StoredNeighborhood, record: i16, kind: i32) -> Vec<(u32, u16)> {
    stored
        .inventories
        .iter()
        .flatten()
        .filter(|i| i.neighbor == record)
        .flat_map(|i| &i.items)
        .filter(|t| t.kind == kind)
        .map(|t| (t.guid, t.count))
        .collect()
}

/// An adult of another family to take on the date.
fn date_for(world: &sims_sim::World, leader: u32) -> u32 {
    let family = world
        .neighbors
        .first_of(leader)
        .expect("the leader's record")
        .data[FAMILY];
    world
        .neighbors
        .slots
        .iter()
        .flatten()
        .find(|n| {
            n.guid != leader
                && n.data.len() > KIND
                && n.data[FAMILY] != 0
                && n.data[FAMILY] != family
                && n.data[AGE] >= 18
                && (0..=1).contains(&n.data[KIND])
        })
        .expect("an adult of another family")
        .guid
}

/// The household's house and secondary house in `stored`, by its id.
fn houses_of(stored: &StoredNeighborhood, family: u16) -> (i32, i32) {
    let fam = stored
        .families
        .get(&family)
        .unwrap_or_else(|| panic!("family {family} in {:?}", stored.families.keys()));
    (fam.house, fam.secondary_house())
}

/// Picks `action` for the traveler on each cab sign in turn, answering
/// the call's dialogs with their first button, until `done` holds or the
/// Sim gives up on the sign. Runs the frames as the window does: run what
/// is owed, then let the shell carry out a trip the lot asked for.
/// `before_pump` sees each frame between the two.
fn call_cab(
    shell: &mut NeighborhoodShell,
    leader: u32,
    action: u16,
    mut before_pump: impl FnMut(&mut NeighborhoodShell),
    mut done: impl FnMut(&NeighborhoodShell) -> bool,
) {
    let game = shell.game_mut().expect("the Downtown lot");
    let traveler = {
        let world = game.lot().world();
        world
            .objects
            .values()
            .find(|o| o.guid == leader && o.person.is_some())
            .map(|o| o.id)
            .expect("the traveler is on the lot")
    };
    let mut offers = Vec::new();
    let objects: Vec<u16> = game.lot().world().objects.keys().copied().collect();
    for object in objects {
        let Ok(menu) = game.lot_mut().pie_menu(traveler, object) else {
            continue;
        };
        if let Some(entry) = menu.entries.iter().find(|e| {
            e.action.action == action && e.name.as_deref().is_some_and(|n| n.contains("Call Cab"))
        }) {
            offers.push(entry.action.clone());
        }
    }
    assert!(!offers.is_empty(), "no cab sign offers {action:#x}");
    for offer in offers {
        let game = shell.game_mut().expect("the Downtown lot");
        game.lot_mut()
            .pick(traveler, offer)
            .expect("pick the cab call");
        for _ in 0..CALL_FRAMES {
            if done(shell) {
                return;
            }
            let Some(game) = shell.game_mut() else {
                panic!("the shell left the lot");
            };
            assert!(game.stopped().is_none(), "lot halted: {:?}", game.stopped());
            if let Some(asking) = game.asking() {
                let object = asking.shown.object;
                game.answer(object, 0);
            } else if game.asking_special_panel().is_none() {
                game.run(1);
            }
            before_pump(shell);
            shell.pump_trip_request();
        }
    }
    panic!("no cab call finished");
}

/// A lot save writes `Neighborhood.iff` beside the saves, never into the
/// install, and opening the neighborhood alone writes nothing.
#[test]
fn a_lot_save_writes_neighborhood_iff_beside_the_saves() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run the Neighborhood.iff test");
        return;
    };
    let dir = TempDir::new("nbhd-iff-save");
    let mut shell = open_shell(root.clone(), &dir.0);
    let path = shell.neighborhood_iff_path();
    // The folder is named for the whole saves file name, so `saves.redb`
    // and a `saves.backup` beside it do not share one.
    assert!(
        path.starts_with(dir.0.join("saves.redb.userdata")),
        "{} is not beside the saves",
        path.display()
    );
    assert!(!path.starts_with(&root));
    assert!(!path.exists(), "opening the neighborhood saved it");

    shell.enter_lot(HOME).expect("enter the house");
    let game = shell.game_mut().expect("the house");
    assert_eq!(game.run(30), 30);
    // A stale file is replaced, as the original's NTemp.iff move does.
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("folder");
    std::fs::write(&path, b"old").expect("stale file");
    let before = shell.stored_neighborhood().revision;
    shell.save_current_lot().expect("save the lot");
    assert!(
        shell.stored_neighborhood().revision > before,
        "the save committed"
    );

    let written = read_back(&path, &dir.0);
    let stored = shell.stored_neighborhood();
    let (family, _) = stored.family_in(HOME).expect("the household");
    assert_eq!(written.families.len(), stored.families.len());
    assert_eq!(houses_of(&written, family), houses_of(stored, family));
    assert_eq!(
        written.families[&family].members,
        stored.families[&family].members
    );
    assert!(
        !path.with_file_name("NTemp.iff").exists(),
        "the temporary file was moved over the save"
    );
}

/// A date Downtown and back: the file follows each move. On Downtown, Go
/// Somewhere Else (a trip to another destination) is refused with a notice
/// and dropped, the lot plays on, and Go Home returns the traveler.
#[test]
fn a_destination_go_home_returns_through_the_shell() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run the Go Home test");
        return;
    };
    let dir = TempDir::new("go-home");
    let mut shell = open_shell(root, &dir.0);
    shell.enter_lot(HOME).expect("enter the house");
    let game = shell.game_mut().expect("the house");
    assert_eq!(game.run(30), 30);
    let (family, leader) = {
        let world = game.lot().world();
        let selected = world.selected().expect("a selected Sim");
        (world.globals[9] as u16, world.objects[&selected].guid)
    };
    let date = date_for(game.lot().world(), leader);
    shell
        .depart_to_destination(TravelRequest::downtown_date(DOWNTOWN_LOT, leader, date))
        .expect("travel Downtown");
    assert_eq!(shell.active_lot_house(), Some(DOWNTOWN_LOT));
    let path = shell.neighborhood_iff_path();
    assert_eq!(
        houses_of(&read_back(&path, &dir.0), family),
        (HOME as i32, DOWNTOWN_LOT as i32),
        "the saved neighborhood has the family Downtown"
    );
    let trip_id = shell.active_trip().expect("the trip").trip_id;
    // The arrival places the traveler before anyone calls a cab.
    assert_eq!(shell.game_mut().expect("Downtown").run(400), 400);

    // Go Somewhere Else asks for lot 22 on the Downtown map. The shell
    // refuses a trip between destinations and the lot drops it.
    let mut refused = false;
    call_cab(
        &mut shell,
        leader,
        GO_SOMEWHERE_ELSE,
        |shell| {
            if shell
                .game()
                .is_some_and(|g| g.asking_special_panel().is_some())
            {
                shell.render();
            }
            let picker = shell
                .game()
                .and_then(|g| g.destination_picker())
                .and_then(|p| p.hit_point_for_lot(SIZE, OTHER_DOWNTOWN_LOT));
            if let Some(point) = picker {
                let _ = shell.hover(point);
                let _ = shell.press(point);
                let _ = shell.release(point);
            }
        },
        |shell| {
            refused = shell.game().is_some_and(|g| {
                g.note().is_some_and(|n| {
                    n.contains("Return home before starting another destination trip")
                })
            });
            refused
        },
    );
    assert!(refused);
    assert_eq!(shell.active_lot_house(), Some(DOWNTOWN_LOT));
    assert_eq!(shell.active_trip().map(|t| t.trip_id), Some(trip_id));
    let game = shell.game_mut().expect("the Downtown lot");
    assert!(game.trip_request().is_none(), "the refused trip is dropped");
    assert!(
        game.lot().world().trip_request.is_none(),
        "the lot dropped the request"
    );
    let _ = game.dialog_confirm_key(DialogKey::Enter);
    assert_eq!(game.run(10), 10, "the lot plays on");

    // Go Home asks for house -1, the family's house, and the shell returns.
    // Routine 17 saved the caller's state (kind-2 tokens, with the arrival
    // clock in 7 and 8) when it asked.
    let mut asked: Option<Vec<(u32, u16)>> = None;
    call_cab(
        &mut shell,
        leader,
        GO_HOME,
        |shell| {
            if let Some(game) = shell.game()
                && let Some(trip) = game.trip_request()
            {
                assert_eq!(i32::from(trip.house), HOME as i32, "Go Home asks for home");
                asked = Some(tokens_of(game.lot().world(), leader, SAVED_STATE_TOKEN));
            }
        },
        |shell| shell.active_lot_house() == Some(HOME),
    );
    let asked = asked.expect("Go Home asked for the trip");
    assert!(!asked.is_empty(), "routine 17 saved the caller's state");
    assert!(shell.active_trip().is_none(), "the trip is over");
    let game = shell.game_mut().expect("the house");
    assert!(game.trip_request().is_none());
    let record = {
        let world = game.lot().world();
        assert!(
            world
                .objects
                .values()
                .any(|o| o.guid == leader && o.person.is_some()),
            "the traveler is home"
        );
        world
            .neighbors
            .first_of(leader)
            .expect("the leader's record")
            .id
    };
    // The return took back the saved state Go Home's routine 17 wrote, as
    // the plugin's tail on the home lot would (routines 23 and 21): on the
    // lot, in the store and in the saved file.
    let saved = tokens_of(game.lot().world(), leader, SAVED_STATE_TOKEN);
    assert!(saved.is_empty(), "saved-state tokens came home: {saved:?}");
    assert_eq!(game.run(10), 10, "the house plays on");
    assert_eq!(
        houses_of(shell.stored_neighborhood(), family),
        (HOME as i32, 0)
    );
    assert_eq!(
        stored_tokens(shell.stored_neighborhood(), record, SAVED_STATE_TOKEN),
        [],
        "the store kept the saved state"
    );
    let written = read_back(&path, &dir.0);
    assert_eq!(
        houses_of(&written, family),
        (HOME as i32, 0),
        "the saved neighborhood has the family home"
    );
    assert_eq!(
        stored_tokens(&written, record, SAVED_STATE_TOKEN),
        [],
        "the saved neighborhood kept the saved state"
    );
}
