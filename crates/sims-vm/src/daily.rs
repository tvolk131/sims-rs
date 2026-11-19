//! The day's turn: what the lot update does when the hours reach 24
//! (0x46d3e9), and what signal 0xdc, which it sends, does to the lot.
//!
//! In the lot update, after the hours went up (see [`crate::tick`]):
//!
//! 1. Global 28 (days played) and global 1 (the day of the month) go up
//!    by 1 and the hours go back by 24. The signal sent at the end of the
//!    update is 0xdc instead of 0xdb.
//! 2. Every object on the lot's list, in its order, has its daily update
//!    (0x442628, 0x432243): [`age`], then [`depreciate`].
//! 3. The neighborhood's daily record of the household (0x429f14) is left
//!    out: it counts the household's members by age and gender, reads
//!    the funds, the values of the objects and the architecture, the
//!    motives and the budget, and appends one record to a list of the
//!    family's (0x4109a9, 0x410f87). Nothing ported reads that list;
//!    that the record changes nothing else the simulation keeps is
//!    **unverified**.
//! 4. The budget's days move back one ([`shift_budget`]).
//! 5. When the day of the month is past the month's length (always 30,
//!    0x473544), the month goes up and the day becomes 1; past month 12
//!    the year goes up and the month becomes 1. Global 29 takes the new
//!    month's length, 30 again.
//!
//! Then, after the registry's pass, signal 0xdc goes to the subscribers.
//! The lot's (0x41637f, 0x416453) moves the household's usage counts back
//! one day ([`shift_usage`]) and works out the lot's statistics again
//! ([`Interpreter::day_statistics`], 0x416085). None of the other seven
//! subscribers has a case for 0xdc: the simulator (0x46d2c2) and the
//! architecture (0x47b22a) do not, nor 0x43c0e4 or 0x44302e; the house
//! viewer, the panels and the interface (0x4a82db, 0x4c3ea5, 0x4f65bf)
//! are presentation and were not read in full for it, so that they change
//! nothing kept here is **unverified**.
//!
//! No capture covers a day's turn yet; all of this is read from the
//! original's code and **unverified**.

use crate::factory::word;
use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::placement::{floor, pool_byte, segments, stands_on_lot, tile_flags};
use sims_sim::World;

/// Globals the day's turn changes: the day of the month, the month, the
/// year, the days played and the month's length.
pub(crate) const DAY: usize = 1;
const MONTH: usize = 7;
const YEAR: usize = 8;
pub(crate) const DAYS_PLAYED: usize = 28;
const MONTH_LENGTH: usize = 29;
/// Global 22, which the statistics set when the lot has architecture or
/// an object that needs it (0x416145).
const BUILT: usize = 22;

/// Every month has 30 days (0x473544 returns 0x1e whatever it is given).
pub(crate) const DAYS_IN_MONTH: i16 = 30;

/// Object data words the daily update reads and writes.
mod data {
    /// Age in years (+0x6e).
    pub const YEARS: usize = 18;
    /// Nonzero: the object's value counts half (+0x68, 0x43219a).
    pub const HALVED: usize = 15;
    /// The object's current value (+0x9c).
    pub const VALUE: usize = 41;
    /// The object's flags (+0xa0, 0x4354d5).
    pub const FLAGS: usize = 43;
    /// The date the object was made: year, month, day (+0xa4 to +0xa8).
    pub const MADE: usize = 45;
    /// Age in months (+0xaa).
    pub const MONTHS: usize = 48;
    /// The object's category (+0xc0).
    pub const CATEGORY: usize = 59;
}

/// Definition words the daily update and the statistics read.
mod def {
    /// The first day's loss of value (OBJD +0x46).
    pub const INITIAL_DEPRECIATION: usize = 35;
    /// Each later day's loss of value (+0x48).
    pub const DAILY_DEPRECIATION: usize = 36;
    /// Nonzero: the object keeps its value (+0x4a).
    pub const SELF_DEPRECIATING: usize = 37;
    /// The value the object does not fall below (+0x4c).
    pub const DEPRECIATION_LIMIT: usize = 38;
    /// The three words any of which makes an object count for global 22
    /// (+0x4e, +0x50, +0x8a).
    pub const BUILT: [usize; 3] = [39, 40, 69];
    /// Word 69 of the catalog master, which keeps an object out of the
    /// objects' value (0x4354d5) and, but for 0 and 4, puts it in the
    /// architecture's (0x4451ad).
    pub const BUILD_KIND: usize = 69;
}

