//! The sounds a clip's timed events play (0x5912fd; `clip_events.rs`):
//! "sound", "selectedsound", "deselectedsound" and "footstep". Each picks
//! a name and plays it for the person through the sound events
//! ([`SoundEvent::Play`], 0x57a54c). Nothing here writes the world or
//! draws a random number, so a sound cannot change what the lot does
//! ([`crate::sounds`]).
//!
//! - "sound" (0x591505): the value is played for the person when the
//!   person stands on the lot (0x432595: its tile inside the lot's border,
//!   [`crate::placement::off_lot`]); otherwise, for a person of kind (word
//!   65) above 1, for object -4; otherwise not at all. 0x432595 first takes
//!   the person's place (0x4325b5), which can rewrite the slot word of an
//!   object linked as a tile's first with no object before it; that write
//!   is left out here, so the sound reads the world only.
//! - "selectedsound" and "deselectedsound" (0x59157b, 0x5915a7): played
//!   when the person is, or is not, the selected Sim (0x45e50e: object
//!   +0x114 bit 1, [`sims_sim::Object::selected`]).
//! - "footstep" (0x5915e1, [`footstep`]): the number is a step, its sign
//!   the way the clip must be playing; the name comes from the surface
//!   under the animator's position ([`surface`]).
//!
//! The play's object is the person's ID (object +0xf0).

use crate::fault::Trap;
use crate::placement::{Cursor, off_lot};
use crate::sounds::SoundEvent;
use sims_core::{Ext, F32};
use sims_sim::World;
use sims_sim::animator::{Animator, Layer};
use sims_sim::load::FLOORS;
use std::collections::BTreeMap;

/// The pool array (`Arry 9`): below 16 a pool tile, 16 its edge.
const POOL: u16 = 9;
/// The lot arrays' side.
const SIDE: usize = 64;

/// The x of an animator that has not been placed (0x671edc, -999.0) and
/// how near the footstep's test takes it to be (0x67343c, 1e-6).
const UNPLACED: u32 = 0xc479_c000;
const NEAR: u32 = 0x3586_37bd;
/// 0.5 (0x666578), added before a coordinate is cut to a whole number.
const HALF: u32 = 0x3f00_0000;

/// Person data words a footstep is silenced by, with the bit tested
/// (0x5916f7 to 0x591728).
const SILENCED: [(usize, i16); 3] = [(84, 1), (68, 1), (74, 8)];
/// Person data word 65: the person's kind; above 1 a pet.
const KIND: usize = 65;

/// The suits (animator +0x1b0, the suit last dressed) whose footsteps
/// are the bare-footed ones (0x591631).
const BARE_FOOTED: [i32; 4] = [1, 2, 5, 14];
/// The lots whose ground sounds of snow (0x4c471b: house number 40 to
/// 42), and the lot whose ground is soft (house number 99, 0x5916d0).
const SNOWY: std::ops::RangeInclusive<i16> = 40..=42;
const SOFT_GROUND: i16 = 99;
/// Global 10: the house number (the house's +0x24).
const HOUSE: usize = 10;

/// The objects whose tile sounds of them, ranked (0x589112 to 0x589189):
/// the highest of the uncontained objects on the tile wins.
const OBJECT_SURFACES: [(u32, i32); 11] = [
    (0x96e3_398d, 6),
    (0xa2da_a08c, 6),
    (0xc02b_406a, 6),
    (0x6527_4a4f, 6),
    (0xbf62_f653, 7),
    (0x7f90_7075, 7),
    (0x6341_6ba1, 8),
    (99_999_999, 9),
    (0x9be0_30b9, 10),
    (0x3e74_70f6, 10),
    (0x5c67_fc8f, 10),
];

/// The category of a floor pattern (0x49fd95, kept in the low byte of the
/// pattern's +0x2c): the first byte of its sprite's label, 'C' 3, 'M' 1,
/// 'S' 2, anything else ('H' included) or an empty label 0.
pub(crate) fn floor_category(label: &[u8]) -> u8 {
    match label.first() {
        Some(b'C') => 3,
        Some(b'M') => 1,
        Some(b'S') => 2,
        _ => 0,
    }
}

