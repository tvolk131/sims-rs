//! Calling a cab on the Goth house's phone and going Downtown, end to end.
//!
//! Mortimer picks "Go Downtown - $50" on the phone (PhoneGlobals tree 8309,
//! 0x2075). The call dispatches to the Go Downtown plugin (GUID 0xA6F31853,
//! tree 0x1004), which pays the fare with Alter Budget, asks for a lot (the
//! DowntownLotPicker special panel) and runs Generic Sims Call 17 with
//! (house, 0, 30, mode 0). Routine 17 (0x45a22d) writes Mortimer's saved
//! state and the arrival clock into his neighbor inventory and asks for the
//! trip. `travel::depart_from_request` carries the trip out, and Downtown's
//! middle ped marker places him and restores the saved state with routine
//! 21, which leaves the clock tokens for the plugin's routine 23.
//!
//! Needs an install (`SIMS_INSTALL`); skipped otherwise.

use sims_sim::NeighborhoodStore;
use sims_sim::neighbors::Item;
use sims_vm::boot::{Frames, Lot};
use sims_vm::travel::trip::{ARRIVAL_HOUR_GUID, ARRIVAL_MINUTE_GUID, SAVED_STATE_KIND, ftol_word};
use std::path::PathBuf;

const GOTH_HOUSE: u32 = 5;
const DOWNTOWN_LOT: i16 = 21;
/// The Goth house's phone that Mortimer can reach (the others fail to
/// route), and its GUID.
const PHONE: u16 = 10;
const PHONE_GUID: u32 = 0x6b26_4238;
const FARE: i32 = 50;
/// The motives the saved state holds, by token GUID 0 to 6 (0x45a40a).
const SAVED_MOTIVES: [(u32, usize); 7] = [(0, 9), (1, 6), (2, 5), (3, 15), (4, 7), (5, 8), (6, 14)];
/// Ticks the call may take: the cab comes, Mortimer picks a lot, routine 17.
const CALL_TICKS: u32 = 5000;
/// Ticks the marker may take to place Mortimer and restore his state.
const ARRIVAL_TICKS: u32 = 300;

struct Guard(PathBuf);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn saved_state(lot: &Lot, guid: u32) -> Vec<Item> {
    let world = lot.world();
    let record = world.neighbors.first_of(guid).expect("a neighbor record");
    world
        .neighbors
        .inventories
        .iter()
        .filter(|i| i.neighbor == record.id)
        .flat_map(|i| i.items.iter().cloned())
        .filter(|t| t.kind == SAVED_STATE_KIND)
        .collect()
}

fn person_of(lot: &Lot, guid: u32) -> u16 {
    let world = lot.world();
    world
        .people
        .iter()
        .copied()
        .find(|&id| world.objects[&id].guid == guid)
        .expect("the person is on the lot")
}

