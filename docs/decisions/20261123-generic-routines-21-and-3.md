# Generic Sims Call routines 21 and 3: restore saved state, take down the travel picture

*Decided 2026-11-23.*

A player sent a Sim through a Magic Hole to Magic Town lot 93. The Sim kept
jumping across the lot and back. A headless run showed the cause.

- Magic Town's middle ped marker (GUID 0x12e84823, tree 0x100e) has
  attribute 3 set by its init tree.
- With it set, its main loop puts each member of the arriving family beside
  marker 0x13076784 (Find Location For, node 32). It then runs Generic Sims
  Call 21 for that person (node 35).
- When the family is done, it clears attribute 3 and runs routine 3
  (node 36).
- Routine 21 was not ported. Its fault was caught and the marker's tree was
  reset with attribute 3 still set. So every 21 ticks the marker put the
  first traveler beside it again, and that Sim's walk put it back the next
  tick.

What was decided:

- **Routine 21 is ported from 0x45098a.** It restores a person's saved
  state from the person's neighbor inventory:
  - The running object's temp 0 names the person (native error 0x1c
    otherwise).
  - The inventory is the person's neighbor ID's, and one is added, empty,
    when there is none (0x42a8a1).
  - For each token of kind 2, counted first (0x41f7a7), the first kind-2
    token is found (0x41f69c). Its GUID picks where its count goes:
    - GUIDs 0 to 6 name motives 9, 6, 5, 15, 7, 8 and 14 (0x45d133);
    - GUIDs 9 to 12 name person words 8, 80, 82 and 81.
  - The token is then erased (0x41f56b). A token whose GUID names nothing
    stays, and blocks the ones after it.
- **Routine 3 only takes the true branch.**
  - In the original it takes down the full-screen travel picture that
    0x4fc416 opens on the interface (`[0x6f2a58]` +0x78 and +0x7c). That
    is presentation, and the simulation keeps no such window.
  - What else reads that window (the pause's refusal, Dialog's modal
    check) is not modeled either. If it ever is, routine 3 has to close
    it.
- **A test lists the routines still missing.**
  - `crates/sims-vm/tests/generic_routines.rs` reads every tree in the
    install. It runs each Generic Sims Call routine number they use on the
    synthetic lot, and checks that the refused ones are exactly its
    `RESIDUALS` list.
  - There are 24 today: 0, 1, 4, 5, 13, 17, 19, 20, 22, 23, 24, 25, 27, 29,
    30, 32, 33, 34, 36, 38, 39, 40, 41 and 42.

How it was checked:

- `crates/sims-game/tests/magic_town_arrival.rs` takes the Goth house
  through a Magic Hole to lot 93. It checks three things: the marker's
  arrival pass finishes (attribute 3 back to 0), no Generic Sims Call error
  is caught, and no traveler moves more than a tile and a half in a tick
  over the 600 ticks after arrival. Without the port, attribute 3 stays 1
  and it fails.
- The Goth house's 43,200-tick hash is unchanged; its trees reach neither
  routine.

**Unverified:**

- Whether the original's departure writes the kind-2 tokens routine 21
  restores. Our travel carries live motives on the person, and on the trip
  tested the travelers' inventories hold none, so routine 21 changes
  nothing there.
- Whether the original keeps arriving Sims off the lot, or idle, until the
  marker places them. We place them by the arrival portal at load, so a
  Sim walking when the marker places it still shows in the new place for
  one tick.

Both are follow-ups.

What would reopen this: a run of the original where routine 21 or 3 changes
something this port does not, or a trip whose travelers carry kind-2 tokens
with GUIDs this port ignores.
