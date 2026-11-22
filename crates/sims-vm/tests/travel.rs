//! Live-install Complete Collection 1.91 integration tests for the
//! Away-From-Home Travel Engine (`sims_vm::travel`).

use sims_sim::NeighborhoodStore;
use sims_vm::boot::Frames;
use sims_vm::lifecycle::{enter_lot, save_and_leave_lot};
use sims_vm::travel::{
    DOWNTOWN_FARE, HOTEL_KEY_TOKEN_GUID, HOTEL_KEY_TOKEN_KIND, MAGIC_BLIMP_FARE, MAGIC_HOLE_FARE,
    OLD_TOWN_FARE, STUDIO_TOWN_FARE, TravelDestination, TravelError, TravelParty, TravelRequest,
    TravelTransport, VACATION_FARE, VacationAccommodation, classify_destination,
    depart_to_destination, neighbor_token_count, open_destination_for_edit,
    return_from_destination,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DB: AtomicU64 = AtomicU64::new(0);

struct TempDb(PathBuf);

impl TempDb {
    fn new(label: &str) -> Self {
        let n = NEXT_DB.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sims-vm-travel-{label}-{}-{n}.redb",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(path) => Some(PathBuf::from(path)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping live-install travel test");
            None
        }
    }
}

/// Steps a residential lot (home lot or Magic Town residential lot) in strict
/// mode, asserting zero script or engine faults.
fn step_strict(lot: &mut sims_vm::boot::Lot, ticks: u32, label: &str) {
    let start_ticks = lot.ticks();
    for i in 0..ticks {
        lot.step()
            .unwrap_or_else(|e| panic!("{label}: strict step failed on tick {}: {e}", i + 1));
    }
    assert_eq!(lot.ticks(), start_ticks + ticks, "{label}: ticks advanced");
}

/// Steps a community/unzoned destination lot for `ticks` ticks in play mode,
/// verifying:
/// 1. Every tick succeeds without halting (`step_play` returns `Ok`).
/// 2. Any caught per-object script error is strictly an unported VM primitive
///    (`Fault::Trap(Trap::Unsupported(_))`, e.g. community-lot `Find Best
///    Interaction` or `Generic Sims Call 26`), with ZERO `Trap::Invalid`,
///    `Trap::Native`, or corrupted-object faults.
/// 3. Both `lot.ticks()` and `lot.world().ticks` advance cleanly.
fn step_community(lot: &mut sims_vm::boot::Lot, ticks: u32, label: &str) {
    let start_ticks = lot.ticks();
    let start_world_ticks = lot.world().ticks;
    for i in 0..ticks {
        let update = lot
            .step_play()
            .unwrap_or_else(|e| panic!("{label}: play step halted on tick {}: {e}", i + 1));
        for err in &update.errors {
            assert!(
                err.fault.trap.unported(),
                "{label}: unexpected non-unported fault on tick {} object {} (guid=0x{:08X}, person={}): {}",
                i + 1,
                err.object,
                lot.world().objects.get(&err.object).map_or(0, |o| o.guid),
                lot.world()
                    .objects
                    .get(&err.object)
                    .is_some_and(|o| o.person.is_some()),
                err.fault
            );
            if let Some(then) = &err.then {
                assert!(
                    then.trap.unported(),
                    "{label}: unexpected secondary fault on tick {} object {}: {then}",
                    i + 1,
                    err.object
                );
            }
        }
    }
    assert_eq!(lot.ticks(), start_ticks + ticks, "{label}: ticks advanced");
    assert!(
        lot.world().ticks > start_world_ticks,
        "{label}: world.ticks advanced"
    );
}

