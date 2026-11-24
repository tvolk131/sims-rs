//! The trip request Generic Sims Call 17 makes (`Sims.exe` 1.91, 0x450660
//! → 0x45a22d), and the neighbor inventory tokens it writes.
//!
//! Before a person leaves the lot, 0x45a22d writes its saved state into its
//! neighbor inventory as kind-2 tokens, and the arrival clock as kind-2
//! tokens 7 and 8. It then records the house change for the main loop
//! (0x4fc713). The arrival marker's Generic Sims Call 21 reads the saved
//! state back; Generic Sims Call 23 reads the clock. See
//! `docs/decisions/20261124-phone-downtown-and-departure-tokens.md`.

use crate::fault::Trap;
use crate::interpreter::Machine;
use crate::people;
use sims_sim::TripRequest;
use sims_sim::neighbors::Item;

/// The token kind of a person's saved state (0x45a40a, read back by
/// Generic Sims Call 21).
pub const SAVED_STATE_KIND: i32 = 2;
/// The token kind naming the person a traveler takes along on the way out
/// (Manage Inventory operation 5; read by 0x45a34b and Generic Sims Call
/// 19).
pub const OUTBOUND_COMPANION_KIND: i32 = 1;
/// The token kind naming the person a traveler takes along on the way home
/// (read by 0x45a2ce and Generic Sims Call 20).
pub const HOMEBOUND_COMPANION_KIND: i32 = 3;
/// The saved-state token GUID that holds the arrival hour (0x45a7f6).
pub const ARRIVAL_HOUR_GUID: u32 = 7;
/// The saved-state token GUID that holds the arrival minute (0x45a805).
pub const ARRIVAL_MINUTE_GUID: u32 = 8;

/// The most tokens one inventory holds (0x41f3cb).
const MOST_TOKENS: usize = 0x7fff;
/// Person data words 0x45a22d reads: the neighbor ID (+0x5ca) and the
/// family (+0x606).
const PERSON_NEIGHBOR: usize = 31;
const PERSON_FAMILY: usize = 61;
/// Globals 0x45a785 reads: the hour (+0x10), the minute (+0x1a) and the
/// house (+0x24, through 0x40745f).
const GLOBAL_HOURS: usize = 0;
const GLOBAL_MINUTES: usize = 5;
const GLOBAL_HOUSE: usize = 10;
/// Houses whose arrival time is absolute when leaving a residential lot
/// (0x45a796, Vacation Island).
const ABSOLUTE_CLOCK_HOUSES: std::ops::Range<i16> = 40..50;

/// Where one saved-state token's count comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SavedField {
    /// A motive (person +0x78c), cut to a whole number by `__ftol`.
    Motive(usize),
    /// A person data word.
    Word(usize),
}

/// The saved-state tokens, by GUID, in the order 0x45a40a writes them.
/// Generic Sims Call 21 restores them through the same table (0x450a53,
/// 0x4514ea).
pub(crate) const SAVED_STATE: [(u32, SavedField); 11] = [
    (0, SavedField::Motive(9)),  // +0x7b0
    (1, SavedField::Motive(6)),  // +0x7a4
    (2, SavedField::Motive(5)),  // +0x7a0
    (3, SavedField::Motive(15)), // +0x7c8
    (4, SavedField::Motive(7)),  // +0x7a8
    (5, SavedField::Motive(8)),  // +0x7ac
    (6, SavedField::Motive(14)), // +0x7c4
    (9, SavedField::Word(8)),    // +0x59c
    (10, SavedField::Word(80)),  // +0x62c
    (11, SavedField::Word(82)),  // +0x630
    (12, SavedField::Word(81)),  // +0x62e
];

/// Where a saved-state token's count goes, by its GUID; GUIDs 7, 8 and
/// anything above 12 name nothing.
pub(crate) fn saved_field(guid: u32) -> Option<SavedField> {
    SAVED_STATE
        .iter()
        .find(|&&(g, _)| g == guid)
        .map(|&(_, field)| field)
}

/// `__ftol` (0x636128) of a motive, as the word 0x41f38d takes: truncated
/// toward zero by a 64-bit `fistp` (0x636141) and cut to its low word, and
/// the "integer indefinite" 0x8000_0000_0000_0000 (whose low word is 0) for
/// NaN or a value outside the signed 64-bit range.
#[must_use]
pub fn ftol_word(value: f32) -> i16 {
    if value.is_finite()
        && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value)
    {
        (value as i64) as i16
    } else {
        0
    }
}

/// 0x41f38d(kind, GUID, count) on one inventory's tokens.
///
/// - A count of 0 adds nothing.
/// - A token of that kind and GUID already there (the first, 0x41f721) gets
///   the count added as signed words (0x41f3f6); a sum of 0x7fff or more
///   becomes 0x7fff, and nothing caps it below.
/// - Otherwise the token is appended, unless the inventory already holds
///   0x7fff tokens, which is the only failure.
pub fn add_token(items: &mut Vec<Item>, kind: i32, guid: u32, count: i16) -> bool {
    if count == 0 {
        return true;
    }
    if let Some(item) = items.iter_mut().find(|i| i.kind == kind && i.guid == guid) {
        let sum = i32::from(item.count as i16) + i32::from(count);
        item.count = if sum >= 0x7fff {
            0x7fff
        } else {
            (item.count as i16).wrapping_add(count) as u16
        };
        return true;
    }
    if items.len() >= MOST_TOKENS {
        return false;
    }
    items.push(Item {
        kind,
        guid,
        count: count as u16,
    });
    true
}

