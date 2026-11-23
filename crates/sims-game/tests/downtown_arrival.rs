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

fn install_root() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("SIMS_INSTALL")?);
    dir.is_dir().then_some(dir)
}

#[test]
fn the_middle_ped_marker_finishes_a_downtown_arrival() {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run the Downtown arrival test");
        return;
    };
    let dir = std::env::temp_dir().join(format!("sims-downtown-arrival-{}", std::process::id()));
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
    shell
        .depart_to_destination(TravelRequest::downtown(DOWNTOWN_LOT, leader))
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
    let pets_outside = |game: &sims_game::Game| {
        game.lot().world().objects[&traveler]
            .person
            .as_ref()
            .expect("the traveler is a person")
            .data[PETS_OUTSIDE]
    };
    assert_eq!(pets_outside(game), 0);

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
    // Node 24 follows routine 19 when nobody traveled with the Sim.
    assert_eq!(
        pets_outside(game),
        1,
        "the marker's arrival pass stopped before node 24"
    );
}
