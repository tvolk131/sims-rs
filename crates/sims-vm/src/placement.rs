//! Where objects stand: taking an object off the lot and putting it on a
//! tile or into another object.
//!
//! The lot keeps three things in step:
//!
//! - **The object grid** of each level (lot arrays 3 and 103): for each
//!   tile, the ID of the first object standing on it.
//! - **Each object's slots.** Slot 0 names the next object on the same
//!   tile, or in the same slot of the same container. The others name the
//!   objects it holds.
//! - **Each object's link words.** Data word 26 is the object before it:
//!   the one whose slot names it. Data word 14 is the number of that slot,
//!   or -1 for the first object of a tile. Data words 2 and 3 are the
//!   container and its slot as scripts count them, one less than the slot
//!   number, or 0 and -1.
//!
//! [`Interpreter::take_off`] is the original's "remove from lot" (0x433125)
//! and [`Interpreter::put`] its "set object" (0x433340). A multi-tile
//! object does both for every tile of its group (0x422d86, 0x422c2f). Both
//! run the object's callbacks: function 10 before it leaves, function 9
//! once it stands, function 6 when the walls around it changed, and
//! function 9 of whatever it sits in or beside. [`move_tree`] is the part
//! that writes positions (0x433e50).
//!
//! Ported from the old prototype's `Operation` and `Group`
//! (legacy/crates/sims-sim/src/lot_placement.rs), `Links` and `Cursor`
//! (legacy/crates/sims-sim/src/lot_links.rs), `Adjacency`
//! (legacy/crates/sims-sim/src/lot_adjacency.rs), `plan_position`
//! (legacy/crates/sims-sim/src/lot_loading.rs), `footprint`
//! (legacy/crates/sims-sim/src/spatial.rs) and `placement` and
//! `update_offsets` (legacy/crates/sims-vm/src/multitile.rs). The
//! prototype compared each with the original's code run alone.
//! `docs/decisions/` says what a trace of the running game confirms.
//!
//! Not modeled, being presentation: the redrawing an object asks for as it
//! moves, its shadow, and the lighting around a light source.

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::load::INDICATORS;
use crate::program::DEFINITION_WORDS;
use crate::visits::function;
use sims_sim::World;
use sims_sim::animator::{Animator, HeldPose, Layer};
use std::collections::BTreeSet;

mod dynamic;
mod find;
pub(crate) use find::Rules;
pub(crate) use find::{
    Refusal, closed_directions, floor, pool_byte, pool_tile, segments, tile_flags,
};

/// A person's own tick while its thread runs: the person, and the
/// animator refresh its taking itself off the lot left pending
/// ([`Interpreter::take_self_off`]).
#[derive(Debug)]
pub(crate) struct Refreshing {
    pub person: u16,
    pub pending: Option<Pending>,
}

/// A person's animator refresh at its taking itself off the lot, worked
/// out then (`Interpreter::refresh_at_take_off`): the animator and base
/// layer the take-off left, and those the refresh would have left
/// instead. The refresh writes nothing else.
#[derive(Debug)]
pub(crate) struct Pending {
    pub left: Kept,
    pub refreshed: Kept,
}

/// A person's animator, base layer, the bones its skin has moved
/// ([`World::translated_bones`]) and the pose the load left in it
/// ([`World::held_poses`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Kept {
    pub animator: Option<Animator>,
    pub base: Option<Layer>,
    pub translated: Option<BTreeSet<String>>,
    pub held: Option<HeldPose>,
}

impl Kept {
    pub(crate) fn of(world: &World, id: u16) -> Self {
        Self {
            animator: world.animators.get(&id).cloned(),
            base: world.base_layers.get(&id).cloned(),
            translated: world.translated_bones.get(&id).cloned(),
            held: world.held_poses.get(&id).cloned(),
        }
    }

    /// Puts them back as person `id`'s.
    pub(crate) fn restore(&self, world: &mut World, id: u16) {
        match &self.animator {
            Some(a) => world.animators.insert(id, a.clone()),
            None => world.animators.remove(&id),
        };
        match &self.base {
            Some(layer) => world.base_layers.insert(id, layer.clone()),
            None => world.base_layers.remove(&id),
        };
        match &self.translated {
            Some(bones) => world.translated_bones.insert(id, bones.clone()),
            None => world.translated_bones.remove(&id),
        };
        match &self.held {
            Some(held) => world.held_poses.insert(id, held.clone()),
            None => world.held_poses.remove(&id),
        };
    }
}

/// What a person's animator refresh does with its head slot (0x58dae5 to
/// 0x58dc55): at its taking itself off the lot when the house view drew
/// the person ([`Interpreter::take_self_off`]), and always at its
/// deselection (`Interpreter::deselect`), where the marker has just left
/// the slot. The refresh reads the
/// person's container entry at begin + 0x48 when there are more than two
/// (our slot 2, the head's attachment; it holds the marker over the
/// selected Sim, GUID 0x2F726FFF). Unless the entry names no bone (+0x20)
/// or its object (+0x10) is not found, it takes the object's graphic
/// (virtual function 0x2c; 0x4215c3 for an ordinary object, +0x120), sets
/// the graphic's transform to the bone's place in the person's frame
/// (0x5990e0, 0x462aab; the graphic's virtual functions 0xac, 0x64 with
/// 1 and 0xb8), sets the graphic's +0x100 to the person's motive 3
/// truncated (0x45d117 with 3, +0x798; 0x636128 is the C runtime's float
/// to integer), and calls the object's virtual function 0x78 with (0, 0),
/// which for an ordinary object (vtable 0x666d34) is 0x51d372, a bare
/// `ret 4`.
///
/// So for an ordinary object it writes only the graphic, which nothing
/// the simulation keeps holds. The animator's tick, which follows in the
/// same tick, makes the same calls on the same graphic in its tail
/// (0x58f908 to 0x58fa7f, ending with the graphic's virtual function 0x50
/// instead), and that redraw is left out as presentation (`crate::animator`);
/// this one is too, which is as unverified. A person or a tile of a group
/// in the slot has another class (0x667378, 0x666c20), whose functions
/// there are not read, so it fails.
pub(crate) fn head_slot_redraw(world: &World, id: u16) -> Result<(), Trap> {
    let person = world.objects.get(&id).ok_or_else(missing)?;
    let Some(slot) = person.slots.get(2).filter(|slot| slot.object > 0) else {
        return Ok(());
    };
    let Some(held) = world.objects.get(&(slot.object as u16)) else {
        return Ok(());
    };
    if held.person.is_some() || held.multipart.is_some() {
        return Err(Trap::Unsupported(format!(
            "person {id}'s animator refresh (0x58da12, at its taking itself off the lot or its \
             deselection) redraws {} {} in its head slot through that class's functions \
             (0x58dc08 to 0x58dc52), which are not read",
            if held.person.is_some() {
                "the person"
            } else {
                "the group tile"
            },
            slot.object
        )));
    }
    Ok(())
}

/// Whether a primitive reads nothing of any animator, so it may run while
/// an animator refresh is pending ([`Interpreter::take_self_off`]): Sleep,
/// Expression, Change Suit/Accessory on a suit (operand 1 = 1, which
/// writes person words 8 and 21 only), Set Motive Change and Set To Next.
pub(crate) fn reads_no_animator(opcode: u16, operands: &[u8; 8]) -> bool {
    match opcode {
        0 | 2 | 29 | 31 => true,
        6 => operands[1] == 1,
        _ => false,
    }
}

/// The side of the lot's arrays.
const FULL: i32 = 64;

/// The type the interface keeps a pooled list of (0x448307).
const POOLED: u32 = 0xE933_7020;

/// A container entry in a definition's SLOT list.
const CONTAINER_SLOT: u16 = 0;
/// The object grids of the two levels.
const GRIDS: [u16; 2] = [3, 103];
/// The tile flags of the two levels.
const FLAGS: [u16; 2] = [8, 108];
/// The tile flag a door or window sets on the tile it stands on (0x4330d7).
const PORTAL_TILE: u8 = 8;

/// Data words.
mod data {
    pub const FACING: usize = 1;
    pub const CONTAINER: usize = 2;
    pub const CONTAINER_SLOT: usize = 3;
    pub const WALLS: usize = 5;
    pub const LIGHTS_ROOM: usize = 7;
    pub const FLAGS: usize = 8;
    pub const SLOT: usize = 14;
    /// The eighths a group turns by at each step (object +0x78).
    pub const TURN_STEP: usize = 23;
    pub const BEFORE: usize = 26;
    pub const ROOM: usize = 29;
    pub const FLAGS_2: usize = 40;
}

/// Definition words.
mod word {
    pub const TYPE: usize = 9;
    pub const GROUP: usize = 10;
    pub const SIZE: usize = 0x86 / 2;
    pub const PORTAL: usize = 0x8a / 2;
    pub const INSETS: usize = 0x9a / 2;
}

/// Callbacks placement asks about without running them here.
const SEAT: usize = 26;
const SEAT_CHANGED: usize = 25;
const WALLS_CHANGED: usize = 6;

/// Where an object is, or is to go (0x4325b5, 0x432844).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Place {
    /// Whether the position is on the lot's tiles.
    pub on_lot: bool,
    /// The first two coordinates of the position.
    pub position: [i32; 2],
    pub container: Option<u16>,
    /// The container's slot as scripts count it, or -1.
    pub slot: i32,
    pub level: i32,
}

