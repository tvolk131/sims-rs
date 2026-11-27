# Arrivals set the clock from routine 17's tokens; a family arrival makes every member; a hop can leave travelers behind

*Decided 2026-11-26.*

- **Status:** Accepted
- **Scope:**
  - `crates/sims-vm/src/travel.rs`, `crates/sims-vm/src/travel/trip.rs`, `crates/sims-vm/src/lifecycle.rs`
  - `crates/sims-vm/tests/integration/phone_go_downtown.rs`, `crates/sims-game/tests/integration/downtown_arrival.rs`
- **Track:** Wave 1 (travel and destination completeness)

## Context

The roadmap's travel follow-ups listed five items:
- a hop that leaves travelers behind (#443);
- arrivals made from the tokens alone: routine 17's saved-state and clock tokens, the date's companion, and a family arrival making only the members who traveled (#414);
- `HouseholdGroup` and Magic Town 90–92 snap-backs (#414);
- the outfit-editor routines 29, 30 and 31 (#430);
- the lot shift (0x444991) and 0x467436 at go-live (#443).

Two of these were already closed by #464 (`20261125-group-arrival-trip-tokens-lot-shift.md`), though the roadmap's "Next" list still names them:
- the group snap-back, and Magic Town 90–92, which never snapped back;
- the lot shift, reached only from the `offset_world` developer cheat, which the port does not have; and 0x467436, which empties planner rectangles the port never keeps.

The date's companion is already made by the arrival marker's routine 19 from the leader's kind-1 token (#411, #443), not by the departure.

Rereading 0x4f1ae9 for this work turned up a gap the earlier ADRs missed: **the arrival reads the clock tokens itself.**

## Decision 1: a native arrival takes member 0's clock tokens and sets the clock

Every arrival path in 0x4f1ae9 does the same once member 0 of the held family is made:
- Downtown (houses 21–30, flag +0x140): 0x4f1d1b..0x4f1dad;
- Studio Town (81–89, +0x142): 0x4f1ea6..0x4f1f38;
- the solo path (+0x140): 0x4f210a..0x4f219c;
- the family path (+0x141): 0x4f22b5..0x4f2347, inside the member loop, for member 0 only (0x4f227c).

The steps:
1. Member 0's person is found (0x4e97dc(0) → 0x4e9608), and its inventory is found by its neighbor ID (+0x5ca) or added empty (0x42a8a1).
2. The first kind-2 token of GUID 7 (0x41f69c) gives the hour and is erased (0x41f56b with -1).
3. The first kind-2 token of GUID 8 gives the minute and is erased.
4. The clock is set (0x46d86f).

Either token missing (place -1) stops the read before the clock is set. When only the minute is missing, the hour token has already been erased.

The solo path then sets the clock to 12:00 on house 99 (0x4f21a1..0x4f21b2), whatever the tokens said.

0x46d86f writes global 0 (+0x10) and global 5 (+0x1a), and leaves the second alone. It signals 0xdb, then works the time of day out again from the hour (0x46d525). When that changed, it writes global 4, tells the lighting (0x4c4648) and signals 0xdd.

What the port does:
- **`trip::take_arrival_clock`** reads and erases the tokens with the existing `token_place` / `read_token` / `take_token`, so wrapped places behave as in routine 23.
- **`lifecycle::set_clock`** is the one port of 0x46d86f. The move-in reset (0x42848d) now uses it too. As before, a changed time of day marks every room to be counted again.
- **`boot_destination_lot` calls `arrive_at_clock`** right after making member 0:
  - the traveler, on a solo arrival;
  - FAMI member 0, on a family arrival.

  A trip with no native arrival (`ArrivalFamily::Group`) reads nothing.

The clock tokens therefore no longer stay through routine 21. The earlier tests that asserted they did pinned the port, not a capture:
- `phone_go_downtown`;
- sims-game `downtown_arrival`;
- `saved_motives_and_the_arrival_clock_go_there_and_back`.

They now check two things: the clock is the routine-17 arrival clock as the lot boots, and no saved-state token is left once the marker's routine 21 has run.

The stored neighborhood the departure commits still holds the tokens. A resume boots from it, so a resumed trip arrives at the same clock.

## Decision 2: a family arrival makes every member of the family

0x4f2253..0x4f235c loops over the FAMI's members (0x41024e, 0x410218) and makes each with the person factory (0x4e9753 → 0x4e9719), whether or not it traveled. The port made only the travelers.

`boot_destination_lot` now walks the family's members in order on a `Family` arrival:
- A traveler is made with the state it left with, as before.
- A member that did not travel is made from its record (`Arriving::Member`). It gets no live-state overlay, keeps the fame its record gives it, and joins the trip as a `TravelerRecord` with `home_object_id: None`.
  - Its `had_hotel_key_before_trip` is `true`: check-in gave it no key, so checkout must not take one it got later.
  - It is made with `try_create_person`, so one the installed packs cannot make (a pet without the pet pack) is left out rather than failing the trip.
- The members are the family as the load has it: deleted characters, and entries with no neighborhood record, are skipped. Member 0, whose inventory holds the clock, is the first of those.
- A traveler who is not a member (none today) still comes last.

A departure from home takes every member on the lot, so this changes nothing there. It matters after a hop has left members on another lot (Decision 3).

## Decision 3: a hop can leave travelers behind

Go Somewhere Else for one member of a whole family asks in mode 0 (routine 17 → 0x45a22d), which writes only the caller's saved state (and a kind-1 companion's). 0x4fc59c then saves the neighborhood (0x428843) and loads the next lot with 0x4f1ae9(house, -1). The members who stay simply go with the lot, which is not saved; only their records and inventories remain. The port refused such a hop.

`onward_request` now returns the party with the request, and accepts a party that is a subset of the travelers:
- **Mode 0:** the caller, plus a date's companion when that companion is a traveler.
- **Mode 1:** the members of the caller's family as the lot has it now (`trip.family`) that have a person on the lot, as routine 17 takes them (0x45a3ef). With no such family, the leader goes alone (0x45a4fc). A date's companion stays, and so do a member whose person is gone and one that left the family.

It still refuses:
- a party that takes someone who is not a traveler;
- a leader who is not a household member;
- a party member who is not on the lot.

In `return_home`'s `Leg::OnTo`:
- Every person on the lot is flushed into the neighborhood as before, the travelers who stay included. Their temporary hotel keys are revoked with everyone's.
- Their saved-state tokens are no longer cleared. Nothing touches their inventories as the next lot loads.
- They are dropped from the states the next lot is booted with, so they leave the session.
- Their neighbor IDs are dropped from the next envelope's `departure_tokens`. That leg's abort or token-less return then erases only the tokens of those who went, even when routine 17 also saved a stayer (a second request the shell never took).

Back home, the members who stayed are household members who did not travel. `reconcile_frozen_home_lot` already leaves those where the home lot froze them and attaches them from the latest records (0x4258a3), which now carry what they did on the lot they stayed on. That matches the original:
- the home lot loads from its own file;
- each person attaches its record.

A later family arrival (Decision 2) makes the members who stayed from their records, and they rejoin the trip.

## Tests

- `travel::trip::tests::take_arrival_clock_reads_and_erases_both_tokens_or_stops`: both tokens, the minute missing, the hour missing.
- `travel::tests::a_member_goes_on_alone_and_the_family_stays`: the Goth family on Magic Town lot 93; Mortimer asks in mode 0 for Downtown 21. It checks:
  - only he goes;
  - he arrives in family 0 at the routine-17 clock with his clock tokens taken;
  - a member who stayed keeps a skill gained on lot 93 in its record, and a token in its inventory;
  - his trip home brings only him;
  - the others are home where the lot froze them, with the skill attached from the record.
- `travel::tests::a_family_arrival_makes_the_members_left_on_another_lot`: after the same hop, Mortimer asks in mode 1 for lot 93. All three members are made in FAMI order and become travelers, and the trip home brings all three.

## Consequences

- Destination lots now start at the arrival time routine 17 wrote (0x45a785, `trip::arrival_clock`). That is the clock where the trip was asked plus the travel tree's hour and minute:
  - 0:30 for Downtown and Studio Town;
  - 0:15 for Old Town;
  - 13:00–14:59 for Vacation Island and Magic Town.

  Leaving a residential lot for Vacation Island (houses 40–49), the 13:00–14:59 is the time itself. A single traveler on Magic Town lot 99 starts at 12:00. Before this change, a destination lot started at its saved clock. The Goth `TICK_43200` pin is unchanged, since no home lot runs this code.
- No snapshot or envelope format changes (`TRAVEL_ENVELOPE_VERSION` stays 5).
- Still open:
  - the outfit-editor routines 29, 30 and 31, and persisting edited outfit rows to the neighborhood (#430), are the next PR;
  - the strand when a marker's Find Location places nobody (#464) stays a follow-up; the original most likely strands too;
  - a hop whose party takes a Sim who is not a traveler (a townie met on the lot) is still refused;
  - a member who leaves the family during a visit stays in the stored family's member list, so a later family arrival still makes them. Merging roster changes from a destination lot into the neighborhood is a follow-up.
