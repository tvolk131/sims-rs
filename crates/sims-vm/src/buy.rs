//! Buy Mode simulation operations: mode switching (`0x4cea5d`, `0x4ce44f`),
//! creating a catalog object wrapped in a `"Tile Cursor"` (`0x00000437`,
//! `0x486f5a` → `0x421986` → `0x4215e6`), moving and rotating the held cursor
//! (`0x485a2e` → `0x421e72` → `0x422050`, `0x4226bc` → `0x421be4`), committing
//! placement and charging household funds (`0x486ada` → `0x421a0e`, `0x4929fb`
//! → `0x46d91c`), and cancelling an unplaced purchase (`0x487105`).

use crate::factory::word;
use crate::fault::{Fault, Trap};
use crate::frame::PanelMode;
use crate::interpreter::Interpreter;
use crate::placement::{Place, Refusal, on_lot, step};
use crate::primitives::budget::spend;
use crate::visits::function;
use sims_sim::World;

/// The `"Tile Cursor"` object GUID looked up by name (`0x43ab8f`) in
/// `0x486f5a` and `0x486960`.
pub const TILE_CURSOR_GUID: u32 = 0x0000_0437;

/// Global 19: the simulation's active UI mode (`0x4ce44f`).
const MODE_GLOBAL: usize = 19;

/// Global 26: tutorial flag (`simulator + 0x44`), checked at `0x4878f3`
/// before enabling repeat purchases.
const TUTORIAL_GLOBAL: usize = 26;

/// Global 32: budget mode (`1` means unlimited funds, `0x46d8c9`).
const BUDGET_MODE: usize = 32;

/// Funds reported when budget mode is 1 (`0x46d8c9`).
const UNLIMITED_FUNDS: i32 = 999_999;

/// OBJD word 61 (`+0x7a`): catalog master facing offset (`0x434f8d`),
/// subtracted from the drag quadrant direction in `0x485dad`.
const FACING_OFFSET: usize = 0x7a / 2;

/// OBJD word 69 (`+0x8a`): nonzero for Build-mode objects (budget category 7),
/// zero for Buy-mode objects (budget category 6, `0x4878f3`).
const BUILD_MODE_TYPE: usize = 69;

/// OBJD word 96 (`+0xc0`): `1` for objects that keep the Buy tool armed for
/// repeat purchases after placement (`0x4878f3`).
const KEEP_BUYING: usize = 0xc0 / 2;

/// Object data word 1: facing direction (0..7).
const DATA_FACING: usize = 1;
/// Object data word 4 (`+0x52`): allowed surface-height bitmask (`0x4220a5`).
const DATA_ALLOWED_HEIGHTS: usize = 4;
/// Object data word 8 (`+0x5a`): placement/intersection flags (`0x43244d`).
const DATA_PLACEMENT_FLAGS: usize = 8;
/// Object data word 13 (`+0x64`): wall-attachment rules (`0x43583e`).
const DATA_WALL_RULES: usize = 13;
/// Object data word 23: rotation step in eighths (`object + 0x78`).
const DATA_TURN_STEP: usize = 23;
/// Object data word 42 (`+0x9e`): placement flags (`0x800` = pool tile, `0x43583e`).
const DATA_PLACEMENT: usize = 42;

/// Active Buy Mode placement cursor wrapping a newly created catalog object
/// and its `"Tile Cursor"` (`0x00000437`) helper (`cTSObjectTool` +
/// `cTSTileCursor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyCursor {
    /// Lead object ID of the `"Tile Cursor"` (`0x00000437`).
    pub cursor: u16,
    /// Lead object ID of the catalog object being placed.
    pub held: u16,
    /// Catalog master GUID of the held object.
    pub guid: u32,
    /// Purchase price read from the catalog master's OBJD word 18 (`0x421e42`).
    pub price: i32,
    /// Budget category charged on commit (`6` Buy, `7` Build, `0x4878f3`).
    pub budget_category: u8,
    /// Catalog master facing offset read from OBJD word 61 (`+0x7a`, `0x434f8d`).
    pub facing_offset: i16,
    /// Whether committing this item automatically arms another copy when
    /// affordable (`0x4878f3`: OBJD word 96 == 1 and tutorial global 26 == 0).
    pub keep_buying: bool,
    /// Whether the held object currently sits at a valid, affordable spot.
    pub valid: bool,
    /// Last tile-snapped fine lot position `[x, y, level]`.
    pub position: [i32; 3],
    /// Surface container `(id, slot)` if the held object snapped onto a
    /// surface slot (`0x422050`).
    pub container: Option<(u16, i16)>,
}

/// One committed Buy Mode purchase recorded for Undo / Redo (`0x48ba19` /
/// `0x48beb5` / `0x493259`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyPurchase {
    /// Lead object ID of the placed catalog object.
    pub object: u16,
    /// Catalog master GUID of the purchased object.
    pub guid: u32,
    /// Committed fine lot coordinates `[x, y, level]`.
    pub position: [i32; 3],
    /// Committed facing direction (`data[1]`, `0..7`).
    pub facing: i16,
    /// Surface container `(id, slot)` if placed on a surface slot.
    pub container: Option<(u16, i16)>,
    /// Purchase price charged on commit (`0x421e42`).
    pub price: i32,
    /// Budget category charged/refunded (`6` Buy, `7` Build, `0x48bdbd`).
    pub budget_category: u8,
    /// Whether this record currently holds a paid, on-lot placement (`true`
    /// after commit or successful Redo; `false` after Undo or after a refused
    /// Redo transferred to Undo via `0x48be72` → `0x493353`).
    pub placed: bool,
}

/// Default facing direction for a newly purchased object at camera rotation
/// `rotation` (`0x422776`: `[2, 0, 6, 4][rotation & 3]`).
pub fn default_buy_direction(rotation: u8) -> i16 {
    const TABLE: [i16; 4] = [2, 0, 6, 4];
    TABLE[usize::from(rotation & 3)]
}

/// Snaps a fine lot coordinate pair to its tile centre (`0x485f6d`:
/// `(v & !7) | 8`, which maps every `0..16` fine offset within a tile to `8`).
pub fn snap_to_tile_centre([x, y]: [i32; 2]) -> [i32; 2] {
    [(x & !15) | 8, (y & !15) | 8]
}

/// The panel mode encoded in `world.globals[19]` (`0x4ce44f`).
pub fn panel_mode(world: &World) -> PanelMode {
    match world.globals.get(MODE_GLOBAL).copied().unwrap_or(0) {
        1 => PanelMode::Build,
        2 => PanelMode::Buy,
        3 => PanelMode::Options,
        4 => PanelMode::Camera,
        _ => PanelMode::Live,
    }
}

/// Available household funds as `0x46d8c9` reads them: `999_999` in budget
/// mode 1, otherwise `world.budget.funds`.
pub fn available_funds(world: &World) -> i32 {
    if world.globals.get(BUDGET_MODE).copied().unwrap_or(0) == 1 {
        UNLIMITED_FUNDS
    } else {
        world.budget.funds
    }
}

