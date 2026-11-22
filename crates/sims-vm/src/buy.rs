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
use crate::placement::{Place, Refusal, capture, on_lot, step};
use crate::primitives::budget::spend;
use crate::visits::function;
use sims_sim::World;
use std::collections::BTreeSet;

/// The `"Tile Cursor"` object GUID looked up by name (`0x43ab8f`) in
/// `0x486f5a` and `0x486960`.
pub const TILE_CURSOR_GUID: u32 = 0x0000_0437;

/// The `"comeandsee"` object GUID looked up by name (`0x43ab8f`) in `0x40726a`
/// when returning from Buy/Build Mode to Live Mode (`0x4cea5d` → `0x4ceac2`).
pub const COMEANDSEE_GUID: u32 = 0xf3aa_a62f;

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
/// Object data word 8 (`+0x5a`): placement/intersection/in-use flags (`0x43244d`, `0x4352e8`).
const DATA_PLACEMENT_FLAGS: usize = 8;
/// Object data word 13 (`+0x64`): wall-attachment rules (`0x43583e`).
const DATA_WALL_RULES: usize = 13;
/// Object data word 15 (`+0x68`): broken state; nonzero halves resale value (`0x43219a`).
const DATA_BROKEN: usize = 15;
/// Object data word 23: rotation step in eighths (`object + 0x78`).
const DATA_TURN_STEP: usize = 23;
/// Object data word 41 (`+0x9c`): current depreciated resale value (`0x43219a`).
const DATA_VALUE: usize = 41;
/// Object data word 42 (`+0x9e`): placement flags (`0x800` = pool tile, `0x43583e`).
const DATA_PLACEMENT: usize = 42;
/// Object data word 43 (`+0xa0`): Buy Mode permission bits (`0x02` = CanPickUp,
/// `0x08` = CanDelete, `0x4352e8`, `0x435440`).
const DATA_BUY_FLAGS: usize = 43;
/// Object data word 62 (`+0xc6`): active use count (`0x4352e8`: must be `<= 0`).
const DATA_USE_COUNT: usize = 62;

/// Snapshot of an object's placement and recursively contained surface children,
/// captured for Hand Tool cancel snapback (`0x487191`) and Buy/Move/Sell
/// Undo/Redo (`0x48ba19`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyPlacement {
    /// Lead object ID.
    pub object: u16,
    /// Object definition GUID.
    pub guid: u32,
    /// Fine lot coordinates `[x, y, level]`.
    pub position: [i32; 3],
    /// Facing direction (`data[1]`, `0..7`).
    pub facing: i16,
    /// Surface container `(container_id, 0-based slot)` if held on a surface.
    pub container: Option<(u16, i16)>,
    /// Objects standing in this object's (or its group parts') surface slots.
    pub children: Vec<BuyPlacement>,
}

/// Which Buy Mode operation an Undo/Redo history record represents
/// (`0x48ba19` / `0x48bcc2` / `0x48bd21` / `0x48bdfe`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuyActionKind {
    /// Newly purchased catalog object (native kind `0`).
    Buy,
    /// Existing object picked up and moved/rotated (native kind `1`).
    Move,
    /// Existing object sold/deleted (native kind `3`).
    Sell,
}

/// Active Buy Mode placement cursor wrapping either a newly created catalog
/// object (`origin == None`) or a picked-up existing lot object
/// (`origin == Some(...)`) and its `"Tile Cursor"` (`0x00000437`) helper
/// (`cTSObjectTool` + `cTSTileCursor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyCursor {
    /// Lead object ID of the `"Tile Cursor"` (`0x00000437`).
    pub cursor: u16,
    /// Lead object ID of the catalog or existing object being held.
    pub held: u16,
    /// Catalog master / definition GUID of the held object.
    pub guid: u32,
    /// Purchase price read from the catalog master's OBJD word 18 (`0x421e42`),
    /// or `0` when moving an already-owned lot object.
    pub price: i32,
    /// Budget category charged or refunded (`6` Buy, `7` Build, `0x4878f3`).
    pub budget_category: u8,
    /// Catalog master facing offset read from OBJD word 61 (`+0x7a`, `0x434f8d`).
    pub facing_offset: i16,
    /// Whether committing this item automatically arms another copy when
    /// affordable (`0x4878f3`: OBJD word 96 == 1 and tutorial global 26 == 0).
    /// Always `false` for picked-up existing objects.
    pub keep_buying: bool,
    /// Whether the held object currently sits at a valid, affordable spot.
    pub valid: bool,
    /// Last tile-snapped fine lot position `[x, y, level]`.
    pub position: [i32; 3],
    /// Surface container `(id, slot)` if the held object snapped onto a
    /// surface slot (`0x422050`).
    pub container: Option<(u16, i16)>,
    /// Original placement before pickup when holding an existing lot object
    /// (`0x486960` → `0x48ba19`), or `None` for a new unpaid catalog purchase.
    pub origin: Option<BuyPlacement>,
    /// Monotonic token stamped by `Lot` so retired cursor handles can never
    /// alias newly created objects that reuse a numeric ID.
    pub(crate) token: u64,
}

impl BuyCursor {
    /// Whether this cursor holds an already-owned lot object picked up by the
    /// Hand Tool (`origin.is_some()`, native `[tool + 0x75] == 0`).
    pub fn is_existing(&self) -> bool {
        self.origin.is_some()
    }
}

/// One committed Buy Mode action (Buy, Move, or Sell) recorded for Undo / Redo
/// (`0x48ba19` / `0x48beb5` / `0x493259`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyPurchase {
    /// Which Buy Mode operation this record represents (Buy, Move, or Sell).
    pub kind: BuyActionKind,
    /// Lead object ID of the affected object.
    pub object: u16,
    /// Catalog master / definition GUID of the object.
    pub guid: u32,
    /// Committed fine lot coordinates `[x, y, level]` (for Buy/Move) or
    /// pre-sale coordinates (for Sell).
    pub position: [i32; 3],
    /// Committed facing direction (`data[1]`, `0..7`), or pre-sale facing.
    pub facing: i16,
    /// Surface container `(id, slot)` if placed on a surface slot.
    pub container: Option<(u16, i16)>,
    /// Purchase price (for Buy), `0` (for Move), or resale refund (for Sell).
    pub price: i32,
    /// Budget category charged/refunded (`6` Buy, `7` Build, `0x48bdbd`).
    pub budget_category: u8,
    /// Whether this record currently holds an on-lot placement (`true` after
    /// Buy/Move commit or Sell Undo; `false` after Buy Undo, Sell, or refused
    /// Redo transferred to Undo via `0x48be72` → `0x493353`).
    pub placed: bool,
    /// Pre-action placement snapshot (including surface children) for Move and
    /// Sell (`0x48ba19`).
    pub before: Option<BuyPlacement>,
    /// Post-action placement snapshot (including surface children) for Buy and
    /// Move (`0x486ada`).
    pub after: Option<BuyPlacement>,
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

