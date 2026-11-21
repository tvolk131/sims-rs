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
/// Object data word 23: rotation step in eighths (`object + 0x78`).
const DATA_TURN_STEP: usize = 23;

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
        let keep_buying = words[KEEP_BUYING] == 1
            && world.globals.get(TUTORIAL_GLOBAL).copied().unwrap_or(0) == 0;
        if price > 0 && price > available_funds(world) {
            return Ok(None);
        }
        let held = self.create(world, guid)?;
        let cursor = self.create_tile_cursor(world, held)?;
        self.rotate_held_to(world, cursor, held, direction)?;
        Ok(Some(BuyCursor {
            cursor,
            held,
            guid,
            price,
            budget_category,
            keep_buying,
            valid: false,
            position: [-16, -16, level],
            container: None,
        }))
    }

    /// Moves a Buy Mode cursor and its held object to the tile containing
    /// fine lot coordinates `fine` on `level` (`0x485a2e` → `0x421e72` →
    /// `0x422050` + `0x422c2f`).
    ///
    /// Returns whether the position is valid and affordable. When valid, both
    /// the held object and its `"Tile Cursor"` stand on the tile (with the
    /// cursor placed second so `held.data[26]` links to `cursor`, matching
    /// `0x421e72`). When invalid, the held object stays off the lot.
    pub fn buy_move(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        fine: [i32; 2],
        level: i32,
    ) -> Result<bool, Fault> {
        let pos = snap_to_tile_centre(fine);
        self.remove_cursor_and_held(world, cursor)?;
        cursor.position = [pos[0], pos[1], level];
        cursor.valid = false;
        cursor.container = None;
        if !inside_placement_bounds(world, pos) {
            return Ok(false);
        }
        let affordable = cursor.price <= 0 || cursor.price <= available_funds(world);
        let placement = self.test_buy_placement(world, cursor.cursor, cursor.held, pos, level)?;
        if let Some(container) = placement {
            if affordable {
                cursor.valid = true;
                cursor.container = container;
            }
            let held_place = Place {
                on_lot: true,
                position: pos,
                container: container.map(|(id, _)| id),
                slot: container.map_or(-1, |(_, slot)| i32::from(slot)),
                level,
            };
            self.put(world, cursor.held, held_place, true)?;
        }
        if self.fits(world, cursor.cursor, pos, level)? {
            let cursor_place = Place {
                on_lot: true,
                position: pos,
                container: None,
                slot: -1,
                level,
            };
            self.put(world, cursor.cursor, cursor_place, true)?;
        }
        Ok(cursor.valid)
    }

    /// Rotates the held object to `direction` (`0x4226bc` → `0x421be4`) and
    /// re-evaluates placement at its current tile if it is on the lot. When
    /// the object is already snapped to a surface container slot, keeps that
    /// container without re-running automatic surface-facing alignment
    /// (`0x422050` line `0x422078`).
    pub fn buy_rotate(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        direction: i16,
    ) -> Result<bool, Fault> {
        let [x, y, level] = cursor.position;
        let prev_container = cursor.container;
        self.remove_cursor_and_held(world, cursor)?;
        self.rotate_held_to(world, cursor.cursor, cursor.held, direction)?;
        if !on_lot([x, y]) {
            return Ok(false);
        }
        if let Some((container, slot)) = prev_container {
            let pos = snap_to_tile_centre([x, y]);
            if inside_placement_bounds(world, pos)
                && self.fits_in(world, cursor.held, container, i32::from(slot), pos, level)?
            {
                let affordable = cursor.price <= 0 || cursor.price <= available_funds(world);
                cursor.position = [pos[0], pos[1], level];
                cursor.valid = affordable;
                cursor.container = Some((container, slot));
                let held_place = Place {
                    on_lot: true,
                    position: pos,
                    container: Some(container),
                    slot: i32::from(slot),
                    level,
                };
                self.put(world, cursor.held, held_place, true)?;
                if self.fits(world, cursor.cursor, pos, level)? {
                    let cursor_place = Place {
                        on_lot: true,
                        position: pos,
                        container: None,
                        slot: -1,
                        level,
                    };
                    self.put(world, cursor.cursor, cursor_place, true)?;
                }
                return Ok(cursor.valid);
            }
        }
        self.buy_move(world, cursor, [x, y], level)
    }

    /// Hides the cursor and held object off the lot while the pointer leaves
    /// the house view (`0x487d4e` → `0x421963`).
    pub fn buy_hide(&self, world: &mut World, cursor: &mut BuyCursor) -> Result<(), Fault> {
        self.remove_cursor_and_held(world, cursor)?;
        cursor.position = [-16, -16, cursor.position[2]];
        cursor.valid = false;
        cursor.container = None;
        Ok(())
    }

    /// Commits a valid Buy Mode placement (`0x486ada` → `0x421a0e` →
    /// `0x43562f` / `0x422e99`, `0x4929fb` → `0x46d91c`): removes and deletes
    /// the `"Tile Cursor"`, places the held object permanently at its
    /// validated tile/container, runs OBJf function 11 (`USER_PLACEMENT`) on
    /// the object (and every group tile / held child), and deducts its price
    /// from household funds. Returns the placed object's ID.
    pub fn buy_commit(&self, world: &mut World, cursor: &BuyCursor) -> Result<u16, Fault> {
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
        Ok(cursor.held)
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

    /// Rotates `held` toward `target_dir` in steps of its `DATA_TURN_STEP`
    /// (`0x4226bc` → `0x421be4`), updating multipart offsets on both `held`
    /// and `cursor` when multi-tile while leaving `cursor`'s own `data[1]`
    /// unchanged (matching `0x421be4` when `param_1[0x6b] != 0`).
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
        let target = target_dir.rem_euclid(8);
        let (current, step, is_group) = {
            let h = world
                .objects
                .get(&held)
                .ok_or_else(|| fail(held, Trap::native(10, "missing held object")))?;
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
            let parts = self.parts(world, held).map_err(|t| fail(held, t))?;
            for &part in &parts {
                if let Some(obj) = world.objects.get_mut(&part)
                    && let Some(dir) = obj.data.get_mut(DATA_FACING)
                {
                    *dir = dir.wrapping_add(delta) & 7;
                }
            }
            self.update_offsets(world, held)
                .map_err(|t| fail(held, t))?;
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

    /// Removes `cursor` and `held` from the lot (`0x421963`).
    fn remove_cursor_and_held(&self, world: &mut World, cursor: &BuyCursor) -> Result<(), Fault> {
        if world
            .objects
            .get(&cursor.cursor)
            .is_some_and(|o| on_lot([o.position[0], o.position[1]]))
        {
            self.take_off(world, cursor.cursor, true)?;
        }
        if world
            .objects
            .get(&cursor.held)
            .is_some_and(|o| on_lot([o.position[0], o.position[1]]))
        {
            self.take_off(world, cursor.held, true)?;
        }
        Ok(())
    }

    /// Runs OBJf function 11 (`USER_PLACEMENT`) on `held` (or every part of a
    /// multi-tile group) and on every non-empty object in each part's surface
    /// slots (`0x43562f` / `0x422e99`).
    fn run_user_placement(&self, world: &mut World, held: u16) -> Result<(), Fault> {
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
            self.callback(world, part, function::USER_PLACEMENT, 0)?;
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
                    self.callback(world, child, function::USER_PLACEMENT, 0)?;
                }
            }
        }
        Ok(())
    }

    /// Tests whether `held` fits at `[pos, level]`, either directly on the
    /// tile (`Some(None)`) or — after a collision refusal (`0x0b`) on a
    /// single-tile object — inside a compatible surface slot of an object
    /// standing on that tile (`Some(Some((container_id, slot)))`, `0x422050`).
    ///
    /// When snapping into a surface slot, aligns `held`'s facing to the
    /// container's facing (or, for a multi-tile container, the first of four
    /// cardinal directions around the container whose adjacent tile is not
    /// blocked by an object, `0x4221e4..0x422280` + `0x43244d`).
    fn test_buy_placement(
        &self,
        world: &mut World,
        cursor: u16,
        held: u16,
        pos: [i32; 2],
        level: i32,
    ) -> Result<Option<Option<(u16, i16)>>, Fault> {
        match self.place_test(world, held, pos, level)? {
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
                if self.fits_in(world, held, container, slot as i32, pos, level)? {
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

/// Checks `0x43244d`: whether any object standing on `[pos, level]` blocks a
/// person (when `data[8] & 4 == 0`: blocked if `data[8] & 0x10 == 0`; when
/// `data[8] & 4 != 0`: blocked if `data[8] & 2 != 0`).
fn tile_blocks_person(world: &World, pos: [i32; 2], level: i32) -> Result<bool, Trap> {
    let tx = pos[0] >> 4;
    let ty = pos[1] >> 4;
    if !(0..64).contains(&tx) || !(0..64).contains(&ty) || !(1..=2).contains(&level) {
        return Ok(false);
    }
    let tile = [tx as u8, ty as u8, level as u8];
    for id in crate::placement::standing_on(world, tile)? {
        let flags = world
            .objects
            .get(&id)
            .and_then(|o| o.data.get(DATA_PLACEMENT_FLAGS))
            .copied()
            .unwrap_or(0) as u16;
        let blocked = if (flags & 0x04) == 0 {
            (flags & 0x10) == 0
        } else {
            (flags & 0x02) != 0
        };
        if blocked {
            return Ok(true);
        }
    }
    Ok(false)
}