/// The place 0x41f69c finds, searching from the start: the first token of
/// `kind` whose GUID is `guid`, or of any GUID when `guid` is 0. The place
/// is kept as a word, so -1 is none found, and an index past 0x7fff wraps.
#[must_use]
pub fn token_place(items: &[Item], kind: i32, guid: u32) -> i16 {
    items
        .iter()
        .position(|i| i.kind == kind && (guid == 0 || i.guid == guid))
        .map_or(-1, |index| (index as u16) as i16)
}

/// The token 0x41f6fb reads at `place`. A place that is negative (compared
/// unsigned, so past the end) reads nothing, and the caller's own values
/// stand: kind 0, GUID 0, count 1 (as 0x45a2ef..0x45a2f9 and 0x450b81..
/// 0x450b87 set them).
#[must_use]
pub fn read_token(items: &[Item], place: i16) -> Item {
    usize::try_from(place)
        .ok()
        .and_then(|at| items.get(at))
        .cloned()
        .unwrap_or(Item {
            kind: 0,
            guid: 0,
            count: 1,
        })
}

/// 0x41f56b(place, amount): an amount of 0 does nothing; a place that is
/// negative or past the end (compared unsigned) does nothing; a negative
/// amount erases the token; otherwise the count drops by the amount, and
/// the token is erased when nothing is left.
pub fn take_token(items: &mut Vec<Item>, place: i16, amount: i16) {
    if amount == 0 {
        return;
    }
    let Some(at) = usize::try_from(place).ok().filter(|&at| at < items.len()) else {
        return;
    };
    if amount < 0 {
        items.remove(at);
        return;
    }
    let remaining = i32::from(items[at].count as i16) - i32::from(amount);
    if remaining > 0 {
        items[at].count = remaining as u16;
    } else {
        items.remove(at);
    }
}

/// The arrival clock 0x45a785 works out: with `absolute` (a residential lot
/// leaving for houses 40 to 49) the hour and minute are the time itself.
/// Otherwise they are added to the clock as words: 60 minutes or more carry
/// one hour and keep the rest (0x45a7c3), then 24 hours or more wrap
/// (0x45a7e2). Nothing wraps below zero.
#[must_use]
pub fn arrival_clock(
    clock_hour: i16,
    clock_minute: i16,
    hour: i16,
    minute: i16,
    absolute: bool,
) -> (i16, i16) {
    if absolute {
        return (hour, minute);
    }
    let mut hours = clock_hour;
    let mut minutes = clock_minute.wrapping_add(minute);
    if minutes >= 60 {
        hours = hours.wrapping_add(1);
        minutes %= 60;
    }
    hours = hours.wrapping_add(hour);
    if hours >= 24 {
        hours %= 24;
    }
    (hours, minutes)
}