/// Collects every object ID (root + recursive surface children) referenced by
/// `purchases` into `out`, so purging discarded history never deletes an
/// off-lot object still owned by a surviving Undo/Redo entry (`0x48c20e`).
pub(crate) fn collect_placement_ids(placement: &BuyPlacement, out: &mut BTreeSet<u16>) {
    out.insert(placement.object);
    for child in &placement.children {
        collect_placement_ids(child, out);
    }
}

pub(crate) fn collect_buy_ids(purchases: &[BuyPurchase], out: &mut BTreeSet<u16>) {
    for purchase in purchases {
        out.insert(purchase.object);
        if let Some(before) = &purchase.before {
            collect_placement_ids(before, out);
        }
        if let Some(after) = &purchase.after {
            collect_placement_ids(after, out);
        }
    }
}

pub(crate) fn collect_cursor_ids(cursor: &BuyCursor, out: &mut BTreeSet<u16>) {
    out.insert(cursor.cursor);
    out.insert(cursor.held);
    if let Some(origin) = &cursor.origin {
        collect_placement_ids(origin, out);
    }
}

/// Returns whether the root object and every recursive surface child in
/// `target` are currently placed on-lot (preventing `Move` Undo/Redo from
/// resurrecting a detached/refunded child for free).
fn all_placement_objects_on_lot(world: &World, target: &BuyPlacement) -> bool {
    world
        .objects
        .get(&target.object)
        .is_some_and(|o| on_lot([o.position[0], o.position[1]]))
        && target
            .children
            .iter()
            .all(|child| all_placement_objects_on_lot(world, child))
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

    /// Spawns or repositions `"comeandsee"` (`0xf3aaa62f`) markers for newly
    /// bought / moved objects when leaving Buy or Build Mode back to Live Mode
    /// (`0x4cea5d` → `0x4ceac2` → `0x40726a`).
    pub fn update_comeandsee(&self, world: &mut World) -> Result<(), Fault> {
        if self.catalog.object(COMEANDSEE_GUID).is_none() {
            return Ok(());
        }
        let lot = world.globals.get(10).copied().unwrap_or(0);
        let zoning = if lot == 0 {
            1
        } else {
            self.program.zoning.get(&lot).copied().unwrap_or(0)
        };
        // 0x407299..0x4072e7: skip Downtown (21..=30), Studio (81..=89),
        // Vacation (40..=49), and Magic (zoning == 2).
        if (21..=30).contains(&lot)
            || (81..=89).contains(&lot)
            || (40..=49).contains(&lot)
            || zoning == 2
        {
            return Ok(());
        }
        let cur_hour = world.globals.first().copied().unwrap_or(0);
        let cur_day = world.globals.get(1).copied().unwrap_or(0);
        let cur_min = world.globals.get(5).copied().unwrap_or(0);
        let cur_month = world.globals.get(7).copied().unwrap_or(0);
        let cur_year = world.globals.get(8).copied().unwrap_or(0);
        // 0x4072ed: walk the object registry linked list (`[registry + 0x30]`
        // / `[obj + 0xe0]`, mirrored in `world.order` newest-first).
        let ids: Vec<u16> = world.order.clone();
        for id in ids {
            let Some(obj) = world.objects.get(&id) else {
                continue;
            };
            if !on_lot([obj.position[0], obj.position[1]]) {
                continue;
            }
            // 0x40731e (`[obj + 0x8e] == 0`, data[34]) and 0x40732c
            // (`[obj + 0x5b] & 0x40 == 0`, data[8] & 0x4000).
            if obj.data.get(34).copied().unwrap_or(0) != 0 {
                continue;
            }
            if (obj.data.get(8).copied().unwrap_or(0) as u16 & 0x4000) != 0 {
                continue;
            }
            // 0x407351..0x40735f: only process the lead tile of multi-tile objects.
            if obj.multipart.as_ref().is_some_and(|m| m.parent != 0) {
                continue;
            }
            let fail = |trap| Fault {
                object: id,
                at: None,
                trap,
            };
            let words = self.words(world, id).map_err(fail)?;
            if words[word::TYPE] != 4 {
                continue;
            }
            // 0x407365 → 0x4074df: check whether `id` already owns a comeandsee
            // marker via its relationship table (`row.values[0] > 0`).
            let existing = world.objects.get(&id).and_then(|o| {
                o.relationships.iter().find_map(|row| {
                    if row.present == 0 || row.values.first().copied().unwrap_or(0) <= 0 {
                        return None;
                    }
                    let target = u16::try_from(row.target).ok()?;
                    world
                        .objects
                        .get(&target)
                        .filter(|t| t.guid == COMEANDSEE_GUID)
                        .map(|_| target)
                })
            });
            let helper = if let Some(helper) = existing {
                helper
            } else {
                // 0x407374 → 0x43520d: age in sim-minutes must be 0 (bought at
                // the current clock minute) and lot must be residential (`0x40745f`).
                let Some(o) = world.objects.get(&id) else {
                    continue;
                };
                let age_zero = o.data.get(21).copied().unwrap_or(0) == cur_min
                    && o.data.get(24).copied().unwrap_or(0) == cur_hour
                    && o.data.get(45).copied().unwrap_or(0) == cur_year
                    && o.data.get(46).copied().unwrap_or(0) == cur_month
                    && o.data.get(47).copied().unwrap_or(0) == cur_day;
                if !age_zero || zoning != 1 {
                    continue;
                }
                let helper = self.create(world, COMEANDSEE_GUID)?;
                // 0x4073bb (`0x464fe2`) + 0x4073cb (`0x4650c3`): set relationship
                // `(id -> helper)[0] = 1`.
                if let Some(o) = world.objects.get_mut(&id) {
                    let key = i32::from(helper);
                    if let Some(row) = o
                        .relationships
                        .iter_mut()
                        .find(|r| r.present != 0 && r.target == key)
                    {
                        row.values.resize(1, 0);
                        row.values[0] = 1;
                    } else {
                        o.relationships
                            .push(sims_formats::saved_objects::Relationship {
                                present: 1,
                                target: key,
                                values: vec![1],
                            });
                    }
                }
                helper
            };
            // 0x4073d5 → 0x4071a3: average fine `(x, y)` across all group tiles.
            let parts = if world
                .objects
                .get(&id)
                .is_some_and(|o| o.multipart.is_some())
            {
                self.parts(world, id).map_err(fail)?
            } else {
                vec![id]
            };
            let mut sum_x = 0i64;
            let mut sum_y = 0i64;
            let mut count = 0i64;
            for part in parts {
                if let Some(o) = world.objects.get(&part) {
                    sum_x += i64::from(o.position[0]);
                    sum_y += i64::from(o.position[1]);
                    count += 1;
                }
            }
            let Some(o) = world.objects.get(&id) else {
                continue;
            };
            let level = o.position[2];
            let target_pos = if count > 0 {
                [(sum_x / count) as i32, (sum_y / count) as i32]
            } else {
                [o.position[0], o.position[1]]
            };
            let needs_place = world.objects.get(&helper).is_some_and(|h| {
                !on_lot([h.position[0], h.position[1]])
                    || h.position != [target_pos[0], target_pos[1], level]
            });
            if needs_place {
                self.put(
                    world,
                    helper,
                    Place {
                        on_lot: true,
                        position: target_pos,
                        container: None,
                        slot: -1,
                        level,
                    },
                    true,
                )?;
            }
        }
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
            origin: None,
            token: 0,
        };
        self.buy_rotate(world, &mut out, direction)?;
        Ok(Some(out))
    }

    /// Resolves any part of a multi-tile object to its group lead (`+0x190`).
    pub fn lead_object(&self, world: &World, id: u16) -> u16 {
        world
            .objects
            .get(&id)
            .and_then(|o| o.multipart.as_ref())
            .and_then(|m| u16::try_from(m.parent).ok())
            .filter(|&parent| parent != 0 && world.objects.contains_key(&parent))
            .unwrap_or(id)
    }

    /// Returns whether `id` is a reserved, sentinel, person, or global
    /// controller object that must never be picked up or deleted even when
    /// `move_objects` is enabled.
    fn is_reserved_or_unpickable(&self, world: &World, id: u16) -> Result<bool, Fault> {
        if id == 0 || world.indicators.contains(&id) {
            return Ok(true);
        }
        let Some(obj) = world.objects.get(&id) else {
            return Ok(true);
        };
        if obj.person.is_some()
            || obj.guid == TILE_CURSOR_GUID
            || obj.guid == crate::WALL_CURSOR_GUID
            || obj.guid == COMEANDSEE_GUID
        {
            return Ok(true);
        }
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let words = self.words(world, id).map_err(fail)?;
        // OBJD word 9 == 7 (system/controller) or word 42 != 0 (global controller).
        Ok(words[word::TYPE] == 7 || words[42] != 0)
    }

    /// Checks that `id` (or every part of its multi-tile group) lands strictly
    /// inside lot interior bounds (`1..size - 1`) on `level in 1..=2`.
    fn all_parts_within_lot_bounds(
        &self,
        world: &World,
        id: u16,
        pos: [i32; 2],
        level: i32,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        if !on_lot(pos) {
            return Ok(false);
        }
        let Some(selected) = world
            .objects
            .get(&id)
            .and_then(|o| o.multipart.as_ref())
            .map(|m| m.offset)
        else {
            return Ok((1..=2).contains(&level) && inside_placement_bounds(world, pos));
        };
        let origin = [
            pos[0].wrapping_sub(selected[0].wrapping_mul(16)),
            pos[1].wrapping_sub(selected[1].wrapping_mul(16)),
        ];
        let origin_level = level.wrapping_sub(selected[2]);
        for part in self.parts(world, id).map_err(fail)? {
            let Some(offset) = world
                .objects
                .get(&part)
                .and_then(|o| o.multipart.as_ref())
                .map(|m| m.offset)
            else {
                return Ok(false);
            };
            let at = [
                origin[0].wrapping_add(offset[0].wrapping_mul(16)),
                origin[1].wrapping_add(offset[1].wrapping_mul(16)),
            ];
            let part_level = origin_level.wrapping_add(offset[2]);
            if !(1..=2).contains(&part_level) || !inside_placement_bounds(world, at) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Cursor fit check for Buy Mode: delegates to [`Self::fits`] normally, or
    /// enforces strict lot interior bounds + valid floor level when
    /// `self.move_objects` is enabled.
    fn cursor_fits_for_buy(
        &self,
        world: &mut World,
        cursor: u16,
        pos: [i32; 2],
        level: i32,
    ) -> Result<bool, Fault> {
        if self.move_objects {
            self.all_parts_within_lot_bounds(world, cursor, pos, level)
        } else {
            self.fits(world, cursor, pos, level)
        }
    }

    /// Bounds-checked ground placement fallback used only when
    /// `self.move_objects` is enabled: still requires tile-centred coordinates
    /// (`(x & 15) == 8 && (y & 15) == 8`), `level in 1..=2`, and every part of
    /// the object strictly inside `1..size - 1`.
    fn user_test_place_ground_override(
        &self,
        world: &World,
        id: u16,
        pos: [i32; 2],
        level: i32,
    ) -> Result<bool, Fault> {
        if (pos[0] & 0x0f) != 8 || (pos[1] & 0x0f) != 8 {
            return Ok(false);
        }
        self.all_parts_within_lot_bounds(world, id, pos, level)
    }

    /// Checks `cTSObject::CanPickUp` (`0x4352e8`) / `cTSMultiTileObject::CanPickUp`
    /// (`0x422fdc`): every part in `id`'s group must have `data[43] & 0x02 != 0`,
    /// `data[62] <= 0`, `data[8] & 0x20 == 0`, level-1 tile flag `0x20` clear,
    /// and every object in its surface containment slots must recursively pass
    /// `can_pick_up`.
    ///
    /// When `self.move_objects` is enabled, bypasses the `data[43]`, in-use
    /// (`data[62]`, `data[8] & 0x20`), and locked-tile (`tile_flags & 0x20`)
    /// guards while still refusing reserved/unpickable sentinels, persons, and
    /// global controllers.
    pub fn can_pick_up(&self, world: &World, id: u16) -> Result<bool, Fault> {
        let mut visited = BTreeSet::new();
        self.can_pick_up_inner(world, id, &mut visited)
    }

    fn can_pick_up_inner(
        &self,
        world: &World,
        id: u16,
        visited: &mut BTreeSet<u16>,
    ) -> Result<bool, Fault> {
        let lead = self.lead_object(world, id);
        if !visited.insert(lead) {
            return Ok(true);
        }
        let Some(lead_obj) = world.objects.get(&lead) else {
            return Ok(false);
        };
        if self.is_reserved_or_unpickable(world, lead)? {
            return Ok(false);
        }
        let fail = |trap| Fault {
            object: lead,
            at: None,
            trap,
        };
        let parts = if lead_obj.multipart.is_some() {
            self.parts(world, lead).map_err(fail)?
        } else {
            vec![lead]
        };
        for part in parts {
            if !self.can_pick_up_part(world, part, visited)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn can_pick_up_part(
        &self,
        world: &World,
        part: u16,
        visited: &mut BTreeSet<u16>,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: part,
            at: None,
            trap,
        };
        let Some(obj) = world.objects.get(&part) else {
            return Ok(false);
        };
        if self.is_reserved_or_unpickable(world, part)? {
            return Ok(false);
        }
        if !self.move_objects {
            if (obj.data.get(DATA_BUY_FLAGS).copied().unwrap_or(0) & 0x02) == 0 {
                return Ok(false);
            }
            if obj.data.get(DATA_USE_COUNT).copied().unwrap_or(0) > 0
                || (obj.data.get(DATA_PLACEMENT_FLAGS).copied().unwrap_or(0) & 0x20) != 0
            {
                return Ok(false);
            }
        }
        let children: Vec<u16> = obj
            .slots
            .iter()
            .skip(1)
            .filter_map(|s| u16::try_from(s.object).ok().filter(|&cid| cid != 0))
            .collect();
        for child in children {
            if world.objects.contains_key(&child)
                && !self.can_pick_up_inner(world, child, visited)?
            {
                return Ok(false);
            }
        }
        if !self.move_objects {
            let tx = obj.position[0] >> 4;
            let ty = obj.position[1] >> 4;
            let size = i32::from(world.size);
            if (0..size).contains(&tx) && (0..size).contains(&ty) {
                let flags =
                    crate::placement::tile_flags(world, tx as i8, ty as i8, 1).map_err(fail)?;
                if (flags & 0x20) != 0 {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Checks `cTSObject::CanDelete` (`0x435440`) / `cTSMultiTileObject::CanDelete`
    /// (`0x423135`): every part in `id`'s group must pass `can_pick_up`, have
    /// `data[43] & 0x08 != 0`, and have all surface containment slots empty.
    ///
    /// When `self.move_objects` is enabled, bypasses the `data[43] & 0x08`
    /// guard while still refusing reserved/unpickable sentinels, persons,
    /// global controllers, and surfaces carrying children.
    pub fn can_delete(&self, world: &World, id: u16) -> Result<bool, Fault> {
        let lead = self.lead_object(world, id);
        let Some(lead_obj) = world.objects.get(&lead) else {
            return Ok(false);
        };
        if self.is_reserved_or_unpickable(world, lead)? {
            return Ok(false);
        }
        let fail = |trap| Fault {
            object: lead,
            at: None,
            trap,
        };
        let parts = if lead_obj.multipart.is_some() {
            self.parts(world, lead).map_err(fail)?
        } else {
            vec![lead]
        };
        let mut visited = BTreeSet::new();
        for part in parts {
            if !self.can_pick_up_part(world, part, &mut visited)? {
                return Ok(false);
            }
            let Some(obj) = world.objects.get(&part) else {
                return Ok(false);
            };
            if !self.move_objects
                && (obj.data.get(DATA_BUY_FLAGS).copied().unwrap_or(0) & 0x08) == 0
            {
                return Ok(false);
            }
            let has_child = obj.slots.iter().skip(1).any(|s| {
                u16::try_from(s.object)
                    .ok()
                    .is_some_and(|cid| cid != 0 && world.objects.contains_key(&cid))
            });
            if has_child {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Computes the resale/depreciation refund of `id` (`0x43219a`): reads the
    /// lead tile's `data[41]` (`+0x9c`) and halves it with signed integer
    /// division if any part in the group has `data[15] != 0` (`+0x68`, broken).
    pub fn resale_value(&self, world: &World, id: u16) -> Result<i32, Fault> {
        let lead = self.lead_object(world, id);
        let Some(lead_obj) = world.objects.get(&lead) else {
            return Ok(0);
        };
        let fail = |trap| Fault {
            object: lead,
            at: None,
            trap,
        };
        let value = i32::from(lead_obj.data.get(DATA_VALUE).copied().unwrap_or(0));
        let parts = if lead_obj.multipart.is_some() {
            self.parts(world, lead).map_err(fail)?
        } else {
            vec![lead]
        };
        let broken = parts.iter().any(|&p| {
            world
                .objects
                .get(&p)
                .is_some_and(|o| o.data.get(DATA_BROKEN).copied().unwrap_or(0) != 0)
        });
        Ok(if broken { value / 2 } else { value })
    }

    /// Captures the placement snapshot of `id` and all recursively contained
    /// surface children (`0x48ba19`).
    fn capture_placement(&self, world: &mut World, id: u16) -> Result<BuyPlacement, Fault> {
        let mut visited = BTreeSet::new();
        self.capture_placement_inner(world, id, &mut visited)
    }

    fn capture_placement_inner(
        &self,
        world: &mut World,
        id: u16,
        visited: &mut BTreeSet<u16>,
    ) -> Result<BuyPlacement, Fault> {
        let lead = self.lead_object(world, id);
        visited.insert(lead);
        let fail = |trap| Fault {
            object: lead,
            at: None,
            trap,
        };
        let place = capture(world, lead).map_err(fail)?;
        let (guid, facing, is_group) = {
            let obj = world
                .objects
                .get(&lead)
                .ok_or_else(|| fail(Trap::native(10, "missing object for placement capture")))?;
            (
                obj.guid,
                obj.data
                    .get(DATA_FACING)
                    .copied()
                    .unwrap_or(0)
                    .rem_euclid(8),
                obj.multipart.is_some(),
            )
        };
        let container = place.container.and_then(|cid| {
            (place.slot >= 0).then_some((cid, i16::try_from(place.slot).unwrap_or(0)))
        });
        let parts = if is_group {
            self.parts(world, lead).map_err(fail)?
        } else {
            vec![lead]
        };
        let mut child_ids = Vec::new();
        for part in parts {
            if let Some(obj) = world.objects.get(&part) {
                for slot in obj.slots.iter().skip(1) {
                    if let Ok(cid) = u16::try_from(slot.object)
                        && cid != 0
                        && world.objects.contains_key(&cid)
                    {
                        let child_lead = self.lead_object(world, cid);
                        if !visited.contains(&child_lead) {
                            child_ids.push(child_lead);
                        }
                    }
                }
            }
        }
        let mut children = Vec::with_capacity(child_ids.len());
        for cid in child_ids {
            if !visited.contains(&cid) {
                children.push(self.capture_placement_inner(world, cid, visited)?);
            }
        }
        Ok(BuyPlacement {
            object: lead,
            guid,
            position: [place.position[0], place.position[1], place.level],
            facing,
            container,
            children,
        })
    }

    /// Picks up an existing lot object with the Buy Mode Hand Tool
    /// (`0x485f98` → `0x486960` → `0x4215e6` → `0x48ba19`).
    ///
    /// Returns `Ok(None)` without mutating `world` if [`Interpreter::can_pick_up`]
    /// refuses `id`. Otherwise wraps `id`'s group in a `"Tile Cursor"`
    /// (`0x00000437`), returning the cursor part and held part corresponding
    /// to the clicked tile `id` (so multi-tile rotation pivots around the
    /// grabbed tile and `0x486960` computes the grab offset from that tile),
    /// runs `USER_PICKUP` (OBJf 12) + `take_off`, and re-places both at `id`'s
    /// original tile/container with `origin` saving the group lead's original
    /// placement.
    pub fn buy_pick_up(&self, world: &mut World, id: u16) -> Result<Option<BuyCursor>, Fault> {
        let Some(picked_pos) = world.objects.get(&id).map(|o| o.position) else {
            return Ok(None);
        };
        let lead = self.lead_object(world, id);
        let Some(lead_obj) = world.objects.get(&lead) else {
            return Ok(None);
        };
        let lead_pos = lead_obj.position;
        if lead_obj.guid == TILE_CURSOR_GUID
            || !on_lot([picked_pos[0], picked_pos[1]])
            || !on_lot([lead_pos[0], lead_pos[1]])
        {
            return Ok(None);
        }
        if !self.can_pick_up(world, lead)? {
            return Ok(None);
        }
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let origin = self.capture_placement(world, lead)?;
        let guid = origin.guid;
        let words = self
            .machine(world, lead, 0, false)
            .definition_words(guid)
            .map_err(fail)?;
        let budget_category = if words[BUILD_MODE_TYPE] != 0 { 7 } else { 6 };
        let facing_offset = words[FACING_OFFSET];
        let cursor = self.create_tile_cursor(world, id)?;
        // Native `0x4215e6`: call `UserPickup(0)` (`OBJf 12` + `take_off`),
        // then test/put the cursor and held object back at the clicked tile.
        self.run_user_pickup(world, lead)?;
        self.take_off(world, lead, true)?;
        let [px, py, level] = picked_pos;
        let mut out = BuyCursor {
            cursor,
            held: id,
            guid,
            price: 0,
            budget_category,
            facing_offset,
            keep_buying: false,
            valid: false,
            position: picked_pos,
            container: origin.container,
            origin: Some(origin),
            token: 0,
        };
        let retained = out.container;
        if on_lot([px, py])
            && self.cursor_fits_for_buy(world, out.cursor, [px, py], level)?
            && !self.try_place_buy_at_mode(world, &mut out, [px, py], level, retained, true)?
        {
            let cursor_place = Place {
                on_lot: true,
                position: [px, py],
                container: None,
                slot: -1,
                level,
            };
            self.put(world, out.cursor, cursor_place, true)?;
            out.valid = false;
        }
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
        if !inside_placement_bounds(world, pos) {
            // Native `0x485c75` -> `0x422632`: exit immediately on border
            // rejection without overwriting the last saved turn position.
            cursor.valid = false;
            return Ok(false);
        }
        cursor.valid = false;
        cursor.container = None;
        // Native `0x485ced`: check whether `"Tile Cursor"` fits at snapped
        // `pos` BEFORE writing `cursor.position` (`0x421e72`) or trying
        // snapped placement / wall auto-rotation (`0x48736b`); if it does not
        // fit, jump straight to unsnapped fallback (`0x485f23`).
        if self.cursor_fits_for_buy(world, cursor.cursor, pos, level)? {
            cursor.position = [pos[0], pos[1], level];
            if self.try_place_buy_at_mode(world, cursor, pos, level, retained, false)? {
                return Ok(cursor.valid);
            }
            // Native `0x485cee` → `0x421e72` → `0x421fc3` (`0x422c2f`): even
            // when the held object fails snapped placement, `"Tile Cursor"` is
            // still placed at `pos` before `try_wall_auto_rotate` (`0x48736b`)
            // runs its first turn.
            let cursor_place = Place {
                on_lot: true,
                position: pos,
                container: None,
                slot: -1,
                level,
            };
            self.put(world, cursor.cursor, cursor_place, true)?;
            if self.try_wall_auto_rotate(world, cursor, pos, level)? {
                return Ok(cursor.valid);
            }
            // Under `move_objects`, if strict placement and wall auto-rotation
            // both refused, allow bounds-checked ground placement override.
            if self.move_objects {
                self.remove_cursor_and_held(world, cursor)?;
                cursor.valid = false;
                cursor.container = None;
                if self.try_place_buy_at_mode(world, cursor, pos, level, retained, true)? {
                    return Ok(cursor.valid);
                }
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
        if self.cursor_fits_for_buy(world, cursor.cursor, fine, level)? {
            // Native `0x485f44` always calls `0x421e72` at `fine` (even when
            // `fine == pos`): `try_place_buy_at` runs `UserTestPlace`
            // (`0x435522` / `0x422de4`), which succeeds only if `fine` is
            // tile-centred and otherwise runs `test_tile` (`0x4355b5`) on the
            // first part before rejecting unsnapped coordinates (`0x4355bc`).
            if self.try_place_buy_at_mode(world, cursor, fine, level, None, true)? {
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
            // Native `0x485f49`: when both snapped and fallback fits fail,
            // `0x421e72` never runs; call `TakeOff(1)` unconditionally so the
            // Tile Cursor becomes physically off-lot (`[-16, -16]`) while
            // preserving the last saved `cursor.position` written by `0x421e72`
            // for manual `Turn` (`0x421be4`).
            self.take_off_cursor_and_held_unconditional(world, cursor)?;
        }
        Ok(false)
    }

    /// Tries placing `cursor.held` (and `cursor.cursor`) at `[at, level]`
    /// (`0x421e72` → `0x422050`). Callers on the cursor-gated paths
    /// (`0x485ced`, `0x485f2f`, `0x422752`, `0x485e59`) check
    /// `self.cursor_fits_for_buy(world, cursor.cursor, at, level)` first,
    /// while `buy_recheck` (`0x485bda`) calls `0x421e72` directly without a
    /// prior fit check. Returns `Ok(true)` if the held object fit and was
    /// placed (setting `cursor.valid` based on affordability).
    fn try_place_buy_at_mode(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        at: [i32; 2],
        level: i32,
        retained: Option<(u16, i16)>,
        allow_override: bool,
    ) -> Result<bool, Fault> {
        let affordable = cursor.price <= 0 || cursor.price <= available_funds(world);
        let container = match self.test_buy_placement(
            world,
            cursor.cursor,
            cursor.held,
            at,
            level,
            retained,
        )? {
            Some(container) => container,
            None if allow_override
                && self.move_objects
                && self.user_test_place_ground_override(world, cursor.held, at, level)? =>
            {
                None
            }
            None => return Ok(false),
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
            self.buy_turn_steps_mode(world, cursor, i32::from(k), None, level, false)?;
            if cursor.valid {
                return Ok(true);
            }
            self.buy_turn_steps_mode(world, cursor, -i32::from(k), None, level, false)?;
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
        self.buy_turn_steps_mode(world, cursor, steps, retained, level, true)
    }

    fn buy_turn_steps_mode(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        steps: i32,
        retained: Option<(u16, i16)>,
        level: i32,
        allow_override: bool,
    ) -> Result<bool, Fault> {
        // Native `0x421bff`: unconditional `TakeOff(1)` on cursor and held.
        self.take_off_cursor_and_held_unconditional(world, cursor)?;
        // Native `0x421c0d`: turn held object by signed `steps` and sync cursor
        // multipart offsets (`0x42238d`).
        self.turn_held_by(world, cursor.cursor, cursor.held, steps)?;
        // Native `0x421db6`: test fit and place at saved coordinates, clearing
        // valid on fit refusal.
        self.put_if_cursor_fits_mode(world, cursor, retained, level, true, allow_override)
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
        self.put_if_cursor_fits_mode(world, cursor, retained, level, clear_on_refusal, true)
    }

    fn put_if_cursor_fits_mode(
        &self,
        world: &mut World,
        cursor: &mut BuyCursor,
        retained: Option<(u16, i16)>,
        level: i32,
        clear_on_refusal: bool,
        allow_override: bool,
    ) -> Result<bool, Fault> {
        let [x, y, _] = cursor.position;
        if !on_lot([x, y]) || !inside_placement_bounds(world, [x, y]) {
            if clear_on_refusal {
                cursor.valid = false;
                cursor.container = None;
            }
            return Ok(false);
        }
        if !self.cursor_fits_for_buy(world, cursor.cursor, [x, y], level)? {
            if clear_on_refusal {
                cursor.valid = false;
                cursor.container = None;
            }
            return Ok(cursor.valid);
        }
        cursor.valid = false;
        cursor.container = None;
        if self.try_place_buy_at_mode(world, cursor, [x, y], level, retained, allow_override)? {
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
        if self.try_place_buy_at_mode(world, cursor, [x, y], level, retained, true)? {
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
    /// `TakeOff(1)` unconditionally whenever a cursor exists while preserving
    /// the saved coordinates and surface container read by `Turn` `0x421be4`).
    pub fn buy_hide(&self, world: &mut World, cursor: &mut BuyCursor) -> Result<(), Fault> {
        self.take_off_cursor_and_held_unconditional(world, cursor)?;
        cursor.valid = false;
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
        if !world
            .objects
            .get(&cursor.held)
            .is_some_and(|o| on_lot([o.position[0], o.position[1]]))
            || cursor.container.is_some_and(|(cid, _)| {
                !world
                    .objects
                    .get(&cid)
                    .is_some_and(|o| on_lot([o.position[0], o.position[1]]))
            })
        {
            return Err(fail(Trap::Invalid(
                "cannot commit Buy Mode placement: held object or surface container is off-lot"
                    .into(),
            )));
        }
        if cursor.price > 0 && cursor.price > available_funds(world) {
            return Err(fail(Trap::Invalid(
                "cannot commit Buy Mode placement: insufficient funds".into(),
            )));
        }
        self.remove_cursor_and_held(world, cursor)?;
        self.remove_now(world, self.lead_object(world, cursor.cursor))?;
        let [x, y, level] = cursor.position;
        let place = Place {
            on_lot: true,
            position: [x, y],
            container: cursor.container.map(|(id, _)| id),
            slot: cursor.container.map_or(-1, |(_, slot)| i32::from(slot)),
            level,
        };
        self.put(world, cursor.held, place, true)?;
        let held_lead = self.lead_object(world, cursor.held);
        self.run_user_placement(world, held_lead)?;
        if !cursor.is_existing() {
            if cursor.price > 0 && cursor.price > available_funds(world) {
                return Err(fail(Trap::Invalid(
                    "cannot commit Buy Mode placement: insufficient funds".into(),
                )));
            }
            spend(world, cursor.budget_category, cursor.price).map_err(fail)?;
        }
        let after = self.capture_placement(world, held_lead)?;
        let kind = if cursor.is_existing() {
            BuyActionKind::Move
        } else {
            BuyActionKind::Buy
        };
        Ok(BuyPurchase {
            kind,
            object: held_lead,
            guid: cursor.guid,
            position: after.position,
            facing: after.facing,
            container: after.container,
            price: if cursor.is_existing() {
                0
            } else {
                cursor.price
            },
            budget_category: cursor.budget_category,
            placed: true,
            before: cursor.origin.clone(),
            after: Some(after),
        })
    }

    /// Commits a valid Buy Mode placement and returns the placed object's ID.
    pub fn buy_commit(&self, world: &mut World, cursor: &BuyCursor) -> Result<u16, Fault> {
        self.buy_commit_record(world, cursor).map(|p| p.object)
    }

    /// Sells or deletes the held Buy Mode object (`0x487e18` → `0x435440` /
    /// `0x423135`, `0x43219a`, `0x46d91c`, `0x487105`).
    ///
    /// - Returns `Ok(None)` without mutating `world` if `can_delete` refuses
    ///   `cursor.held` (e.g. a table with objects still on it, or `data[43] & 8 == 0`).
    /// - For a new unpaid catalog preview (`!cursor.is_existing()`), cancels
    ///   and deletes both `"Tile Cursor"` and `cursor.held` with `0` refund
    ///   and returns `Ok(Some(None))`.
    /// - For a picked-up existing object (`cursor.is_existing()`), takes off
    ///   both cursor and held if on-lot, refunds `resale_value` to household
    ///   funds, deletes only `"Tile Cursor"`, leaves `cursor.held` detached
    ///   off-lot (`[-16, -16]`) for Undo, and returns `Ok(Some(Some(record)))`
    ///   with `kind = BuyActionKind::Sell`.
    pub fn buy_sell(
        &self,
        world: &mut World,
        cursor: &BuyCursor,
    ) -> Result<Option<Option<BuyPurchase>>, Fault> {
        if !self.can_delete(world, cursor.held)? {
            return Ok(None);
        }
        if !cursor.is_existing() {
            self.buy_cancel(world, cursor)?;
            return Ok(Some(None));
        }
        let held_lead = self.lead_object(world, cursor.held);
        let fail = |trap| Fault {
            object: held_lead,
            at: None,
            trap,
        };
        self.remove_cursor_and_held(world, cursor)?;
        let refund = self.resale_value(world, held_lead)?;
        spend(world, cursor.budget_category, -refund).map_err(fail)?;
        self.remove_now(world, self.lead_object(world, cursor.cursor))?;
        let mut origin = cursor.origin.clone().unwrap_or(BuyPlacement {
            object: held_lead,
            guid: cursor.guid,
            position: cursor.position,
            facing: world
                .objects
                .get(&held_lead)
                .and_then(|o| o.data.get(DATA_FACING).copied())
                .unwrap_or(0)
                .rem_euclid(8),
            container: cursor.container,
            children: Vec::new(),
        });
        // `can_delete` already verified all surface slots on `held_lead` are
        // empty right now, so a `Sell` record owns only the sold object itself
        // and never any previously-present children that were moved or sold.
        origin.children.clear();
        Ok(Some(Some(BuyPurchase {
            kind: BuyActionKind::Sell,
            object: held_lead,
            guid: cursor.guid,
            position: origin.position,
            facing: origin.facing,
            container: origin.container,
            price: refund,
            budget_category: cursor.budget_category,
            placed: false,
            before: Some(origin),
            after: None,
        })))
    }

    /// Restores a single [`BuyPlacement`] and recursively restores all of its
    /// surface children (`0x48bfab`). Returns `Ok(Some(id))` if the root
    /// placement and every surface child succeeded, or `Ok(None)` if any was
    /// refused (rolling back any partial parent/sibling placement atomically).
    fn restore_placement(
        &self,
        world: &mut World,
        target: &BuyPlacement,
        allow_create: bool,
    ) -> Result<Option<u16>, Fault> {
        let [x, y, level] = target.position;
        if !on_lot([x, y]) || !inside_placement_bounds(world, [x, y]) {
            return Ok(None);
        }
        let snapshot = (!target.children.is_empty()).then(|| world.clone());
        let (id, created_fresh) = if world.objects.contains_key(&target.object) {
            (target.object, false)
        } else if allow_create {
            (self.create(world, target.guid)?, true)
        } else {
            return Ok(None);
        };
        // Native `0x48bfab` line `0x48bfcb`: invoke `UserPickup(0)` (`0x435751`
        // / `0x422ff5` -> OBJf 12 + take-off `0x435835` -> OBJf 10 `LEAVING`)
        // on the instance before rotating or testing placement.
        self.run_user_pickup(world, id)?;
        self.take_off(world, id, true)?;
        self.rotate_object_to(world, id, target.facing)?;
        let fits = match target.container {
            Some((cid, slot)) => {
                self.user_fits_in(world, id, cid, i32::from(slot), [x, y], level)?
            }
            None => {
                self.user_test_place_ground(world, id, [x, y], level)?
                    .is_ok()
                    || (self.move_objects
                        && self.user_test_place_ground_override(world, id, [x, y], level)?)
            }
        };
        if !fits {
            if created_fresh {
                self.remove_now(world, id)?;
            } else if allow_create {
                // Native `0x48c053` calls `0x48c0ea` when `0x48bfab` returns
                // `0`, invoking `USER_PICKUP` (OBJf 12) and `take_off` (OBJf 10
                // `LEAVING`) again before the refused record transfers to Undo.
                self.run_user_pickup(world, id)?;
                self.take_off(world, id, true)?;
            }
            return Ok(None);
        }
        let place = Place {
            on_lot: true,
            position: [x, y],
            container: target.container.map(|(cid, _)| cid),
            slot: target.container.map_or(-1, |(_, slot)| i32::from(slot)),
            level,
        };
        self.put(world, id, place, true)?;
        self.run_user_placement(world, id)?;
        for child in &target.children {
            if self
                .restore_placement(world, child, allow_create)?
                .is_none()
            {
                if let Some(saved) = snapshot {
                    *world = saved;
                }
                if allow_create && !created_fresh {
                    self.run_user_pickup(world, id)?;
                    self.take_off(world, id, true)?;
                }
                return Ok(None);
            }
        }
        Ok(Some(id))
    }

    /// Undoes a committed Buy Mode action (Buy, Move, or Sell; `0x48bd21`).
    ///
    /// - **Buy (`kind == Buy`):** runs `USER_PICKUP` + `take_off` (detaching
    ///   off-lot for Redo) and refunds `price`.
    /// - **Move (`kind == Move`):** restores the on-lot object (and any surface
    ///   children) to `before` via `0x48bfab` with no funds change.
    /// - **Sell (`kind == Sell`):** restores the detached object to `before`
    ///   via `0x48c053` → `0x48bfab` and re-deducts the refunded `price`.
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
        match purchase.kind {
            BuyActionKind::Buy => {
                if !purchase.placed
                    || !world
                        .objects
                        .get(&purchase.object)
                        .is_some_and(|o| on_lot([o.position[0], o.position[1]]))
                {
                    return Ok(false);
                }
                self.run_user_pickup(world, purchase.object)?;
                self.take_off(world, purchase.object, true)?;
                spend(world, purchase.budget_category, -purchase.price).map_err(fail)?;
                purchase.placed = false;
                Ok(true)
            }
            BuyActionKind::Move => {
                let Some(before) = purchase.before.clone() else {
                    return Ok(false);
                };
                if !all_placement_objects_on_lot(world, &before) {
                    return Ok(false);
                }
                let Some(id) = self.restore_placement(world, &before, false)? else {
                    return Ok(false);
                };
                purchase.object = id;
                purchase.position = before.position;
                purchase.facing = before.facing;
                purchase.container = before.container;
                purchase.placed = true;
                Ok(true)
            }
            BuyActionKind::Sell => {
                if purchase.placed
                    || (purchase.price > 0 && available_funds(world) < purchase.price)
                {
                    return Ok(false);
                }
                let target = purchase.before.clone().unwrap_or(BuyPlacement {
                    object: purchase.object,
                    guid: purchase.guid,
                    position: purchase.position,
                    facing: purchase.facing,
                    container: purchase.container,
                    children: Vec::new(),
                });
                if let Some(existing) = world.objects.get(&target.object)
                    && on_lot([existing.position[0], existing.position[1]])
                {
                    return Ok(false);
                }
                let before_restore = world.clone();
                let Some(id) = self.restore_placement(world, &target, true)? else {
                    return Ok(false);
                };
                if purchase.price > 0 && available_funds(world) < purchase.price {
                    *world = before_restore;
                    return Ok(false);
                }
                spend(world, purchase.budget_category, purchase.price).map_err(fail)?;
                purchase.object = id;
                purchase.placed = true;
                Ok(true)
            }
        }
    }

    /// Redoes a previously undone Buy Mode action (Buy, Move, or Sell;
    /// `0x48bdfe`).
    ///
    /// - **Buy (`kind == Buy`):** reuses the detached instance (or creates a
    ///   fallback), places it via `0x48bfab`, and re-deducts `price`.
    /// - **Move (`kind == Move`):** restores the on-lot object (and any surface
    ///   children) to `after` via `0x48bfab` with no funds change.
    /// - **Sell (`kind == Sell`):** detaches the object off-lot (`USER_PICKUP`
    ///   + `take_off`) and refunds `price`.
    pub fn buy_redo_purchase(
        &self,
        world: &mut World,
        purchase: &mut BuyPurchase,
    ) -> Result<bool, Fault> {
        let fail = |trap| Fault {
            object: purchase.object,
            at: None,
            trap,
        };
        match purchase.kind {
            BuyActionKind::Buy => {
                if purchase.placed
                    || (purchase.price > 0 && available_funds(world) < purchase.price)
                {
                    return Ok(false);
                }
                let target = purchase.after.clone().unwrap_or(BuyPlacement {
                    object: purchase.object,
                    guid: purchase.guid,
                    position: purchase.position,
                    facing: purchase.facing,
                    container: purchase.container,
                    children: Vec::new(),
                });
                if let Some(existing) = world.objects.get(&target.object)
                    && on_lot([existing.position[0], existing.position[1]])
                {
                    return Ok(false);
                }
                let before_restore = world.clone();
                let Some(id) = self.restore_placement(world, &target, true)? else {
                    return Ok(false);
                };
                if purchase.price > 0 && available_funds(world) < purchase.price {
                    *world = before_restore;
                    return Ok(false);
                }
                spend(world, purchase.budget_category, purchase.price).map_err(fail)?;
                purchase.object = id;
                purchase.placed = true;
                Ok(true)
            }
            BuyActionKind::Move => {
                let Some(after) = purchase.after.clone() else {
                    return Ok(false);
                };
                if !all_placement_objects_on_lot(world, &after) {
                    return Ok(false);
                }
                let Some(id) = self.restore_placement(world, &after, false)? else {
                    return Ok(false);
                };
                purchase.object = id;
                purchase.position = after.position;
                purchase.facing = after.facing;
                purchase.container = after.container;
                purchase.placed = true;
                Ok(true)
            }
            BuyActionKind::Sell => {
                if !purchase.placed
                    || !world
                        .objects
                        .get(&purchase.object)
                        .is_some_and(|o| on_lot([o.position[0], o.position[1]]))
                    || !self.can_delete(world, purchase.object)?
                {
                    return Ok(false);
                }
                self.run_user_pickup(world, purchase.object)?;
                self.take_off(world, purchase.object, true)?;
                spend(world, purchase.budget_category, -purchase.price).map_err(fail)?;
                purchase.placed = false;
                Ok(true)
            }
        }
    }

    /// Deletes any detached (off-lot) instances referenced by discarded undo
    /// or redo history records (`0x48c20e` / `0x48c258`), leaving on-lot
    /// objects and any object IDs still referenced by surviving history or an
    /// active cursor (`keep`) untouched.
    pub fn buy_purge_detached(
        &self,
        world: &mut World,
        purchases: &[BuyPurchase],
        keep: &BTreeSet<u16>,
    ) -> Result<(), Fault> {
        fn collect_children(placement: &BuyPlacement, ids: &mut Vec<u16>) {
            for child in &placement.children {
                collect_children(child, ids);
                ids.push(child.object);
            }
        }
        for purchase in purchases {
            // Native `0x48c20e` deletes only instances marked detached by
            // `0x48c0ea` (`Buy` undone or `Sell` committed/redone). A `Move`
            // record never owns a detached object for deletion.
            if purchase.kind == BuyActionKind::Move {
                continue;
            }
            let mut ids = Vec::new();
            if let Some(before) = &purchase.before {
                collect_children(before, &mut ids);
            }
            if let Some(after) = &purchase.after {
                collect_children(after, &mut ids);
            }
            ids.push(purchase.object);
            for id in ids {
                if keep.contains(&id) {
                    continue;
                }
                if world
                    .objects
                    .get(&id)
                    .is_some_and(|o| !on_lot([o.position[0], o.position[1]]))
                {
                    self.remove_now(world, id)?;
                }
            }
        }
        Ok(())
    }

    /// Deletes only `"Tile Cursor"` by object ID (when it still refers to a
    /// `TILE_CURSOR_GUID` object), leaving any other object untouched.
    pub fn buy_discard_tile_cursor_id(
        &self,
        world: &mut World,
        cursor_id: u16,
    ) -> Result<(), Fault> {
        let cur_lead = self.lead_object(world, cursor_id);
        if let Some(obj) = world.objects.get(&cur_lead)
            && obj.guid == TILE_CURSOR_GUID
        {
            if on_lot([obj.position[0], obj.position[1]]) {
                self.take_off(world, cur_lead, true)?;
            }
            self.remove_now(world, cur_lead)?;
        }
        Ok(())
    }

    /// Deletes only `"Tile Cursor"` for a stale existing-object cursor whose
    /// held object was already detached/refunded by an intervening Undo/Sell,
    /// without resurrecting the detached object back onto the lot.
    pub fn buy_discard_tile_cursor(
        &self,
        world: &mut World,
        cursor: &BuyCursor,
    ) -> Result<(), Fault> {
        self.buy_discard_tile_cursor_id(world, cursor.cursor)
    }

    /// Cancels an active Buy Mode cursor (`0x487191` / `0x487105`):
    /// - For a new unpaid catalog preview (`!cursor.is_existing()`): runs
    ///   unconditional `TakeOff(1)` on both the `"Tile Cursor"` and the
    ///   unplaced catalog object before deleting them without charging funds.
    /// - For a picked-up existing object (`cursor.is_existing()`): takes off
    ///   both, deletes ONLY the `"Tile Cursor"`, and restores the held object
    ///   (and any surface children) back to its original pre-pickup placement
    ///   via `0x48bfab` without charging funds.
    pub fn buy_cancel(&self, world: &mut World, cursor: &BuyCursor) -> Result<(), Fault> {
        self.take_off_cursor_and_held_unconditional(world, cursor)?;
        self.remove_now(world, self.lead_object(world, cursor.cursor))?;
        if let Some(origin) = &cursor.origin {
            if self.restore_placement(world, origin, false)?.is_none() {
                return Err(Fault {
                    object: cursor.held,
                    at: None,
                    trap: Trap::Invalid(
                        "cannot restore picked-up object to its original placement".into(),
                    ),
                });
            }
        } else {
            self.remove_now(world, self.lead_object(world, cursor.held))?;
        }
        Ok(())
    }

    /// Creates one `"Tile Cursor"` (`0x00000437`) per tile of `held`'s group
    /// (`0x4215e6` → `0x421afb` → `0x42231d`) and returns the cursor tile
    /// corresponding to `held` itself (so grabbing a non-lead multi-tile part
    /// pivots around that tile).
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
        let mut cursor_for_held = None;
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
            if part_id == held {
                cursor_for_held = Some(id);
            }
        }
        cursor_for_held
            .or(cursor_lead)
            .ok_or_else(|| fail(held, Trap::Invalid("no tile cursor created".into())))
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
        if !self.move_objects {
            let a = i8::try_from(pos[0] >> 4).unwrap_or(-1);
            let b = i8::try_from(pos[1] >> 4).unwrap_or(-1);
            if (crate::placement::tile_flags(world, a, b, 1).map_err(fail)? & 0x20) != 0 {
                return Ok(false);
            }
        }
        if !self.fits_in(world, id, container, slot, pos, level)? {
            return Ok(false);
        }
        if (pos[0] & 0x0f) != 8 || (pos[1] & 0x0f) != 8 {
            return Ok(false);
        }
        if self.move_objects {
            return Ok(true);
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