const NOWHERE: Place = Place {
    on_lot: false,
    position: [0, 0],
    container: None,
    slot: -1,
    level: 1,
};

/// Off the lot.
const OFF_LOT: [i32; 3] = [-16, -16, 1];

/// The type of the star marker the registry puts over a famous Sim
/// (0x4481eb).
const STAR: u32 = 0xe933_7020;

fn missing() -> Trap {
    Trap::native(10, "missing object")
}

/// The object an ID names, if it is registered.
fn registered(world: &World, id: i16) -> Option<u16> {
    u16::try_from(id)
        .ok()
        .filter(|id| *id > 0 && world.objects.contains_key(id))
}

fn datum(world: &World, id: u16, index: usize) -> Result<i16, Trap> {
    world
        .objects
        .get(&id)
        .and_then(|o| o.data.get(index))
        .copied()
        .ok_or_else(missing)
}

fn set_datum(world: &mut World, id: u16, index: usize, value: i16) -> Result<(), Trap> {
    *world
        .objects
        .get_mut(&id)
        .and_then(|o| o.data.get_mut(index))
        .ok_or_else(missing)? = value;
    Ok(())
}

fn position(world: &World, id: u16) -> Result<[i32; 3], Trap> {
    world
        .objects
        .get(&id)
        .map(|o| o.position)
        .ok_or_else(missing)
}

/// A position's tile and level, as the original narrows them to bytes.
fn tile(position: [i32; 3]) -> [i8; 3] {
    World::tile(position)
}

/// Whether a position is on a tile inside the border of the lot's arrays.
/// **Unverified** whether the original tests against the arrays' 64 tiles,
/// as the old prototype does, or against the lot's size: no object of the
/// house the trace loads stands between the two.
pub(crate) fn on_lot(position: [i32; 2]) -> bool {
    let inside = |n: i32| {
        let n = i32::from((n >> 4) as i8);
        n > 0 && n < FULL - 1
    };
    inside(position[0]) && inside(position[1])
}

/// Whether a point is off the lot's inner tiles (0x422632): either
/// coordinate's tile, as a signed byte, below 1 or not below the lot's
/// size less 1.
pub(crate) fn off_lot(world: &World, point: [i32; 2]) -> bool {
    let size = i32::from(world.size);
    let ok = |c: i32| {
        let n = i32::from((c >> 4) as i8);
        n >= 1 && n < size - 1
    };
    !ok(point[0]) || !ok(point[1])
}

/// Whether a tile is inside the lot's own border: each coordinate at
/// least 1 and below the lot's size less 1 (the lot's +0x10, as the
/// passes over the seats and walls beside an object and the door mark
/// test it: 0x43363f to 0x433668, 0x4336c8 to 0x4336eb, 0x435000 to
/// 0x435028, 0x4330da to 0x433100).
fn tile_inside_lot(world: &World, tile: [i8; 3]) -> bool {
    find::inside(world, tile[0], tile[1])
}

/// The grid array and the byte offset of a tile's cell.
fn grid_cell(position: [i32; 2], level: i32) -> Option<(u16, usize)> {
    let column = i32::from((position[0] >> 4) as i8);
    let row = i32::from((position[1] >> 4) as i8);
    let level = usize::try_from(level - 1).ok().filter(|&l| l < 2)?;
    ((0..FULL).contains(&column) && (0..FULL).contains(&row))
        .then(|| (GRIDS[level], ((column * FULL + row) * 2) as usize))
}

fn read_head(world: &World, position: [i32; 2], level: i32) -> Option<u16> {
    let (array, cell) = grid_cell(position, level)?;
    let bytes = world.arrays.get(&array)?.data.get(cell..cell + 2)?;
    registered(world, i16::from_le_bytes([bytes[0], bytes[1]]))
}

fn write_head(world: &mut World, position: [i32; 2], level: i32, id: u16) -> Result<(), Trap> {
    let Some((array, cell)) = grid_cell(position, level) else {
        return Ok(());
    };
    let bytes = world
        .arrays
        .get_mut(&array)
        .and_then(|a| a.data.get_mut(cell..cell + 2))
        .ok_or_else(|| Trap::Invalid(format!("the lot has no object grid (array {array})")))?;
    bytes.copy_from_slice(&id.to_le_bytes());
    Ok(())
}

/// The first object on a tile.
pub(crate) fn head(world: &World, tile: [i8; 3]) -> Option<u16> {
    read_head(
        world,
        [i32::from(tile[0]) * 16, i32::from(tile[1]) * 16],
        i32::from(tile[2]),
    )
}

/// The object a slot names. Slot 0 is the next object on the tile.
fn slot(world: &World, parent: u16, slot: i32) -> Result<Option<u16>, Trap> {
    let object = world.objects.get(&parent).ok_or_else(missing)?;
    let entry = usize::try_from(slot)
        .ok()
        .and_then(|i| object.slots.get(i))
        .ok_or_else(|| Trap::Invalid(format!("object {parent} has no slot {slot}")))?;
    Ok(registered(world, entry.object))
}

fn write_slot(world: &mut World, parent: u16, slot: i32, id: u16) -> Result<(), Trap> {
    let object = world.objects.get_mut(&parent).ok_or_else(missing)?;
    let entry = usize::try_from(slot)
        .ok()
        .and_then(|i| object.slots.get_mut(i))
        .ok_or_else(|| Trap::Invalid(format!("object {parent} has no slot {slot}")))?;
    entry.object = id as i16;
    Ok(())
}

/// Where an object is by its link words, slot numbers as stored (0x432844).
fn capture_raw(world: &World, id: u16) -> Result<Place, Trap> {
    let p = position(world, id)?;
    Ok(Place {
        on_lot: on_lot([p[0], p[1]]),
        position: [p[0], p[1]],
        container: registered(world, datum(world, id, data::BEFORE)?),
        slot: i32::from(datum(world, id, data::SLOT)?),
        level: p[2],
    })
}

/// Where an object is, as [`Interpreter::put`] takes it (0x4325b5). An
/// object that follows another on its tile is where the first of them is:
/// in that one's container, or on the tile.
pub(crate) fn capture(world: &mut World, id: u16) -> Result<Place, Trap> {
    let p = position(world, id)?;
    let on_lot = on_lot([p[0], p[1]]);
    let mut root = id;
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(root) {
            return Err(Trap::Invalid(format!(
                "the objects before object {id} on its tile form a loop"
            )));
        }
        if datum(world, root, data::SLOT)? != 0 {
            break;
        }
        let Some(before) = registered(world, datum(world, root, data::BEFORE)?) else {
            break;
        };
        root = before;
    }
    let physical = datum(world, root, data::SLOT)?;
    let container = registered(world, datum(world, root, data::BEFORE)?);
    if physical == 0 {
        // A slot 0 with nothing before it is put right.
        set_datum(world, root, data::SLOT, -1)?;
    }
    Ok(Place {
        on_lot,
        position: if on_lot { [p[0], p[1]] } else { [-16, -16] },
        container,
        slot: if physical == -1 {
            -1
        } else {
            i32::from(physical) - 1
        },
        level: p[2],
    })
}

/// Takes an object out of the slot or grid cell that names it.
fn unlink(world: &mut World, id: u16) -> Result<(), Trap> {
    let old = capture_raw(world, id)?;
    if let Some(parent) = old.container {
        write_slot(world, parent, old.slot, 0)?;
    } else if old.on_lot {
        write_head(world, old.position, old.level, 0)?;
    }
    set_datum(world, id, data::SLOT, -1)?;
    set_datum(world, id, data::BEFORE, 0)
}

/// Writes an object into a slot or a grid cell. `place.slot` is a slot
/// number as stored.
fn attach(world: &mut World, id: u16, place: Place) -> Result<(), Trap> {
    if !place.on_lot {
        unlink(world, id)?;
    }
    if let Some(parent) = place.container {
        write_slot(world, parent, place.slot, id)?;
        set_datum(world, id, data::SLOT, place.slot as i16)?;
        set_datum(world, id, data::BEFORE, parent as i16)?;
    } else {
        if !place.on_lot {
            return Ok(());
        }
        write_head(world, place.position, place.level, id)?;
        set_datum(world, id, data::SLOT, -1)?;
        set_datum(world, id, data::BEFORE, 0)?;
    }
    world.objects.get_mut(&id).ok_or_else(missing)?.position[2] = place.level;
    Ok(())
}

/// Moves an object's links to a place, or off the lot (0x4329c1). Taken
/// off, the object that followed it takes its place. Put on, it goes in
/// front of whatever was there. Positions and data words 2 and 3 are the
/// caller's.
fn relink(world: &mut World, id: u16, place: Place) -> Result<(), Trap> {
    if !place.on_lot {
        let old = capture_raw(world, id)?;
        unlink(world, id)?;
        if let Some(next) = slot(world, id, 0)? {
            unlink(world, next)?;
            attach(world, next, old)?;
        }
        return Ok(());
    }
    let raw = Place {
        slot: if place.slot == -1 { -1 } else { place.slot + 1 },
        ..place
    };
    let existing = match raw.container {
        Some(parent) => slot(world, parent, raw.slot)?,
        None => read_head(world, raw.position, raw.level),
    };
    if existing == Some(id) {
        return Ok(());
    }
    if let Some(existing) = existing {
        unlink(world, existing)?;
    }
    attach(world, id, raw)?;
    if let Some(existing) = existing {
        let level = position(world, id)?[2];
        attach(
            world,
            existing,
            Place {
                on_lot: true,
                position: raw.position,
                container: Some(id),
                slot: 0,
                level,
            },
        )?;
    }
    Ok(())
}

