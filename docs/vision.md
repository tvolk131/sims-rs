# Vision and scope

This is the north star for sims-rs. Decisions here take precedence over older
milestone documents. Agreed November 13, 2026.

## The end result

A modern macOS game that:

1. **Loads and uses the original game files** (base game plus all seven
   expansions), which players supply themselves. Nothing from the original is
   shipped.
2. **Runs a simulation faithful to the original**: the same rules, interactions,
   motive effects, autonomy choices and outcomes. Exact equivalence (identical
   random draws, tick for tick) is a stretch goal measured against the
   original, not a design rule.
3. **Looks and behaves like the original UI**, built on modern foundations.
4. **Ships with co-op multiplayer over iroh** in v1.

## Non-goals

- Emulating Windows APIs, or running the original executable's code in the
  product. The original executable is only a test reference (see Verification).
- Matching the original's internal data structures, memory layout or call
  order, except where that's needed for an observable behavior.
- Writing the original save format.

## Simulation

- **Deterministic from day one.** The same starting state and commands give
  identical results on every machine (Intel and Apple Silicon):
  - all randomness comes from seeded generators owned by the simulation;
  - no iteration order that depends on hashing;
  - no wall-clock reads;
  - no platform math libraries (`sin`, `cos`, etc.) in game state;
  - any floating-point use is audited.
- **Everything external enters as a command**, stamped with a tick: player
  actions, speed changes, pause, Buy/Build edits and dialog answers. The UI
  never changes simulation state directly. The same mechanism serves
  multiplayer, replays and tests.
- **State is cheap to snapshot and restore.** One mechanism serves saves,
  mid-game joins, recovery when copies drift apart, and later rollback.
- **Floating-point math policy:**
  - Plain IEEE operations (`+ - * /`, `sqrt`, `mul_add`) give identical
    results on x86_64, aarch64 and wasm in Rust. Rust never fuses or reorders
    them on its own.
  - Platform math functions (`sin`, `cos`, `exp`, `pow`, `atan2`, ...) do not.
    Simulation crates use the pure-Rust `libm` crate instead, and a clippy
    `disallowed_methods` lint bans the `std` versions there.
  - Simulation floats go through one wrapper type that defines their
    semantics. The original stores motives as 32-bit floats but computed them
    with x87 extended precision. The wrapper is therefore the single place to
    match the original once the reference oracle shows how.
  - NaN bit patterns differ between architectures, so NaNs are normalized
    before hashing or saving.
  - Prefer integers or fixed point (the `fixed` crate) for any new quantity
    the original didn't define.
  - CI replays the same scenario on two architectures (e.g. aarch64 macOS and
    x86_64 Linux) and compares state fingerprints every N ticks.
- **Reproduce the original's bugs by default**; individual fixes are opt-in
  settings.
- Game scripts are executed, not re-written: the SimAntics VM and its
  primitives are the engine, and the game's own scripts supply the behavior.

## Content

- Supported installs: the Legacy Collection (primary) and the original
  CD/Complete Collection.
- All expansion content is always loaded. With expansions installed, the
  game's shared core scripts are expansion versions, so there's no separate
  "base game only" mode.
- Custom content and hacked objects are supported. A faithful VM covers most of
  it, so treat failures as engine bugs.
- Every language's text is supported, with English the tested default.
- The shipped starting neighborhoods load read-only. Importing a player's own
  original neighborhoods is low priority.

## UI and presentation

- **Modern window behavior:** widescreen, freely resizable, smooth zoom,
  high-DPI.
- **The HUD, dialogs, panels and catalogs use the original artwork, in roughly
  the original layout**, adapted to any resolution rather than pixel-locked to
  800×600.
- Rendering: objects aim for near-exact visual parity (they're pre-rendered
  sprites with depth). The 3D Sims aim for close approximate parity.
- Enhancements beyond the original are allowed, but default to faithful
  behavior.
- Audio (music, sound effects, Simlish voices) is in scope.

## Saves

- Our own format, stored in REDB:
  - versioned schema with explicit migrations;
  - multiple neighborhoods;
  - autosave;
  - exact continuation when restoring in the middle of an action.
- A missing or corrupt save gives a clear error and never replaces or deletes
  the previous save.
- The original format is read only, to import the starting saves.

## Multiplayer (v1)

**Model: co-op on one lot.** The simulation is exactly the single-player
simulation; several players give it commands. Money and the clock are shared.

**Networking:**
- iroh transport.
- Host-sequenced lockstep: the host assigns each command a tick and broadcasts
  tick batches. Players run a short buffer behind the host. Pacing adapts to
  the slowest machine.
- Joining mid-game and recovering from drift both use a REDB snapshot from the
  host.
- Players compare game-state fingerprints regularly to detect drift.
- If the host leaves, another player can take over.
- A file manifest check ensures everyone has identical game files and custom
  content.

**Time control:**
- Anyone can change speed, pause or unpause, with an optional host lock.
- The UI shows who changed it.
- The game is paused while any pause reason holds: an explicit pause, any
  player in Buy or Build mode, or an unanswered blocking dialog.

**Per-player state:**
- Local to each player: camera position, viewed floor and zoom.
- The selected Sim is **simulation state, not local UI state**, because game
  scripts read it (see below). Selection changes are commands.