/// The object flag (data word 43) that keeps an object out of the
/// objects' value (0x4354d8).
const NOT_VALUED: i16 = 0x10;

/// Definition word 9 of a person (0x4354e7).
const PERSON: i16 = 2;

/// Definition word 9 of an object the statistics pass over (0x44508a).
const SKIPPED_TYPE: i16 = 7;

/// The tile flag that keeps a tile out of the statistics (0x4452cc,
/// 0x47b067).
const UNCOUNTED: u8 = 0x20;

/// The rest of the clock's day turn (0x46d3e9 to 0x46d49f), once the
/// hours went up to 24 and went back by 24 (the caller's part).
pub(crate) fn turn_day(interpreter: &Interpreter<'_>, world: &mut World) -> Result<(), Fault> {
    let fail = |object, trap| Fault {
        object,
        at: None,
        trap,
    };
    let global = |world: &World, index: usize| {
        world
            .globals
            .get(index)
            .copied()
            .ok_or_else(|| fail(0, Trap::Invalid(format!("the lot has no global {index}"))))
    };
    let set =
        |world: &mut World, index: usize, value: i16| {
            *world.globals.get_mut(index).ok_or_else(|| {
                fail(0, Trap::Invalid(format!("the lot has no global {index}")))
            })? = value;
            Ok::<(), Fault>(())
        };
    let played = global(world, DAYS_PLAYED)?.wrapping_add(1);
    set(world, DAYS_PLAYED, played)?;
    let day = global(world, DAY)?.wrapping_add(1);
    set(world, DAY, day)?;
    for id in world.order.clone() {
        age(world, id).map_err(|trap| fail(id, trap))?;
        depreciate(interpreter, world, id).map_err(|trap| fail(id, trap))?;
    }
    shift_budget(world).map_err(|trap| fail(0, trap))?;
    if global(world, DAY)? > DAYS_IN_MONTH {
        let mut month = global(world, MONTH)?.wrapping_add(1);
        set(world, DAY, 1)?;
        if month >= 13 {
            let year = global(world, YEAR)?.wrapping_add(1);
            set(world, YEAR, year)?;
            month = 1;
        }
        set(world, MONTH, month)?;
        set(world, MONTH_LENGTH, DAYS_IN_MONTH)?;
    }
    Ok(())
}

/// An object's age (0x4322e7): data word 18 becomes its age in whole
/// years and data word 48 in whole months, from the date it was made
/// (data words 45 to 47) to the lot's date (globals 8, 7 and 1), in the
/// original's wrapping 16-bit arithmetic. A day of the month before the
/// one it was made on takes a month off. The factory's reset works it out
/// the same way (`Machine::reset`).
pub(crate) fn age(world: &mut World, id: u16) -> Result<(), Trap> {
    let globals = &world.globals;
    let (Some(&year), Some(&month), Some(&day)) =
        (globals.get(YEAR), globals.get(MONTH), globals.get(DAY))
    else {
        return Err(Trap::Invalid("the lot has no date globals".into()));
    };
    let data = &mut world
        .objects
        .get_mut(&id)
        .ok_or_else(|| Trap::Invalid(format!("object {id} on the lot's list is gone")))?
        .data;
    if data.len() <= data::MONTHS {
        return Err(Trap::Invalid(format!("object {id} has too few data words")));
    }
    let made = [data[data::MADE], data[data::MADE + 1], data[data::MADE + 2]];
    let mut years = year.wrapping_sub(made[0]);
    let mut months = month.wrapping_sub(made[1]);
    if day.wrapping_sub(made[2]) < 0 {
        months = months.wrapping_sub(1);
    }
    if months < 0 {
        months = months.wrapping_add(12);
        years = years.wrapping_sub(1);
    }
    data[data::YEARS] = years;
    data[data::MONTHS] = months.wrapping_add(years.wrapping_mul(12));
    Ok(())
}