/// Walks an object and everything linked below it, slot by slot, slot 0
/// included (0x43f364, 0x43f379). The next object is read after the
/// caller has dealt with the current one.
pub(crate) struct Cursor {
    root: u16,
    current: Option<u16>,
}

impl Cursor {
    /// A walk of the objects on a tile, as 0x43f331 starts it from a
    /// three-byte location (column, row, level; 0x432500): none for a tile
    /// outside the lot's size (lot +0x10) or off levels 1 and 2.
    pub(crate) fn on_tile(world: &World, at: [i8; 3]) -> Cursor {
        let size = i32::from(world.size);
        let inside = |n: i8| n >= 0 && i32::from(n) < size;
        let root = if inside(at[0]) && inside(at[1]) && (1..=2).contains(&at[2]) {
            head(world, at)
        } else {
            None
        };
        Cursor {
            root: root.unwrap_or(0),
            current: root,
        }
    }

    /// The object the walk is at; None when it has ended.
    pub(crate) fn current(&self) -> Option<u16> {
        self.current
    }

    /// Moves to the next object (0x43f379), reading the links as they are
    /// now.
    pub(crate) fn advance(&mut self, world: &World) -> Result<(), Trap> {
        let Some(mut node) = self.current else {
            return Ok(());
        };
        let mut from = 0;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(node) {
                return Err(Trap::Invalid("the objects on a tile link in a loop".into()));
            }
            let object = world.objects.get(&node).ok_or_else(missing)?;
            for entry in object.slots.iter().skip(from) {
                if let Some(child) = registered(world, entry.object) {
                    self.current = Some(child);
                    return Ok(());
                }
            }
            self.current = None;
            if node == self.root {
                return Ok(());
            }
            let before = registered(world, datum(world, node, data::BEFORE)?)
                .ok_or_else(|| Trap::Invalid(format!("object {node} is linked from no object")))?;
            from = usize::try_from(i32::from(datum(world, node, data::SLOT)?) + 1)
                .map_err(|_| Trap::Invalid(format!("object {node} is linked from no slot")))?;
            node = before;
        }
    }
}

/// Whether an object stands on the lot (0x432595).
pub(crate) fn stands_on_lot(world: &mut World, id: u16) -> Result<bool, Trap> {
    Ok(capture(world, id)?.on_lot)
}

/// The objects standing on a tile, with everything they hold, in the
/// order the lot links them (0x43f331, 0x43f379). The tile is given as
/// the lot's rooms list it: column, row and level.
pub(crate) fn standing_on(world: &World, at: [u8; 3]) -> Result<Vec<u16>, Trap> {
    let Some(root) = head(world, [at[0] as i8, at[1] as i8, at[2] as i8]) else {
        return Ok(Vec::new());
    };
    let mut cursor = Cursor {
        root,
        current: Some(root),
    };
    let mut chain = Vec::new();
    while let Some(current) = cursor.current {
        if chain.contains(&current) {
            return Err(Trap::Invalid("the objects on a tile link in a loop".into()));
        }
        chain.push(current);
        cursor.advance(world)?;
    }
    Ok(chain)
}

/// The first object on a tile inside the lot's size, for scripts (Set To
/// Next mode 9).
pub(crate) fn first_on(world: &World, at: [i32; 2], level: i8) -> Option<u16> {
    let size = i32::from(world.size);
    if !(0..size.min(FULL)).contains(&at[0]) || !(0..size.min(FULL)).contains(&at[1]) {
        return None;
    }
    head(world, [at[0] as i8, at[1] as i8, level])
}

/// The first object on a point's tile that is not passed over (0x43244d),
/// the point given as the lot links objects (the first coordinate,
/// object +0xf8, first). The tile's first object (0x43248a, 0x432500:
/// none outside the lot's size or off levels 1 and 2), then each next
/// one on the tile (slot 0, 0x432823), is passed over when its data word
/// 8 has bit 2 set and bit 1 clear, or bit 2 clear and bit 4 set.
pub(crate) fn first_kept_on(world: &World, at: [i32; 2], level: i32) -> Result<Option<u16>, Trap> {
    let size = i32::from(world.size);
    let column = i32::from((at[0] >> 4) as i8);
    let row = i32::from((at[1] >> 4) as i8);
    if !(0..size).contains(&column) || !(0..size).contains(&row) {
        return Ok(None);
    }
    let mut next = read_head(world, at, level);
    let mut seen = BTreeSet::new();
    while let Some(id) = next {
        if !seen.insert(id) {
            return Err(Trap::Invalid("the objects on a tile link in a loop".into()));
        }
        let flags = datum(world, id, data::FLAGS)?;
        let passed = if flags & 4 != 0 {
            flags & 2 == 0
        } else {
            flags & 0x10 != 0
        };
        if !passed {
            return Ok(Some(id));
        }
        next = slot(world, id, 0)?;
    }
    Ok(None)
}

/// The objects on the tile an object stands on, with everything they
/// hold, in the order the lot links them, for scripts (Set To Next mode
/// 8). None if the object is linked from no tile inside the lot's size.
pub(crate) fn on_tile(world: &World, id: u16) -> Result<Option<Vec<u16>>, Trap> {
    let mut root = id;
    let mut seen = BTreeSet::new();
    while let Some(before) = registered(world, datum(world, root, data::BEFORE)?) {
        if !seen.insert(root) {
            return Err(Trap::Invalid("the objects on a tile link in a loop".into()));
        }
        root = before;
    }
    let at = tile(position(world, root)?);
    if first_on(world, [i32::from(at[0]), i32::from(at[1])], at[2]) != Some(root) {
        return Ok(None);
    }
    let mut cursor = Cursor {
        root,
        current: Some(root),
    };
    let mut chain = Vec::new();
    while let Some(current) = cursor.current {
        if chain.contains(&current) {
            return Err(Trap::Invalid("the objects on a tile link in a loop".into()));
        }
        chain.push(current);
        cursor.advance(world)?;
    }
    Ok(Some(chain))
}

/// An object's footprint as the lot stores it (0x433d16, 0x43519f): the
/// least first and second coordinates, then the greatest. A person is 6
/// wide, or nothing when person data word 68 or 84 has bit 0 set. Any
/// other object is as wide as its definition says, 16 if that is 0. Each
/// side then moves in by the definition's inset for it, turned to the
/// object's facing.
fn footprint(
    position: [i32; 3],
    words: &[i16; DEFINITION_WORDS],
    facing: i16,
    person: Option<&[i16]>,
) -> Result<[i32; 4], Trap> {
    if !(0..=7).contains(&facing) {
        return Err(Trap::Unsupported(format!(
            "the footprint of an object facing {facing}"
        )));
    }
    let field = |index: usize| -> Result<u16, Trap> {
        match person {
            None => Ok(0),
            Some(data) => data
                .get(index)
                .map(|&word| word as u16)
                .ok_or_else(|| Trap::Invalid(format!("a person has no data word {index}"))),
        }
    };
    let width = if person.is_some() {
        if (field(68)? | field(84)?) & 1 != 0 {
            0
        } else {
            6
        }
    } else if words[word::SIZE] == 0 {
        16
    } else {
        i32::from(words[word::SIZE])
    };
    let half = width / 2;
    // The old prototype's order: greatest second coordinate, greatest
    // first, least second, least first.
    let (a, b) = (position[1], position[0]);
    let mut bounds = [a + half, b + half, a - half, b - half];
    let extension = if person.is_some() && facing & 1 == 0 {
        field(73)?
    } else {
        0
    };
    let mask = words[word::INSETS] as u16;
    let inset: [i32; 4] = std::array::from_fn(|i| {
        i32::from((mask >> (i * 4)) & 15) - i32::from((extension >> (i * 4)) & 15)
    });
    let i = usize::from((8 - facing as u8) / 2) % 4;
    bounds[2] += inset[i];
    bounds[1] -= inset[(i + 1) % 4];
    bounds[0] -= inset[(i + 2) % 4];
    bounds[3] += inset[(i + 3) % 4];
    Ok([bounds[3], bounds[2], bounds[1], bounds[0]])
}