impl Machine<'_, '_> {
    /// 0x45a22d(house, hour, minute, mode) for the running person, as
    /// Generic Sims Call 17 calls it with temps 0 to 3. Returns the request
    /// 0x4fc713 records, or None when it records none: the leader has no
    /// neighbor record, or the mode is neither 0 nor 1.
    ///
    /// 1. The leader's record is the first with its type's GUID (0x42571f);
    ///    without one the routine returns at once. Its inventory is found,
    ///    or added empty (0x42a8a1).
    /// 2. House -1 is the trip home: the house becomes the house (+0x110)
    ///    of the leader's family (person word 61, 0x425d69). The game's
    ///    family becoming family 0 (`game +0x10`) is not modeled: the lot
    ///    is left before anything reads it.
    /// 3. Mode 0 takes a companion: the first token of kind 3 on the way
    ///    home, kind 1 on the way out, any GUID (0x41f69c). Its GUID must
    ///    name a person type in the interface's table (0x4e9753) and an
    ///    on-lot person of it (0x4e9608); that person's record's inventory
    ///    is used (0x4572af, 0x42571f, 0x42a8a1). Mode 1 takes the leader's
    ///    family; a missing family writes the leader alone.
    /// 4. The companion's saved state is written (0x45a3f7). Then, with a
    ///    family, each member on the lot (0x44093d) is saved to its record
    ///    (0x425a69) and its saved state written (0x45a5f9); member 0's
    ///    inventory, or the leader's when member 0 is not on the lot, gets
    ///    the clock. Without one the leader's own saved state is written
    ///    and its inventory gets the clock (0x45a507).
    /// 5. The clock ([`arrival_clock`]) goes in as tokens 7 and 8. It is
    ///    absolute when the lot in global 10 is zoned residential (0x40745f
    ///    → 0x42b9e3, where lot 0 is residential) and `house` (as passed)
    ///    is 40 to 49.
    /// 6. The neighborhood is saved (0x4252cc). The world holds the
    ///    inventories, and they are written when the lot is saved, so that
    ///    is not modeled.
    pub(crate) fn request_trip(
        &mut self,
        house: i16,
        hour: i16,
        minute: i16,
        mode: i16,
    ) -> Result<Option<TripRequest>, Trap> {
        let leader = self.me;
        let leader_guid = self
            .world
            .objects
            .get(&leader)
            .ok_or(Trap::native(10, "missing object"))?
            .guid;
        let Some(leader_record) = self.world.neighbors.first_of(leader_guid).map(|n| n.id) else {
            return Ok(None);
        };
        let family_id = self.trip_person_word(leader, PERSON_FAMILY)?;

        let (destination, companion_kind) = if house == -1 {
            let home = self.trip_family(family_id)?.house;
            (home as i16, HOMEBOUND_COMPANION_KIND)
        } else {
            (house, OUTBOUND_COMPANION_KIND)
        };

        // Mode 1: the family's members on the lot and their records (0x45a3ef;
        // a missing family is null, and 0x45a4fc then writes the leader
        // alone). They are found before anything is added or written: the
        // original reads through a missing record (0x45a662) and crashes, so
        // a trap here must leave the world as it was.
        let mut members = None;
        if mode == 1
            && let Some(family) = self
                .world
                .families
                .iter()
                .find(|f| f.id == i32::from(family_id))
        {
            let mut found = Vec::new();
            for (index, &guid) in family.members.iter().enumerate() {
                let Some(person) = self.first_person_of_type(guid) else {
                    continue;
                };
                let record = self
                    .world
                    .neighbors
                    .first_of(guid)
                    .map(|n| n.id)
                    .ok_or_else(|| {
                        Trap::Invalid(format!(
                            "family member 0x{guid:08X} has no neighbor record \
                             (0x45a662 reads through a null record)"
                        ))
                    })?;
                found.push((index, person, record));
            }
            members = Some(found);
        }

        let leader_inventory = self.inventory_slot(leader_record);
        let mut companion = None;
        if mode == 0 {
            let items = &self.world.neighbors.inventories[leader_inventory].items;
            let place = token_place(items, companion_kind, 0);
            if place != -1 {
                let guid = read_token(items, place).guid;
                if let Some(person) = self.portrait_person(guid)? {
                    let person_guid = self.world.objects[&person].guid;
                    let record = self
                        .world
                        .neighbors
                        .first_of(person_guid)
                        .map(|n| n.id)
                        .ok_or_else(|| {
                            Trap::Invalid(format!(
                                "the companion 0x{person_guid:08X} has no neighbor record \
                                 (0x45a3c3 reads through a null record)"
                            ))
                        })?;
                    companion = Some((person, self.inventory_slot(record)));
                }
            }
        }

        // The neighbor ids whose inventories get the saved state.
        let mut saved = Vec::new();
        if let Some((person, inventory)) = companion {
            saved.push(self.write_saved_state(person, inventory)?);
        }
        let clock_inventory = if let Some(members) = members {
            let mut first = leader_inventory;
            for (index, person, record) in members {
                people::save_to_record(self.world, person)?;
                let inventory = self.inventory_slot(record);
                if index == 0 {
                    first = inventory;
                }
                saved.push(self.write_saved_state(person, inventory)?);
            }
            first
        } else {
            saved.push(self.write_saved_state(leader, leader_inventory)?);
            leader_inventory
        };

        let global = |index: usize| self.world.globals.get(index).copied().unwrap_or(0);
        // 0x42b9e3 gives lot 0 zoning 1 and an unlisted lot zoning 0.
        let lot = global(GLOBAL_HOUSE);
        let residential = lot == 0 || self.program.zoning.get(&lot) == Some(&1);
        let absolute = residential && ABSOLUTE_CLOCK_HOUSES.contains(&house);
        let (arrival_hour, arrival_minute) = arrival_clock(
            global(GLOBAL_HOURS),
            global(GLOBAL_MINUTES),
            hour,
            minute,
            absolute,
        );
        let items = &mut self.world.neighbors.inventories[clock_inventory].items;
        add_token(items, SAVED_STATE_KIND, ARRIVAL_HOUR_GUID, arrival_hour);
        add_token(items, SAVED_STATE_KIND, ARRIVAL_MINUTE_GUID, arrival_minute);

        Ok(match mode {
            0 => Some(TripRequest {
                house: destination,
                leader_guid,
                family: 0,
                caller: leader,
                saved,
            }),
            1 => Some(TripRequest {
                house: destination,
                leader_guid: 0,
                family: family_id,
                caller: leader,
                saved,
            }),
            _ => None,
        })
    }

    /// Erases every saved-state token of `inventory` (0x41f4e5(2)), then
    /// writes `person`'s, in [`SAVED_STATE`]'s order (0x41f38d). Returns the
    /// inventory's neighbor id.
    fn write_saved_state(&mut self, person: u16, inventory: usize) -> Result<i16, Trap> {
        let data = self
            .world
            .objects
            .get(&person)
            .and_then(|o| o.person.as_ref())
            .ok_or(Trap::native(0x1c, "object is not a person"))?;
        let mut counts = [(0u32, 0i16); SAVED_STATE.len()];
        for (slot, &(guid, field)) in counts.iter_mut().zip(SAVED_STATE.iter()) {
            let count =
                match field {
                    SavedField::Motive(motive) => {
                        ftol_word(data.motives.get(motive).copied().ok_or_else(|| {
                            Trap::Invalid(format!("a person has no motive {motive}"))
                        })?)
                    }
                    SavedField::Word(word) => data.data.get(word).copied().ok_or_else(|| {
                        Trap::Invalid(format!("a person has no data word {word}"))
                    })?,
                };
            *slot = (guid, count);
        }
        let inventory = &mut self.world.neighbors.inventories[inventory];
        inventory.items.retain(|i| i.kind != SAVED_STATE_KIND);
        for (guid, count) in counts {
            add_token(&mut inventory.items, SAVED_STATE_KIND, guid, count);
        }
        Ok(inventory.neighbor)
    }

    /// A person's data word.
    fn trip_person_word(&self, person: u16, word: usize) -> Result<i16, Trap> {
        self.world
            .objects
            .get(&person)
            .and_then(|o| o.person.as_ref())
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .data
            .get(word)
            .copied()
            .ok_or_else(|| Trap::Invalid(format!("a person has no data word {word}")))
    }

