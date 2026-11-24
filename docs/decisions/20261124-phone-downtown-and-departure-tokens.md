# Phone Go Downtown, the trip routines and departure tokens

*Decided 2026-11-24.*

Calling a cab on the phone ("Call Cab… / Go Downtown - $50", PhoneGlobals
tree 8309, 0x2075) ran up to the Go Downtown plugin's Generic Sims Call 17
and stopped: "unsupported: Generic Sims Call routine 17" (GUID 0xA6F31853,
tree 0x1004, node 13). The API departure reached Downtown another way. It
overlaid each traveler's live state from a snapshot, and it made the date
companion itself, with mutual date links. The original does neither: the
trip routines carry the state in neighbor inventory tokens, and the arrival
marker's scripts read them back.

## The native flow (`Sims.exe` 1.91)

Generic Sims Call's table is at 0x45143a and its true branch at 0x44f7f0.

- **Routine 17** (0x450660): the running object must be a person (otherwise
  the cast at 0x635abe throws error 0x1c). If the interface window
  `[0x6f3b2c]` is alive, the routine sends it {3, 0x10, 0, 0}, which is
  presentation. It then calls 0x45a22d(temp 0 house, temp 1 hour, temp 2
  minute, temp 3 mode) and sets the game's byte `[[0x6e48bc]+0x1c]+0xe1`.
  The main loop reads that byte to leave the house between ticks.
- **0x45a22d**, the trip request:
  - The leader's neighbor record is found by the GUID of its type
    (0x42571f). Without one, nothing is written and no trip is asked for.
    The leader's inventory is found, or added empty (0x42a8a1).
  - **House -1 is the trip home.** The game's family (`game +0x10`)
    becomes family 0 (0x425d69(0)), and the house becomes the house
    (+0x110) of the leader's family (person word 61, +0x606).
  - **Mode 0** looks for a companion. On the way home it is the first
    kind-3 token; on the way out it is the first kind-1 token. The token's
    GUID must name an on-lot person (0x4e9753, 0x4e9608), whose neighbor
    inventory is then used (0x4572af, 0x42571f, 0x42a8a1).
  - **Mode 1** takes the leader's family (0x425d69(word 61)). A missing
    family is null, and 0x45a4fc then writes the leader alone. Other modes
    write the leader's tokens only and ask for no trip.
  - **Saved state.** Each inventory that gets tokens first loses every kind-2
    token (0x41f4e5(2)). It then gets kind-2 tokens through 0x41f38d, with
    counts that are `__ftol` (0x636128, a 64-bit `fistp` cut to its low
    word) of the motives, or the data words:

    | GUID | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 9 | 10 | 11 | 12 |
    | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
    | source | motive 9 | motive 6 | motive 5 | motive 15 | motive 7 | motive 8 | motive 14 | word 8 | word 80 | word 82 | word 81 |

    The motives are at +0x7b0, +0x7a4, +0x7a0, +0x7c8, +0x7a8, +0x7ac and
    +0x7c4. The words are at +0x59c, +0x62c, +0x630 and +0x62e.
  - **Who gets tokens.**
    - The companion, if any, gets its tokens first (0x45a3f7).
    - In mode 1, each family member on the lot is saved to its record
      (0x425a69) and gets tokens (0x45a5f9).
    - Otherwise the leader gets its own tokens (0x45a507).
    - The clock tokens then go to the leader's inventory, or to member 0's
      inventory when member 0 is on the lot.
  - **0x41f38d(kind, GUID, count).** A count of 0 adds nothing. An existing
    token of that kind and GUID gets the count added (0x41f3f6), capped at
    0x7fff above. Otherwise a new token is appended, unless the inventory
    already holds 0x7fff tokens.
  - **The arrival clock** goes into tokens GUID 7 (hour) and GUID 8 (minute)
    (0x45a785).
    - If the current lot (global 10) is zoned residential (0x40745f,
      0x42b9e3, which gives lot 0 zoning 1 and an unlisted lot 0) and the
      house is in 40..50, temps 1 and 2 are the absolute time.
    - Otherwise they are added to the clock (globals 0 and 5). The minutes
      carry into the hour at 60, and the hour wraps at 24.
  - The neighborhood is then saved (0x4252cc), and 0x4fc713 records the
    request:
    - mode 0 records (house, leader GUID, 0);
    - mode 1 records (house, 0, family);
    - a request while one is pending (byte +0x130) is ignored, so the
      first stands. The tokens are written either way.
- **Routine 20** (0x45087d) works like routine 19 but reads kind 3. Temp 0
  names a person (error 0x1c otherwise).
  - The first kind-3 token picks the person of its GUID. If there is none,
    one is made (0x4e9719).
  - Temp 0 becomes that person's object ID, or -1, or -1 when there is no
    token at all (0x450cde).
  - The token's count drops by one (0x41f56b(place, 1)).
