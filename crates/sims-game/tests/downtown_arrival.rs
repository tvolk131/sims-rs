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
use sims_ui::shop::{CurrencyKind, ShopCommand};
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

/// A trip Downtown and its first ARRIVAL ticks.
struct Arrived {
    shell: NeighborhoodShell,
    /// The leader's object on the Downtown lot.
    traveler: u16,
    /// The home family and its funds before departure.
    home_family: u16,
    home_funds: i32,
}

/// The Goth house 30 ticks in, sent Downtown by `request(leader)`, where
/// `leader` (a GUID) is the selected Sim unless `pick` names another. Then
/// the lot's ARRIVAL ticks, checked to have run with no error from the
/// marker. The leader arrives off the lot, as the person factory makes it
/// (0x4e9719 -> 0x440b2d); the marker's placement is its one jump, and it
/// ends on the lot.
fn arrive(
    dir_name: &str,
    pick: impl FnOnce(&sims_game::Game) -> Option<u32>,
    request: impl FnOnce(&sims_game::Game, u32) -> TravelRequest,
) -> Option<Arrived> {
    let Some(root) = install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to run the Downtown arrival test");
        return None;
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
    let leader = pick(game).unwrap_or_else(|| {
        let world = game.lot().world();
        world.objects[&world.selected().expect("selected Goth Sim")].guid
    });
    let (home_family, home_funds) = {
        let world = game.lot().world();
        (world.globals[9] as u16, world.budget.funds)
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

    let mut last = game.lot().world().objects[&traveler].position;
    assert!(last[0] < 0, "the traveler arrived on the lot at {last:?}");
    let mut jumps = Vec::new();
    for _ in 0..ARRIVAL {
        // An unanswered dialog would stop the ticks without stopping the lot.
        assert_eq!(game.run(1), 1, "the lot stopped advancing");
        assert!(game.stopped().is_none(), "lot halted: {:?}", game.stopped());
        let now = game.lot().world().objects[&traveler].position;
        // A walking Sim moves about a sixteenth of a tile a tick; more than
        // a tile and a half at once is a jump.
        let step = (now[0] - last[0]).abs().max((now[1] - last[1]).abs());
        if step > 24 || now[2] != last[2] {
            let tick = game.lot().ticks();
            jumps.push(format!("tick {tick}: {last:?} -> {now:?}"));
        }
        last = now;
    }
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
    assert!(last[0] >= 0, "the marker never placed the traveler");
    assert_eq!(
        jumps.len(),
        1,
        "the traveler should jump once, when the marker places it: {jumps:?}"
    );
    Some(Arrived {
        shell,
        traveler,
        home_family,
        home_funds,
    })
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
    let Some(mut arrived) = arrive(
        "sims-downtown-arrival",
        |_| None,
        |_, leader| TravelRequest::downtown(DOWNTOWN_LOT, leader),
    ) else {
        return;
    };
    let game = arrived.shell.game_mut().expect("the Downtown lot");
    // Node 24 follows routine 19 when nobody traveled with the Sim.
    assert_eq!(
        person_word(game, arrived.traveler, PETS_OUTSIDE),
        1,
        "the marker's arrival pass stopped before node 24"
    );
}

#[test]
fn the_middle_ped_marker_brings_back_a_date() {
    let companion_guid = std::cell::Cell::new(0);
    let Some(mut arrived) = arrive(
        "sims-downtown-date",
        |_| None,
        |game, leader| {
            let guid = date_for(game, leader);
            companion_guid.set(guid);
            TravelRequest::downtown_date(DOWNTOWN_LOT, leader, guid)
        },
    ) else {
        return;
    };
    let traveler = arrived.traveler;
    let game = arrived.shell.game_mut().expect("the Downtown lot");
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
}

/// A solo trip by a household member other than the first. The traveler
/// leads family 0 on the destination (0x4f1c87..0x4f1cec: 0x410a41 erases
/// the family's members and adds the traveler), so Generic Sims Call 18
/// (0x4e97dc(0)) names them and the marker places them. Global 9 stays 0,
/// the funds are the traveler's family's (0x4f1cc0..0x4f1ce7), the lot is
/// played rather than edited (0x4f1b7c..0x4f1b9d), and a purchase there is
/// carried home.
#[test]
fn a_solo_traveler_who_is_not_the_first_member_leads_the_visit() {
    let Some(mut arrived) = arrive(
        "sims-downtown-second",
        |game| {
            let world = game.lot().world();
            let family = i32::from(world.globals[9]);
            let members = &world
                .families
                .iter()
                .find(|f| f.id == family)
                .expect("the Goth household")
                .members;
            let second = members.iter().skip(1).copied().find(|&guid| {
                world
                    .neighbors
                    .first_of(guid)
                    .is_some_and(|n| n.data[AGE] >= 18)
            });
            Some(second.expect("an adult Goth who is not member 0"))
        },
        |_, leader| TravelRequest::downtown(DOWNTOWN_LOT, leader),
    ) else {
        return;
    };
    let traveler = arrived.traveler;
    let home_family = arrived.home_family;
    let game = arrived.shell.game_mut().expect("the Downtown lot");
    assert_eq!(person_word(game, traveler, PETS_OUTSIDE), 1);
    assert!(!game.edit_mode(), "a solo visit opened in edit mode");
    let world = game.lot().world();
    assert_eq!(world.globals[9], 0, "a solo visit leaves global 9 at 0");
    let funds = world.budget.funds;
    assert!(
        funds > 0 && funds < arrived.home_funds,
        "the visit should hold the family's funds less the fare: {funds} of {}",
        arrived.home_funds
    );
    let panel = game.snapshot(world);
    assert_eq!(panel.funds, funds);
    let shown: Vec<u16> = panel.residents.iter().map(|r| r.object).collect();
    assert_eq!(shown, vec![traveler], "the panel shows the traveler alone");

    let kind = game.shop_kind_for_lot().expect("Downtown's shop");
    assert!(game.open_shop(kind), "the shop refused a solo visit");
    let (item_id, price) = game
        .shop_snapshot()
        .expect("the shop's items")
        .items
        .iter()
        .find(|i| {
            i.currency == CurrencyKind::Simoleons
                && i.available
                && !i.owned
                && (1..=funds).contains(&i.price)
        })
        .map(|i| (i.id.clone(), i.price))
        .expect("an item for sale in Simoleons");
    let _ = game.execute_shop_command(ShopCommand::Buy {
        item_id,
        currency: CurrencyKind::Simoleons,
        price,
    });
    let spent = funds - price;
    assert_eq!(
        game.lot().world().budget.funds,
        spent,
        "the purchase was not charged"
    );

    arrived
        .shell
        .return_from_destination()
        .expect("return home");
    assert_eq!(arrived.shell.active_lot_house(), Some(5));
    let home = arrived.shell.game().expect("the Goth house again");
    assert_eq!(home.lot().world().globals[9] as u16, home_family);
    assert_eq!(
        home.lot().world().budget.funds,
        spent,
        "the purchase did not come home"
    );
    assert_eq!(
        arrived.shell.stored_neighborhood().families[&home_family].budget,
        spent
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
