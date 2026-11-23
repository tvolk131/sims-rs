//! A Sim who arrives Downtown is seen through the middle ped marker's
//! arrival pass.
//!
//! Downtown's middle ped marker (EP3 `PedMarkers.iff`, GUID 0xbc17335c,
//! tree 0x100e "main - middle ped marker") puts the arriving family's
//! first member beside it, clears its attribute 3, and runs Generic Sims
//! Call 19 (node 11) to bring back whoever traveled with that Sim. With
//! nobody (temp 0 is -1) it sets the Sim's PetsOutside word (node 24),
//! waits 30 ticks, restores the Sim's saved state (routine 21) and takes
//! down the travel picture (routine 3). Routine 19 used to be missing: its
//! fault reset the marker's tree, so nothing after node 11 ran.
//!
//! On a date the leader's inventory holds a token of kind 1 naming the
//! companion (Manage Inventory operation 5 at departure). Routine 19 takes
//! it and names the companion, and the marker places the companion and
//! links the two by their DateFlag words (nodes 16 and 23).
//!
//! Needs an install (`SIMS_INSTALL`); skipped otherwise.

use sims_game::Frames;
use sims_game::neighborhood::NeighborhoodShell;
use sims_vm::travel::TravelRequest;
use std::path::PathBuf;

const DOWNTOWN_LOT: u32 = 21;
const MIDDLE_PED_MARKER_GUID: u32 = 0xbc17_335c;
/// Ticks the marker's arrival pass may take: it places the Sim about ten
/// ticks in and waits 30 after routine 19.
const ARRIVAL: u32 = 120;
/// Person data word 34, PetsOutside.
const PETS_OUTSIDE: usize = 34;
/// Person data word 77, DateFlag.
const DATE_FLAG: usize = 77;
/// Person data words 58 (age), 61 (family) and 65 (kind: 0 or 1 for a
/// human).
const AGE: usize = 58;
const FAMILY: usize = 61;
const KIND: usize = 65;
/// The token kind that names a traveler's companion.
const COMPANION_TOKEN: i32 = 1;

fn install_root() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    dir.is_dir().then_some(dir)
}

/// The Goth house's selected Sim, 30 ticks in, sent Downtown by
/// `request(leader)`; then the lot's ARRIVAL ticks, checked to have run
/// with no error from the marker, and `check` given the game, the
/// traveler's object and the marker's.
fn arrive(
    dir_name: &str,
    request: impl FnOnce(&sims_game::Game, u32) -> TravelRequest,
    check: impl FnOnce(&sims_game::Game, u16, u16),
) {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run the Downtown arrival test");
        return;
    };
    let dir = std::env::temp_dir().join(format!("{dir_name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut shell = NeighborhoodShell::open(
        root,
        dir.join("saves.redb"),
        1,
        true,
        vec![3],
        Frames::default(),
    )
    .expect("open the neighborhood");
    shell.resize([1024, 768]);
    shell.enter_lot(5).expect("enter the Goth house");
    let game = shell.game_mut().expect("active lot");
    assert_eq!(game.run(30), 30);
    let leader = {
        let world = game.lot().world();
        world.objects[&world.selected().expect("selected Goth Sim")].guid
    };
    let request = request(game, leader);
    shell
        .depart_to_destination(request)
        .expect("travel Downtown");
    assert_eq!(shell.active_lot_house(), Some(DOWNTOWN_LOT));

    let game = shell.game_mut().expect("the Downtown lot");
    let (traveler, marker) = {
        let world = game.lot().world();
        let traveler = world
            .objects
            .values()
            .find(|o| o.guid == leader && o.person.is_some())
            .map(|o| o.id)
            .expect("the traveler arrived");
        let marker = world
            .objects
            .values()
            .find(|o| o.guid == MIDDLE_PED_MARKER_GUID)
            .map(|o| o.id)
            .expect("the lot has a middle ped marker");
        (traveler, marker)
    };
    assert_eq!(person_word(game, traveler, PETS_OUTSIDE), 0);

    // An unanswered dialog would stop the ticks without stopping the lot.
    assert_eq!(game.run(ARRIVAL), ARRIVAL, "the lot stopped advancing");
    assert!(game.stopped().is_none(), "lot halted: {:?}", game.stopped());
    let world = game.lot().world();
    assert_eq!(
        world.objects[&marker].attributes[3], 0,
        "the marker never started its arrival pass"
    );
    let from_marker = format!("object {marker}, tree 0x100e");
    let caught: Vec<&String> = game
        .errors()
        .iter()
        .filter(|e| e.contains(&from_marker) || e.contains("Generic Sims Call routine 19"))
        .collect();
    assert!(caught.is_empty(), "caught: {caught:?}");
    check(game, traveler, marker);
}