    /// The family with `id` (0x425d69). The original reads through the null
    /// it returns when there is none.
    fn trip_family(&self, id: i16) -> Result<&sims_sim::neighbors::Family, Trap> {
        self.world
            .families
            .iter()
            .find(|f| f.id == i32::from(id))
            .ok_or_else(|| {
                Trap::Invalid(format!(
                    "no family {id} (0x425d69 returns null and 0x45a22d reads through it)"
                ))
            })
    }

    /// The first person in the lot's person list whose type is `guid`
    /// (0x44093d, 0x4e9608).
    pub(crate) fn first_person_of_type(&self, guid: u32) -> Option<u16> {
        self.world
            .people
            .iter()
            .copied()
            .find(|id| self.world.objects.get(id).is_some_and(|o| o.guid == guid))
    }

    /// 0x4e9753 then 0x4e9608: the first person of type `guid`, if the
    /// type is in the interface's table of person types.
    fn portrait_person(&self, guid: u32) -> Result<Option<u16>, Trap> {
        if !self.in_portrait_table(guid)? {
            return Ok(None);
        }
        Ok(self.first_person_of_type(guid))
    }
}

/// The neighbor ID (person data word 31) of `person` in `world`, as
/// 0x45a22d and the routines that read tokens take it.
pub(crate) fn person_neighbor(world: &sims_sim::World, person: u16) -> Option<i16> {
    world
        .objects
        .get(&person)?
        .person
        .as_ref()?
        .data
        .get(PERSON_NEIGHBOR)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: i32, guid: u32, count: u16) -> Item {
        Item { kind, guid, count }
    }

    #[test]
    fn add_token_skips_zero_appends_and_saturates_only_upwards() {
        let mut items = Vec::new();
        assert!(add_token(&mut items, 2, 0, 0));
        assert!(items.is_empty(), "a count of 0 adds nothing");
        assert!(add_token(&mut items, 2, 0, 40));
        assert!(add_token(&mut items, 2, 1, -55));
        assert_eq!(items, vec![item(2, 0, 40), item(2, 1, (-55i16) as u16)]);
        assert!(add_token(&mut items, 2, 0, 0x7fff));
        assert_eq!(items[0].count, 0x7fff, "the sum is capped at 0x7fff");
        assert!(add_token(&mut items, 2, 1, -32000));
        assert_eq!(
            items[1].count as i16,
            (-55i16).wrapping_add(-32000),
            "nothing caps the sum below; the word wraps"
        );
        // A different kind with the same GUID is a different token.
        assert!(add_token(&mut items, 3, 0, 1));
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn add_token_refuses_a_full_inventory() {
        let mut items = vec![item(5, 1, 1); MOST_TOKENS];
        assert!(!add_token(&mut items, 2, 0, 1));
        assert_eq!(items.len(), MOST_TOKENS);
        assert!(
            add_token(&mut items, 5, 1, 2),
            "adding to a token still works"
        );
        assert_eq!(items[0].count, 3);
    }

    #[test]
    fn token_place_matches_any_guid_for_zero_and_wraps_as_a_word() {
        let items = vec![item(1, 9, 1), item(2, 7, 3), item(2, 8, 4)];
        assert_eq!(token_place(&items, 2, 0), 1);
        assert_eq!(token_place(&items, 2, 8), 2);
        assert_eq!(token_place(&items, 3, 0), -1);
        assert_eq!(read_token(&items, 2), item(2, 8, 4));
        assert_eq!(
            read_token(&items, -2),
            item(0, 0, 1),
            "a negative place reads nothing"
        );
        let mut many = vec![item(0, 0, 1); 0x8000];
        many.push(item(2, 7, 1));
        assert_eq!(token_place(&many, 2, 7), (0x8000u16) as i16);
    }

    #[test]
    fn take_token_decrements_erases_and_ignores_bad_places() {
        let mut items = vec![item(3, 1, 2), item(2, 7, 9)];
        take_token(&mut items, 0, 1);
        assert_eq!(items[0].count, 1);
        take_token(&mut items, 0, 1);
        assert_eq!(items, vec![item(2, 7, 9)], "the last one erases the token");
        take_token(&mut items, -5, -1);
        take_token(&mut items, 4, -1);
        take_token(&mut items, 0, 0);
        assert_eq!(items, vec![item(2, 7, 9)]);
        take_token(&mut items, 0, -1);
        assert!(items.is_empty(), "a negative amount erases outright");
    }

    #[test]
    fn arrival_clock_carries_minutes_and_wraps_hours() {
        assert_eq!(arrival_clock(9, 10, 0, 30, false), (9, 40));
        assert_eq!(arrival_clock(9, 45, 0, 30, false), (10, 15));
        assert_eq!(arrival_clock(23, 50, 0, 30, false), (0, 20));
        assert_eq!(arrival_clock(20, 0, 14, 0, false), (10, 0));
        assert_eq!(arrival_clock(23, 59, 0, 15, false), (0, 14));
        assert_eq!(
            arrival_clock(9, 45, 14, 0, true),
            (14, 0),
            "Vacation: absolute"
        );
        // 0x45a7c3 compares, then takes the rest once: 130 minutes carry
        // one hour, not two.
        assert_eq!(arrival_clock(1, 10, 0, 120, false), (2, 10));
    }

    #[test]
    fn ftol_truncates_toward_zero() {
        assert_eq!(ftol_word(57.9), 57);
        assert_eq!(ftol_word(-57.9), -57);
        assert_eq!(ftol_word(-0.4), 0);
        assert_eq!(ftol_word(f32::NAN), 0);
        // A 64-bit fistp cut to its low word: 2^33 + 1024 and 2^31 + 256 are
        // exact in f32, past the signed 32-bit range, with non-zero low words.
        assert_eq!(ftol_word(8_589_935_616.0), 1024);
        assert_eq!(ftol_word(2_147_483_904.0), 256);
        assert_eq!(ftol_word(-2_147_483_904.0), -256);
        // Outside the signed 64-bit range: integer indefinite, low word 0.
        assert_eq!(ftol_word(1.0e19), 0);
        assert_eq!(ftol_word(-1.0e19), 0);
        assert_eq!(ftol_word(f32::INFINITY), 0);
    }

    #[test]
    fn saved_state_table_matches_routine_21() {
        let guids: Vec<u32> = SAVED_STATE.iter().map(|&(g, _)| g).collect();
        assert_eq!(guids, [0, 1, 2, 3, 4, 5, 6, 9, 10, 11, 12]);
        assert_eq!(saved_field(7), None);
        assert_eq!(saved_field(8), None);
        assert_eq!(saved_field(13), None);
        assert_eq!(saved_field(3), Some(SavedField::Motive(15)));
        assert_eq!(saved_field(12), Some(SavedField::Word(81)));
    }
}

