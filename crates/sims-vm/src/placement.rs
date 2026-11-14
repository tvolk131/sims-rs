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
//! `docs/decisions.md` says what a trace of the running game confirms.
//!
//! Not modeled, being presentation: the redrawing an object asks for as it
//! moves, its shadow, and the lighting around a light source.

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::load::INDICATORS;
use crate::program::DEFINITION_WORDS;
use crate::visits::function;
use sims_sim::World;
use std::collections::BTreeSet;

mod find;
pub(crate) use find::Rules;

/// The side of the lot's arrays.
const FULL: i32 = 64;
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

fn tile_on_lot(tile: [i8; 3]) -> bool {
    let inside = |n: i8| n > 0 && i32::from(n) < FULL - 1;
    inside(tile[0]) && inside(tile[1])
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
fn head(world: &World, tile: [i8; 3]) -> Option<u16> {
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
struct Cursor {
    root: u16,
    current: Option<u16>,
}

impl Cursor {
    fn advance(&mut self, world: &World) -> Result<(), Trap> {
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
    fn words(&self, world: &World, id: u16) -> Result<&[i16; DEFINITION_WORDS], Trap> {
        let object = world.objects.get(&id).ok_or_else(missing)?;
        let definition = self
            .catalog
            .object(object.guid)
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
        world
            .objects
            .get(&id)
            .and_then(|object| self.catalog.object(object.guid))
            .and_then(|definition| {
                self.program
                    .libraries
                    .get(&definition.resource)
                    .and_then(|l| l.callbacks.get(&definition.chunk))
            })
            .and_then(|table| table.get(function))
            .is_some_and(|f| f.action != 0)
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
        if !tile_on_lot(at) {
            return Ok(());
        }
        let Some(root) = head(world, at) else {
            return Ok(());
        };
        let mut seat = false;
        for direction in [0usize, 2, 4, 6] {
            let other = forward(at, direction as i16).map_err(fail)?;
            if !tile_on_lot(other) {
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
        if tile_on_lot([x, y, 0]) {
            let walls = world
                .arrays
                .get(&[2u16, 102][level])
                .filter(|a| a.stride == 14 && a.data.len() == (FULL * FULL * 14) as usize)
                .ok_or_else(|| fail(Trap::Invalid("the lot has no wall array".into())))?;
            let cell = |x: i32, y: i32| {
                let at = ((x * FULL + y) * 14) as usize;
                &walls.data[at..at + 14]
            };
            let style = |cell: &[u8], which: usize| {
                u16::from_le_bytes([cell[2 + which * 2], cell[3 + which * 2]])
            };
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
            let rotation = (i32::from(datum(world, id, data::FACING).map_err(fail)?) / -2) & 3;
            for source in 0..4 {
                if own[0] & (1 << source) == 0 {
                    continue;
                }
                let style = styles[source].ok_or_else(|| {
                    fail(Trap::Unsupported(
                        "a wall segment its neighbouring tile does not mirror".into(),
                    ))
                })?;
                if !matches!(style, 2 | 12 | 13 | 14) {
                    flags |= [1, 4, 2, 8][(source + rotation as usize) % 4];
                }
            }
        }
        if flags as i16 != old {
            set_datum(world, id, data::WALLS, flags as i16).map_err(fail)?;
            self.callback(world, id, WALLS_CHANGED, 0)?;
        }
        Ok(())
    }

    /// Sets or clears the mark a door or window keeps on its tile.
    fn mark_portal(&self, world: &mut World, at: [i8; 3], set: bool) -> Result<(), Trap> {
        if !tile_on_lot(at) {
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
        if words[word::PORTAL] == 6 {
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
        let portal = words[word::PORTAL] == 6;
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
    /// person the original then moves the indicators to the selected Sim,
    /// which is not ported and fails.
    fn placed_from_outside(&self, world: &mut World, id: u16) -> Result<(), Fault> {
        self.ensure_indicators(world)?;
        if world.objects.get(&id).is_some_and(|o| o.person.is_some()) {
            return Err(Fault {
                object: id,
                at: None,
                trap: Trap::Unsupported(
                    "a person coming onto the lot moves the indicators; that is not ported".into(),
                ),
            });
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
    /// A person is refused: its own way of leaving is not ported.
    pub(crate) fn take_off(&self, world: &mut World, id: u16, notify: bool) -> Result<(), Fault> {
        let fail = |trap| Fault {
            object: id,
            at: None,
            trap,
        };
        let object = world.objects.get(&id).ok_or_else(|| fail(missing()))?;
        if object.person.is_some() {
            return Err(fail(Trap::Unsupported(
                "taking a person off the lot is not ported".into(),
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

/// The tile one step from a tile in a direction: 0 is north, toward lower
/// second coordinates, and the directions go clockwise in eighths.
fn forward(at: [i8; 3], facing: i16) -> Result<[i8; 3], Trap> {
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