/// What a timed event's sound key plays, if anything (0x591505 to
/// 0x59175e). `floors` gives each floor pattern's category
/// ([`floor_category`]) by its key; a footstep on a pattern it does not
/// list plays nothing.
pub(crate) fn pick(
    world: &World,
    id: u16,
    a: &Animator,
    layer: &Layer,
    key: &str,
    value: &str,
    floors: Option<&BTreeMap<u16, u8>>,
) -> Result<Option<SoundEvent>, Trap> {
    let number = crate::clip_events::atoi(value);
    let object = world
        .objects
        .get(&id)
        .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?;
    let person = object
        .person
        .as_ref()
        .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?;
    let word = |w: usize| {
        person
            .data
            .get(w)
            .copied()
            .ok_or_else(|| Trap::Invalid(format!("person data has no word {w}")))
    };
    let me = id as i16;
    let play = |object: i16, name: &str| {
        Some(SoundEvent::Play {
            object,
            name: name.to_owned(),
        })
    };
    Ok(match key {
        "sound" => {
            let [x, y, _] = object.position;
            if !off_lot(world, [x, y]) {
                play(me, value)
            } else if word(KIND)? > 1 {
                play(-4, value)
            } else {
                None
            }
        }
        "selectedsound" => object.selected.then(|| play(me, value)).flatten(),
        "deselectedsound" => (!object.selected).then(|| play(me, value)).flatten(),
        "footstep" => {
            footstep(world, id, a, layer, number, floors, &word)?.and_then(|name| play(me, &name))
        }
        _ => None,
    })
}

/// A footstep's name, or None when it plays nothing (0x5915f6 to
/// 0x59174d), in the original's order:
///
/// 1. An animator at x -999 (not placed; within 1e-6 after the
///    subtraction, single precision) plays nothing. A NaN x goes on.
/// 2. The surface under the animator ([`surface`]); -1 plays nothing.
/// 3. The name by surface: 1 and 2 "hard", 3 and 5 "medium", 4 "soft",
///    each "_noshoe" for a bare-footed suit; 6 to 11 "plant", "trash",
///    "ash", "roach", "puddle" and "pool_swim_stroke"; anything else is the
///    ground: "snow" on house 40 to 42, else "soft" on house 99, else
///    "terrain", the last two "_noshoe" for a bare-footed suit. Each is
///    "footstep_" and the word.
/// 4. A person with word 84 bit 0, word 68 bit 0 or word 74 bit 3 set
///    plays nothing.
/// 5. The step must go the way the event's layer plays: forward (speed at
///    least 0) a step of 1 or 2, else (backward, or a NaN speed) -1 or -2.
fn footstep(
    world: &World,
    id: u16,
    a: &Animator,
    layer: &Layer,
    number: i32,
    floors: Option<&BTreeMap<u16, u8>>,
    word: &dyn Fn(usize) -> Result<i16, Trap>,
) -> Result<Option<String>, Trap> {
    let off = (Ext::from_f32(a.position[0]) - Ext::from_f32(F32::from_bits(UNPLACED)))
        .single()
        .abs();
    if at_least(Ext::from_f32(F32::from_bits(NEAR)), off) {
        return Ok(None);
    }
    let Some(surface) = surface(world, id, a, floors)? else {
        return Ok(None);
    };
    if surface == -1 {
        return Ok(None);
    }
    let bare = world
        .person_outfits
        .get(&id)
        .is_some_and(|o| BARE_FOOTED.contains(&o.last_dressed[0]));
    let shod = |word: &str| {
        if bare {
            format!("{word}_noshoe")
        } else {
            word.to_owned()
        }
    };
    let house = world.globals.get(HOUSE).copied().unwrap_or(0);
    let name = match surface {
        1 | 2 => shod("hard"),
        3 | 5 => shod("medium"),
        4 => shod("soft"),
        6 => "plant".into(),
        7 => "trash".into(),
        8 => "ash".into(),
        9 => "roach".into(),
        10 => "puddle".into(),
        11 => "pool_swim_stroke".into(),
        _ if SNOWY.contains(&house) => "snow".into(),
        _ if house == SOFT_GROUND => shod("soft"),
        _ => shod("terrain"),
    };
    for (w, bit) in SILENCED {
        if word(w)? & bit != 0 {
            return Ok(None);
        }
    }
    let forward = at_least(Ext::from_f32(layer.speed), Ext::from_f32(F32::ZERO));
    let way = if forward {
        matches!(number, 1 | 2)
    } else {
        matches!(number, -1 | -2)
    };
    Ok(way.then(|| format!("footstep_{name}")))
}