/// An object's loss of value for the day (0x432243, after [`age`]).
///
/// A tile of a multi-tile object (definition word 10 not 0) loses value
/// only if it is its group's lead (+0x190 is none or itself), and then by
/// the words of its group's catalog master (0x45728d), not its own; a
/// tile of a group that is not a multi-tile object (the cast to 0x6c0ef8
/// fails), or whose master is not found, loses none. An object whose
/// definition's word 37 is set keeps its value. Otherwise data word 41,
/// the value, goes down by word 35 when it still equals the price (word
/// 18) and word 35 is not 0, else by word 36, and not below word 38, all
/// in 16 bits.
pub(crate) fn depreciate(
    interpreter: &Interpreter<'_>,
    world: &mut World,
    id: u16,
) -> Result<(), Trap> {
    let object = world
        .objects
        .get(&id)
        .ok_or_else(|| Trap::Invalid(format!("object {id} on the lot's list is gone")))?;
    let own = interpreter.words(world, id)?;
    let words = if own[word::GROUP] == 0 {
        own
    } else {
        let Some(part) = &object.multipart else {
            return Ok(());
        };
        if part.parent != 0 && part.parent as u16 != id {
            return Ok(());
        }
        let definition =
            crate::interpreter::object_definition(interpreter.catalog, world, object.guid)
                .ok_or_else(|| Trap::Invalid("object has no definition".into()))?;
        match interpreter
            .program
            .master_definition(definition.resource, definition.chunk)
            .map_err(Trap::Invalid)?
        {
            Some(master) => master,
            None => return Ok(()),
        }
    };
    if words[def::SELF_DEPRECIATING] != 0 {
        return Ok(());
    }
    let value = *object
        .data
        .get(data::VALUE)
        .ok_or_else(|| Trap::Invalid(format!("object {id} has too few data words")))?;
    let initial = words[def::INITIAL_DEPRECIATION];
    let mut value = if value == words[word::PRICE] && initial != 0 {
        value.wrapping_sub(initial)
    } else {
        value.wrapping_sub(words[def::DAILY_DEPRECIATION])
    };
    let limit = words[def::DEPRECIATION_LIMIT];
    if value < limit {
        value = limit;
    }
    world
        .objects
        .get_mut(&id)
        .ok_or_else(|| Trap::Invalid(format!("object {id} on the lot's list is gone")))?
        .data[data::VALUE] = value;
    Ok(())
}

/// The budget's days move back one (0x46d417 to 0x46d44d): the four
/// oldest (+0x104 to +0x164) each take what the one before held, in a
/// forward copy, so that all four end up with yesterday's (+0xe4);
/// yesterday takes today's (+0x184), and today is zeroed. The original
/// quirk of the forward copy is kept.
///
/// [`sims_sim::Budget::days`] holds today's first, then +0xe4 and on. It
/// must hold all six days, and today and yesterday must be days the save
/// held in full: a day it marked with flag 0 is not kept, so what would
/// be copied from it is not known.
pub(crate) fn shift_budget(world: &mut World) -> Result<(), Trap> {
    let days = &mut world.budget.days;
    if days.len() != 6 {
        return Err(Trap::Unsupported(format!(
            "the day turns with {} budget days kept, not 6 (0x46d417)",
            days.len()
        )));
    }
    let (Some(today), Some(yesterday)) = (days[0], days[1]) else {
        return Err(Trap::Unsupported(
            "the day turns with today's or yesterday's budget marked as not saved: its \
             contents are not kept (0x46d417)"
                .into(),
        ));
    };
    for day in &mut days[2..] {
        *day = Some(yesterday);
    }
    days[1] = Some(today);
    days[0] = Some([0; 8]);
    Ok(())
}

/// The household's usage counts move back one day (0x416453 to
/// 0x41647b), with the same forward copy as the budget's: the four oldest
/// days all take the most recent one's, the most recent takes today's,
/// and today's is zeroed.
pub(crate) fn shift_usage(world: &mut World) {
    let usage = &mut world.usage;
    let recent = usage.history[0];
    for day in &mut usage.history[1..] {
        *day = recent;
    }
    usage.history[0] = usage.today;
    usage.today = Default::default();
}