#[test]
fn goth_downtown_house21_round_trip_and_community_edit_persistence() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("downtown-21");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    let nh = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(
        classify_destination(21, &nh.zoning),
        Some(TravelDestination::Downtown)
    );
    let goth_members = nh.family_in(5).unwrap().1.members.clone();
    let mortimer_guid = goth_members[0];
    let bella_guid = goth_members[1];

    // Edit Downtown House 21 in Build/Buy mode from Neighborhood View and save
    // to REDB — matching Sims 1's community-lot editing flow.
    let mut dt_edit =
        open_destination_for_edit(&root, &store, "UserData", 21, &[3], Frames::default()).unwrap();
    assert_eq!(dt_edit.world().globals[32], 1, "edit mode unlimited budget");
    dt_edit.set_lot_info(
        "Goth Downtown Lounge".to_string(),
        "Persisted community edit".to_string(),
    );
    save_and_leave_lot(&root, &store, "UserData", &mut dt_edit).unwrap();

    // Boot Goth House 5, tick 10 strict ticks so home lot has live state.
    let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    step_strict(&mut goth, 10, "goth home pre-departure");
    let home_ticks_before = goth.ticks();
    let home_world_ticks_before = goth.world().ticks;
    let home_clock_before = (
        goth.world().globals[0],
        goth.world().globals[5],
        goth.world().globals[6],
    );
    let home_funds_before = goth.world().budget.funds;
    let bella_pos_before = goth
        .world()
        .people
        .iter()
        .find_map(|id| {
            let o = &goth.world().objects[id];
            (o.guid == bella_guid).then_some(o.position)
        })
        .unwrap();

    // Depart Mortimer solo to Downtown House 21 ($50 cab).
    let req = TravelRequest::downtown(21, mortimer_guid);
    let dep = depart_to_destination(&root, &store, "UserData", &mut goth, &req).unwrap();
    let mut dt = dep.destination_lot;
    let session = dep.session;
    assert_eq!(session.destination, TravelDestination::Downtown);
    assert_eq!(session.fare_paid, DOWNTOWN_FARE);
    assert_eq!(session.funds_at_arrival, home_funds_before - DOWNTOWN_FARE);
    assert_eq!(dt.world().budget.funds, home_funds_before - DOWNTOWN_FARE);
    assert_eq!(dt.world().globals[32], 0, "live community spending mode");
    assert_eq!(dt.world().globals[10], 21);
    assert_eq!(dt.world().globals[9], 5);
    assert_eq!(session.travelers.len(), 1);
    assert_eq!(dt.lot_name(), "Goth Downtown Lounge");
    assert_eq!(dt.lot_description(), "Persisted community edit");

    // While away, REDB records `secondary_house == 21` on family 5.
    let nh_away = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(nh_away.families[&5].secondary_house(), 21);

    // Tick Downtown lot 220 ticks cleanly.
    step_community(&mut dt, 220, "downtown house 21");
    let dt_mortimer_id = session.travelers[0].destination_object_id;
    let dt_motives_after = dt.world().objects[&dt_mortimer_id]
        .person
        .as_ref()
        .unwrap()
        .motives;
    let dt_funds_after = dt.world().budget.funds;

    // Return home and verify frozen home clock, non-traveler preservation,
    // returning traveler reconciliation, and zero duplicates.
    let ret =
        return_from_destination(&root, &store, &session, &mut dt, &[3], Frames::default()).unwrap();
    let mut home_after = ret.home_lot;
    assert_eq!(ret.report.restored_home_ticks, home_ticks_before);
    assert_eq!(
        ret.report.restored_home_world_ticks,
        home_world_ticks_before
    );
    assert_eq!(ret.report.restored_home_clock, home_clock_before);
    assert_eq!(home_after.ticks(), home_ticks_before);
    assert_eq!(home_after.world().ticks, home_world_ticks_before);
    assert_eq!(home_after.world().budget.funds, dt_funds_after);

    // Non-traveler Bella stayed untouched at her frozen home position.
    let bella_pos_after = home_after
        .world()
        .people
        .iter()
        .find_map(|id| {
            let o = &home_after.world().objects[id];
            (o.guid == bella_guid).then_some(o.position)
        })
        .unwrap();
    assert_eq!(bella_pos_after, bella_pos_before);

    // Exactly one Mortimer on home lot with returning motives intact.
    let mortimers: Vec<u16> = home_after
        .world()
        .people
        .iter()
        .copied()
        .filter(|id| home_after.world().objects[id].guid == mortimer_guid)
        .collect();
    assert_eq!(mortimers.len(), 1);
    assert_eq!(
        home_after.world().objects[&mortimers[0]]
            .person
            .as_ref()
            .unwrap()
            .motives,
        dt_motives_after
    );

    // Verify secondary_house cleared back to 0 in REDB.
    let nh_after = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(nh_after.families[&5].secondary_house(), 0);
    assert_eq!(nh_after.families[&5].budget, dt_funds_after);

    // Home lot ticks cleanly in strict mode after returning.
    step_strict(&mut home_after, 50, "goth home post-downtown return");

    // Second visit to Downtown House 21 verifies community-lot REDB persistence
    // (lot metadata kept, zero duplicate Mortimer stranded from Visit 1).
    let req2 = TravelRequest::downtown(21, mortimer_guid);
    let dep2 = depart_to_destination(&root, &store, "UserData", &mut home_after, &req2).unwrap();
    let mut dt2 = dep2.destination_lot;
    assert_eq!(dt2.lot_name(), "Goth Downtown Lounge");
    assert_eq!(dt2.lot_description(), "Persisted community edit");
    let dt2_mortimers = dt2
        .world()
        .people
        .iter()
        .filter(|id| dt2.world().objects[id].guid == mortimer_guid)
        .count();
    assert_eq!(dt2_mortimers, 1, "no duplicate traveler on second visit");
    step_community(&mut dt2, 60, "downtown house 21 second visit");
    let ret2 = return_from_destination(
        &root,
        &store,
        &dep2.session,
        &mut dt2,
        &[3],
        Frames::default(),
    )
    .unwrap();
    let mut home_after_2 = ret2.home_lot;
    step_strict(&mut home_after_2, 30, "goth home post-second-downtown");
}