fn person_word(game: &sims_game::Game, id: u16, word: usize) -> i16 {
    game.lot().world().objects[&id]
        .person
        .as_ref()
        .expect("a person")
        .data[word]
}

#[test]
fn the_middle_ped_marker_finishes_a_downtown_arrival() {
    arrive(
        "sims-downtown-arrival",
        |_, leader| TravelRequest::downtown(DOWNTOWN_LOT, leader),
        |game, traveler, _| {
            // Node 24 follows routine 19 when nobody traveled with the Sim.
            assert_eq!(
                person_word(game, traveler, PETS_OUTSIDE),
                1,
                "the marker's arrival pass stopped before node 24"
            );
        },
    );
}

#[test]
fn the_middle_ped_marker_brings_back_a_date() {
    let companion_guid = std::cell::Cell::new(0);
    arrive(
        "sims-downtown-date",
        |game, leader| {
            let guid = date_for(game, leader);
            companion_guid.set(guid);
            TravelRequest::downtown_date(DOWNTOWN_LOT, leader, guid)
        },
        |game, traveler, _| {
            let world = game.lot().world();
            let companion = world
                .objects
                .values()
                .find(|o| o.guid == companion_guid.get() && o.person.is_some())
                .map(|o| o.id)
                .expect("the companion is on the lot");
            let leader_guid = world.objects[&traveler].guid;
            assert_eq!(
                companion_tokens(game, leader_guid),
                0,
                "routine 19 left the companion's token"
            );
            // Node 26 marks the companion, node 16 links it to the leader.
            assert_eq!(person_word(game, companion, PETS_OUTSIDE), 1);
            assert_eq!(
                person_word(game, companion, DATE_FLAG),
                traveler as i16,
                "the companion's DateFlag does not name the leader"
            );
        },
    );
}

/// An adult human of another family than `leader`'s, from the lot's
/// neighbor records.
fn date_for(game: &sims_game::Game, leader: u32) -> u32 {
    let world = game.lot().world();
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

/// The kind-1 tokens in `guid`'s neighbor inventory.
fn companion_tokens(game: &sims_game::Game, guid: u32) -> usize {
    let world = game.lot().world();
    let record = world.neighbors.first_of(guid).expect("a neighbor record");
    world
        .neighbors
        .inventories
        .iter()
        .filter(|i| i.neighbor == record.id)
        .flat_map(|i| &i.items)
        .filter(|t| t.kind == COMPANION_TOKEN)
        .count()
}

#[test]
fn an_aborted_date_takes_back_the_companion_token() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run the Downtown arrival test");
        return;
    };
    let dir = std::env::temp_dir().join(format!("sims-downtown-abort-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let open = || {
        let mut shell = NeighborhoodShell::open(
            root.clone(),
            dir.join("saves.redb"),
            1,
            true,
            vec![3],
            Frames::default(),
        )
        .expect("open the neighborhood");
        shell.resize([1024, 768]);
        shell
    };
    let mut shell = open();
    shell.enter_lot(5).expect("enter the Goth house");
    let game = shell.game_mut().expect("active lot");
    assert_eq!(game.run(30), 30);
    let leader = {
        let world = game.lot().world();
        world.objects[&world.selected().expect("selected Goth Sim")].guid
    };
    let companion = date_for(game, leader);
    shell
        .depart_to_destination(TravelRequest::downtown_date(
            DOWNTOWN_LOT,
            leader,
            companion,
        ))
        .expect("travel Downtown");
    // The token is there on arrival, before the marker takes it.
    assert_eq!(
        companion_tokens(shell.game_mut().expect("the Downtown lot"), leader),
        1
    );
    drop(shell);

    let mut shell = open();
    assert_eq!(shell.abort_active_trip(5).expect("abort the trip"), 5);
    shell.enter_lot(5).expect("enter the Goth house again");
    let game = shell.game_mut().expect("the Goth house again");
    assert_eq!(
        companion_tokens(game, leader),
        0,
        "the aborted date kept its token"
    );
}