- **Camera rotation is simulation state too.** Game scripts read it (Look
  Towards, the tile-depth primitive, Generic Sims Call 12, Find Location).
  Each player's rotation is kept per player in the simulation and changed by
  commands. A script asking on a Sim's behalf uses the rotation of the player
  who has that Sim selected, falling back to the host's.

**Other hidden inputs, fixed at the source:**
- Text measurement that scripts depend on uses the game's own bitmap fonts or
  a font bundled with the game, never system fonts.
- The host timer readings the original uses to seed some lots at load time
  become seeded values recorded in the save.

**Game globals that assume a single player.** A scan of all 28,778 scripts
across the base game and expansions found these:

| Global | Script readers | Multiplayer policy |
| --- | --- | --- |
| 3 Selected Person ID | ~400 (autonomy, social eligibility, menu checks, Downtown and Vacation party/follow/charge, Superstar sets) | Answer depends on who's asking (below) |
| 19 Mode (Live/Buy/Build) | 45 (help system, Build-mode helpers) | Build if any player is in Build, else Buy if any is in Buy, else Live |
| 32 Simless Build Mode | 40 (community-lot controllers) | Shared lot state, unchanged |
| 30 Free Will | 27 (autonomy) | Shared session setting |
| 15 Current Language | 22 (newspaper, some paintings) | The host's language for the simulation; text shown in each player's language |
| 33 Machine Level | 11 (visitor/pedestrian counts on community lots) | **Fixed for the session.** It reflects hardware, so differing values would make copies drift apart |
| 2 Zoom, 12 Last button, 16 Speed, 25 Debug flags | 2 each (tutorial and help system) | Tutorial/help is single-player only. Speed is shared |

**How "Selected Person ID" is used.** Scripts use it in two ways:
- *"Am I the selected Sim?"*, for example `MyData[11] == Global[3]` in
  `PersonGlobals` *try autonomy*, *person main loop*, *get social
  eligibility*, *Allow Greet*, and `Global` *Leave*.
  - **Policy:** when a Sim's own script asks, answer "yes" if any player has
    that Sim selected. Every player's Sim then behaves exactly as a selected
    Sim does in single player.
- *"Who is the player's Sim?"*, for example `StackObjID := Global[3]` in
  *Downtown Auto Follow*, *Am I in Selected Sim's Party*, and *Charge for Away
  from Home*.
  - **Policy:** answer with the most relevant player's selected Sim: the one
    whose party or command started this activity, falling back to the host's.
  - These cases cluster in travel destinations (build-order step 3), so the
    policy can be refined there.
- One script writes the selection (*Magic – Kid To Adult Backfire*). A
  script-driven selection change applies to the player who had the affected
  Sim selected.

**Other rules:**
- Pie menu availability checks run game scripts, which may consume random
  numbers. They run per player, with that player's selected Sim, against a
  throwaway copy of the state, and never against the shared simulation.
- Dialogs that block the game (the game's own "engage and block Sim" types
  pause all Sims) are shown to everyone. The first answer wins, in the order
  the host receives them.
- Only the host can leave the lot (neighborhood view, Create-a-Family).
- Rollback prediction isn't in v1, but the state is designed so it can be
  added without re-architecture.
- **Two kinds of problem, kept separate:**
  - *Sync risks* make copies drift apart: hidden inputs such as Machine
    Level, language, menu-check scripts, hash order or floats. Each one is
    fixed at its source. Regular fingerprint checks plus snapshot resync turn
    any missed case into a logged, recoverable event instead of a broken
    session.
  - *Gameplay-semantics questions* only decide what happens, and can't cause
    drift as long as the policy is deterministic: which player's Sim a date
    follows, pause disputes, simultaneous Build edits, single-player flows
    (phone calls, move-in, deaths, career popups). Each gets a decision as
    it's built.
- Multiplayer is built during step 1, not bolted on later, so single-player
  assumptions surface early.

## Platform

macOS first. Web may come later; avoid choices that rule it out without good
reason.

## Build order and v1

All expansion content is loaded from the start, and the data model covers all
expansions (multiple towns, community lots, travel, families away from home,
pets, magic, fame). Features are built in this order:

1. **Home-lot life:** Live mode, needs, autonomy, social, careers, NPCs, and
   expansion objects and pets at home, plus co-op multiplayer on that lot.
2. **Buy and Build modes, the neighborhood view, Create-a-Family.**
3. **Travel destinations:** Downtown, Vacation Island, Studio Town, Magic Town.

v1 = all three. If an earlier release is wanted, the natural cut is after
step 2 ("the full home game with all expansion content"), with travel in v1.1.

## Verification

- The original game runs in a VM (QEMU) as a **reference oracle**. Simulation
  traces from the same starting state are compared against ours: motives,
  random draws, chosen actions, object state.
- Static reading of the executable (e.g. Ghidra) explains disagreements. It is
  never copied.
- Progress is measured in gameplay that works and matches the reference, not
  in counts of passing checks.

## Legal

Same stance as OpenRCT2: open source, players supply their own game files, no
original assets or code are distributed, and private evidence stays out of the
public repository (`scripts/check_repository.py`).