#[test]
fn goth_vacation_house40_whole_family_hotel_checkin_round_trip() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("vacation-40");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    let nh = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(
        classify_destination(40, &nh.zoning),
        Some(TravelDestination::VacationIsland)
    );
    let goth_members = nh.family_in(5).unwrap().1.members.clone();
    let mortimer_guid = goth_members[0];

    let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    step_strict(&mut goth, 5, "goth home pre-vacation");
    let funds_before = goth.world().budget.funds;

    let req = TravelRequest::vacation(
        40,
        mortimer_guid,
        VacationAccommodation::HotelCheckIn {
            grant_hotel_key: true,
        },
    );
    let dep = depart_to_destination(&root, &store, "UserData", &mut goth, &req).unwrap();
    let mut vac = dep.destination_lot;
    let session = dep.session;
    assert_eq!(session.destination, TravelDestination::VacationIsland);
    assert_eq!(session.fare_paid, VACATION_FARE);
    assert_eq!(session.travelers.len(), goth_members.len());
    assert_eq!(vac.world().budget.funds, funds_before - VACATION_FARE);
    assert_eq!(vac.world().globals[32], 0);

    // Every family member received a temporary Vacation hotel key on check-in.
    for t in &session.travelers {
        assert_eq!(
            neighbor_token_count(
                vac.world(),
                t.neighbor_id,
                HOTEL_KEY_TOKEN_KIND,
                HOTEL_KEY_TOKEN_GUID,
            ),
            1
        );
    }

    step_community(&mut vac, 220, "vacation house 40");

    let ret = return_from_destination(&root, &store, &session, &mut vac, &[3], Frames::default())
        .unwrap();
    let mut home_after = ret.home_lot;
    assert_eq!(ret.report.returning_travelers.len(), goth_members.len());

    // Temporary hotel keys were cleanly revoked upon Vacation checkout.
    for t in &session.travelers {
        assert_eq!(
            neighbor_token_count(
                home_after.world(),
                t.neighbor_id,
                HOTEL_KEY_TOKEN_KIND,
                HOTEL_KEY_TOKEN_GUID,
            ),
            0
        );
    }

    step_strict(&mut home_after, 50, "goth home post-vacation return");
}

