# Sweep burn-down: `Idle For Input` missing-tree cancel, Superstar star-marker pool, and `Animate Sim` global clip sources 1 & 2

*Decided 2026-11-22.*

## Context

The 9-lot Complete Collection interactive object sweep introduced in PR #373
(`crates/sims-vm/tests/interaction_sweep.rs`, 3,103 objects, 1,833 unique
interactions) surfaced 144 `Trap::Unsupported` hits that collapsed to exactly
three root causes:

1. **108 hits (Goth, Newbies, Charming, Old Town, Magic Town Comm):**
   `Idle For Input` trapped when a queued interaction's target object did not
   define the referenced action tree (`0x1000..=0x10e8`).
2. **30 hits (Studio Town `House81`):** `Find Location For` trapped in
   `placed_from_outside` (`0x447a82`) when a non-selected famous Sim
   (`person.data[81] >= 10`) arrived on the lot and requested a pooled Superstar
   star marker (`0xE933_7020`, `StarBob.iff`).
3. **6 hits (Downtown `House21`):** `Animate Sim` trapped on clip source `2`
   (`0x58944b`) during `DTS Bar System` / `Bartender Leonardo` -> `Order Drink`.

## What the original does (`Sims.exe` 1.91)

### 1. `Idle For Input` push-tree failure (`0x45dc90..0x45dd1f`, `0x45e59b`, `0x452b98`, `0x47484a`)

When a queued action passes its check tree and has a nonzero action tree ID,
`Idle For Input` calls the person's virtual function `0x34` (`0x45e59b`), which
calls `0x452b98`:

1. `0x452baa` wakes the person (`0x44574b`) **before** attempting to push the
   target's tree (`0x47484a`).
2. `0x47484a` looks up the BHAV (`0x401ffb`). When the tree is missing, it
   returns `0` (`0x4749a2`) without pushing a frame, and `0x45e59b` returns `0`
   without recording an object use (`+0xb00`) or incrementing the target's use
   count (`+0xc6`).
3. Back in `Idle For Input` (`0x45dcb2`), `al == 0` falls through to the cancel
   branch (`0x45dcb6`): runs the target's function `4` (action cancelled) with
   `me` as stack object (`0x422d46`), restores any saved temps (`0x4582dd`), and
   returns `2` (`Flow::Yield`) without modifying `active`, `primitive_state`, or
   the count parameter.

### 2. Superstar star-marker pool (`0x446356`, `0x447a2d..0x447d27`, `0x446bf0..0x446ee5`, `0x4467d7`, `0x4481eb`, `0x447d2b`, `0x445f84`)

`cObjectRegistry` keeps two vectors of pooled `StarBob.iff` objects (GUID
`0xE933_7020`): `pool_in_use` (`+0xd0`) and `pool_avail` (`+0xdc`).

- **Allocation (`0x446356`):** when `pool_avail` is empty, checks Superstar
  expansion bit `0x40` (`[0x6e48bc]+0x66`, `Program::expansions`) and looks up
  GUID `0xE933_7020` (`0x43ac10`). Creates **4** instances if both vectors are
  empty (first use on the lot) or **2** more if `pool_in_use` is non-empty,
  marking each hidden (`data[8] |= 0x4000`) and pushing them onto `pool_avail`.
- **Placement (`0x447a2d..0x447d27` on arrival, `0x446bf0..0x446ee5` on deselect):**
  requires Superstar bit `0x40`, `!selected`, and `person.data[81] >= 10`.
  Ensures an available star via `0x446356`, skips border tiles (`0x422632`),
  checks the person's head slot (slot `1`, record `2`), releases any object held
  there via `0x4481eb`, puts the star marker into slot `1` via `put` (`+0x58`,
  `notify = true`), and moves it from `pool_avail` to `pool_in_use`.