impl Interpreter<'_> {
    /// The footprint an object that is not a person gets off the lot,
    /// as taking it off the lot works it out (`move_tree`).
    pub(crate) fn off_lot_footprint(&self, world: &World, id: u16) -> Result<[i32; 4], Trap> {
        let words = self.words(world, id)?;
        footprint(OFF_LOT, words, datum(world, id, data::FACING)?, None)
    }

    /// An object's footprint worked out again where it stands: 0x433d16
    /// with the object's own position (+0xf4), whose result the caller
    /// stores at +0x100. `person` stands in for the person's data words,
    /// so a caller can work it out as it will be after changing them; it
    /// is None for an object that is not a person.
    pub(crate) fn standing_footprint(
        &self,
        world: &World,
        id: u16,
        person: Option<&[i16]>,
    ) -> Result<[i32; 4], Trap> {
        let words = self.words(world, id)?;
        let object = world.objects.get(&id).ok_or_else(missing)?;
        if object.person.is_some() != person.is_some() {
            return Err(Trap::Invalid(
                "person data given for an object that is not a person, or none for one".into(),
            ));
        }
        if person.is_some() != (words[word::TYPE] == 2) {
            return Err(Trap::Invalid(
                "an object and its definition disagree on whether it is a person".into(),
            ));
        }
        footprint(
            object.position,
            words,
            datum(world, id, data::FACING)?,
            person,
        )
    }

    /// The definition an object stands for (0x45728d): its own, or for a
    /// tile of a group the group's master ([`crate::Program::master_definition`]).
    /// A group with no master fails, naming `site`, since the original
    /// reads through a null pointer there.
    pub(crate) fn master_words(
        &self,
        world: &World,
        id: u16,
        site: &str,
    ) -> Result<&[i16; DEFINITION_WORDS], Trap> {
        let object = world.objects.get(&id).ok_or_else(missing)?;
        let definition = crate::interpreter::object_definition(self.catalog, world, object.guid)
            .ok_or_else(|| Trap::Invalid(format!("object {id} has no definition")))?;
        self.program
            .master_definition(definition.resource, definition.chunk)
            .map_err(Trap::Invalid)?
            .ok_or_else(|| {
                Trap::Unsupported(format!(
                    "object {id}'s group has no master definition in its file, and {site} \
                     reads through a null pointer (0x45728d)"
                ))
            })
    }

    /// An object's footprint were it at `position` facing `facing`
    /// (0x433d16 with a position given, as Snap's wall test calls it with
    /// its facing set for the call, 0x455263).
    pub(crate) fn footprint_at(
        &self,
        world: &World,
        id: u16,
        position: [i32; 3],
        facing: i16,
    ) -> Result<[i32; 4], Trap> {
        let words = self.words(world, id)?;
        let object = world.objects.get(&id).ok_or_else(missing)?;
        let person = object.person.as_ref().map(|p| p.data.as_slice());
        if person.is_some() != (words[word::TYPE] == 2) {
            return Err(Trap::Invalid(
                "an object and its definition disagree on whether it is a person".into(),
            ));
        }
        footprint(position, words, facing, person)
    }

    pub(crate) fn words(&self, world: &World, id: u16) -> Result<&[i16; DEFINITION_WORDS], Trap> {
        let object = world.objects.get(&id).ok_or_else(missing)?;
        let definition = crate::interpreter::object_definition(self.catalog, world, object.guid)
            .ok_or_else(|| Trap::Invalid("object has no definition".into()))?;
        self.program
            .definition(definition.resource, definition.chunk)
            .ok_or_else(|| Trap::Invalid("the program holds no definition for an object".into()))
    }

    /// Whether an object's callback table names a tree for a function.
    /// An object's type: word 9 of its definition.
    pub(crate) fn kind(&self, world: &World, id: u16) -> Result<i16, Trap> {
        Ok(self.words(world, id)?[word::TYPE])
    }

    pub(crate) fn has_function(&self, world: &World, id: u16, function: usize) -> bool {
        self.function_tree(world, id, function) != 0
    }

    /// The action tree an object's definition names for a function (the
    /// first word of its callback entry, 0x457c38, 0x455fa9), or 0 for
    /// none, a function past the table's end, or a missing object.
    pub(crate) fn function_tree(&self, world: &World, id: u16, function: usize) -> u16 {
        world
            .objects
            .get(&id)
            .and_then(|object| {
                crate::interpreter::object_definition(self.catalog, world, object.guid)
            })
            .and_then(|definition| {
                self.program
                    .libraries
                    .get(&definition.resource)
                    .and_then(|l| l.callbacks.get(&definition.chunk))
            })
            .and_then(|table| table.get(function))
            .map_or(0, |f| f.action)
    }

    /// Moves an object and everything it holds to a position (0x433e50).
    /// Each gets the position, its footprint there, and the room of the
    /// position in data word 29. When that changes the room of an object
    /// that lights its room (data word 7), the old and the new room are
    /// marked as not counted.
    ///
    /// The objects it holds are the ones its slots after slot 0 name, and
    /// theirs. An ID that names no object is skipped.
    pub(crate) fn move_tree(&self, world: &mut World, root: u16, to: [i32; 3]) -> Result<(), Trap> {
        let room = world
            .room_at(to)
            .map_err(|error| Trap::Unsupported(error.to_string()))?;
        let mut pending = vec![root];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                return Err(Trap::Invalid(format!(
                    "the objects object {root} holds form a loop"
                )));
            }
            let words = self.words(world, id)?;
            let object = world.objects.get(&id).ok_or_else(missing)?;
            let person = object.person.as_ref().map(|p| p.data.as_slice());
            if person.is_some() != (words[word::TYPE] == 2) {
                return Err(Trap::Invalid(
                    "an object and its definition disagree on whether it is a person".into(),
                ));
            }
            let bounds = footprint(to, words, datum(world, id, data::FACING)?, person)?;
            let old = datum(world, id, data::ROOM)?;
            let lights = datum(world, id, data::LIGHTS_ROOM)? != 0;
            pending.extend(
                object
                    .slots
                    .iter()
                    .skip(1)
                    .rev()
                    .filter_map(|entry| registered(world, entry.object)),
            );
            if old != room && lights {
                world.mark_room(old);
                world.mark_room(room);
            }
            let object = world.objects.get_mut(&id).ok_or_else(missing)?;
            object.position = to;
            object.footprint = bounds;
            object.data[data::ROOM] = room;
        }
        Ok(())
    }

    /// Tells the objects on a tile whether a seat faces them (the
    /// original's pass over a tile after something beside it moved). A
    /// seat is an object with function 26 that is the first solid object
    /// on a tile next to this one and faces this one. Every object on the
    /// tile, and everything those hold, gets flag 0x100 of data word 8 set
    /// or cleared, and one whose flag changed and that has function 25 is
    /// asked function 9.
    fn seats_beside(&self, world: &mut World, at: [i8; 3]) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: 0,
            at: None,
            trap,
        };
        if !tile_inside_lot(world, at) {
            return Ok(());
        }
        let Some(root) = head(world, at) else {
            return Ok(());
        };
        let mut seat = false;
        for direction in [0usize, 2, 4, 6] {
            let other = forward(at, direction as i16).map_err(fail)?;
            if !tile_inside_lot(world, other) {
                continue;
            }
            let mut current = head(world, other);
            let mut seen = BTreeSet::new();
            while let Some(id) = current {
                if !seen.insert(id) {
                    return Err(fail(Trap::Invalid(
                        "the objects on a tile link in a loop".into(),
                    )));
                }
                if datum(world, id, data::FLAGS).map_err(fail)? & 4 == 0 {
                    break;
                }
                current = slot(world, id, 0).map_err(fail)?;
            }
            if let Some(id) = current
                && self.has_function(world, id, SEAT)
                && datum(world, id, data::FACING).map_err(fail)? as u16
                    == ((direction as u16 + 4) & 7)
            {
                seat = true;
                break;
            }
        }
        let mut cursor = Cursor {
            root,
            current: Some(root),
        };
        let mut seen = BTreeSet::new();
        while let Some(id) = cursor.current {
            if !seen.insert(id) {
                return Err(fail(Trap::Invalid(
                    "the objects on a tile link in a loop".into(),
                )));
            }
            let old = datum(world, id, data::FLAGS).map_err(fail)? as u16;
            let flags = if seat { old | 0x100 } else { old & !0x100 };
            set_datum(world, id, data::FLAGS, flags as i16).map_err(fail)?;
            if old != flags && self.has_function(world, id, SEAT_CHANGED) {
                self.callback(world, id, function::PLACED, 0)?;
            }
            cursor.advance(world).map_err(fail)?;
        }
        Ok(())
    }

    /// Works out which sides of an object's tile have a wall (0x434fac)
    /// and writes them to the low six bits of data word 5, for an object
    /// with function 6, which is asked that function when the bits change.
    /// A side counts if it has a wall segment that is not a fence (styles
    /// 2, 12, 13 and 14). The bits are turned to the object's facing.
    fn walls_beside(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        if !self.has_function(world, id, WALLS_CHANGED) {
            return Ok(());
        }
        let old = datum(world, id, data::WALLS).map_err(fail)?;
        let p = position(world, id).map_err(fail)?;
        let level = usize::try_from(p[2] - 1)
            .ok()
            .filter(|&l| l < 2)
            .ok_or_else(|| fail(Trap::Unsupported(format!("an object on level {}", p[2]))))?;
        let [x, y, _] = tile(p);
        let mut flags = old as u16 & 0xffc0;
        if tile_inside_lot(world, [x, y, 0]) {
            // 0x435032: the facing over -2, kept to 0 to 3.
            let rotation = (i32::from(datum(world, id, data::FACING).map_err(fail)?) / -2) & 3;
            let sides = wall_sides(world, [x, y], level, rotation as u32).map_err(fail)?;
            for (side, style) in sides.into_iter().enumerate() {
                if style.is_some_and(|style| !is_fence(style)) {
                    flags |= [1, 4, 2, 8][side];
                }
            }
        }
        if flags as i16 != old {
            set_datum(world, id, data::WALLS, flags as i16).map_err(fail)?;
            self.callback(world, id, WALLS_CHANGED, 0)?;
        }
        Ok(())
    }

    /// Sets or clears the mark a door or window keeps on its tile
    /// (0x4330d7), which does nothing outside the lot's border.
    fn mark_portal(&self, world: &mut World, at: [i8; 3], set: bool) -> Result<(), Trap> {
        if !tile_inside_lot(world, at) {
            return Ok(());
        }
        let level = usize::try_from(i32::from(at[2]) - 1)
            .ok()
            .filter(|&l| l < 2)
            .ok_or_else(|| Trap::Unsupported(format!("a door on level {}", at[2])))?;
        let cell = (i32::from(at[0]) * FULL + i32::from(at[1])) as usize;
        let flags = world
            .arrays
            .get_mut(&FLAGS[level])
            .and_then(|a| a.data.get_mut(cell))
            .ok_or_else(|| Trap::Invalid("the lot has no tile flags".into()))?;
        *flags = (*flags & !PORTAL_TILE) | if set { PORTAL_TILE } else { 0 };
        Ok(())
    }

    /// One object leaves the lot (0x433125). An object that is not a tile
    /// of a group is asked function 10 first; a group asks all its tiles
    /// before any leaves. Then the object's links go to the object behind
    /// it, it and what it holds move off the lot, a seat it faced is
    /// worked out again, and whatever held it is asked function 9.
    fn take_off_one(&self, world: &mut World, id: u16, notify: bool) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let words = self.words(world, id).map_err(fail)?;
        if words[word::GROUP] == 0 {
            self.callback(world, id, function::LEAVING, 0)?;
        }
        let old = position(world, id).map_err(fail)?;
        let old_tile = tile(old);
        let old_facing = datum(world, id, data::FACING).map_err(fail)?;
        if datum(world, id, data::FLAGS_2).map_err(fail)? as u16 & 0x400 != 0 {
            let room = datum(world, id, data::ROOM).map_err(fail)?;
            world.mark_room(room);
        }
        let parent = capture(world, id).map_err(fail)?.container;
        relink(world, id, NOWHERE).map_err(fail)?;
        set_datum(world, id, data::CONTAINER, 0).map_err(fail)?;
        set_datum(world, id, data::CONTAINER_SLOT, -1).map_err(fail)?;
        self.move_tree(world, id, OFF_LOT).map_err(fail)?;
        let flags = datum(world, id, data::FLAGS).map_err(fail)?;
        set_datum(world, id, data::FLAGS, flags & !0x100).map_err(fail)?;
        // The original reads the level again after the move, so the tile
        // it uses from here on is on level 1.
        let level = position(world, id).map_err(fail)?[2] as i8;
        let was = [old_tile[0], old_tile[1], level];
        if self.has_function(world, id, SEAT) {
            self.seats_beside(world, forward(was, old_facing).map_err(fail)?)?;
        }
        if let Some(parent) = parent {
            self.callback(world, parent, function::PLACED, 0)?;
        }
        // 0x4332b7: the door test reads the definition the object stands
        // for, not its own.
        let door = self
            .master_words(world, id, "taking a door off (0x4332af)")
            .map_err(fail)?[word::PORTAL]
            == 6;
        if door {
            self.mark_portal(world, was, false).map_err(fail)?;
        }
        if notify {
            self.left_lot(world, id)?;
        }
        Ok(())
    }

    /// What the object registry does when it is told an object left the
    /// lot (0x447d2b, 0x4481eb): an indicator object is taken off the lot
    /// once more, this time without telling the registry. So an indicator
    /// that leaves is asked function 10 twice. Any other object changes
    /// nothing, and neither does a person: the original looks up what the
    /// person holds in its slot 1 and then goes by the person's own type.
    ///
    /// A star marker (GUID 0xE9337020) is also returned to the pool the
    /// registry keeps of them, which is not ported and fails.
    ///
    /// The registry does this only while it is switched on (its byte
    /// 0x11). In a trace of a load it is switched off only while the lot
    /// is being cleared, which does not come through here.
    fn left_lot(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
        if object.guid == STAR {
            return Err(fail(Trap::Unsupported(
                "a star marker leaving the lot goes back to the registry's pool; that is not \
                 ported"
                    .into(),
            )));
        }
        if !INDICATORS.contains(&object.guid) || object.person.is_some() {
            return Ok(());
        }
        self.take_off_one(world, id, false)
    }

    /// One object is put at a place (0x433340).
    fn put_one(&self, world: &mut World, id: u16, to: Place, notify: bool) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let words = self.words(world, id).map_err(fail)?;
        let old = position(world, id).map_err(fail)?;
        let was_outside = !capture_raw(world, id).map_err(fail)?.on_lot;
        let level = match to.container {
            Some(parent) => position(world, parent).map_err(fail)?[2],
            None => to.level,
        };
        let old_facing = datum(world, id, data::FACING).map_err(fail)?;
        let old_tile = tile(old);
        relink(world, id, NOWHERE).map_err(fail)?;
        let now = position(world, id).map_err(fail)?[2] as i8;
        let was = [old_tile[0], old_tile[1], now];
        if self.has_function(world, id, SEAT) {
            self.seats_beside(world, forward(was, old_facing).map_err(fail)?)?;
        }
        // 0x433427: as on taking off, the definition the object stands for.
        let portal = self
            .master_words(world, id, "placing a door (0x43341f)")
            .map_err(fail)?[word::PORTAL]
            == 6;
        if portal {
            self.mark_portal(world, was, false).map_err(fail)?;
        }
        let point = match to.container {
            Some(parent) => {
                let p = position(world, parent).map_err(fail)?;
                [p[0], p[1]]
            }
            None => to.position,
        };
        let place = Place {
            on_lot: true,
            position: point,
            level,
            slot: if to.container.is_some() { to.slot } else { -1 },
            container: to.container,
        };
        relink(world, id, place).map_err(fail)?;
        set_datum(
            world,
            id,
            data::CONTAINER,
            to.container.map_or(0, |parent| parent as i16),
        )
        .map_err(fail)?;
        set_datum(world, id, data::CONTAINER_SLOT, place.slot as i16).map_err(fail)?;
        self.move_tree(world, id, [point[0], point[1], level])
            .map_err(fail)?;
        let new_tile = tile(position(world, id).map_err(fail)?);
        self.seats_beside(world, new_tile)?;
        if self.has_function(world, id, SEAT) {
            let level = position(world, id).map_err(fail)?[2] as i8;
            let facing = datum(world, id, data::FACING).map_err(fail)?;
            let ahead = forward([new_tile[0], new_tile[1], level], facing).map_err(fail)?;
            self.seats_beside(world, ahead)?;
        }
        self.walls_beside(world, id)?;
        if words[word::GROUP] == 0 {
            self.callback(world, id, function::PLACED, 0)?;
        }
        if let Some(parent) = to.container {
            self.callback(world, parent, function::PLACED, 0)?;
        }
        let now = position(world, id).map_err(fail)?;
        if now[..2] != old[..2] && was_outside && notify {
            self.placed_from_outside(world, id)?;
        }
        if portal {
            let level = position(world, id).map_err(fail)?[2] as i8;
            self.mark_portal(world, [new_tile[0], new_tile[1], level], true)
                .map_err(fail)?;
        }
        if datum(world, id, data::FLAGS_2).map_err(fail)? as u16 & 0x400 != 0 {
            let room = datum(world, id, data::ROOM).map_err(fail)?;
            world.mark_room(room);
        }
        Ok(())
    }

    /// What the interface does when an object comes onto the lot from
    /// outside (0x44752e). If the lot has no indicator objects yet it
    /// creates them ([`Interpreter::ensure_indicators`], 0x445e4a). For a
    /// person it then does more, in this order:
    ///
    /// - the person is the selected Sim: the first indicator goes into its
    ///   head slot ([`Interpreter::mark_selected`], 0x4475d2);
    /// - person word 77 names an object (is above 0): the second
    ///   indicator moves to that object (0x4477fb), which is not ported
    ///   and fails;
    /// - the person is not selected and person word 81 is 10 or more:
    ///   with bit 0x40 of the installed-expansions mask (application field
    ///   +0x66, set on a Complete Collection install and not modeled), a
    ///   pooled object moves to the person (0x447a82). This is not ported
    ///   and fails whatever the mask.
    ///
    /// A person word 77 below 0 names no object and does nothing.
    fn placed_from_outside(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        self.ensure_indicators(world)?;
        let Some(object) = world.objects.get(&id) else {
            return Ok(());
        };
        if object.person.is_none() {
            return Ok(());
        }
        let selected = object.selected;
        if selected {
            self.mark_selected(world, id)?;
        }
        let object = world.objects.get(&id).ok_or_else(|| Fault {
            object: id,
            at: None,
            trap: missing(),
        })?;
        let person = object.person.as_ref().ok_or_else(|| Fault {
            object: id,
            at: None,
            trap: Trap::Invalid("a person stopped being one".into()),
        })?;
        let word = |index: usize| person.data.get(index).copied().unwrap_or(0);
        let case = if word(77) > 0 {
            Some(
                "a person whose person word 77 names an object moves the second indicator there (0x4477fb)",
            )
        } else if !selected && word(81) >= 10 {
            Some("a person whose person word 81 is 10 or more takes a pooled object (0x447a82)")
        } else {
            None
        };
        match case {
            Some(what) => Err(Fault {
                object: id,
                at: None,
                trap: Trap::Unsupported(format!("{what}; not ported")),
            }),
            None => Ok(()),
        }
    }

    /// The selected Sim `id` has come onto the lot (0x4475d2 to 0x4477bb),
    /// with the lot's first indicator (the marker over the selected Sim;
    /// none, nothing happens):
    ///
    /// 1. An indicator that is not in the person (0x43265e) leaves the lot
    ///    (its virtual function 0x50 with 1, [`Interpreter::take_off`]).
    /// 2. A person on the lot's border tiles (0x422632: a tile below 1 or
    ///    at the lot's size less 1 or more) stops here.
    /// 3. The person's slot 1 as scripts count it (record 2) is its head
    ///    slot, when its definition has no container slots of its own
    ///    (the person's records, 0x45962b); one with any fails loudly.
    ///    The head slot has flag 2 (record +0x1c), so the step goes on.
    /// 4. The object the head slot holds, if any, is let go
    ///    ([`Interpreter::release_from_head`], 0x4481eb).
    /// 5. The indicator is put into the head slot with the lot told
    ///    (virtual function 0x58: the person's position and level, slot 1,
    ///    last argument 1).
    ///
    /// The rest (0x447719 to 0x4477bb) is drawing: the indicator's sprite
    /// is told the person's draw group and shown.
    fn mark_selected(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let indicator = world.indicators[0];
        if indicator == 0 || !world.objects.contains_key(&indicator) {
            return Ok(());
        }
        if capture(world, indicator).map_err(fail)?.container != Some(id) {
            self.take_off(world, indicator, true)?;
        }
        let [p0, p1, level] = position(world, id).map_err(fail)?;
        let size = i32::from(world.size);
        let border = |p: i32| {
            let t = i32::from((p >> 4) as i8);
            t < 1 || t >= size - 1
        };
        if border(p0) || border(p1) {
            return Ok(());
        }
        let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
        let definition = crate::interpreter::object_definition(self.catalog, world, object.guid)
            .ok_or_else(|| fail(Trap::Invalid("object has no definition".into())))?;
        let own = self
            .program
            .slots(definition.resource, definition.chunk)
            .iter()
            .filter(|entry| entry.kind == CONTAINER_SLOT)
            .count();
        if own != 0 {
            return Err(fail(Trap::Unsupported(format!(
                "the selected Sim {id} comes onto the lot with {own} container slots of its \
                 own: its slot 1 (0x4476a2) is then one of them, whose flags are not read"
            ))));
        }
        let held = object
            .slots
            .get(2)
            .ok_or_else(|| fail(Trap::Invalid(format!("person {id} has no head slot"))))?
            .object;
        if let Some(held) = registered(world, held) {
            self.release_from_head(world, held)?;
        }
        self.put(
            world,
            indicator,
            Place {
                on_lot: on_lot([p0, p1]),
                position: [p0, p1],
                container: Some(id),
                slot: 1,
                level,
            },
            true,
        )
    }

    /// 0x4481eb, for an object in the selected Sim's head slot: for a
    /// person, the object in its own head slot instead (record 2; none,
    /// nothing happens). Then by the first object's GUID: an indicator's
    /// leaves the lot unannounced (virtual function 0x50 with 0); type
    /// 0xE9337020 first comes out of the interface's pooled list
    /// (0x4483e7, 0x448663), which is not ported and fails; any other
    /// does nothing.
    fn release_from_head(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
        let guid = object.guid;
        let target = if self.words(world, id).map_err(fail)?[word::TYPE] == 2 {
            object
                .slots
                .get(2)
                .and_then(|s| registered(world, s.object))
        } else {
            Some(id)
        };
        let Some(target) = target else {
            return Ok(());
        };
        if guid == POOLED {
            return Err(fail(Trap::Unsupported(format!(
                "object {id} of type 0x{POOLED:08X} let go from the selected Sim's head slot: \
                 it leaves the interface's pooled list first (0x4483e7, 0x448663); not ported"
            ))));
        }
        if INDICATORS.contains(&guid) {
            self.take_off(world, target, false)?;
        }
        Ok(())
    }

    /// The tiles of the group an object belongs to, lead tile first, or
    /// the object alone if it has one tile.
    pub(crate) fn parts(&self, world: &World, id: u16) -> Result<Vec<u16>, Trap> {
        let object = world.objects.get(&id).ok_or_else(missing)?;
        let Some(part) = &object.multipart else {
            return Ok(vec![id]);
        };
        let lead = if part.parent == 0 {
            id
        } else {
            part.parent as u16
        };
        let mut parts = vec![lead];
        let mut last = lead;
        while let Some(&next) = world.next_parts.get(&last) {
            if parts.contains(&next) {
                return Err(Trap::Invalid("the tiles of a group form a loop".into()));
            }
            parts.push(next);
            last = next;
        }
        if !parts.contains(&id) || parts.iter().any(|p| !world.objects.contains_key(p)) {
            return Err(Trap::Invalid(format!(
                "object {id} is not among the tiles of its group"
            )));
        }
        Ok(parts)
    }

    /// Takes an object off the lot. It stays registered, at the off-lot
    /// position, in no container and no room. A multi-tile object takes
    /// its whole group off (0x422d86): every tile is asked function 10,
    /// then every tile leaves.
    ///
    /// A person is refused here: only a person on its way to removal is
    /// ported ([`Interpreter::take_off_to_remove`]).
    pub(crate) fn take_off(&self, world: &mut World, id: u16, notify: bool) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
        if object.person.is_some() {
            return Err(fail(Trap::Unsupported(
                "taking a person off the lot other than to remove it is not ported: its \
                 animator is then refreshed (0x58da12, from 0x4332ed) if the house view found \
                 it on screen at its last draw (animator byte +0x109, 0x590b8b), which is not \
                 kept"
                    .into(),
            )));
        }
        if object.multipart.is_none() {
            return self.take_off_one(world, id, notify);
        }
        let parts = self.parts(world, id).map_err(fail)?;
        for &part in &parts {
            self.callback(world, part, function::LEAVING, 0)?;
        }
        for &part in &parts {
            self.take_off_one(world, part, notify)?;
        }
        Ok(())
    }

    /// Person `id`'s animator refresh at a take-off on its way to removal
    /// that is not its own Remove Object Instance that yields (0x4332e0
    /// to 0x4332ed, 0x58da12). The refresh runs only if the house view
    /// drew the person (animator byte +0x109), which is not kept. It is
    /// worked out on a copy of the lot as the take-off left it
    /// (`Interpreter::refresh_at_take_off`), after the head slot and hand
    /// are checked as for [`Interpreter::take_self_off`], and must write
    /// nothing but the person's animator and base layer and play no
    /// sound. The lot goes on without it.
    ///
    /// The two branches then differ only in that person's animator until
    /// its deletion at the end of the update destroys it, and nothing may
    /// read it before: a person queued for removal fails if it starts its
    /// tick (`Interpreter::tick_person`) or runs a primitive that reads an
    /// animator ([`reads_no_animator`]), and so does a head that looks at
    /// it (`crate::head`) and a person left queued after a partial pass
    /// (`Interpreter::end_of_update`). That the deletion's last reset
    /// (0x45e791) reads none of it is **unverified**, as for every removal
    /// of a person ([`Interpreter::take_off_to_remove`]).
    pub(crate) fn refresh_before_removal(&self, world: &World, id: u16) -> Result<Pending, Trap> {
        let object = world.objects.get(&id).ok_or_else(missing)?;
        if object.slots.get(1).is_some_and(|slot| slot.object != 0) {
            return Err(Trap::Unsupported(format!(
                "person {id}, whose hand holds something, taken off the lot on its way to \
                 removal other than by its own Remove Object Instance that yields: its \
                 animator's refresh (0x58da12), made only if the house view drew it, is taken \
                 to read only its head slot (0x58dae5 to 0x58dc55)"
            )));
        }
        head_slot_redraw(world, id)?;
        self.refresh_at_take_off(world, id)
    }

    /// Person `id` takes itself off the lot, while its own tick runs its
    /// thread (Find Location For mode 1 on itself, 0x44b519). The take-off
    /// is the one-tile path of [`Interpreter::take_off`] (0x433125, which
    /// a person's vtable 0x667378 shares), and at its end (0x4332e0 to
    /// 0x4332ed, before the registry is told) the person's animator is
    /// refreshed (its virtual function 0x54 with 1, 0x58da12), which does
    /// nothing unless animator byte +0x109 is set (0x58da2c).
    ///
    /// That byte is what the house view's last draw found (virtual
    /// function 0x6c, 0x590b8b): byte +0xd9, or else whether the person's
    /// bounds were on screen (0x5a0a8a). It is presentation and not kept.
    /// When it is set, the refresh does in small what the animator's tick
    /// does (0x58dc72), at the same clock: it reads the clock (+0xb4),
    /// removes a blend layer with no weight (+0x10; none is made here),
    /// updates the draw state and the face (0x434101, 0x58faf6, left out
    /// as in the tick), forces the skin's update at the clock (0x5992b3
    /// with 1; its second argument only gates the pose, 0x59820a), places
    /// the animator (virtual function 0x60), and runs the head's step,
    /// measurement and restoration (0x590768, 0x5904d2, 0x590b27). Then it
    /// redraws what the person's head slot holds (0x58dae5 to 0x58dc55;
    /// [`head_slot_redraw`]).
    ///
    /// Whether the byte was set is not known, so both branches are kept
    /// apart until they meet. The refresh is worked out here, on a copy
    /// of the lot as the take-off leaves it
    /// (`Interpreter::refresh_at_take_off`), and fails unless it writes
    /// nothing but the person's animator and base layer and plays no
    /// sound. The lot goes on without it. The person's own animator tick
    /// follows in the same tick, at the same clock, after its thread
    /// yields; until then only primitives that read nothing of an
    /// animator may run ([`reads_no_animator`]), so they cannot tell the
    /// branches apart. The tick settles it
    /// (`Interpreter::tick_animator_after_take_off`). A person whose head
    /// slot holds a person or a tile of a group fails
    /// ([`head_slot_redraw`]), and so does one whose hand (slot 1) holds
    /// anything, and a take-off outside the person's own tick.
    pub(crate) fn take_self_off(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let refreshing = self
            .refreshing
            .filter(|r| r.borrow().person == id && r.borrow().pending.is_none());
        let Some(refreshing) = refreshing else {
            return self.take_off(world, id, true);
        };
        let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
        if object.multipart.is_some() {
            return Err(fail(Trap::Invalid("a person with several tiles".into())));
        }
        // The refresh comes after the callbacks below, so its head slot
        // redraw reads the slot there ([`head_slot_redraw`]); it is read
        // before them too, which can only add stops. A person whose hand
        // holds something (slot 1) also fails, which can only add stops:
        // the refresh reads no other container entry.
        let holding = |world: &World| {
            let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
            if object.slots.get(1).is_some_and(|slot| slot.object != 0) {
                return Err(fail(Trap::Unsupported(
                    "a person whose hand holds something taking itself off the lot: its \
                     animator's refresh, if the house view drew it, is taken to read only its \
                     head slot (0x58dae5 to 0x58dc55)"
                        .into(),
                )));
            }
            head_slot_redraw(world, id).map_err(fail)
        };
        holding(world)?;
        // 0x4332e0 to 0x4332ed: the refresh comes after the holder's
        // function 9 and the portal's tiles, before the registry is told,
        // which changes nothing for a person (`left_lot`).
        self.take_off_one(world, id, false)?;
        holding(world)?;
        let pending = self.refresh_at_take_off(world, id).map_err(fail)?;
        self.left_lot(world, id)?;
        refreshing.borrow_mut().pending = Some(pending);
        Ok(())
    }

    /// Person `id`'s second animator refresh before its animator's tick:
    /// it took itself off the lot earlier in its tick
    /// ([`Interpreter::take_self_off`]), and now leaves it again on its
    /// way to removal, removing itself and yielding (the removal queue,
    /// 0x442d70, then 0x433125 and 0x58da12 again). Whether the house view
    /// drew the person (animator byte +0x109) was decided by its last
    /// draw, and no draw comes in between, so both refreshes are made or
    /// neither is. The drawn branch's refresh is made again on top of the
    /// first, on a copy of the lot as the second take-off leaves it, and
    /// must write nothing but the animator and its base layer and play
    /// no sound ([`Interpreter::refresh_at_take_off`]). The person's
    /// animator's tick then checks that both refreshes and the tick reach
    /// what the tick alone does
    /// ([`Interpreter::tick_animator_after_take_off`]). As at the first
    /// take-off, a hand (slot 1) that holds anything fails, and so does a
    /// head slot whose redraw is not read ([`head_slot_redraw`]).
    pub(crate) fn refresh_again(
        &self,
        world: &World,
        id: u16,
        pending: &mut Pending,
    ) -> Result<(), Trap> {
        let person = world.objects.get(&id).ok_or_else(missing)?;
        if person.slots.get(1).is_some_and(|slot| slot.object != 0) {
            return Err(Trap::Unsupported(
                "a person whose hand holds something leaving the lot a second time before its \
                 animator's tick: its animator's refresh, if the house view drew it, is taken \
                 to read only its head slot (0x58dae5 to 0x58dc55)"
                    .into(),
            ));
        }
        head_slot_redraw(world, id)?;
        let mut drawn = world.clone();
        pending.refreshed.restore(&mut drawn, id);
        pending.refreshed = self.refresh_at_take_off(&drawn, id)?.refreshed;
        Ok(())
    }

    /// Takes an object off the lot on its way to removal: from the removal
    /// queue (0x442d70) or the removal itself (0x44123f), both of which
    /// call its virtual function 0x50 with 1. An object that is not a
    /// person goes as [`Interpreter::take_off`] takes it.
    ///
    /// A person (one tile; vtable 0x667378 shares 0x433125 with an
    /// ordinary object) leaves as an object does ([`Interpreter::take_off`]'s
    /// one-tile path), with one step more: its virtual function 0x2c gives
    /// its animator (person +0xadc, 0x4594de), whose virtual function 0x54
    /// is called with 1 (0x4332e0 to 0x4332ed, 0x58da12). That refresh
    /// returns at once unless animator byte +0x109 is set (0x58da2c): the
    /// result of the draw bounds' test of the house view's last draw
    /// (virtual function 0x6c, 0x590b8b), which is presentation and not
    /// kept. When it is set, the refresh reads the animator's clock again
    /// (virtual function 0x34), forces the skin's update at it (0x5992b3
    /// with 1), places the animator (virtual function 0x60), runs the
    /// head's step, measurement and restoration (0x590768, 0x5904d2,
    /// 0x590b27), and redraws what the person carries in its slot 1.
    ///
    /// Here the refresh returns at once, as for a person the view did not
    /// draw: **unverified**. The byte is not always set for a drawn
    /// person (in a capture the salesman has 1 at update 900 and 0 at
    /// updates 950 to 1000), and the animator's tick writes it too. When
    /// set, the head's step (0x590768) advances the head's clock and
    /// counts person word 45 down, which a script could read. So the
    /// callers take only the cases where that cannot be read first: the
    /// Remove Object Instance queue takes a person as it is when it
    /// removes itself and yields (`Machine::queue_removal`), and otherwise
    /// works the refresh out on a copy and holds it until the person's
    /// animator's tick ([`Interpreter::refresh_before_removal`]). Removal at once
    /// ([`Interpreter::remove_now`], which the end of the update runs for
    /// the queued person, so the original refreshes it a second time)
    /// deletes the person right after, so only the deletion's last reset
    /// could read what the refresh wrote (**unverified** that it does
    /// not). Other cases fail loudly. Either way it writes only the person's own
    /// animator and data words, and the drawing of what it carries, and
    /// the removal at the end of the same lot update destroys the
    /// animator and the person. Before that, the person's own tick (after
    /// its thread yields at Remove Object Instance) brings the skin up to
    /// date at the same clock as the refresh would have, and runs the
    /// head's step. What could still differ is read only by another
    /// person's head that looks at this one (`crate::head`, 0x59293d) or
    /// by a person still queued after the update ([`Interpreter::end_of_update`]);
    /// both fail loudly.
    pub(crate) fn take_off_to_remove(
        &self,
        world: &mut World,
        id: u16,
        notify: bool,
    ) -> Result<(), Fault> {
        let object = world.objects.get(&id).ok_or_else(|| Fault {
            object: id,
            at: None,
            trap: missing(),
        })?;
        if object.person.is_none() {
            return self.take_off(world, id, notify);
        }
        if object.multipart.is_some() {
            return Err(Fault {
                object: id,
                at: None,
                trap: Trap::Invalid("a person with several tiles".into()),
            });
        }
        self.take_off_one(world, id, notify)
    }

    /// Puts an object at a place. A multi-tile object puts its whole
    /// group (0x422c2f): each tile goes where its offset from the given
    /// tile says, then every tile is asked function 9. A place off the lot
    /// takes the group off instead.
    pub(crate) fn put(
        &self,
        world: &mut World,
        id: u16,
        to: Place,
        notify: bool,
    ) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
        let Some(selected) = object.multipart.as_ref().map(|part| part.offset) else {
            return self.put_one(world, id, to, notify);
        };
        if object.person.is_some() {
            return Err(fail(Trap::Invalid("a person with several tiles".into())));
        }
        let was_outside = !on_lot([object.position[0], object.position[1]]);
        let parts = self.parts(world, id).map_err(fail)?;
        for &part in &parts {
            let offset = world
                .objects
                .get(&part)
                .and_then(|o| o.multipart.as_ref())
                .ok_or_else(|| fail(missing()))?
                .offset;
            if on_lot(to.position) {
                let place = Place {
                    position: [
                        to.position[0]
                            .wrapping_sub(selected[0].wrapping_mul(16))
                            .wrapping_add(offset[0].wrapping_mul(16)),
                        to.position[1]
                            .wrapping_sub(selected[1].wrapping_mul(16))
                            .wrapping_add(offset[1].wrapping_mul(16)),
                    ],
                    level: to.level.wrapping_sub(selected[2]).wrapping_add(offset[2]),
                    ..to
                };
                self.put_one(world, part, place, notify)?;
            } else {
                self.take_off_one(world, part, notify)?;
            }
        }
        for &part in &parts {
            self.callback(world, part, function::PLACED, 0)?;
        }
        if was_outside && notify {
            self.placed_from_outside(world, id)?;
        }
        Ok(())
    }

    /// Turns a group by `steps` times its turn step (a group's virtual
    /// function 0x4c, 0x422937), keeping the tile `id` where it stands:
    ///
    /// 1. The tile's position and level are noted, and the group is taken
    ///    off the lot with the lot told ([`Interpreter::take_off`],
    ///    0x422d86).
    /// 2. The turn is the tile's data word 23 times `steps`, in eighths.
    ///    A turn of 0 ends here, with the group off the lot.
    /// 3. Each tile, from the lead along the list, turns by it (data word
    ///    1, kept to 0 to 7), and the offsets are worked out again
    ///    ([`Interpreter::update_offsets`], vtable +0x88).
    /// 4. If the group does not fit where the tile was
    ///    ([`Interpreter::fits`], 0x422abb) and that place is on the lot
    ///    (0x422632), each tile turns back and the offsets are worked out
    ///    again.
    /// 5. If the group fits where the tile was, it is put there with the
    ///    lot told ([`Interpreter::put`], 0x422c2f).
    ///
    /// Not modeled, being presentation: each tile's redraw (0x434101) and
    /// the lighting around it (0x434ed6).
    pub(crate) fn turn_group(&self, world: &mut World, id: u16, steps: i32) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let [x, y, level] = position(world, id).map_err(fail)?;
        self.take_off(world, id, true)?;
        let step = (i32::from(datum(world, id, data::TURN_STEP).map_err(fail)?).wrapping_mul(steps)
            & 7) as i16;
        if step == 0 {
            return Ok(());
        }
        let parts = self.parts(world, id).map_err(fail)?;
        let turn = |world: &mut World, by: i16| -> Result<(), Fault> {
            for &part in &parts {
                let facing = datum(world, part, data::FACING).map_err(fail)?;
                set_datum(world, part, data::FACING, facing.wrapping_add(by) & 7).map_err(fail)?;
            }
            self.update_offsets(world, id).map_err(fail)
        };
        turn(world, step)?;
        if !self.fits(world, id, [x, y], level)? && !off_lot(world, [x, y]) {
            turn(world, step.wrapping_neg())?;
        }
        if self.fits(world, id, [x, y], level)? {
            let to = Place {
                on_lot: on_lot([x, y]),
                position: [x, y],
                container: None,
                slot: -1,
                level,
            };
            self.put(world, id, to, true)?;
        }
        Ok(())
    }

    /// Turns a one-tile object by `steps` times its turn step (virtual
    /// function 0x4c of the object and person classes, vtables 0x666d34
    /// and 0x667378: 0x433f75), where it stands:
    ///
    /// 1. A `steps` of 0 or a turn step (data word 23) of 0 changes
    ///    nothing.
    /// 2. The facing (data word 1) becomes the old one plus the turn step
    ///    times `steps`, kept to 0 to 7.
    /// 3. If the object is a seat (it has function 26, 0x457c38,
    ///    0x455fa9), the tile in front of its old facing and the tile in
    ///    front of its new one are told whether a seat faces them
    ///    ([`Interpreter::seats_beside`], 0x433612), in that order. The
    ///    tiles are the object's tile one step that way (0x6c2934 and
    ///    0x6c2958 are [`forward`]'s steps) on its level. The second tile
    ///    is worked out after the first call, from the object's place and
    ///    facing as they are then, since that call can run a tree.
    /// 4. The walls beside it are worked out again
    ///    ([`Interpreter::walls_beside`], 0x434fac).
    ///
    /// Not modeled, being presentation: the redraw of the object or its
    /// outermost container (0x434101).
    pub(crate) fn turn_one(&self, world: &mut World, id: u16, steps: i32) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let step = datum(world, id, data::TURN_STEP).map_err(fail)?;
        if steps == 0 || step == 0 {
            return Ok(());
        }
        let old = datum(world, id, data::FACING).map_err(fail)?;
        let new = (i32::from(step)
            .wrapping_mul(steps)
            .wrapping_add(i32::from(old))
            & 7) as i16;
        set_datum(world, id, data::FACING, new).map_err(fail)?;
        if self.has_function(world, id, SEAT) {
            let at = tile(position(world, id).map_err(fail)?);
            self.seats_beside(world, forward(at, old).map_err(fail)?)?;
            // The first call can run a tree (0x4337d8), so the place and
            // facing are read again (0x434002 to 0x43400c).
            let at = tile(position(world, id).map_err(fail)?);
            let facing = datum(world, id, data::FACING).map_err(fail)?;
            self.seats_beside(world, forward(at, facing).map_err(fail)?)?;
        }
        self.walls_beside(world, id)
    }

    /// Works out each tile's offset from its group's lead tile again
    /// (0x4229c0): the difference of their places in the group, turned to
    /// the lead tile's facing.
    pub(crate) fn update_offsets(&self, world: &mut World, id: u16) -> Result<(), Trap> {
        let parts = self.parts(world, id)?;
        let lead = world.objects.get(&parts[0]).ok_or_else(missing)?;
        let base = lead.multipart.as_ref().ok_or_else(missing)?.group;
        let facing = datum(world, parts[0], data::FACING)?;
        for part in parts {
            let part = world
                .objects
                .get_mut(&part)
                .and_then(|o| o.multipart.as_mut())
                .ok_or_else(missing)?;
            let [x, y, z]: [i32; 3] = std::array::from_fn(|i| part.group[i].wrapping_sub(base[i]));
            let (x, y) = match facing & 7 {
                0 => (x, y),
                2 => (y.wrapping_neg(), x),
                4 => (x.wrapping_neg(), y.wrapping_neg()),
                6 => (y, x.wrapping_neg()),
                _ => (0, 0),
            };
            part.offset = [x, y, z];
        }
        Ok(())
    }
}