/// Generic Sims Call 17, 20, 22, 23 and 25 on the Goth house, run straight
/// through the interpreter. Need an install (`SIMS_INSTALL`); skipped
/// otherwise.
#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::boot::{Frames, Lot};
    use crate::interpreter::{Interpreter, Stream};
    use sims_sim::NeighborhoodStore;
    use std::path::PathBuf;

    struct Goth {
        lot: Lot,
        members: Vec<u32>,
        _db: Guard,
    }

    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn goth(name: &str) -> Option<Goth> {
        let Some(root) = std::env::var_os("SIMS_INSTALL").map(PathBuf::from) else {
            eprintln!("skipping: set SIMS_INSTALL to run the trip routine tests");
            return None;
        };
        let db = std::env::temp_dir().join(format!("sims-trip-{name}-{}.redb", std::process::id()));
        let _ = std::fs::remove_file(&db);
        let guard = Guard(db.clone());
        let store = NeighborhoodStore::open(&db).unwrap();
        store
            .import_directory("UserData", &root.join("UserData"), 0xff)
            .unwrap();
        let nh = store.load_neighborhood("UserData").unwrap().unwrap();
        let members = nh.family_in(5).unwrap().1.members.clone();
        let mut lot =
            crate::lifecycle::enter_lot(&root, &store, "UserData", 5, &[3], Frames::default())
                .unwrap();
        for _ in 0..5 {
            lot.step().unwrap();
        }
        Some(Goth {
            lot,
            members,
            _db: guard,
        })
    }

    impl Goth {
        fn person(&self, guid: u32) -> u16 {
            let world = &self.lot.world;
            world
                .people
                .iter()
                .copied()
                .find(|&id| world.objects[&id].guid == guid)
                .expect("the person is on the lot")
        }

        fn record(&self, guid: u32) -> i16 {
            self.lot.world.neighbors.first_of(guid).unwrap().id
        }

        fn tokens(&self, guid: u32) -> Vec<Item> {
            let record = self.record(guid);
            self.lot
                .world
                .neighbors
                .inventories
                .iter()
                .find(|i| i.neighbor == record)
                .map(|i| i.items.clone())
                .unwrap_or_default()
        }

        fn set_tokens(&mut self, guid: u32, items: Vec<Item>) {
            let record = self.record(guid);
            let inventories = &mut self.lot.world.neighbors.inventories;
            inventories.retain(|i| i.neighbor != record);
            inventories.push(sims_sim::neighbors::Inventory {
                neighbor: record,
                items,
            });
        }

        fn set_temps(&mut self, me: u16, temps: &[i16]) {
            let object = self.lot.world.objects.get_mut(&me).unwrap();
            object.temps[..temps.len()].copy_from_slice(temps);
        }

        fn temps(&self, me: u16) -> [i16; 4] {
            let temps = &self.lot.world.objects[&me].temps;
            [temps[0], temps[1], temps[2], temps[3]]
        }

        fn call(&mut self, me: u16, routine: u16) -> Result<bool, Trap> {
            let interpreter =
                Interpreter::new(&self.lot.program, &self.lot.catalog, Stream::Global);
            let mut operands = [0u8; 8];
            operands[..2].copy_from_slice(&routine.to_le_bytes());
            interpreter
                .machine(&mut self.lot.world, me, 0, true)
                .generic_call(&operands)
        }

        fn person_mut(&mut self, id: u16) -> &mut sims_formats::saved_objects::Person {
            self.lot
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
        }
    }

    fn saved(items: &[Item]) -> Vec<(u32, i16)> {
        items
            .iter()
            .filter(|i| i.kind == SAVED_STATE_KIND)
            .map(|i| (i.guid, i.count as i16))
            .collect()
    }

    #[test]
    fn routine_17_writes_the_leaders_saved_state_and_clock_and_asks_for_the_trip() {
        let Some(mut goth) = goth("r17") else {
            return;
        };
        let mortimer_guid = goth.members[0];
        let mortimer = goth.person(mortimer_guid);
        {
            let person = goth.person_mut(mortimer);
            person.motives[9] = 0.4; // cut to 0: no token
            person.motives[6] = -12.75;
            person.motives[5] = 63.9;
            person.data[8] = 3;
            person.data[81] = 15;
        }
        goth.set_tokens(
            mortimer_guid,
            vec![
                Item {
                    kind: 2,
                    guid: 99,
                    count: 5,
                },
                Item {
                    kind: 0,
                    guid: 0xfeed,
                    count: 2,
                },
            ],
        );
        goth.lot.world.globals[0] = 23;
        goth.lot.world.globals[5] = 45;
        goth.set_temps(mortimer, &[21, 0, 30, 0]);
        assert_eq!(goth.call(mortimer, 17), Ok(true));

        assert_eq!(
            goth.lot.world.trip_request,
            Some(TripRequest {
                house: 21,
                leader_guid: mortimer_guid,
                family: 0,
                caller: mortimer,
                saved: vec![goth.record(mortimer_guid)],
            })
        );
        let person = goth.lot.world.objects[&mortimer].person.clone().unwrap();
        let motive = |m: usize| ftol_word(person.motives[m]);
        let tokens = goth.tokens(mortimer_guid);
        assert_eq!(
            tokens[0],
            Item {
                kind: 0,
                guid: 0xfeed,
                count: 2
            },
            "other kinds stay"
        );
        let mut expected = vec![
            (1, -12),
            (2, 63),
            (3, motive(15)),
            (4, motive(7)),
            (5, motive(8)),
            (6, motive(14)),
            (9, 3),
            (11, person.data[82]),
            (12, 15),
            // 23:45 and half an hour is 00:15.
            (7, 0),
            (8, 15),
        ];
        expected.retain(|&(_, count)| count != 0);
        if person.data[80] != 0 {
            expected.insert(7, (10, person.data[80]));
        }
        assert_eq!(saved(&tokens), expected);
    }

    #[test]
    fn routine_17_mode_1_saves_every_member_and_vacation_times_are_absolute() {
        let Some(mut goth) = goth("r17-family") else {
            return;
        };
        let leader = goth.person(goth.members[1]);
        let family = goth.lot.world.objects[&leader]
            .person
            .as_ref()
            .unwrap()
            .data[61];
        goth.person_mut(leader).data[9] = 777;
        assert_eq!(goth.lot.world.globals[10], 5, "the Goth house is lot 5");
        goth.set_temps(leader, &[40, 14, 0, 1]);
        assert_eq!(goth.call(leader, 17), Ok(true));
        assert_eq!(
            goth.lot.world.trip_request,
            Some(TripRequest {
                house: 40,
                leader_guid: 0,
                family,
                caller: leader,
                saved: goth.members.iter().map(|&g| goth.record(g)).collect(),
            })
        );
        // Each member on the lot was saved to its record (0x425a69).
        let leader_record = goth.record(goth.members[1]);
        assert_eq!(
            goth.lot.world.neighbors.get(leader_record).unwrap().data[9],
            777
        );
        let members = goth.members.clone();
        for (index, &guid) in members.iter().enumerate() {
            let tokens = saved(&goth.tokens(guid));
            let guids: Vec<u32> = tokens.iter().map(|&(g, _)| g).collect();
            assert!(guids.contains(&2), "member {index} has its saved state");
            // A residential lot leaving for 40..49: the clock is absolute,
            // and member 0 holds it.
            assert_eq!(
                tokens.iter().any(|&(g, c)| g == 7 && c == 14),
                index == 0,
                "member {index}: {tokens:?}"
            );
            assert!(!guids.contains(&8), "a minute of 0 adds no token");
        }
    }

    #[test]
    fn routine_17_keeps_a_pending_request_and_reads_lot_0_as_residential() {
        let Some(mut goth) = goth("r17-pending") else {
            return;
        };
        let mortimer_guid = goth.members[0];
        let mortimer = goth.person(mortimer_guid);
        let pending = TripRequest {
            house: 58,
            leader_guid: 0,
            family: 1,
            caller: 3,
            saved: vec![-7],
        };
        goth.lot.world.trip_request = Some(pending.clone());
        goth.lot.world.globals[0] = 9;
        goth.lot.world.globals[5] = 10;
        // 0x42b9e3 gives lot 0 zoning 1: leaving for 40..49 is absolute.
        goth.lot.world.globals[10] = 0;
        goth.set_temps(mortimer, &[40, 14, 0, 0]);
        assert_eq!(goth.call(mortimer, 17), Ok(true));
        assert_eq!(
            goth.lot.world.trip_request,
            Some(TripRequest {
                saved: vec![-7, goth.record(mortimer_guid)],
                ..pending
            }),
            "0x4fc713 ignores a request while one is pending; its tokens are the trip's"
        );
        let clock = |tokens: &[(u32, i16)]| -> Vec<(u32, i16)> {
            tokens
                .iter()
                .copied()
                .filter(|&(g, _)| g == ARRIVAL_HOUR_GUID || g == ARRIVAL_MINUTE_GUID)
                .collect()
        };
        let tokens = saved(&goth.tokens(mortimer_guid));
        assert!(
            tokens.iter().any(|&(g, _)| g == 2),
            "the tokens are written"
        );
        assert_eq!(clock(&tokens), [(7, 14)]);

        // A lot missing from the zoning table is zoning 0: relative, 9:10
        // and 14 hours is 23:10.
        goth.lot.world.trip_request = None;
        goth.lot.world.globals[10] = 0x7f00;
        assert!(!goth.lot.program.zoning.contains_key(&0x7f00));
        assert_eq!(goth.call(mortimer, 17), Ok(true));
        assert_eq!(
            goth.lot.world.trip_request.as_ref().map(|r| r.house),
            Some(40)
        );
        let tokens = saved(&goth.tokens(mortimer_guid));
        // The leader's saved state erased the old kind-2 tokens first
        // (0x41f4e5(2)), the clock with them.
        assert_eq!(clock(&tokens), [(7, 23), (8, 10)]);
    }

    #[test]
    fn routine_17_mode_1_without_a_family_writes_the_leader_alone() {
        let Some(mut goth) = goth("r17-no-family") else {
            return;
        };
        let members = goth.members.clone();
        let leader = goth.person(members[1]);
        for &guid in &members {
            goth.set_tokens(guid, Vec::new());
        }
        assert!(!goth.lot.world.families.iter().any(|f| f.id == 0x7ff0));
        goth.person_mut(leader).data[61] = 0x7ff0;
        goth.lot.world.globals[0] = 9;
        goth.lot.world.globals[5] = 10;
        goth.set_temps(leader, &[21, 0, 30, 1]);
        assert_eq!(goth.call(leader, 17), Ok(true));
        assert_eq!(
            goth.lot.world.trip_request,
            Some(TripRequest {
                house: 21,
                leader_guid: 0,
                family: 0x7ff0,
                caller: leader,
                saved: vec![goth.record(members[1])],
            })
        );
        for (index, &guid) in members.iter().enumerate() {
            let tokens = saved(&goth.tokens(guid));
            if index == 1 {
                assert!(tokens.iter().any(|&(g, _)| g == 2), "{tokens:?}");
                assert!(tokens.contains(&(ARRIVAL_HOUR_GUID, 9)), "{tokens:?}");
                assert!(tokens.contains(&(ARRIVAL_MINUTE_GUID, 40)), "{tokens:?}");
            } else {
                assert!(tokens.is_empty(), "member {index}: {tokens:?}");
            }
        }
    }

    #[test]
    fn routine_17_on_the_way_home_takes_the_kind_3_companion_and_names_the_home_house() {
        let Some(mut goth) = goth("r17-home") else {
            return;
        };
        let mortimer_guid = goth.members[0];
        let companion_guid = goth.members[1];
        let mortimer = goth.person(mortimer_guid);
        goth.set_tokens(
            mortimer_guid,
            vec![
                Item {
                    kind: 1,
                    guid: goth.members[2],
                    count: 1,
                },
                Item {
                    kind: 3,
                    guid: companion_guid,
                    count: 1,
                },
            ],
        );
        goth.lot.world.globals[0] = 9;
        goth.lot.world.globals[5] = 10;
        goth.set_temps(mortimer, &[-1, 0, 30, 0]);
        assert_eq!(goth.call(mortimer, 17), Ok(true));
        let request = goth.lot.world.trip_request.clone().unwrap();
        assert_eq!(request.house, 5, "house -1 is the leader's family's house");
        assert_eq!(request.leader_guid, mortimer_guid);
        assert_eq!(
            request.saved,
            [goth.record(companion_guid), goth.record(mortimer_guid)],
            "the companion is saved first"
        );
        assert!(
            saved(&goth.tokens(companion_guid))
                .iter()
                .any(|&(g, _)| g == 2),
            "the kind-3 companion got its saved state"
        );
        assert!(
            saved(&goth.tokens(goth.members[2])).is_empty(),
            "the kind-1 token names nobody on the way home"
        );
        let leader = saved(&goth.tokens(mortimer_guid));
        assert!(leader.iter().any(|&(g, _)| g == 2));
        assert!(leader.ends_with(&[(7, 9), (8, 40)]), "{leader:?}");
    }

    #[test]
    fn routine_17_with_another_mode_writes_tokens_but_asks_for_nothing() {
        let Some(mut goth) = goth("r17-mode2") else {
            return;
        };
        let mortimer_guid = goth.members[0];
        let mortimer = goth.person(mortimer_guid);
        goth.lot.world.globals[0] = 9;
        goth.lot.world.globals[5] = 10;
        goth.set_temps(mortimer, &[21, 0, 30, 2]);
        assert_eq!(goth.call(mortimer, 17), Ok(true));
        assert_eq!(goth.lot.world.trip_request, None);
        assert!(saved(&goth.tokens(mortimer_guid)).ends_with(&[(7, 9), (8, 40)]));

        let phone = goth
            .lot
            .world
            .objects
            .values()
            .find(|o| o.person.is_none())
            .map(|o| o.id)
            .unwrap();
        assert_eq!(goth.call(phone, 17).unwrap_err().code(), Some(0x1c));
    }

    #[test]
    fn routine_20_brings_back_the_kind_3_companion_one_token_at_a_time() {
        let Some(mut goth) = goth("r20") else {
            return;
        };
        let mortimer_guid = goth.members[0];
        let mortimer = goth.person(mortimer_guid);
        let companion = goth.person(goth.members[1]);
        goth.set_tokens(
            mortimer_guid,
            vec![
                Item {
                    kind: 1,
                    guid: goth.members[2],
                    count: 1,
                },
                Item {
                    kind: 3,
                    guid: goth.members[1],
                    count: 2,
                },
            ],
        );
        let caller = companion;
        for left in [1, 0] {
            goth.set_temps(caller, &[mortimer as i16]);
            assert_eq!(goth.call(caller, 20), Ok(true));
            assert_eq!(goth.temps(caller)[0], companion as i16);
            let kind_3: Vec<u16> = goth
                .tokens(mortimer_guid)
                .iter()
                .filter(|i| i.kind == 3)
                .map(|i| i.count)
                .collect();
            assert_eq!(kind_3.iter().sum::<u16>(), left);
        }
        goth.set_temps(caller, &[mortimer as i16]);
        assert_eq!(goth.call(caller, 20), Ok(true));
        assert_eq!(goth.temps(caller)[0], -1, "no kind-3 token left");
        assert_eq!(
            goth.tokens(mortimer_guid).len(),
            1,
            "the kind-1 token stays"
        );

        goth.set_temps(caller, &[0]);
        assert_eq!(goth.call(caller, 20).unwrap_err().code(), Some(0x1c));
    }

    #[test]
    fn routine_19_makes_the_kind_1_companion_who_is_not_on_the_lot() {
        let Some(mut goth) = goth("r19") else {
            return;
        };
        let mortimer_guid = goth.members[0];
        let mortimer = goth.person(mortimer_guid);
        let family = goth.lot.world.objects[&mortimer]
            .person
            .as_ref()
            .unwrap()
            .data[61];
        let world = &goth.lot.world;
        let townie = world
            .neighbors
            .slots
            .iter()
            .flatten()
            .find(|n| {
                n.data.len() > 65
                    && n.data[61] != 0
                    && n.data[61] != family
                    && n.data[58] >= 18
                    && (0..=1).contains(&n.data[65])
                    && !world
                        .people
                        .iter()
                        .any(|id| world.objects[id].guid == n.guid)
            })
            .expect("an adult of another family, not on the lot")
            .guid;
        goth.set_tokens(
            mortimer_guid,
            vec![
                Item {
                    kind: 2,
                    guid: 3,
                    count: 50,
                },
                Item {
                    kind: 1,
                    guid: townie,
                    count: 1,
                },
            ],
        );
        goth.set_temps(mortimer, &[mortimer as i16]);
        assert_eq!(goth.call(mortimer, 19), Ok(true));
        let made = goth.temps(mortimer)[0];
        assert!(made > 0, "routine 19 made the companion");
        let made = made as u16;
        assert_eq!(goth.lot.world.objects[&made].guid, townie);
        assert!(goth.lot.world.people.contains(&made));
        assert_eq!(
            goth.tokens(mortimer_guid),
            vec![Item {
                kind: 2,
                guid: 3,
                count: 50
            }],
            "the companion token is taken"
        );
        // Run again, the token is gone: -1.
        goth.set_temps(mortimer, &[mortimer as i16]);
        assert_eq!(goth.call(mortimer, 19), Ok(true));
        assert_eq!(goth.temps(mortimer)[0], -1);
    }

    #[test]
    fn routine_22_is_native_error_0x2a() {
        let Some(mut goth) = goth("r22") else {
            return;
        };
        let mortimer = goth.person(goth.members[0]);
        assert_eq!(goth.call(mortimer, 22).unwrap_err().code(), Some(0x2a));
    }

    #[test]
    fn routine_23_reads_the_arrival_clock_and_erases_it() {
        let Some(mut goth) = goth("r23") else {
            return;
        };
        let mortimer_guid = goth.members[0];
        let mortimer = goth.person(mortimer_guid);
        let others = vec![
            Item {
                kind: 2,
                guid: 3,
                count: 50,
            },
            Item {
                kind: 7,
                guid: 8,
                count: 1,
            },
        ];
        let mut items = others.clone();
        items.push(Item {
            kind: 2,
            guid: 8,
            count: 45,
        });
        items.push(Item {
            kind: 2,
            guid: 7,
            count: 20,
        });
        goth.set_tokens(mortimer_guid, items);
        goth.set_temps(mortimer, &[-5, -6]);
        assert_eq!(goth.call(mortimer, 23), Ok(true));
        assert_eq!(goth.temps(mortimer)[..2], [20, 45]);
        assert_eq!(goth.tokens(mortimer_guid), others);

        // Only an hour: temp 0 gets it, temp 1 stays.
        let mut items = others.clone();
        items.push(Item {
            kind: 2,
            guid: 7,
            count: 9,
        });
        goth.set_tokens(mortimer_guid, items);
        goth.set_temps(mortimer, &[-5, -6]);
        assert_eq!(goth.call(mortimer, 23), Ok(true));
        assert_eq!(goth.temps(mortimer)[..2], [9, -6]);
        assert_eq!(goth.tokens(mortimer_guid), others);

        // No hour: nothing is read, not even a minute.
        let mut items = others.clone();
        items.push(Item {
            kind: 2,
            guid: 8,
            count: 30,
        });
        goth.set_tokens(mortimer_guid, items.clone());
        goth.set_temps(mortimer, &[-5, -6]);
        assert_eq!(goth.call(mortimer, 23), Ok(true));
        assert_eq!(goth.temps(mortimer)[..2], [-5, -6]);
        assert_eq!(goth.tokens(mortimer_guid), items);
    }

    #[test]
    fn routine_25_saves_the_person_to_its_record() {
        let Some(mut goth) = goth("r25") else {
            return;
        };
        let mortimer_guid = goth.members[0];
        let mortimer = goth.person(mortimer_guid);
        goth.person_mut(mortimer).data[9] = 654;
        let record = goth.record(mortimer_guid);
        assert_ne!(goth.lot.world.neighbors.get(record).unwrap().data[9], 654);
        assert_eq!(goth.call(mortimer, 25), Ok(true));
        let saved = goth.lot.world.neighbors.get(record).unwrap();
        assert_eq!(saved.data[9], 654);
        assert_eq!(saved.revision, crate::people::SAVED_REVISION);
    }
}