- **Release (`0x4481eb`, called from `0x447d2b` `left_lot`, `0x4476d2`
  `mark_selected`, `0x447c4a` `mark_star`, `0x4467d7` `move_indicators`):**
  when the head-slot object's GUID is `0xE933_7020`, returns it from
  `pool_in_use` to `pool_avail` and calls `take_off(..., false)` (off-lot,
  unannounced).
- **Lot clear (`0x445f84`, `0x44602d..0x446133`):** removes `indicators[0..2]`,
  then `pool_in_use`, then `pool_avail` before clearing remaining objects.

### 3. `Animate Sim` global clip sources 1 & 2 (`0x58944b`, `0x59deb4`, `0x59defc`, `0x59d8a0..0x59d970`)

`[0x6f7698]` loads global animation string tables from `Global.iff`:

- **Source 1 (`0x59defc`, `[0x6f7698] + 0x2f4`):** `Global.iff` `STR# 128`
  (`0x80`, `"Animations"`), independent of person age.
- **Source 2 (`0x59deb4`, `[0x6f7698] + 0x288`):** `Global.iff` `STR# 130`
  (`0x82`, `"PersonAnimations"`), independent of person age.
- **Source 3 (`0x59dd16`, `+0x360/+0x3cc`):** `Global.iff` `STR# 156`/`157`
  (`"MiscAdult"`/`"MiscChild"`), already ported.
- **Source 0 (`0x59dd6d`):** object's own `STR# 129..=132`, already ported.

## Decision

1. **`crates/sims-vm/src/primitives/idle.rs`, `interpreter.rs`:** Wake `me`,
   attempt `try_enter_on` (returning `Ok(false)` on the three non-throwing
   rejections of `0x47484a`: missing tree `0x4748b0`, empty BHAV `0x474921`,
   and negative kind-4 entry `0x474917`), and on `Ok(false)` cancel via
   `callback(target, 4, me)` and return `Flow::Yield` (with `put_back_temps`
   still running in `start_queued`). Other traps (e.g., `StackOverflow`,
   `Invalid`) still propagate.
2. **`crates/sims-vm/src/placement.rs`, `selection.rs`, `removal.rs`:** Derive
   the star-marker pool directly from `World::objects` + `World::order` (no
   `sims-sim` schema or `Saved` format change needed): an off-lot uncontained
   `0xE933_7020` object is available and a contained one is in-use. Because
   `create` prepends to `world.order`, scanning `world.order` front-to-back
   matches `Sims.exe`'s LIFO `pool_avail.back()` order and persists across
   save/restore automatically. Gate `PortraitPutPlan::FastPlaced` when a person
   would take a head marker so portrait placement always uses a real tile, and
   use `off_lot` (`0x422632`) in `move_indicators` (`0x446727`) and `mark_star`
   so border-tile Sims on smaller lots keep their head slot untouched.
3. **`crates/sims-vm/src/primitives/animate.rs`:** Add `GLOBAL_CLIPS = 128`
   (source `1`) and `PERSON_CLIPS = 130` (source `2`) in `Machine::clip`.
4. **Verification & regression floor (`crates/sims-vm/tests/interaction_sweep.rs`):**
   Re-ran the full 9-lot Complete Collection sweep (3,103 objects, 1,833 unique
   interactions): **1,829 clean (99.8%)**, **0 / 1,833 `Trap::Unsupported`**,
   **0 `pie_menu` unsupported**. Locked `tot_unsup == 0` in the sweep test and
   added fast live-install tests covering Studio Town `House81` celebrity star
   markers and Downtown `House21` bar `Order Drink` interactions.

## What would reopen this

- A live `Sims.exe` trace showing observable behavioral divergence from deriving
  LIFO star-marker reuse order from `world.order` (e.g. if an older star marker
  is released while a newer one is already idle and a script inspects its object
  ID).
- Supporting `person.data[77] > 0` (Hot Date date-indicator placement,
  `0x4477fb`) or `person.data[81]` live write-callback updates (`0x446ee9`) if
  future sweeps or gameplay tests hit them.