#[test]
fn goth_studio_town_house81_round_trip() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("studio-81");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    let nh = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(
        classify_destination(81, &nh.zoning),
        Some(TravelDestination::StudioTown)
    );
    let mortimer_guid = nh.family_in(5).unwrap().1.members[0];

    let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    step_strict(&mut goth, 5, "goth home pre-studio");
    let funds_before = goth.world().budget.funds;

    let req = TravelRequest::studio_town(81, mortimer_guid);
    let dep = depart_to_destination(&root, &store, "UserData", &mut goth, &req).unwrap();
    let mut st = dep.destination_lot;
    let session = dep.session;
    assert_eq!(session.destination, TravelDestination::StudioTown);
    assert_eq!(session.fare_paid, STUDIO_TOWN_FARE);
    assert_eq!(st.world().budget.funds, funds_before - STUDIO_TOWN_FARE);
    assert_eq!(st.world().globals[32], 0);
    assert_eq!(st.world().globals[10], 81);

    step_community(&mut st, 220, "studio town house 81");

    let ret =
        return_from_destination(&root, &store, &session, &mut st, &[3], Frames::default()).unwrap();
    let mut home_after = ret.home_lot;
    step_strict(&mut home_after, 50, "goth home post-studio return");
}

#[test]
fn goth_magic_town_house90_and_house93_round_trips() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("magic-90-93");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    let nh = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(
        classify_destination(90, &nh.zoning),
        Some(TravelDestination::MagicTown)
    );
    assert_eq!(
        classify_destination(93, &nh.zoning),
        Some(TravelDestination::MagicTown)
    );
    let goth_members = nh.family_in(5).unwrap().1.members.clone();
    let mortimer_guid = goth_members[0];
    let cassandra_guid = goth_members[2];

    // Trip 1: Magic Town residential lot House 90 via free Magic Hole ($0), whole family.
    let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    step_strict(&mut goth, 5, "goth home pre-magic-90");
    let funds_before = goth.world().budget.funds;

    let req90 = TravelRequest::magic_town(
        90,
        TravelParty::WholeFamily {
            leader_guid: mortimer_guid,
        },
        TravelTransport::MagicHole,
    );
    let dep90 = depart_to_destination(&root, &store, "UserData", &mut goth, &req90).unwrap();
    let mut mt90 = dep90.destination_lot;
    let session90 = dep90.session;
    assert_eq!(session90.destination, TravelDestination::MagicTown);
    assert_eq!(session90.fare_paid, MAGIC_HOLE_FARE);
    assert_eq!(mt90.world().budget.funds, funds_before);
    // House 90 is zoned residential (1) in LotZoning.iff and ticks all 220 ticks in STRICT mode!
    step_strict(&mut mt90, 220, "magic town house 90");

    let ret90 = return_from_destination(
        &root,
        &store,
        &session90,
        &mut mt90,
        &[3],
        Frames::default(),
    )
    .unwrap();
    let mut goth_after_90 = ret90.home_lot;
    step_strict(&mut goth_after_90, 30, "goth home post-magic-90");

    // Trip 2: Magic Town community lot House 93 via Magic Blimp ($50), solo child Cassandra!
    let req93 = TravelRequest::magic_town(
        93,
        TravelParty::Single {
            sim_guid: cassandra_guid,
        },
        TravelTransport::MagicBlimp,
    );
    let dep93 =
        depart_to_destination(&root, &store, "UserData", &mut goth_after_90, &req93).unwrap();
    let mut mt93 = dep93.destination_lot;
    let session93 = dep93.session;
    assert_eq!(session93.fare_paid, MAGIC_BLIMP_FARE);
    assert_eq!(mt93.world().budget.funds, funds_before - MAGIC_BLIMP_FARE);
    step_community(&mut mt93, 220, "magic town house 93");

    let ret93 = return_from_destination(
        &root,
        &store,
        &session93,
        &mut mt93,
        &[3],
        Frames::default(),
    )
    .unwrap();
    let mut goth_after_93 = ret93.home_lot;
    step_strict(&mut goth_after_93, 30, "goth home post-magic-93");
}