/// The surface under the animator's position (0x58900f, 0x5890a8): -1
/// off the lot's arrays (either coordinate below 0, or NaN); else the
/// tile is each coordinate plus 0.5 (single precision), cut toward 0
/// (0x636128) and shifted down 4, as signed bytes, on the person's level
/// (object +0x110), and on it, first that applies:
///
/// 1. On level 0, water would make it 10 (0x478f4a), but 0x478f4a answers
///    only for level 1, so it never does.
/// 2. The highest rank of the objects on the tile ([`OBJECT_SURFACES`]),
///    walked as 0x43f331 and 0x43f379 do ([`Cursor::on_tile`]), passing
///    over those in a container (0x43265e).
/// 3. On level 1, a pool tile (`Arry 9` below 16, 0x478f76) is 11, its
///    edge (16, 0x478fa3) 1. A lot without the pool array has neither
///    (0x478f76 tests for it; 0x478fa3, asked first, does not).
/// 4. The floor pattern (0x4330b7): none 0; 255 2; else by its category
///    (0x4a1463; [`floor_category`]): 0 is 3, 1 is 2, 2 is 4, 3 is 1. A
///    pattern the original's catalog lacks is category 2.
///
/// None when the pattern's category is not in `floors`: only the
/// patterns of the base game's own file and those the lot used when it
/// loaded are read ([`crate::lifecycle::LotPrices`]).
pub(crate) fn surface(
    world: &World,
    id: u16,
    a: &Animator,
    floors: Option<&BTreeMap<u16, u8>>,
) -> Result<Option<i32>, Trap> {
    let zero = Ext::from_f32(F32::ZERO);
    let [x, y, _] = a.position;
    if !at_least(Ext::from_f32(x), zero) || !at_least(Ext::from_f32(y), zero) {
        return Ok(Some(-1));
    }
    let cut = |v: F32| {
        let whole = (Ext::from_f32(v) + Ext::from_f32(F32::from_bits(HALF)))
            .single()
            .to_i32_truncate();
        (whole >> 4) as i8
    };
    let level = world
        .objects
        .get(&id)
        .ok_or_else(|| Trap::native(0x1c, "object is not a person"))?
        .position[2] as i8;
    let tile = [cut(x), cut(y), level];

    let mut rank = 0;
    let mut cursor = Cursor::on_tile(world, tile);
    while let Some(current) = cursor.current() {
        if crate::placement::container(world, current)?.is_none() {
            let guid = world.objects.get(&current).map_or(0, |o| o.guid);
            if let Some(&(_, r)) = OBJECT_SURFACES.iter().find(|(g, _)| *g == guid) {
                rank = rank.max(r);
            }
        }
        cursor.advance(world)?;
    }
    if rank != 0 {
        return Ok(Some(rank));
    }

    if level == 1 && world.arrays.get(&POOL).is_some() {
        let pool = cell(world, POOL, 1, tile)?[0];
        if pool < 16 {
            return Ok(Some(11));
        }
        if pool == 16 {
            return Ok(Some(1));
        }
    }
    let floors_array = usize::try_from(i32::from(level) - 1)
        .ok()
        .and_then(|l| FLOORS.get(l))
        .ok_or_else(|| {
            Trap::Invalid(format!(
                "a footstep (0x4330b7) on level {level}, which has no floor"
            ))
        })?;
    let bytes = cell(world, *floors_array, 2, tile)?;
    let covering = u16::from_le_bytes([bytes[0], bytes[1]]);
    Ok(match covering {
        0 => Some(0),
        255 => Some(2),
        _ => floors
            .and_then(|f| f.get(&covering))
            .map(|category| match category {
                0 => 3,
                1 => 2,
                2 => 4,
                3 => 1,
                _ => 0,
            }),
    })
}

/// A tile's cell of a lot array (column-major, as the original's arrays
/// of columns are read). The original reads its arrays at any tile
/// without a test; a tile off them fails here.
fn cell(world: &World, array: u16, stride: usize, tile: [i8; 3]) -> Result<&[u8], Trap> {
    let off = || {
        Trap::Invalid(format!(
            "a footstep reads tile ({}, {}) of lot array {array}, off the array",
            tile[0], tile[1]
        ))
    };
    let a = world.arrays.get(&array).ok_or_else(|| {
        Trap::Invalid(format!(
            "a footstep reads lot array {array}, which the lot has not"
        ))
    })?;
    let column = usize::try_from(tile[0]).map_err(|_| off())?;
    let row = usize::try_from(tile[1]).map_err(|_| off())?;
    if a.stride != stride || column >= a.width.min(SIDE) || row >= a.height.min(SIDE) {
        return Err(off());
    }
    let at = (column * a.height + row) * stride;
    a.data.get(at..at + stride).ok_or_else(off)
}

/// `a >= b`, false when either is NaN (a compare and `jae`: unordered sets the carry).
fn at_least(a: Ext, b: Ext) -> bool {
    a >= b
}