- **Routine 22** shares routine 7's entry (0x45141f): native error 0x2a.
- **Routine 23** (0x450b2b) reads the clock back. The running person's
  kind-2 GUID 7 token goes to temp 0 and is erased (0x41f56b(place, -1)).
  Then GUID 8 goes to temp 1 and is erased. A missing token ends the
  routine at once on the true branch.
- **Routine 25** (0x450c02) casts the running object to a person and saves
  it to its record (0x425a69, `people::save_to_record`).
- **Routine 24** (0x44ff6e) is the outfit editor:
  - the editor struct is 0x408b3b, and it loads through 0x409315;
  - 18 category accessors (0x40d4ff..0x40e1f9);
  - the commit is 0x409492.

  `outfits.rs` has 0x409315 but not the editor or the commit.
- **The Go Downtown plugin** passes (house, 0, 30, mode 0): arrival is half
  an hour later. Downtown's middle ped marker (EP3 PedMarkers tree 0x100e)
  then:
  1. places the leader;
  2. brings the companion back with routine 19, from the kind-1 token, and
     places it;
  3. writes companion word 77 = leader, then leader word 77 = companion,
     and then leader word 77 *= -1;
  4. runs routine 21 on the leader and on the companion.

  The clock tokens 7 and 8 name nothing for routine 21, so they stay. The
  plugin's tail reads them on the home lot with routine 23 after the trip
  home.

## Decision

- **Routine 17 is ported** as `Machine::request_trip`
  (`travel/trip.rs`), a line-by-line port of 0x45a22d. It writes the
  tokens into the world's neighbor inventories and leaves the request in
  `World::trip_request`. That field is the game byte +0xe1 plus 0x4fc713's
  fields, and it is `#[serde(skip)]`: the original keeps it in the game
  object, not in the house, and it lives only until the shell leaves the
  lot. It is not in the snapshot or the hash, so the Goth hash is
  unchanged.
- **Routines 20, 22, 23 and 25 are ported** in `primitives/generic.rs`.
  Routines 19 and 20 share one token reader, parameterized by kind.
- **`travel::depart_from_request`** carries a routine-17 request out. It
  turns the request into a `TravelRequest` and departs:
  - mode 0 is `Single`, or `Date` when the leader holds a kind-1 token;
  - mode 1 is `WholeFamily`;
  - the fare is 0, because the plugin already paid it with Alter Budget;
  - the tokens are left as routine 17 wrote them.

  - the leader's kind-1 token is left as the date script wrote it.

  Wiring the request into the game loop is a Track 1 follow-up (it lives in
  `sims-game/src`).
- **The API Downtown departure writes the same tokens.**
  `depart_to_destination` runs the same port (mode 0, +0 h 30 min, as the
  Go Downtown plugin does) on the frozen home lot. So Downtown motives
  arrive as the marker's routine 21 leaves them: whole numbers. A traveler
  holding the tokens keeps the motives it is made with, not the live ones,
  because a motive cut to 0 has no token and its live fraction would
  outlive the restore.
  - Other destinations keep the snapshot overlay for now.
  - Their plugins pass: Vacation (14, *, mode from attribute 8), absolute
    on 40..49; Old Town (0, 15, *); Studio Town (0, 30, 0); Magic Town phone
    (14, *, 1); Clown Portal and Beanstalk (14, *, 0). Here * is temp 2,
    which the tree leaves as the lot picker set it. These are follow-ups.
- **The date companion comes from the token.** The departure no longer
  makes the companion or writes date links. The marker's routine 19 makes
  the companion from the kind-1 token, and the marker writes word 77 (so
  the leader's link is negative, as natively). The companion's traveler
  record keeps destination object ID 0, and the trip home finds it by
  GUID.
  - If the trip goes home before the marker makes the companion, the
    companion comes home as it left (its departure snapshot), and the
    leader's kind-1 token goes back with the trip, as an abort's does.
  - A version 1 trip's departure wrote no kind-1 token. When the leader
    holds none for the companion, the companion is made on arrival with
    the mutual date links, as that build did.
- **The envelope moves to version 5** (`TRIP_TOKEN_ENVELOPE_VERSION`).
  After the arrival family it records whether the departure left kind-2
  tokens: the API Downtown departure and `depart_from_request` do, other
  API departures do not. Only such a trip's return or abort takes kind-2
  tokens out (the stand-in for the plugin tail, below), so a trip that
  wrote none leaves the ones the travelers held. Versions 1 to 4 still
  decode, and their departures wrote none.
- **Routine 24 is a follow-up.** It needs the outfit editor's accessors
  and commit.

## Not modeled

- **The trip home.** Routine 17 with house -1 sets the game's family to
  family 0. Here the destination lot is discarded when it is left, so
  nothing reads that.
- **The neighborhood saves** (0x4252cc) are not modeled: the inventories
  live in the world and are written when the lot is saved.
- **The frozen home stack.** The trip home re-places the travelers at the
  curb (`reconcile_frozen_home_lot`), which drops the frozen stack. So the
  Go Downtown tail (routine 23 clock catch-up, routine 21 and the routine-20
  companion) does not run there; the API return overlay stands in for it.