/// Maps `PanelMode` to the value `0x4ce44f` writes to global 19.
fn mode_global(mode: PanelMode) -> i16 {
    match mode {
        PanelMode::Live => 0,
        PanelMode::Build => 1,
        PanelMode::Buy => 2,
        PanelMode::Options => 3,
        PanelMode::Camera => 4,
    }
}

/// Checks `0x422632`: both tile coordinates strictly inside `1..size - 1`.
fn inside_placement_bounds(world: &World, [x, y]: [i32; 2]) -> bool {
    let limit = i32::from(world.size).saturating_sub(1);
    let tx = x >> 4;
    let ty = y >> 4;
    tx > 0 && tx < limit && ty > 0 && ty < limit
}

impl Interpreter<'_> {
    /// Switches the simulation's active UI mode (`0x4cea5d` + `0x4ce44f`).
    ///
    /// Transitioning from a non-Live mode back to [`PanelMode::Live`] runs
    /// [`Interpreter::go_live`] (`0x416534`: marks all rooms uncounted,
    /// updates lot statistics, and sets global 19 to 0). Entering Buy, Build,
    /// Options, or Camera writes global 19 directly (`0x4ce44f`).
    pub fn set_mode(&self, world: &mut World, mode: PanelMode) -> Result<(), Fault> {
        let current = panel_mode(world);
        if mode == PanelMode::Live && current != PanelMode::Live {
            return self.go_live(world);
        }
        let target = mode_global(mode);
        let len = world.globals.len();
        let slot = world.globals.get_mut(MODE_GLOBAL).ok_or_else(|| Fault {
            object: 0,
            at: None,
            trap: Trap::Invalid(format!("global {MODE_GLOBAL} out of range ({len} globals)")),
        })?;
        *slot = target;
        Ok(())
    }

    /// Starts placing a newly purchased catalog object of catalog master GUID
    /// `guid` (`0x486f5a` → `0x421986` → `0x4215e6` → `0x42231d` → `0x4226bc`).
    ///
    /// Returns `Ok(None)` without mutating `world` if the household cannot
    /// afford the item (`price > available_funds`). Otherwise creates the
    /// object (`0x440b2d`), wraps it in a `"Tile Cursor"` (`0x00000437`), and
    /// rotates the held object to `direction` (`0x422776`). Both start off
    /// the lot until [`Interpreter::buy_move`] places them under the pointer.
    pub fn buy_start(
        &self,
        world: &mut World,
        guid: u32,
        level: i32,
        direction: i16,
    ) -> Result<Option<BuyCursor>, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let words = self
            .machine(world, 0, 0, false)
            .definition_words(guid)
            .map_err(|trap| fail(0, trap))?;
        let price = i32::from(words[word::PRICE]);
        let budget_category = if words[BUILD_MODE_TYPE] != 0 { 7 } else { 6 };
        let facing_offset = words[FACING_OFFSET];
        let keep_buying = words[KEEP_BUYING] == 1
            && world.globals.get(TUTORIAL_GLOBAL).copied().unwrap_or(0) == 0;
        if price > 0 && price > available_funds(world) {
            return Ok(None);
        }
        let held = self.create(world, guid)?;
        let cursor = self.create_tile_cursor(world, held)?;
        let mut out = BuyCursor {
            cursor,
            held,
            guid,
            price,
            budget_category,
            facing_offset,
            keep_buying,
            valid: false,
            position: [-16, -16, level],
            container: None,
        };
        self.buy_rotate(world, &mut out, direction)?;
        Ok(Some(out))
    }

    /// Moves a Buy Mode cursor and its held object to the tile containing
    /// fine lot coordinates `fine` on `level` (`0x485a2e` → `0x421e72` →
    /// `0x422050` + `0x422c2f`).
    ///
    /// Returns whether the position is valid and affordable. When staying on
    /// the same snapped `[pos, level]`, retains any existing surface container
    /// (`0x421e72` / `0x422050`) so manual rotations on a surface are not
    /// overwritten. If snapped placement fails:
    /// 1. For wall- or pool-edge-attached objects (`0x43583e`), tries alternate
    ///    rotations in steps of `data[23]` at the snapped tile (`0x48736b`).
    /// 2. Otherwise runs the unsnapped fallback (`0x485f23` → `0x421e72`),
    ///    where `UserTestPlace` (`0x4355bc..0x4355cd`) rejects unsnapped
    ///    coordinates for the held object, leaving the held object off the lot
    ///    and placing the `"Tile Cursor"` alone at `fine` when it fits.
    pub fn buy_move(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        fine: [i32; 2],
        level: i32,
    ) -> Result<bool, Fault> {
        let pos = snap_to_tile_centre(fine);
        let retained = (cursor.position == [pos[0], pos[1], level])
            .then_some(cursor.container)
            .flatten();
        self.remove_cursor_and_held(world, cursor)?;
        cursor.position = [pos[0], pos[1], level];
        cursor.valid = false;
        cursor.container = None;
        if !inside_placement_bounds(world, pos) {
            return Ok(false);
        }
        // Native `0x485ced`: check whether `"Tile Cursor"` fits at snapped
        // `pos` BEFORE trying snapped placement or wall/pool auto-rotation
        // (`0x48736b`); if it does not fit, jump straight to unsnapped
        // fallback (`0x485f23`).
        if self.fits(world, cursor.cursor, pos, level)? {
            if self.try_place_buy_at(world, cursor, pos, level, retained)? {
                return Ok(cursor.valid);
            }
            if self.try_wall_auto_rotate(world, cursor, pos, level)? {
                return Ok(cursor.valid);
            }
        }
        // Snapped placement (and any wall/pool auto-rotation) failed: native
        // `0x485f2f` checks whether `"Tile Cursor"` fits at unsnapped `fine`
        // FIRST, and only if it fits enters `0x421e72` → `0x422050` →
        // `0x435522`, running the held object's base placement test (including
        // OBJf 5 overlap callbacks at `0x4355b5`) before rejecting unsnapped
        // coordinates at `0x4355bc..0x4355cd`, leaving the held object off-lot
        // and placing `"Tile Cursor"` alone at `fine`. Native
        // `0x421fcf..0x421fed` saves the cursor's actual coordinates so manual
        // rotation (`0x422705..0x422716`) rechecks at `fine` instead of
        // jumping to the tile centre.
        cursor.valid = false;
        cursor.container = None;
        if self.fits(world, cursor.cursor, fine, level)? {
            // Native `0x485f44` always calls `0x421e72` at `fine` (even when
            // `fine == pos`): `try_place_buy_at` runs `UserTestPlace`
            // (`0x435522` / `0x422de4`), which succeeds only if `fine` is
            // tile-centred and otherwise runs `test_tile` (`0x4355b5`) on the
            // first part before rejecting unsnapped coordinates (`0x4355bc`).
            if self.try_place_buy_at(world, cursor, fine, level, None)? {
                return Ok(cursor.valid);
            }
            let cursor_place = Place {
                on_lot: true,
                position: fine,
                container: None,
                slot: -1,
                level,
            };
            self.put(world, cursor.cursor, cursor_place, true)?;
            cursor.position = [fine[0], fine[1], level];
        } else {
            // Native `0x485f49`: when fallback fit fails, call `TakeOff(1)`
            // unconditionally and keep Tile Cursor physically off-lot
            // (`[-16, -16]`) so subsequent `0x485b62` rechecks do not put it
            // back on the rejected tile.
            self.take_off_cursor_and_held_unconditional(world, cursor)?;
            cursor.position = [-16, -16, level];
        }
        Ok(false)
    }

    /// Tries placing `cursor.held` (and `cursor.cursor`) at `[at, level]`
    /// (`0x421e72` → `0x422050`). Callers on the cursor-gated paths
    /// (`0x485ced`, `0x485f2f`, `0x422752`, `0x485e59`) check
    /// `self.fits(world, cursor.cursor, at, level)` first, while `buy_recheck`
    /// (`0x485bda`) calls `0x421e72` directly without a prior fit check.
    /// Returns `Ok(true)` if the held object fit and was placed (setting
    /// `cursor.valid` based on affordability).
    fn try_place_buy_at(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        at: [i32; 2],
        level: i32,
        retained: Option<(u16, i16)>,
    ) -> Result<bool, Fault> {
        let affordable = cursor.price <= 0 || cursor.price <= available_funds(world);
        let Some(container) =
            self.test_buy_placement(world, cursor.cursor, cursor.held, at, level, retained)?
        else {
            return Ok(false);
        };
        cursor.position = [at[0], at[1], level];
        cursor.container = container;
        if affordable {
            cursor.valid = true;
        }
        let held_place = Place {
            on_lot: true,
            position: at,
            container: container.map(|(id, _)| id),
            slot: container.map_or(-1, |(_, slot)| i32::from(slot)),
            level,
        };
        self.put(world, cursor.held, held_place, true)?;
        let cursor_place = Place {
            on_lot: true,
            position: at,
            container: None,
            slot: -1,
            level,
        };
        self.put(world, cursor.cursor, cursor_place, true)?;
        Ok(true)
    }

    /// Native `0x48736b` + `0x43583e`: for wall-attached objects (`data[13] &
    /// 0x0f != 0`) or mixed pool-edge multi-tile objects (some part has
    /// `data[42] & 0x800 != 0` and another has it clear), tries each remaining
    /// rotation step `k = 1..(8 / step)` at `[pos, level]` via paired
    /// `cTSTileCursor::Turn(+k)` / `Turn(-k)` calls (`0x4873bb` / `0x4873d7` →
    /// `0x421be4`), keeping the first valid facing and returning to the
    /// original facing after each failed candidate.
    fn try_wall_auto_rotate(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        pos: [i32; 2],
        level: i32,
    ) -> Result<bool, Fault> {
        if !self.needs_wall_auto_rotate(world, cursor.held)? {
            return Ok(false);
        }
        let step = world
            .objects
            .get(&cursor.held)
            .and_then(|o| o.data.get(DATA_TURN_STEP).copied())
            .unwrap_or(0);
        if step <= 0 {
            return Ok(false);
        }
        cursor.position = [pos[0], pos[1], level];
        let count = 8 / step;
        for k in 1..count {
            self.buy_turn_steps(world, cursor, i32::from(k), None, level)?;
            if cursor.valid {
                return Ok(true);
            }
            self.buy_turn_steps(world, cursor, -i32::from(k), None, level)?;
        }
        Ok(false)
    }

    /// Checks `0x43583e`: whether `held` (or any tile in its group) requires
    /// an adjacent wall (`data[13] & 0x0f != 0`) or spans both pool and
    /// non-pool tiles (`data[42] & 0x800`).
    fn needs_wall_auto_rotate(&self, world: &World, held: u16) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: held,
            at: None,
            trap,
        };
        let Some(obj) = world.objects.get(&held) else {
            return Ok(false);
        };
        if (obj.data.get(DATA_WALL_RULES).copied().unwrap_or(0) & 0x0f) != 0 {
            return Ok(true);
        }
        if obj.multipart.is_none() {
            return Ok(false);
        }
        let parts = self.parts(world, held).map_err(fail)?;
        let mut any_pool = false;
        let mut any_non_pool = false;
        for part in parts {
            let Some(tile) = world.objects.get(&part) else {
                continue;
            };
            if (tile.data.get(DATA_WALL_RULES).copied().unwrap_or(0) & 0x0f) != 0 {
                return Ok(true);
            }
            if (tile.data.get(DATA_PLACEMENT).copied().unwrap_or(0) & 0x0800) != 0 {
                any_pool = true;
            } else {
                any_non_pool = true;
            }
        }
        Ok(any_pool && any_non_pool)
    }

    /// Turns the held object by signed `steps` of `DATA_TURN_STEP` in one
    /// `cTSTileCursor::Turn(steps)` call (`0x486cb8` / `0x4873bb` →
    /// `0x421be4`): runs unconditional `TakeOff(1)` (`0x421bff`), rotates the
    /// held object (and syncs cursor offsets), and runs `put_if_cursor_fits`
    /// (`0x421db6`) at the saved coordinates.
    pub fn buy_turn(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        steps: i32,
    ) -> Result<bool, Fault> {
        let level = cursor.position[2];
        let retained = cursor.container;
        self.buy_turn_steps(world, cursor, steps, retained, level)
    }

    fn buy_turn_steps(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        steps: i32,
        retained: Option<(u16, i16)>,
        level: i32,
    ) -> Result<bool, Fault> {
        // Native `0x421bff`: unconditional `TakeOff(1)` on cursor and held.
        self.take_off_cursor_and_held_unconditional(world, cursor)?;
        // Native `0x421c0d`: turn held object by signed `steps` and sync cursor
        // multipart offsets (`0x42238d`).
        self.turn_held_by(world, cursor.cursor, cursor.held, steps)?;
        // Native `0x421db6`: test fit and place at saved coordinates, clearing
        // valid on fit refusal.
        self.put_if_cursor_fits(world, cursor, retained, level, true)
    }

    /// Rotates the held object to `direction` (`0x4226bc` → `0x421be4`),
    /// stepping one `DATA_TURN_STEP` at a time: takes the preview off-lot once
    /// at `0x42271d`, then for each step invokes `0x421be4(1)` (off-lot
    /// `TakeOff(1)` at `0x421bff`, one turn step, and in-place fit+placement
    /// check at `0x421db6`), and finishes with the in-place `0x422752`
    /// fit+placement check at the saved coordinates without running
    /// pointer-move wall auto-rotation (`0x48736b`). When already snapped to a
    /// surface container slot at that tile, retains that container without
    /// re-running automatic surface-facing alignment (`0x422050` line
    /// `0x422078`).
    pub fn buy_rotate(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        direction: i16,
    ) -> Result<bool, Fault> {
        let target = direction.rem_euclid(8);
        let (mut current, step) = world
            .objects
            .get(&cursor.held)
            .map(|o| {
                (
                    o.data.get(DATA_FACING).copied().unwrap_or(0).rem_euclid(8),
                    o.data.get(DATA_TURN_STEP).copied().unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        if current == target || step <= 0 || (target - current).rem_euclid(8) % step != 0 {
            return Ok(cursor.valid);
        }
        let [x, y, level] = cursor.position;
        let retained = cursor.container;
        // Native `0x42271d`: take the cursor and held object off-lot once
        // before stepping through `0x421be4(1)`.
        self.take_off_cursor_and_held_unconditional(world, cursor)?;
        cursor.valid = false;
        cursor.container = None;
        for _ in 0..8 {
            if current == target {
                break;
            }
            // Native `0x422737` → `0x421be4(1)`: per-step take-off (`0x421bff`),
            // one rotation step, and per-step fit+placement check (`0x421db6`).
            cursor.position = [x, y, level];
            self.buy_turn_steps(world, cursor, 1, retained, level)?;
            current = world
                .objects
                .get(&cursor.held)
                .and_then(|o| o.data.get(DATA_FACING).copied())
                .unwrap_or(0)
                .rem_euclid(8);
        }
        // Native `0x422752` → `0x42276d`: final in-place Tile Cursor fit check
        // and placement at saved coordinates (branching past placement and its
        // state writes if fit fails).
        cursor.position = [x, y, level];
        let final_retained = cursor.container.or(retained);
        self.put_if_cursor_fits(world, cursor, final_retained, level, false)
    }

    /// Native `0x485e59` → `0x485e72`: tests `Tile Cursor` fit in place at
    /// the saved pre-recheck `(x, y)` (`local_20` from `0x485b62`) on `level`
    /// (without taking the preview off-lot first) and, if it fits, calls
    /// `0x421e72`. Branches past placement without altering
    /// `cursor.valid`/`cursor.container` if fit fails (`0x485e5e`).
    pub fn buy_put_if_fits(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        level: i32,
    ) -> Result<bool, Fault> {
        let retained = cursor.container;
        cursor.position[2] = level;
        self.put_if_cursor_fits(world, cursor, retained, level, false)
    }

    fn put_if_cursor_fits(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        retained: Option<(u16, i16)>,
        level: i32,
        clear_on_refusal: bool,
    ) -> Result<bool, Fault> {
        let [x, y, _] = cursor.position;
        if !on_lot([x, y]) || !inside_placement_bounds(world, [x, y]) {
            if clear_on_refusal {
                cursor.valid = false;
                cursor.container = None;
            }
            return Ok(false);
        }
        if !self.fits(world, cursor.cursor, [x, y], level)? {
            if clear_on_refusal {
                cursor.valid = false;
                cursor.container = None;
            }
            return Ok(cursor.valid);
        }
        cursor.valid = false;
        cursor.container = None;
        if self.try_place_buy_at(world, cursor, [x, y], level, retained)? {
            return Ok(cursor.valid);
        }
        cursor.position = [x, y, level];
        cursor.valid = false;
        cursor.container = None;
        let cursor_place = Place {
            on_lot: true,
            position: [x, y],
            container: None,
            slot: -1,
            level,
        };
        self.put(world, cursor.cursor, cursor_place, true)?;
        Ok(false)
    }

    /// Re-evaluates Buy Mode placement directly at the `"Tile Cursor"`'s
    /// physical `(x, y)` on the active `level` without gating on `"Tile
    /// Cursor"` fitting first (`0x485b62..0x485bda` → `0x421e72`). When the
    /// `"Tile Cursor"` is on-lot (`0x485b74..0x485b84`), invokes `0x421963`
    /// (`TakeOff(1)` on both `cursor` and `held`, running `held`'s `LEAVING`
    /// even if `held` was off-lot) before re-evaluating placement; when
    /// off-lot (`0x485b7b`), skips the initial take-off and still runs the
    /// `0x485bda` → `0x421e72` → `0x422c2f` placement pass.
    pub fn buy_recheck(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        level: i32,
    ) -> Result<bool, Fault> {
        let Some([x, y]) = world
            .objects
            .get(&cursor.cursor)
            .map(|o| [o.position[0], o.position[1]])
        else {
            return Ok(false);
        };
        let cursor_on_lot = on_lot([x, y]);
        let retained = (cursor_on_lot && cursor.position == [x, y, level])
            .then_some(cursor.container)
            .flatten();
        // Native `0x485b74..0x485b84` → `0x421963`: when cursor is on-lot, take
        // off both cursor and held unconditionally (running `held`'s LEAVING
        // callback even if only the Tile Cursor was on-lot). When off-lot,
        // skip the initial take-off (`0x485b7b`) and still run `0x485bda`.
        if cursor_on_lot {
            self.take_off_cursor_and_held_unconditional(world, cursor)?;
        }
        cursor.valid = false;
        cursor.container = None;
        if self.try_place_buy_at(world, cursor, [x, y], level, retained)? {
            return Ok(cursor.valid);
        }
        cursor.valid = false;
        cursor.container = None;
        if cursor_on_lot && inside_placement_bounds(world, [x, y]) {
            cursor.position = [x, y, level];
            let cursor_place = Place {
                on_lot: true,
                position: [x, y],
                container: None,
                slot: -1,
                level,
            };
            self.put(world, cursor.cursor, cursor_place, true)?;
        } else {
            // Native `0x422c2f` lines `0x422cfc`..`0x422d37`: off-lot target
            // calls `TakeOff(1)` and OBJf 9 (`PLACED`) on the Tile Cursor
            // parts while keeping the saved cursor position off-lot.
            self.take_off(world, cursor.cursor, true)?;
            let fail = |trap| Fault {
                object: cursor.cursor,
                at: None,
                trap,
            };
            let cursor_parts = if world
                .objects
                .get(&cursor.cursor)
                .is_some_and(|o| o.multipart.is_some())
            {
                self.parts(world, cursor.cursor).map_err(fail)?
            } else {
                vec![cursor.cursor]
            };
            for part in cursor_parts {
                self.callback(world, part, function::PLACED, 0)?;
            }
            cursor.position = [-16, -16, level];
        }
        Ok(false)
    }

    /// Hides the cursor and held object off the lot while the pointer leaves
    /// the house view (`0x487d4e` → `0x487d60` → `0x421963`, calling
    /// `TakeOff(1)` unconditionally whenever a cursor exists).
    pub fn buy_hide(&self, world: &mut World, cursor: &mut BuyCursor) -> Result<(), Fault> {
        self.take_off_cursor_and_held_unconditional(world, cursor)?;
        cursor.position = [-16, -16, cursor.position[2]];
        cursor.valid = false;
        cursor.container = None;
        Ok(())
    }

    /// Commits a valid Buy Mode placement (`0x486ada` → `0x421a0e` →
    /// `0x43562f` / `0x422e99`, `0x4929fb` → `0x46d91c`): removes and deletes
    /// the `"Tile Cursor"`, places the held object permanently at its
    /// validated tile/container, runs OBJf function 11 (`USER_PLACEMENT`) on
    /// the object (and every group tile / held child), deducts its price
    /// from household funds, and returns the recorded [`BuyPurchase`] for
    /// Undo/Redo (`0x48ba19` / `0x48beb5`).
    pub fn buy_commit_record(
        &self,
        world: &mut World,
        cursor: &BuyCursor,
    ) -> Result<BuyPurchase, Fault> {
        let held = cursor.held;
        let fail = |trap| Fault {
            object: held,
            at: None,
            trap,
        };
        if !cursor.valid {
            return Err(fail(Trap::Invalid(
                "cannot commit Buy Mode placement at an invalid location".into(),
            )));
        }
        if cursor.price > 0 && cursor.price > available_funds(world) {
            return Err(fail(Trap::Invalid(
                "cannot commit Buy Mode placement: insufficient funds".into(),
            )));
        }
        self.remove_cursor_and_held(world, cursor)?;
        self.remove_now(world, cursor.cursor)?;
        let [x, y, level] = cursor.position;
        let place = Place {
            on_lot: true,
            position: [x, y],
            container: cursor.container.map(|(id, _)| id),
            slot: cursor.container.map_or(-1, |(_, slot)| i32::from(slot)),
            level,
        };
        self.put(world, cursor.held, place, true)?;
        self.run_user_placement(world, cursor.held)?;
        spend(world, cursor.budget_category, cursor.price).map_err(fail)?;
        let facing = world
            .objects
            .get(&cursor.held)
            .and_then(|o| o.data.get(DATA_FACING).copied())
            .unwrap_or(0)
            .rem_euclid(8);
        Ok(BuyPurchase {
            object: cursor.held,
            guid: cursor.guid,
            position: cursor.position,
            facing,
            container: cursor.container,
            price: cursor.price,
            budget_category: cursor.budget_category,
            placed: true,
        })
    }

    /// Commits a valid Buy Mode placement and returns the placed object's ID.
    pub fn buy_commit(&self, world: &mut World, cursor: &BuyCursor) -> Result<u16, Fault> {
        self.buy_commit_record(world, cursor).map(|p| p.object)
    }

    /// Undoes a committed Buy Mode purchase (`0x48bd21` → `0x48c0ea` →
    /// `0x435751` / `0x422ff5` + `0x48bd88` → `0x48bdbd`): whenever the
    /// purchase's object instance is still registered, runs OBJf function 12
    /// (`USER_PICKUP`) on the object (and every group tile / held child),
    /// takes it off the lot while keeping the instance registered for Redo,
    /// and refunds `purchase.price` to household funds and today's budget
    /// category (matching native `0x48bd21` even when a refused Redo entry was
    /// transferred back to Undo via `0x493353`). Returns `Ok(false)` only if
    /// the object instance is no longer registered.
    pub fn buy_undo_purchase(
        &self,
        world: &mut World,
        purchase: &mut BuyPurchase,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: purchase.object,
            at: None,
            trap,
        };
        if !world.objects.contains_key(&purchase.object) {
            return Ok(false);
        }
        self.run_user_pickup(world, purchase.object)?;
        self.take_off(world, purchase.object, true)?;
        spend(world, purchase.budget_category, -purchase.price).map_err(fail)?;
        purchase.placed = false;
        Ok(true)
    }

    /// Redoes a previously undone Buy Mode purchase (`0x48bdfe` → `0x48c053`
    /// → `0x48c07b..0x48c082` + `0x48bfab` + `0x48bdbd`): reuses the detached
    /// registered instance when still present (falling back to `create`
    /// `0x440b2d` only if absent), rotates it to `purchase.facing`, validates
    /// and places it at `purchase.position`/`purchase.container`, runs OBJf
    /// function 11 (`USER_PLACEMENT`, `*piVar2 + 0x64`), and re-deducts
    /// `purchase.price`. Updates `purchase.object` to the placed ID and
    /// returns `Ok(true)`, or returns `Ok(false)` if it can no longer be
    /// placed.
    pub fn buy_redo_purchase(
        &self,
        world: &mut World,
        purchase: &mut BuyPurchase,
    ) -> Result<bool, Fault> {
        let [x, y, level] = purchase.position;
        if !on_lot([x, y]) || !inside_placement_bounds(world, [x, y]) {
            return Ok(false);
        }
        let (id, created_fresh) = if let Some(existing) = world.objects.get(&purchase.object) {
            if on_lot([existing.position[0], existing.position[1]]) {
                return Ok(false);
            }
            (purchase.object, false)
        } else {
            (self.create(world, purchase.guid)?, true)
        };
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        // Native `0x48bfab` line `0x48bfcb`: invoke `UserPickup(0)` (`0x435751`
        // / `0x422ff5` -> OBJf 12 + take-off `0x435835` -> OBJf 10 `LEAVING`)
        // on the instance before rotating or testing placement.
        self.run_user_pickup(world, id)?;
        self.take_off(world, id, true)?;
        self.rotate_object_to(world, id, purchase.facing)?;
        // Native `UserTestPlace` (`0x435522` / `0x422de4`): requires
        // tile-centred coordinates (`(x & 0xf) == 8 && (y & 0xf) == 8`),
        // lot-interior tiles without ground flag `0x20`, and (for surface
        // placements) an existing on-lot container whose slot height is not
        // in `5..=7` (`0x43560c..0x435623`).
        let fits = match purchase.container {
            Some((cid, slot)) => {
                self.user_fits_in(world, id, cid, i32::from(slot), [x, y], level)?
            }
            None => self
                .user_test_place_ground(world, id, [x, y], level)?
                .is_ok(),
        };
        if !fits {
            if created_fresh {
                self.remove_now(world, id)?;
            } else {
                // Native `0x48c053` calls `0x48c0ea` when `0x48bfab` returns
                // `0`, invoking `USER_PICKUP` (OBJf 12) and `take_off` (OBJf 10
                // `LEAVING`) again before the refused record transfers to Undo.
                self.run_user_pickup(world, id)?;
                self.take_off(world, id, true)?;
            }
            return Ok(false);
        }
        let place = Place {
            on_lot: true,
            position: [x, y],
            container: purchase.container.map(|(cid, _)| cid),
            slot: purchase.container.map_or(-1, |(_, slot)| i32::from(slot)),
            level,
        };
        self.put(world, id, place, true)?;
        self.run_user_placement(world, id)?;
        spend(world, purchase.budget_category, purchase.price).map_err(fail)?;
        purchase.object = id;
        purchase.placed = true;
        Ok(true)
    }

    /// Deletes any detached (off-lot) instances referenced by discarded undo
    /// or redo history records (`0x48c20e` / `0x48c258`), leaving on-lot
    /// objects untouched.
    pub fn buy_purge_detached(
        &self,
        world: &mut World,
        purchases: &[BuyPurchase],
    ) -> Result<(), Fault> {
        for purchase in purchases {
            if world
                .objects
                .get(&purchase.object)
                .is_some_and(|o| !on_lot([o.position[0], o.position[1]]))
            {
                self.remove_now(world, purchase.object)?;
            }
        }
        Ok(())
    }

    /// Cancels an active Buy Mode purchase (`0x487105`): deletes both the
    /// `"Tile Cursor"` and the unplaced catalog object without charging funds.
    pub fn buy_cancel(&self, world: &mut World, cursor: &BuyCursor) -> Result<(), Fault> {
        self.remove_cursor_and_held(world, cursor)?;
        self.remove_now(world, cursor.cursor)?;
        self.remove_now(world, cursor.held)?;
        Ok(())
    }

    /// Creates one `"Tile Cursor"` (`0x00000437`) per tile of `held`
    /// (`0x4215e6` → `0x421afb` → `0x42231d`).
    fn create_tile_cursor(&self, world: &mut World, held: u16) -> Result<u16, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let is_group = world
            .objects
            .get(&held)
            .ok_or_else(|| fail(held, Trap::native(10, "missing held object")))?
            .multipart
            .is_some();
        let held_parts = if is_group {
            self.parts(world, held).map_err(|t| fail(held, t))?
        } else {
            vec![held]
        };
        let mut cursor_lead = None;
        let mut cursor_parts = Vec::with_capacity(held_parts.len());
        for &part_id in &held_parts {
            let id = (1..=i16::MAX as u16)
                .find(|id| !world.objects.contains_key(id))
                .ok_or_else(|| fail(0, Trap::Unsupported("no unused object ID".into())))?;
            let tile = if is_group { Some(cursor_lead) } else { None };
            let mut machine = self.machine(world, id, 0, false);
            let words = machine
                .construct_tile_cursor(TILE_CURSOR_GUID, tile)
                .map_err(|trap| fail(id, trap))?;
            // 0x4215e6 runs post-constructor (+0x3c) then reset (+0x40(0)) on
            // single-tile cursors, or +0x40(1) on multi-tile cursors, without
            // calling the factory's turn or finish (so registry_word stays
            // 0xffff).
            machine
                .reset(words)
                .and_then(|_| machine.install_main())
                .map_err(|trap| fail(id, trap))?;
            self.init(world, id)?;
            // 0x42231d copies rotation step (+0x78, word 23) and facing
            // (+0x4c, word 1) from the wrapped object, and 0x42238d copies
            // multipart offsets for multi-tile objects.
            let (turn_step, facing, multipart_info) = {
                let h = world
                    .objects
                    .get(&part_id)
                    .ok_or_else(|| fail(part_id, Trap::native(10, "missing held part")))?;
                (
                    h.data.get(DATA_TURN_STEP).copied().unwrap_or(0),
                    h.data.get(DATA_FACING).copied().unwrap_or(0),
                    h.multipart.as_ref().map(|m| (m.offset, m.group)),
                )
            };
            if let Some(c) = world.objects.get_mut(&id) {
                if let Some(slot) = c.data.get_mut(DATA_TURN_STEP) {
                    *slot = turn_step;
                }
                if let Some(slot) = c.data.get_mut(DATA_FACING) {
                    *slot = facing;
                }
                if let (Some(cm), Some((offset, group))) = (c.multipart.as_mut(), multipart_info) {
                    cm.offset = offset;
                    cm.group = group;
                }
            }
            if cursor_lead.is_none() {
                cursor_lead = Some(id);
            }
            cursor_parts.push(id);
        }
        cursor_lead.ok_or_else(|| fail(held, Trap::Invalid("no tile cursor created".into())))
    }

    /// Rotates `id` (or its whole multi-tile group) toward `target_dir` in
    /// steps of its `DATA_TURN_STEP`, updating multipart offsets when
    /// multi-tile.
    fn rotate_object_to(&self, world: &mut World, id: u16, target_dir: i16) -> Result<(), Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let target = target_dir.rem_euclid(8);
        let (current, step, is_group) = {
            let h = world
                .objects
                .get(&id)
                .ok_or_else(|| fail(id, Trap::native(10, "missing object")))?;
            (
                h.data.get(DATA_FACING).copied().unwrap_or(0).rem_euclid(8),
                h.data.get(DATA_TURN_STEP).copied().unwrap_or(0),
                h.multipart.is_some(),
            )
        };
        if current == target || step <= 0 {
            return Ok(());
        }
        let delta = (target - current).rem_euclid(8);
        if delta % step != 0 {
            return Ok(());
        }
        let steps = i32::from(delta / step);
        if is_group {
            // Native `0x421c0d` dispatches to `cTSMultiTileObject::Turn`
            // (`0x422937`), which runs `TakeOff(1)` (`0x422961`) on the group
            // before updating facings and offsets.
            self.turn_group(world, id, steps)?;
        } else {
            self.turn_one(world, id, steps)?;
        }
        Ok(())
    }

    /// Turns `held` by signed `steps` of its `DATA_TURN_STEP`, updating
    /// multipart offsets on both `held` and `cursor` when multi-tile while
    /// leaving `cursor`'s own `data[1]` unchanged (`0x421c0d` + `0x42238d`).
    fn turn_held_by(
        &self,
        world: &mut World,
        cursor: u16,
        held: u16,
        steps: i32,
    ) -> Result<(), Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let is_group = world
            .objects
            .get(&held)
            .ok_or_else(|| fail(held, Trap::native(10, "missing held object")))?
            .multipart
            .is_some();
        if is_group {
            // Native `0x421c0d` dispatches to `cTSMultiTileObject::Turn`
            // (`0x422937`), which runs `TakeOff(1)` (`0x422961`) on the group
            // before updating facings and offsets.
            self.turn_group(world, held, steps)?;
            let parts = self.parts(world, held).map_err(|t| fail(held, t))?;
            // Sync cursor parts' offsets from held parts (`0x42238d`).
            let cursor_parts = self.parts(world, cursor).map_err(|t| fail(cursor, t))?;
            for (&hp, &cp) in parts.iter().zip(cursor_parts.iter()) {
                let offset = world
                    .objects
                    .get(&hp)
                    .and_then(|o| o.multipart.as_ref())
                    .map(|m| m.offset);
                if let (Some(offset), Some(cm)) = (
                    offset,
                    world
                        .objects
                        .get_mut(&cp)
                        .and_then(|o| o.multipart.as_mut()),
                ) {
                    cm.offset = offset;
                }
            }
        } else {
            self.turn_one(world, held, steps)?;
        }
        Ok(())
    }

    /// Rotates `held` toward `target_dir` in steps of its `DATA_TURN_STEP`,
    /// updating multipart offsets on both `held` and `cursor` when multi-tile
    /// while leaving `cursor`'s own `data[1]` unchanged (matching `0x421be4`
    /// when `param_1[0x6b] != 0`).
    fn rotate_held_to(
        &self,
        world: &mut World,
        cursor: u16,
        held: u16,
        target_dir: i16,
    ) -> Result<(), Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let is_group = world
            .objects
            .get(&held)
            .ok_or_else(|| fail(held, Trap::native(10, "missing held object")))?
            .multipart
            .is_some();
        self.rotate_object_to(world, held, target_dir)?;
        if is_group {
            let parts = self.parts(world, held).map_err(|t| fail(held, t))?;
            // Sync cursor parts' offsets from held parts (`0x42238d`).
            let cursor_parts = self.parts(world, cursor).map_err(|t| fail(cursor, t))?;
            for (&hp, &cp) in parts.iter().zip(cursor_parts.iter()) {
                let offset = world
                    .objects
                    .get(&hp)
                    .and_then(|o| o.multipart.as_ref())
                    .map(|m| m.offset);
                if let (Some(offset), Some(cm)) = (
                    offset,
                    world
                        .objects
                        .get_mut(&cp)
                        .and_then(|o| o.multipart.as_mut()),
                ) {
                    cm.offset = offset;
                }
            }
        }
        Ok(())
    }

    /// Invokes `TakeOff(1)` (`0x421963` → `0x433148`, running OBJf 10
    /// `LEAVING`) unconditionally on `cursor` and `held`, even while off-lot
    /// (`0x42271d`, `0x421bff`).
    fn take_off_cursor_and_held_unconditional(
        &self,
        world: &mut World,
        cursor: &BuyCursor,
    ) -> Result<(), Fault> {
        if world.objects.contains_key(&cursor.cursor) {
            self.take_off(world, cursor.cursor, true)?;
        }
        if world.objects.contains_key(&cursor.held) {
            self.take_off(world, cursor.held, true)?;
        }
        Ok(())
    }

    /// Removes `cursor` and `held` from the lot (`0x485c37` / `0x487d4e` →
    /// `0x421963`): whenever `Tile Cursor` (or `held`) is on-lot, calls
    /// `TakeOff(1)` on both `cursor` and `held` unconditionally.
    fn remove_cursor_and_held(&self, world: &mut World, cursor: &BuyCursor) -> Result<(), Fault> {
        let cursor_on_lot = world
            .objects
            .get(&cursor.cursor)
            .is_some_and(|o| on_lot([o.position[0], o.position[1]]));
        let held_on_lot = world.objects.get(&cursor.held).is_some_and(|o| {
            on_lot([o.position[0], o.position[1]]) || o.data.get(2).copied().unwrap_or(0) != 0
        });
        if cursor_on_lot || held_on_lot {
            self.take_off_cursor_and_held_unconditional(world, cursor)?;
        }
        Ok(())
    }

    /// Runs OBJf callback `callback` (`USER_PLACEMENT` = 11 or `USER_PICKUP` =
    /// 12) on `held` (or every part of a multi-tile group) and on every
    /// non-empty object in each part's surface slots (`0x43562f` / `0x422e99`,
    /// `0x435751` / `0x422ff5`).
    fn run_user_callback(
        &self,
        world: &mut World,
        held: u16,
        callback: usize,
    ) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: held,
            at: None,
            trap,
        };
        let is_group = world
            .objects
            .get(&held)
            .is_some_and(|o| o.multipart.is_some());
        let parts = if is_group {
            self.parts(world, held).map_err(fail)?
        } else {
            vec![held]
        };
        for part in parts {
            self.callback(world, part, callback, 0)?;
            let slot_count = world
                .objects
                .get(&part)
                .map_or(0, |o| o.slots.len().saturating_sub(1));
            for slot in 0..slot_count {
                let child = world
                    .objects
                    .get(&part)
                    .and_then(|o| o.slots.get(slot + 1))
                    .map_or(0, |s| s.object);
                if let Ok(child) = u16::try_from(child)
                    && child != 0
                {
                    self.callback(world, child, callback, 0)?;
                }
            }
        }
        Ok(())
    }

    fn run_user_placement(&self, world: &mut World, held: u16) -> Result<(), Fault> {
        self.run_user_callback(world, held, function::USER_PLACEMENT)
    }

    fn run_user_pickup(&self, world: &mut World, held: u16) -> Result<(), Fault> {
        self.run_user_callback(world, held, function::USER_PICKUP)
    }

    /// Native `UserTestPlace` (`0x435522`, multi-tile `0x422de4`) for ground
    /// placements: checks each part in native order, stopping immediately at
    /// the first refusal (`0x422e82`):
    /// 1. Lot-interior bounds `1..size - 1` and level-1 tile flag `0x20` clear
    ///    (array `8`, `0x43553e..0x435597`).
    /// 2. Base single-tile placement test (`self.test_tile`, `0x4355b5`).
    /// 3. Tile-centred coordinates `(x & 0xf) == 8 && (y & 0xf) == 8`
    ///    (`0x4355bc..0x4355cd`).
    fn user_test_place_ground(
        &self,
        world: &mut World,
        id: u16,
        pos: [i32; 2],
        level: i32,
    ) -> Result<Result<(), Refusal>, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let test_one = |world: &mut World,
                        part: u16,
                        at: [i32; 2],
                        part_level: i32|
         -> Result<Result<(), Refusal>, Fault> {
            if !inside_placement_bounds(world, at) {
                return Ok(Err(Refusal::OffLot));
            }
            let a = i8::try_from(at[0] >> 4).unwrap_or(-1);
            let b = i8::try_from(at[1] >> 4).unwrap_or(-1);
            let flags = crate::placement::tile_flags(world, a, b, 1).map_err(fail)?;
            if (flags & 0x20) != 0 {
                return Ok(Err(Refusal::OffLot));
            }
            if let Err(refusal) = self.test_tile(world, part, at, part_level, None)? {
                return Ok(Err(refusal));
            }
            if (at[0] & 0x0f) != 8 || (at[1] & 0x0f) != 8 {
                return Ok(Err(Refusal::Other));
            }
            Ok(Ok(()))
        };
        let Some(selected) = world
            .objects
            .get(&id)
            .and_then(|o| o.multipart.as_ref())
            .map(|m| m.offset)
        else {
            return test_one(world, id, pos, level);
        };
        let origin = [
            pos[0].wrapping_sub(selected[0].wrapping_mul(16)),
            pos[1].wrapping_sub(selected[1].wrapping_mul(16)),
        ];
        let origin_level = level.wrapping_sub(selected[2]);
        for part in self.parts(world, id).map_err(fail)? {
            let offset = world
                .objects
                .get(&part)
                .and_then(|o| o.multipart.as_ref())
                .map(|m| m.offset)
                .ok_or_else(|| fail(Trap::native(10, "missing multipart part")))?;
            let at = [
                origin[0].wrapping_add(offset[0].wrapping_mul(16)),
                origin[1].wrapping_add(offset[1].wrapping_mul(16)),
            ];
            let part_level = origin_level.wrapping_add(offset[2]);
            if let Err(refusal) = test_one(world, part, at, part_level)? {
                return Ok(Err(refusal));
            }
        }
        Ok(Ok(()))
    }

    /// Native `UserTestPlace` (`0x435522`) for container placements: requires
    /// an existing on-lot container, lot-interior bounds and ground flag
    /// `0x20` clear, `fits_in` (`0x4355b5`), tile-centred coordinates
    /// (`0x4355bc..0x4355cd`), and containment slot height not in `5..=7`
    /// (`0x43560c..0x435623`) when `move_objects` is off.
    fn user_fits_in(
        &self,
        world: &mut World,
        id: u16,
        container: u16,
        slot: i32,
        pos: [i32; 2],
        level: i32,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        if !world
            .objects
            .get(&container)
            .is_some_and(|c| on_lot([c.position[0], c.position[1]]))
        {
            return Ok(false);
        }
        if !inside_placement_bounds(world, pos) {
            return Ok(false);
        }
        let a = i8::try_from(pos[0] >> 4).unwrap_or(-1);
        let b = i8::try_from(pos[1] >> 4).unwrap_or(-1);
        if (crate::placement::tile_flags(world, a, b, 1).map_err(fail)? & 0x20) != 0 {
            return Ok(false);
        }
        if !self.fits_in(world, id, container, slot, pos, level)? {
            return Ok(false);
        }
        if (pos[0] & 0x0f) != 8 || (pos[1] & 0x0f) != 8 {
            return Ok(false);
        }
        let height = self
            .container_slot_height(world, container, slot)
            .map_err(fail)?;
        Ok(!matches!(height, Some(5..=7)))
    }

    /// Tests whether `held` fits at `[pos, level]` (`0x422050`): first tries
    /// any `retained` surface container at the same tile without re-aligning
    /// facing (`0x422050` line `0x422078`), then runs `UserTestPlace`
    /// (`0x435522` / `0x422de4`, `Some(None)`), and — after a collision
    /// refusal (`0x0b`) on a single-tile object — searches compatible surface
    /// slots on that tile (`Some(Some((container_id, slot)))`), rejecting slot
    /// heights `5..=7` (`0x43560c..0x435623`) and aligning facing via
    /// `0x4221e4..0x422280` + `0x43244d`.
    fn test_buy_placement(
        &self,
        world: &mut World,
        cursor: u16,
        held: u16,
        pos: [i32; 2],
        level: i32,
        retained: Option<(u16, i16)>,
    ) -> Result<Option<Option<(u16, i16)>>, Fault> {
        if let Some((container, slot)) = retained
            && self.user_fits_in(world, held, container, i32::from(slot), pos, level)?
        {
            return Ok(Some(Some((container, slot))));
        }
        match self.user_test_place_ground(world, held, pos, level)? {
            Ok(()) => return Ok(Some(None)),
            Err(Refusal::Collision(_)) => {}
            Err(_) => return Ok(None),
        }
        // Single-tile object fallback after collision (0x0b): check surface
        // containers on this tile using data word 4 (+0x52, allowed heights,
        // `0x4220a5`).
        let (is_group, allowed_heights, current_facing) = {
            let Some(obj) = world.objects.get(&held) else {
                return Ok(None);
            };
            (
                obj.multipart.is_some(),
                obj.data.get(DATA_ALLOWED_HEIGHTS).copied().unwrap_or(0),
                obj.data.get(DATA_FACING).copied().unwrap_or(0),
            )
        };
        if is_group || allowed_heights == 0 {
            return Ok(None);
        }
        let tile = [
            u8::try_from(pos[0] >> 4).unwrap_or(0),
            u8::try_from(pos[1] >> 4).unwrap_or(0),
            u8::try_from(level).unwrap_or(1),
        ];
        let candidates = crate::placement::standing_on(world, tile).map_err(|trap| Fault {
            object: held,
            at: None,
            trap,
        })?;
        for container in candidates {
            let slot_count = world
                .objects
                .get(&container)
                .map_or(0, |o| o.slots.len().saturating_sub(1));
            for slot in 0..slot_count {
                if self.user_fits_in(world, held, container, slot as i32, pos, level)? {
                    let target_dir =
                        surface_aligned_direction(world, container, current_facing, level)?;
                    self.rotate_held_to(world, cursor, held, target_dir)?;
                    return Ok(Some(Some((container, slot as i16))));
                }
            }
        }
        Ok(None)
    }
}