#[test]
fn calling_a_cab_on_the_phone_goes_downtown_with_the_saved_state() {
    let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
        eprintln!("skipping: set SIMS_INSTALL to run the phone Go Downtown test");
        return;
    };
    let db = std::env::temp_dir().join(format!("sims-phone-downtown-{}.redb", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let _guard = Guard(db.clone());
    let store = NeighborhoodStore::open(&db).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();
    let nh = store.load_neighborhood("UserData").unwrap().unwrap();
    let mortimer_guid = nh.family_in(GOTH_HOUSE).unwrap().1.members[0];

    let mut home = sims_vm::lifecycle::enter_lot(
        &root,
        &store,
        "UserData",
        GOTH_HOUSE,
        &[3],
        Frames::default(),
    )
    .unwrap();
    for _ in 0..5 {
        home.step().unwrap();
    }
    let mortimer = person_of(&home, mortimer_guid);
    assert_eq!(home.world().objects[&PHONE].guid, PHONE_GUID);
    let funds_before = home.world().budget.funds;

    let menu = home.pie_menu(mortimer, PHONE).unwrap();
    let go = menu
        .entries
        .iter()
        .find(|e| e.name.as_deref().is_some_and(|n| n.contains("Go Downtown")))
        .expect("the phone offers Go Downtown")
        .action
        .clone();
    assert_eq!(go.action, 0x2075, "PhoneGlobals' Call Plugin");
    home.pick(mortimer, go).unwrap();
    home.wait_for_answers();

    // Two dialogs (call the cab, then confirm), each answered with its first
    // button; then the lot picker, answered with lot 21.
    let mut dialogs = 0;
    let mut panels = 0;
    let mut asked = None;
    for _ in 0..CALL_TICKS {
        match home.step_play() {
            Ok(update) => {
                assert!(update.errors.is_empty(), "errors: {:?}", update.errors);
            }
            Err(error) => {
                if home.asking().is_some() {
                    dialogs += 1;
                    home.answer(2).unwrap();
                    continue;
                }
                if home.asking_special_panel().is_some() {
                    panels += 1;
                    home.answer_special_panel(sims_vm::SpecialPanelResponse::Lot(Some(
                        DOWNTOWN_LOT,
                    )))
                    .unwrap();
                    continue;
                }
                panic!("the call stopped: {error}");
            }
        }
        if let Some(request) = home.world().trip_request {
            asked = Some(request);
            break;
        }
    }
    assert_eq!((dialogs, panels), (2, 1));
    let request = asked.expect("routine 17 asked for the trip");
    assert_eq!(
        request,
        sims_sim::TripRequest {
            house: DOWNTOWN_LOT,
            leader_guid: mortimer_guid,
            family: 0,
            caller: mortimer,
        }
    );
    // The plugin paid the fare itself, with Alter Budget.
    assert_eq!(home.world().budget.funds, funds_before - FARE);

    // Routine 17 ran this tick: the tokens are Mortimer's motives cut to
    // whole numbers, and the clock half an hour on.
    let tokens = saved_state(&home, mortimer_guid);
    let world = home.world();
    let motives = &world.objects[&mortimer].person.as_ref().unwrap().motives;
    let mut expected: Vec<(u32, i16)> = SAVED_MOTIVES
        .iter()
        .map(|&(guid, motive)| (guid, ftol_word(motives[motive])))
        .filter(|&(_, count)| count != 0)
        .collect();
    let data = &world.objects[&mortimer].person.as_ref().unwrap().data;
    for (guid, word) in [(9, 8), (10, 80), (11, 82), (12, 81)] {
        if data[word] != 0 {
            expected.push((guid, data[word]));
        }
    }
    let mut minutes = world.globals[5] + 30;
    let mut hours = world.globals[0];
    if minutes >= 60 {
        hours += 1;
        minutes %= 60;
    }
    hours %= 24;
    for (guid, count) in [(ARRIVAL_HOUR_GUID, hours), (ARRIVAL_MINUTE_GUID, minutes)] {
        if count != 0 {
            expected.push((guid, count));
        }
    }
    let written: Vec<(u32, i16)> = tokens.iter().map(|t| (t.guid, t.count as i16)).collect();
    assert_eq!(written, expected);
    let clock_tokens: Vec<Item> = tokens
        .iter()
        .filter(|t| t.guid == ARRIVAL_HOUR_GUID || t.guid == ARRIVAL_MINUTE_GUID)
        .cloned()
        .collect();

    // The shell leaves the house between ticks.
    let departed =
        sims_vm::travel::depart_from_request(&root, &store, "UserData", &mut home).unwrap();
    assert_eq!(
        home.world().trip_request,
        None,
        "the request is carried out"
    );
    let session = departed.session;
    assert_eq!(session.destination_house, DOWNTOWN_LOT as u32);
    assert_eq!(session.leader_guid, mortimer_guid);
    assert_eq!(session.fare_paid, 0, "the plugin already paid");
    assert_eq!(session.funds_at_arrival, funds_before - FARE);
    assert_eq!(session.travelers.len(), 1);
    let mut downtown = departed.destination_lot;
    assert_eq!(
        saved_state(&downtown, mortimer_guid),
        tokens,
        "the tokens travel with Mortimer"
    );

    // The marker places Mortimer and restores his saved state (routine
    // 21): on that tick his motives are the tokens' counts.
    let traveler = person_of(&downtown, mortimer_guid);
    let mut restored = false;
    for _ in 0..ARRIVAL_TICKS {
        let update = downtown.step_play().unwrap();
        assert!(update.errors.is_empty(), "errors: {:?}", update.errors);
        let now = saved_state(&downtown, mortimer_guid);
        if now.len() < tokens.len() {
            assert_eq!(now, clock_tokens, "routine 21 leaves only the clock");
            let motives = &downtown.world().objects[&traveler]
                .person
                .as_ref()
                .unwrap()
                .motives;
            for &(guid, motive) in &SAVED_MOTIVES {
                let count = tokens
                    .iter()
                    .find(|t| t.guid == guid)
                    .map_or(0, |t| t.count as i16);
                assert!(
                    (motives[motive] - f32::from(count)).abs() < 0.5,
                    "motive {motive} is {} after routine 21, not about {count}",
                    motives[motive]
                );
            }
            restored = true;
            break;
        }
    }
    assert!(restored, "the marker never restored Mortimer's saved state");
    let position = downtown.world().objects[&traveler].position;
    assert!(
        position[0] >= 0 && position[1] >= 0,
        "the marker placed Mortimer"
    );
}