/// The walls on the four sides of a tile, turned `rotation` quarter turns
/// (0x4795c5, which reads the tile's wall cell, then 0x53bc11). Entry `j`
/// is the side whose edge bit is `1 << j` after the turn: the style of the
/// wall there, or None for no wall. `level` is 0 for the ground and 1 for
/// the floor above.
///
/// The cell's byte 0 says which of its sides have a segment: bits 1 and 2
/// are the tile's own (styles at bytes 2 and 4), 4 and 8 its east and
/// south neighbours' (the east tile's first style, the south tile's
/// second), which must mirror them in their own byte 0. A turn moves each
/// edge bit one place up, 8 becoming 1 (the table at 0x6f4670, filled by
/// 0x53ba5f), and the style with it. The tile must be inside the lot.
pub(crate) fn wall_sides(
    world: &World,
    [x, y]: [i8; 2],
    level: usize,
    rotation: u32,
) -> Result<[Option<u16>; 4], Trap> {
    let walls = world
        .arrays
        .get(&[2u16, 102][level])
        .filter(|a| a.stride == 14 && a.data.len() == (FULL * FULL * 14) as usize)
        .ok_or_else(|| Trap::Invalid("the lot has no wall array".into()))?;
    let cell = |x: i32, y: i32| {
        let at = ((x * FULL + y) * 14) as usize;
        &walls.data[at..at + 14]
    };
    let style =
        |cell: &[u8], which: usize| u16::from_le_bytes([cell[2 + which * 2], cell[3 + which * 2]]);
    let (x, y) = (i32::from(x), i32::from(y));
    let own = cell(x, y);
    let east = Some(cell(x + 1, y)).filter(|c| c[0] & 1 != 0);
    let south = Some(cell(x, y + 1)).filter(|c| c[0] & 2 != 0);
    let styles = [
        Some(style(own, 0)),
        Some(style(own, 1)),
        east.map(|c| style(c, 0)),
        south.map(|c| style(c, 1)),
    ];
    let mut sides = [None; 4];
    for source in 0..4 {
        if own[0] & (1 << source) == 0 {
            continue;
        }
        let style = styles[source].ok_or_else(|| {
            Trap::Unsupported("a wall segment its neighbouring tile does not mirror".into())
        })?;
        sides[(source + rotation as usize) % 4] = Some(style);
    }
    Ok(sides)
}