impl Interpreter<'_> {
    /// The lot's statistics as signal 0xdc works them out (0x416085,
    /// 0x444cb9, 0x47b006).
    ///
    /// 1. The object pass (0x444cb9), over the lot's list in its order:
    ///    an object of category 1 is asked whether it stands on the lot
    ///    (0x432595, which can change its slot word, [`stands_on_lot`]).
    ///    Then a tile of a multi-tile group that is not its lead, and an
    ///    object of type 7, are passed over. Every other object is asked
    ///    again, twice (0x445094, 0x44510f), and one that stands on the
    ///    lot with any of definition words 39, 40 and 69 set, on a tile
    ///    without flag 0x20 (0x4452cc), counts for global 22.
    /// 2. The architecture (0x47b006) counts for global 22 when a tile of
    ///    level 1 or 2 inside the lot's size, without flag 0x20, has a
    ///    floor (0x4330b7) or, on level 1, a pool byte up to 0x10. A tile
    ///    with walls counts too; working out its walls (0x4795c5) is not
    ///    ported, so a lot where only walls could count fails loudly.
    /// 3. Global 23 becomes the lot's size, global 22 1 when either step
    ///    counted and else 0, and global 37 the lot's features, as at a
    ///    load ([`World::update_statistics`]).
    ///
    /// The object pass also sums values ([`value`], 0x43219a). An object
    /// it does not pass over that stands on the lot (its third ask,
    /// 0x445112) and is valued ([`valued`], 0x4354d5) adds to the
    /// objects' value. Any other adds to the architecture's when its
    /// catalog master's word 69 is not 0 or 4 (0x445190 to 0x445223).
    /// The objects' value becomes the budget's
    /// ([`sims_sim::Budget::objects_value`], simulator +0xa0, 0x416118).
    ///
    /// **Not ported: the architecture's value** (+0xa4, 0x416126), the
    /// objects' sum above plus what 0x47b006 returns: half the prices of
    /// the walls' styles (0x6e5838), the wall patterns' prices (0x4c63cf),
    /// the floors' prices (0x4a1439, half for a diagonal half floor) and
    /// the pool's (0x6c5ff8). The house panel works it out on its own
    /// (`sims_ui::house::Values`), but the simulation does not, so
    /// [`sims_sim::Budget::architecture_value`] keeps the value the house
    /// was saved with. Nothing ported reads it; a save written after a day
    /// turns would hold a stale value. This is a known gap, not the
    /// original's behaviour.
    ///
    /// Two flags of the selected family (+0x138: bit 1 when an object of
    /// category 1 stands on the lot, bit 2 when there is one of category
    /// 9) are not kept either (a script that reads them fails loudly,
    /// `memory.rs`).
    pub(crate) fn day_statistics(&self, world: &mut World) -> Result<(), Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        let mut built = false;
        let mut objects_value = 0i32;
        for id in world.order.clone() {
            let object = world.objects.get(&id).ok_or_else(|| {
                fail(id, Trap::Invalid("object on the lot's list is gone".into()))
            })?;
            let category = object.data.get(data::CATEGORY).copied().unwrap_or(0);
            let lead = object
                .multipart
                .as_ref()
                .is_none_or(|part| part.parent == 0 || part.parent as u16 == id);
            let words = *self.words(world, id).map_err(|trap| fail(id, trap))?;
            if category == 1 {
                stands_on_lot(world, id).map_err(|trap| fail(id, trap))?;
            }
            if (words[word::GROUP] != 0 && !lead) || words[word::TYPE] == SKIPPED_TYPE {
                continue;
            }
            let on_lot = stands_on_lot(world, id).map_err(|trap| fail(id, trap))?;
            if on_lot && def::BUILT.iter().any(|&w| words[w] != 0) {
                let position = world.objects[&id].position;
                let [a, b, level] = World::tile(position);
                let flags =
                    tile_flags(world, a, b, i32::from(level)).map_err(|trap| fail(id, trap))?;
                if flags & UNCOUNTED == 0 {
                    built = true;
                }
            }
            let on_lot = stands_on_lot(world, id).map_err(|trap| fail(id, trap))?;
            if on_lot && self.valued(world, id).map_err(|trap| fail(id, trap))? {
                let value = value(world, &words, id).map_err(|trap| fail(id, trap))?;
                objects_value = objects_value.wrapping_add(i32::from(value));
            }
            // Otherwise the object's value goes to the architecture's,
            // which is not ported (above).
        }
        world.budget.objects_value = objects_value;
        if !built {
            built = architecture_counts(world).map_err(|trap| fail(0, trap))?;
        }
        world
            .update_statistics()
            .map_err(|error| fail(0, Trap::Invalid(error.to_string())))?;
        *world
            .globals
            .get_mut(BUILT)
            .ok_or_else(|| fail(0, Trap::Invalid("the lot has no global 22".into())))? =
            i16::from(built);
        Ok(())
    }
}