/// Computes the surface-aligned facing direction for a single-tile object
/// placed onto `container` (`0x4221e4..0x422280`): uses `container.data[1]`
/// directly for single-tile containers, or walks up to four quarter-turns for
/// a multi-tile container until the tile one step forward is not blocked by an
/// object (`0x43244d`), keeping `fallback_facing` if all four sides are blocked.
fn surface_aligned_direction(
    world: &World,
    container: u16,
    fallback_facing: i16,
    level: i32,
) -> Result<i16, Fault> {
    let Some(cont) = world.objects.get(&container) else {
        return Ok(fallback_facing);
    };
    let mut dir = cont.data.get(DATA_FACING).copied().unwrap_or(0);
    if cont.multipart.is_none() {
        return Ok(dir);
    }
    let origin = [cont.position[0], cont.position[1]];
    for _ in 0..4 {
        let delta = step(dir);
        let probe = [
            origin[0].wrapping_add(delta[0].wrapping_mul(16)),
            origin[1].wrapping_add(delta[1].wrapping_mul(16)),
        ];
        if !tile_blocks_person(world, probe, level).map_err(|trap| Fault {
            object: container,
            at: None,
            trap,
        })? {
            return Ok(dir);
        }
        dir = dir.wrapping_add(2);
    }
    Ok(fallback_facing)
}