#[test]
fn goth_old_town_community_house58_and_atomicity_guards() {
    let Some(root) = install() else {
        return;
    };
    let db = TempDb::new("oldtown-58");
    let store = NeighborhoodStore::open(&db.0).unwrap();
    store
        .import_directory("UserData", &root.join("UserData"), 0xff)
        .unwrap();

    let nh = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(classify_destination(7, &nh.zoning), None); // Newbie residential
    assert_eq!(classify_destination(50, &nh.zoning), None); // Old Town residential
    assert_eq!(
        classify_destination(58, &nh.zoning),
        Some(TravelDestination::OldTown)
    );
    let mortimer_guid = nh.family_in(5).unwrap().1.members[0];
    let rev_before = nh.revision;

    let mut goth = enter_lot(&root, &store, "UserData", 5, &[3], Frames::default()).unwrap();
    step_strict(&mut goth, 5, "goth home pre-oldtown");

    // Failed departures (residential lot, self-travel, unaffordable fare) leave REDB untouched.
    assert!(matches!(
        depart_to_destination(
            &root,
            &store,
            "UserData",
            &mut goth,
            &TravelRequest::old_town(50, mortimer_guid),
        ),
        Err(TravelError::NotTravelDestination { house: 50, .. })
    ));
    assert!(matches!(
        depart_to_destination(
            &root,
            &store,
            "UserData",
            &mut goth,
            &TravelRequest::downtown(5, mortimer_guid),
        ),
        Err(TravelError::SameHouse { house: 5 })
    ));
    let expensive = TravelRequest {
        fare_override: Some(999_999),
        ..TravelRequest::old_town(58, mortimer_guid)
    };
    assert!(matches!(
        depart_to_destination(&root, &store, "UserData", &mut goth, &expensive),
        Err(TravelError::InsufficientFunds { .. })
    ));
    let nh_still = store.load_neighborhood("UserData").unwrap().unwrap();
    assert_eq!(nh_still.revision, rev_before);
    assert_eq!(nh_still.families[&5].secondary_house(), 0);

    // Valid Old Town trip departs cleanly.
    let req = TravelRequest::old_town(58, mortimer_guid);
    let dep = depart_to_destination(&root, &store, "UserData", &mut goth, &req).unwrap();
    let mut ot = dep.destination_lot;
    let session = dep.session;
    assert_eq!(session.fare_paid, OLD_TOWN_FARE);

    // Frozen home handle cannot be saved or re-departed while away, and
    // opening the occupied destination for edit is also rejected.
    assert!(save_and_leave_lot(&root, &store, "UserData", &mut goth).is_err());
    assert!(matches!(
        depart_to_destination(
            &root,
            &store,
            "UserData",
            &mut goth,
            &TravelRequest::downtown(21, mortimer_guid),
        ),
        Err(TravelError::Lot(_))
    ));
    assert!(matches!(
        open_destination_for_edit(&root, &store, "UserData", 58, &[3], Frames::default()),
        Err(TravelError::DestinationOccupied {
            house: 58,
            family: 5
        })
    ));

    step_community(&mut ot, 220, "old town house 58");

    // Return with a mismatched session fails cleanly without clearing destination origin.
    let mut bad_session = session.clone();
    bad_session.destination_house = 21;
    assert!(matches!(
        return_from_destination(
            &root,
            &store,
            &bad_session,
            &mut ot,
            &[3],
            Frames::default()
        ),
        Err(TravelError::SessionMismatch(_))
    ));

    let ret =
        return_from_destination(&root, &store, &session, &mut ot, &[3], Frames::default()).unwrap();
    let mut home_after = ret.home_lot;

    // Double-return on the already-returned destination lot is rejected cleanly.
    assert!(matches!(
        return_from_destination(&root, &store, &session, &mut ot, &[3], Frames::default()),
        Err(TravelError::Lot(_))
    ));

    step_strict(&mut home_after, 30, "goth home post-oldtown");
}