/// Whether a wall style is a fence's, which does not count as a wall
/// beside an object (styles 2, 12, 13 and 14; 0x43517c, 0x467dec).
pub(crate) fn is_fence(style: u16) -> bool {
    matches!(style, 2 | 12 | 13 | 14)
}

/// One step in each direction, in tiles: the first coordinate's
/// (0x6c2934) and the second's (0x6c2958). 0x4222bc steps the same way.
const STEPS: [(i8, i8); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

/// The step one tile in direction `facing`, masked with 7 (0x4222bc), as
/// tiles in each coordinate.
pub(crate) fn step(facing: i16) -> [i32; 2] {
    let (first, second) = STEPS[(facing & 7) as usize];
    [i32::from(first), i32::from(second)]
}

/// The tile one step from a tile in a direction: 0 is north, toward lower
/// second coordinates, and the directions go clockwise in eighths.
fn forward(at: [i8; 3], facing: i16) -> Result<[i8; 3], Trap> {
    let (first, second) = usize::try_from(facing)
        .ok()
        .and_then(|facing| STEPS.get(facing))
        .ok_or_else(|| Trap::Unsupported(format!("an object facing {facing}")))?;
    Ok([
        at[0].wrapping_add(*first),
        at[1].wrapping_add(*second),
        at[2],
    ])
}