/// Checks `0x43244d`: whether any object directly on the tile chain of
/// `[pos, level]` (walking slot `0`, skipping objects held in container slots
/// `1..`, matching `0x43248a` + `0x432823(0)`) blocks a person (when
/// `data[8] & 4 == 0`: blocked if `data[8] & 0x10 == 0`; when
/// `data[8] & 4 != 0`: blocked if `data[8] & 2 != 0`).
fn tile_blocks_person(world: &World, pos: [i32; 2], level: i32) -> Result<bool, Trap> {
    let tx = pos[0] >> 4;
    let ty = pos[1] >> 4;
    if !(0..64).contains(&tx) || !(0..64).contains(&ty) || !(1..=2).contains(&level) {
        return Ok(false);
    }
    let mut current = crate::placement::first_on(world, [tx, ty], level as i8);
    let mut visited = 0usize;
    while let Some(id) = current {
        visited += 1;
        if visited > world.objects.len() {
            return Err(Trap::Invalid("the objects on a tile link in a loop".into()));
        }
        let Some(obj) = world.objects.get(&id) else {
            break;
        };
        let flags = obj.data.get(DATA_PLACEMENT_FLAGS).copied().unwrap_or(0) as u16;
        let blocked = if (flags & 0x04) == 0 {
            (flags & 0x10) == 0
        } else {
            (flags & 0x02) != 0
        };
        if blocked {
            return Ok(true);
        }
        current = obj
            .slots
            .first()
            .and_then(|s| u16::try_from(s.object).ok())
            .filter(|&next| next != 0);
    }
    Ok(false)
}