impl Interpreter<'_> {
    /// Whether object `id` counts in the objects' value (0x4354d5): not
    /// with flag 0x10 in its data word 43; a person (definition word 9 is
    /// 2) does; any other only when its catalog master's word 69 is 0
    /// (0x45728d, `+0x60`, +0x8a). The test of flag 8 after it returns
    /// true here, flag 0x10 being clear.
    fn valued(&self, world: &World, id: u16) -> Result<bool, Trap> {
        let object = world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::Invalid(format!("object {id} on the lot's list is gone")))?;
        let flags = object.data.get(data::FLAGS).copied().unwrap_or(0);
        if flags & NOT_VALUED != 0 {
            return Ok(false);
        }
        if self.words(world, id)?[word::TYPE] == PERSON {
            return Ok(true);
        }
        let definition = crate::interpreter::object_definition(self.catalog, world, object.guid)
            .ok_or_else(|| Trap::Invalid(format!("object {id} has no definition")))?;
        let master = self
            .program
            .master_definition(definition.resource, definition.chunk)
            .map_err(Trap::Invalid)?
            .ok_or_else(|| {
                Trap::Unsupported(format!(
                    "object {id} has no catalog master, which 0x4354f9 reads through a null \
                     pointer"
                ))
            })?;
        Ok(master[def::BUILD_KIND] == 0)
    }
}

/// An object's value as the statistics count it (0x43219a). For an
/// object whose definition word 10 is 0, its data word 41. For a tile
/// of a group: none when it is not a multi-tile object (the cast to
/// 0x6c0ef8 fails), else its lead's word 41 (+0x190; the tile itself
/// when 0). Either way the value is halved, toward zero, when data word
/// 15 is not 0: for a group, on any tile from the lead along the chain
/// (+0x18c).
fn value(world: &World, words: &[i16], id: u16) -> Result<i16, Trap> {
    let object = |id: u16| {
        world
            .objects
            .get(&id)
            .ok_or_else(|| Trap::Invalid(format!("object {id} of a group is gone")))
    };
    let word_of = |id: u16, index: usize| -> Result<i16, Trap> {
        Ok(object(id)?.data.get(index).copied().unwrap_or(0))
    };
    let (value, halved) = if words[word::GROUP] == 0 {
        (word_of(id, data::VALUE)?, word_of(id, data::HALVED)? != 0)
    } else {
        let Some(part) = &object(id)?.multipart else {
            return Ok(0);
        };
        let lead = if part.parent == 0 {
            id
        } else {
            part.parent as u16
        };
        let mut halved = false;
        let mut tile = lead;
        let mut steps = 0;
        loop {
            halved |= word_of(tile, data::HALVED)? != 0;
            let next = world.next_parts.get(&tile).copied().unwrap_or(0);
            if next == 0 {
                break;
            }
            steps += 1;
            if steps > world.objects.len() {
                return Err(Trap::Invalid(format!("the group of object {id} loops")));
            }
            tile = next;
        }
        (word_of(lead, data::VALUE)?, halved)
    };
    Ok(if halved { value / 2 } else { value })
}

/// Whether the lot's architecture counts for global 22 (0x47b006's flag).
/// See [`Interpreter::day_statistics`].
fn architecture_counts(world: &World) -> Result<bool, Trap> {
    let size = i8::try_from(world.size)
        .ok()
        .filter(|&s| s > 0 && i32::from(s) <= 64)
        .ok_or_else(|| Trap::Invalid(format!("the lot's size is {}", world.size)))?;
    let mut walls = false;
    for level in 1..=2 {
        for b in 0..size {
            for a in 0..size {
                walls |= segments(world, a, b, level)? != 0;
                if tile_flags(world, a, b, level)? & UNCOUNTED != 0 {
                    continue;
                }
                if floor(world, a, b, level)? != 0 {
                    return Ok(true);
                }
                if level == 1 && pool_byte(world, a, b)?.is_some_and(|p| p <= 0x10) {
                    return Ok(true);
                }
            }
        }
    }
    // A tile's walls (0x4795c5) come from its own wall cell and its
    // neighbours' inside the lot's size: with no segment anywhere there,
    // no tile has a wall.
    if walls {
        return Err(Trap::Unsupported(
            "the day's statistics on a lot with walls but no floor or pool: whether its walls \
             count for global 22 (0x4795c5 in 0x47b006) is not ported"
                .into(),
        ));
    }
    Ok(false)
}
