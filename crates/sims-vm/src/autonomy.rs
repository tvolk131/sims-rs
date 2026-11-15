//! Find Best Interaction (primitive 3, 0x45dd39): a person picks, by
//! itself, an interaction with an object on the lot.
//!
//! An *advertisement* is what an interaction promises a person's motives:
//! for each motive, an amount it would add. An interaction table (TTAB)
//! entry holds sixteen of them. The person weighs each motive with a
//! *score curve*, and the *score* of a set of motive values is the mean of
//! the curves' values.
//!
//! The primitive runs synchronously and takes the true branch when it has
//! chosen; it then has written the interaction's menu entry into the
//! person's object data word 20 and the object into the frame's stack
//! object.
//!
//! 1. **Gates** (0x45dd46 to 0x45dde2). See [`Machine::find_best_interaction`].
//! 2. **The baseline**: the score of the person's motives as they are
//!    (0x42144e), stored as a float.
//! 3. **The scan** (0x45dedc to 0x45e1a7): every object by rising ID, but
//!    those without an interaction table, those whose scheduler flag
//!    "advertises" is clear, the person itself (unless it is a pet), and
//!    those whose data word 25 is above zero. A lot that is not zoned
//!    scans a quarter of the objects each time, which is not ported.
//! 4. For each object, its interaction target's table entries that the
//!    person may choose ([`Machine::autonomy_entries`], 0x458826), from
//!    the last entry to the first.
//! 5. For each entry, whether it is allowed now
//!    ([`Machine::autonomy_allows`], 0x458319): rules on the person and
//!    the object, then the entry's check tree, run synchronously on the
//!    person's stack.
//! 6. An allowed entry's *improvement* is its score with the
//!    advertisements added, less the baseline, times the attenuation
//!    factor `1 / (attenuation · distance + 1)`. An entry whose score is
//!    exactly 0 is dropped; one whose improvement is at least the
//!    threshold (family or visitor) is a candidate.
//! 7. The candidates are sorted by falling improvement with the C
//!    runtime's `qsort` ([`sort`], 0x6356ba), whose order of equal
//!    elements the choice depends on. A visitor, or a first candidate
//!    whose entry has flag bit 8, takes the first candidate on a
//!    residential lot. Otherwise one draw from the active random stream
//!    picks among the first few (0x45e1de to 0x45e258).
//! 8. A person whose person data word 0 is not 0 refuses a candidate below
//!    the sitting threshold; the primitive then fails.
//!
//! The FPU runs in single precision here (`docs/decisions.md`): every add,
//! subtract, multiply and divide rounds to 24 bits.
//!
//! Ported from the original's code at the addresses given, with the old
//! prototype's `select`, `sort` and `Enumeration::permits`
//! (legacy/crates/sims-vm/src/autonomy.rs), `Query::score`
//! (legacy/crates/sims-vm/src/autonomy/query.rs) and `score_extended`
//! (legacy/crates/sims-vm/src/motives.rs) as guides. Every step was
//! compared with the original's code run on a memory image of the Goth
//! house at the entry of its update 233, where person 21 runs it
//! (`docs/decisions.md`).

use crate::factory::{TURN_BUDGET, flag};
use crate::fault::Trap;
use crate::interpreter::{Flow, Machine};
use crate::primitives::distance;
use sims_core::{Ext, F32};
use sims_formats::behavior::{Interaction, InteractionTable};
use sims_sim::motives::Curve;
use std::cmp::Ordering;

/// The motives the score curves weigh, in the order of their string
/// table's entries.
pub const SCORE_MOTIVES: [usize; 9] = [5, 6, 7, 8, 9, 3, 13, 14, 15];

/// How many points a score curve holds at most.
pub const SCORE_POINTS: usize = 4;

/// The constants Find Best Interaction reads ([`crate::Program::autonomy_constants`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constants {
    /// The least improvement a resident's candidate needs (0x6c4384).
    pub family_threshold: F32,
    /// The same for a visitor (0x6c4388).
    pub visitor_threshold: F32,
    /// The attenuations of codes 2, 3 and 4 (0x6c4d80).
    pub attenuation: [F32; 3],
    /// The same for a visitor (0x6c4d8c).
    pub visitor_attenuation: [F32; 3],
    /// The least improvement a person whose word 0 is set takes
    /// (0x6c438c).
    pub sitting_threshold: F32,
    /// How many of the best candidates the draw picks among (0x6c4390).
    pub random_count: i32,
}

/// The attenuation of an entry whose code is none of 0 to 4 (0x667368).
const DEFAULT_ATTENUATION: u32 = 0x3ca3_d70a;

/// The float 0.001 (0x666a24 and 0x666b7c).
const THOUSANDTH: u32 = 0x3a83_126f;

/// The comparison's tolerance (0x66734c).
const EPSILON: u32 = 0x33d6_bf95;

/// How many advertisements the scratch record the original copies an
/// entry's into holds ([0x6c437c] +4).
pub(crate) const ADVERTISEMENTS: usize = 16;

/// Object data words.
pub(crate) mod data {
    /// Bit 9: the object takes no interactions now.
    pub const FLAGS: usize = 8;
    /// Takes the chosen interaction's menu entry.
    pub const CHOSEN: usize = 20;
    /// Above zero: the object is not to be chosen by itself.
    pub const IN_USE: usize = 25;
    pub const ROOM: usize = 29;
    /// Cleared before each entry is looked at; a check tree may set it.
    pub const CHECKED: usize = 50;
    pub const CATEGORY: usize = 59;
}

/// Person data words.
pub(crate) mod person {
    /// Not 0: the sitting threshold applies.
    pub const POSTURE: usize = 0;
    /// Each 2000 takes one from the random selection count.
    pub const SELECTION: usize = 18;
    /// 0 for a member of the household that lives on the lot.
    pub const VISITOR: usize = 32;
    pub const PETS_OUTSIDE: usize = 34;
    /// The highest autonomy threshold of an entry the person takes.
    pub const AUTONOMY: usize = 36;
    pub const AGE: usize = 58;
    /// Bit 1: a dog; bit 2: a cat.
    pub const KIND: usize = 65;
}

/// Interaction table entry flags.
mod entry {
    pub const VISITORS: u32 = 1;
    /// Ruled out for a child.
    pub const NOT_CHILD: u32 = 0x10;
    /// Visitors may take it on a lot that is not residential.
    pub const COMMUNITY: u32 = 0x20;
    /// Ruled out for a human adult.
    pub const NOT_ADULT: u32 = 0x40;
    /// Never chosen by a person by itself.
    pub const NOT_AUTONOMOUS: u32 = 0x80;
    /// A visitor, or this entry as the best, skips the draw.
    pub const FIRST: u32 = 0x100;
    pub const CATS: u32 = 0x200;
    pub const DOGS: u32 = 0x400;
    /// Another person may join the interaction.
    pub const JOINABLE: u32 = 2;
}

/// Definition word 7: the interaction table's ID.
const TABLE: usize = 7;
/// Definition word 9: the object type. A person is 2; type 7 has no
/// distance and no interactions with people.
const TYPE: usize = 9;

/// Global 10: the lot's number.
const LOT: usize = 10;
/// Global 30: free will.
const FREE_WILL: usize = 30;
/// Global 9: the family that lives on the lot.
const FAMILY: usize = 9;

/// The person data word, and whether it is taken from 1000, that each
/// personality number of an advertisement scales it by (0x666a68).
const PERSONALITY: [(usize, bool); 22] = [
    (2, false),
    (2, true),
    (3, false),
    (3, true),
    (4, false),
    (4, true),
    (5, false),
    (5, true),
    (6, false),
    (6, true),
    (7, false),
    (7, true),
    (9, false),
    (10, false),
    (11, false),
    (12, false),
    (55, false),
    (55, false),
    (15, false),
    (55, false),
    (17, false),
    (18, false),
];

/// One advertisement as the original holds it: a personality number, a
/// minimum and an amount scaled by the personality.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Advertisement {
    pub personality: i16,
    pub minimum: i16,
    pub delta: i16,
}

impl Advertisement {
    /// A field by its place in the original's six bytes: 0 the
    /// personality number, 1 the minimum, 2 the delta.
    pub(crate) fn field(&self, field: usize) -> i16 {
        *[self.personality, self.minimum, self.delta]
            .get(field)
            .unwrap_or(&self.delta)
    }

    pub(crate) fn field_mut(&mut self, field: usize) -> &mut i16 {
        match field {
            0 => &mut self.personality,
            1 => &mut self.minimum,
            _ => &mut self.delta,
        }
    }
}

/// An entry the scan looks at (0x41f00e): the interaction target, the
/// entry's menu number (its TTAB string index), its attenuation and
/// whether its flag bit 8 is set. `join` marks the entry that joins a
/// person's running interaction (0x41ee99), whose word +0 is 1 rather
/// than 0; the check (0x458372) reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    target: u16,
    menu: u32,
    attenuation: F32,
    first: bool,
    join: bool,
}

/// The arguments of an entry's check tree (0x458354 to 0x458385): 1 (the
/// scan's flag), then 1 for the entry that joins a running interaction
/// and 0 for any other, then the join entry's word +0x18, which 0x41ee99
/// clears, and 0; then the tree's ID, the word that follows the four in
/// the original's memory.
fn check_arguments(join: bool, tree: u16) -> [i16; 5] {
    [1, i16::from(join), 0, 0, tree as i16]
}

/// A candidate (16 bytes at person +0x814): the object, the menu entry,
/// the improvement and the entry's flag bit 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub object: u16,
    pub menu: u32,
    pub improvement: F32,
    pub first: bool,
}

/// The score of a person's motives (0x42144e), or, with `ads`, of its
/// motives with the advertisements added (0x4214a1), left in an FPU
/// register. Each curve's motive value is `motive + amount · 0.001`
/// stored as a float, where the amount is the advertisement's minimum
/// plus its delta, or, with a personality number, `delta · word · 0.001 +
/// minimum` for the person data word the number names. The curves'
/// values are summed into a float, and the sum is divided by the number
/// of curves.
pub fn score(
    curves: &[Curve],
    motives: &[F32; 16],
    words: &[i16],
    ads: Option<&[Advertisement; ADVERTISEMENTS]>,
) -> Result<Ext, Trap> {
    let thousandth = F32::from_bits(THOUSANDTH).ext();
    let mut sum = F32::ZERO;
    for curve in curves {
        let value = motives
            .get(curve.motive)
            .copied()
            .ok_or_else(|| Trap::Invalid(format!("no motive {}", curve.motive)))?;
        let value = match ads {
            None => value,
            Some(ads) => {
                let ad = ads
                    .get(curve.motive)
                    .ok_or_else(|| Trap::Invalid(format!("no advertisement {}", curve.motive)))?;
                let amount = if ad.personality != 0 {
                    let &(word, inverted) = usize::try_from(ad.personality)
                        .ok()
                        .and_then(|p| PERSONALITY.get(p.wrapping_sub(1)))
                        .ok_or_else(|| {
                            Trap::Unsupported(format!(
                                "personality number {} lies outside the original's table \
                                 (0x666a68)",
                                ad.personality
                            ))
                        })?;
                    let mut scale = i32::from(words.get(word).copied().unwrap_or(0));
                    if inverted {
                        scale = 1000 - scale;
                    }
                    let scaled =
                        (Ext::from_i32(i32::from(ad.delta)) * Ext::from_i32(scale)).single();
                    ((scaled * thousandth).single() + Ext::from_i32(i32::from(ad.minimum))).single()
                } else {
                    Ext::from_i32(i32::from(ad.minimum) + i32::from(ad.delta))
                };
                ((amount * thousandth).single() + value.ext()).to_f32()
            }
        };
        sum = (curve.weight(value) + sum.ext()).to_f32();
    }
    let count = i32::try_from(curves.len()).unwrap_or(i32::MAX);
    Ok((sum.ext() / Ext::from_i32(count)).single())
}

/// The comparison the candidates are sorted with (0x45e2fb): `b`'s
/// improvement less `a`'s, rounded to single precision; above the
/// tolerance `a` goes after `b`, below minus the tolerance before it, and
/// otherwise they are equal. A difference that is not a number puts `a`
/// first.
fn compare(a: &Candidate, b: &Candidate) -> i32 {
    let epsilon = F32::from_bits(EPSILON).ext();
    let difference = (b.improvement.ext() - a.improvement.ext()).single();
    if difference.partial_cmp(&epsilon) == Some(Ordering::Greater) {
        1
    } else if matches!(
        difference.partial_cmp(&-epsilon),
        Some(Ordering::Greater | Ordering::Equal)
    ) {
        0
    } else {
        -1
    }
}

/// The C runtime's `qsort` (0x6356ba) with the candidates' comparison:
/// below nine elements a selection sort that moves the greatest to the
/// end (0x63580e); otherwise the middle element is swapped to the front
/// as the pivot and the rest partitioned around it, and each side sorted
/// the same way. Ported from the old prototype's `sort`
/// (legacy/crates/sims-vm/src/autonomy.rs) and checked against the code.
pub fn sort(candidates: &mut [Candidate]) {
    let n = candidates.len();
    if n < 2 {
        return;
    }
    if n <= 8 {
        for end in (1..n).rev() {
            let mut greatest = 0;
            for i in 1..=end {
                if compare(&candidates[i], &candidates[greatest]) > 0 {
                    greatest = i;
                }
            }
            candidates.swap(greatest, end);
        }
        return;
    }
    candidates.swap(0, n / 2);
    let (mut low, mut high) = (0, n);
    loop {
        low += 1;
        while low < n && compare(&candidates[low], &candidates[0]) <= 0 {
            low += 1;
        }
        high -= 1;
        while high > 0 && compare(&candidates[high], &candidates[0]) >= 0 {
            high -= 1;
        }
        if high < low {
            break;
        }
        candidates.swap(low, high);
    }
    candidates.swap(0, high);
    sort(&mut candidates[..high]);
    if low < n {
        sort(&mut candidates[low..]);
    }
}

/// What the scan needs of the person.
struct Chooser {
    me: u16,
    visitor: bool,
    child: bool,
    dog: bool,
    cat: bool,
    pets_outside: bool,
    autonomy: i16,
    room: i16,
    residential: bool,
    constants: Constants,
}

impl Machine<'_, '_> {
    /// Find Best Interaction (0x45dd39). See the module's documentation.
    ///
    /// The gates, before anything is written:
    ///
    /// - With free will off (global 30, which the lot's setup copies from
    ///   the option byte 0x6c2980 the original reads, 0x46d24c), a
    ///   resident that is not a pet fails. A pet, or a visitor, fails
    ///   loudly: the original then reads interface state that is not
    ///   modeled (0x6e4f68, and the family the player controls).
    /// - The original also fails for the first member of the family the
    ///   player controls when that family's number is 0, and while two
    ///   interface flags are set (0x6f2a54 here, 0x6e5134 in the
    ///   enumeration). The family's number is taken to be global 9, and
    ///   global 9 being 0 fails loudly; the flags, which only interface
    ///   code writes, are taken to be clear. A memory image of the
    ///   original at its first call agrees with both.
    pub(crate) fn find_best_interaction(&mut self) -> Result<Flow, Trap> {
        let me = self.me;
        let object = self.me()?;
        let person = object
            .person
            .as_ref()
            .ok_or_else(|| Trap::Unsupported("Find Best Interaction on a non-person".into()))?;
        let word = |index: usize| person.data.get(index).copied().unwrap_or(0);
        let kind = word(person::KIND);
        let (dog, cat) = (kind & 2 != 0, kind & 4 != 0);
        let visitor = word(person::VISITOR) != 0;
        let global = |index: usize| self.world.globals.get(index).copied().unwrap_or(0);
        if global(FREE_WILL) == 0 {
            if !visitor && !dog && !cat {
                return Ok(Flow::Branch(false));
            }
            return Err(Trap::Unsupported(
                "Find Best Interaction with free will off for a pet or a visitor, which reads \
                 interface state (0x6e4f68, the family the player controls)"
                    .into(),
            ));
        }
        if global(FAMILY) == 0 {
            return Err(Trap::Unsupported(
                "Find Best Interaction with no family on the lot, where the original compares \
                 the person with the controlled family's first member (0x4e9608)"
                    .into(),
            ));
        }
        let lot = global(LOT);
        let zoning = self.zoning(lot);
        if zoning == 2 || (40..50).contains(&lot) {
            return Err(Trap::Unsupported(format!(
                "Find Best Interaction on lot {lot}, whose scan has rules for community lots \
                 and lots 40 to 49 that are not ported (0x45e2db, 0x4074c8)"
            )));
        }
        if zoning == 0 {
            return Err(Trap::Unsupported(format!(
                "Find Best Interaction on lot {lot}, which is not zoned: the original then \
                 scans a quarter of the objects (0x45de63), which is not ported"
            )));
        }
        let custom = self.definition_words(object.guid)?[51] as u16;
        let (global_table, table) = crate::people::curve_table(word(person::AGE), custom);
        let resource = self.definition(object.guid)?.resource;
        let curves = self
            .program
            .score_curves(global_table, resource, table)
            .map_err(Trap::Unsupported)?;
        let motives = person.motives.map(F32::from_f32);
        let baseline = score(&curves, &motives, &person.data, None)?.to_f32();
        let constants = self.program.autonomy_constants();
        let threshold = if visitor {
            constants.visitor_threshold
        } else {
            constants.family_threshold
        };
        let age = word(person::AGE);
        let chooser = Chooser {
            me,
            visitor,
            child: age != 0 && age < 18,
            dog,
            cat,
            pets_outside: word(person::PETS_OUTSIDE) != 0,
            autonomy: word(person::AUTONOMY),
            room: object.data.get(data::ROOM).copied().unwrap_or(0),
            residential: zoning == 1,
            constants,
        };
        let (posture, selection) = (word(person::POSTURE), word(person::SELECTION));
        let position = object.position;
        let ids: Vec<u16> = self.world.objects.keys().copied().collect();
        let mut saved = None;
        let mut candidates = Vec::new();
        let scanned = self.scan(
            &chooser,
            &ids,
            position,
            &curves,
            baseline,
            threshold,
            &mut saved,
            &mut candidates,
        );
        // The enumeration's destructor puts the temps back (0x4582dd).
        if let Some(temps) = saved {
            self.me_mut()?.temps = temps;
        }
        scanned?;
        if candidates.is_empty() {
            return Ok(Flow::Branch(false));
        }
        sort(&mut candidates);
        let n = candidates.len();
        let count = constants
            .random_count
            .wrapping_sub(i32::from(selection) / 2000)
            .max(1) as u32;
        let index = if (visitor || candidates[0].first) && chooser.residential {
            0
        } else {
            let draw = self.draw();
            (draw % if count > n as u32 { n as u32 } else { count }) as usize
        };
        let chosen = candidates[index];
        if posture != 0
            && chosen
                .improvement
                .ext()
                .partial_cmp(&constants.sitting_threshold.ext())
                != Some(Ordering::Greater)
            && chosen
                .improvement
                .ext()
                .partial_cmp(&constants.sitting_threshold.ext())
                != Some(Ordering::Equal)
        {
            return Ok(Flow::Branch(false));
        }
        self.me_mut()?.data[data::CHOSEN] = chosen.menu as i16;
        self.frame_mut()?.stack_object = chosen.object as i16;
        Ok(Flow::Branch(true))
    }

    /// The lot's zoning (0x42b9e3): lot 0 is residential, and any other
    /// lot has the neighborhood's zoning, or 0 if it has none.
    fn zoning(&self, lot: i16) -> i32 {
        if lot == 0 {
            1
        } else {
            self.program.zoning.get(&lot).copied().unwrap_or(0)
        }
    }

    /// One draw from the active random stream (0x46f457).
    pub(crate) fn draw(&mut self) -> u32 {
        match self.stream {
            crate::interpreter::Stream::Lot => self.world.lot_rng.next_u32(),
            crate::interpreter::Stream::Global => self.world.global_rng.next_u32(),
        }
    }

    /// The scan (0x45dedc to 0x45e1a7). `saved` takes the person's temps
    /// before the first check tree runs.
    #[allow(clippy::too_many_arguments)]
    fn scan(
        &mut self,
        chooser: &Chooser,
        ids: &[u16],
        position: [i32; 3],
        curves: &[Curve],
        baseline: F32,
        threshold: F32,
        saved: &mut Option<Vec<i16>>,
        candidates: &mut Vec<Candidate>,
    ) -> Result<(), Trap> {
        for &id in ids {
            if !self.world.objects.keys().copied().eq(ids.iter().copied()) {
                return Err(Trap::Unsupported(
                    "a check tree made or removed an object during Find Best Interaction's \
                     scan"
                        .into(),
                ));
            }
            let object = &self.world.objects[&id];
            let words = self.definition_words(object.guid)?;
            if words[TABLE] == 0 || object.flags() & flag::ADVERTISES == 0 {
                continue;
            }
            if id == chooser.me && !chooser.dog && !chooser.cat {
                continue;
            }
            if object.data.get(data::IN_USE).is_some_and(|&w| w > 0) {
                continue;
            }
            let target = self.interaction_target(id)?;
            let entries = self.autonomy_entries(chooser, target)?;
            if entries.is_empty() {
                continue;
            }
            let object = &self.world.objects[&id];
            let zero = words[TYPE] == 7;
            let distance = distance(position, object.position, zero).to_f32();
            for entry in entries {
                let allows = self.autonomy_allows(chooser, entry, saved)?;
                let Some(ads) = allows else {
                    continue;
                };
                let one = Ext::from_i32(1);
                let factor = (one
                    / ((entry.attenuation.ext() * distance.ext()).single() + one).single())
                .to_f32();
                let words = &self.me()?.person.as_ref().map(|p| p.data.clone());
                let words = words.as_deref().unwrap_or(&[]);
                let motives = self
                    .me()?
                    .person
                    .as_ref()
                    .map(|p| p.motives.map(F32::from_f32))
                    .unwrap_or([F32::ZERO; 16]);
                let prospective = score(curves, &motives, words, Some(&ads))?;
                if matches!(
                    prospective.partial_cmp(&Ext::ZERO),
                    None | Some(Ordering::Equal)
                ) {
                    continue;
                }
                let improvement = ((prospective - baseline.ext()).single() * factor.ext()).single();
                if !matches!(
                    improvement.partial_cmp(&threshold.ext()),
                    Some(Ordering::Greater | Ordering::Equal)
                ) {
                    continue;
                }
                candidates.push(Candidate {
                    object: entry.target,
                    menu: entry.menu,
                    improvement: improvement.to_f32(),
                    first: entry.first,
                });
            }
        }
        Ok(())
    }

    /// An object's interaction table, as its definition names it (0x457cb0).
    pub(crate) fn table(&self, id: u16) -> Result<Option<&'_ InteractionTable>, Trap> {
        let object = self
            .world
            .objects
            .get(&id)
            .ok_or(Trap::native(0x15, "missing object"))?;
        let definition = self.definition(object.guid)?;
        let table = self
            .program
            .interactions(definition.resource, definition.chunk)
            .map_err(Trap::Unsupported)?;
        if let Some(table) = table
            && table.version < 7
        {
            return Err(Trap::Unsupported(format!(
                "an interaction table of version {}, whose advertisements the original's \
                 loader may fill in differently",
                table.version
            )));
        }
        Ok(table)
    }

    /// The entries of `target`'s table a person may choose by itself
    /// (0x458826), from the last to the first. An entry is left out if:
    ///
    /// - its autonomy threshold, as a signed word, is not above 0, or
    ///   above the person's person data word 36;
    /// - the person is a dog and the entry lacks flag 0x400, or a cat and
    ///   it lacks 0x200;
    /// - the person is a child and the entry has flag 0x10, or a human
    ///   adult and it has 0x40;
    /// - it has flag 0x80;
    /// - the person is a visitor and the entry lacks flag 1, unless the
    ///   lot is not residential and the entry has flag 0x20;
    /// - on a residential lot, the person is a visiting pet whose person
    ///   word 34 is 0 and the target is in a room.
    ///
    /// A target whose type is 7 has no entries for a person that is not a
    /// pet.
    ///
    /// A person target (type 2) whose running interaction has the flag
    /// 0x2 (joinable) in its own table gets one more entry, after the
    /// others (0x458aee, 0x41ee99). That entry goes to the interaction
    /// target of the running interaction's target. Its menu number is the
    /// joining index of that target's first entry with the running
    /// interaction's menu number. The attenuation and flag bit 8 come as
    /// for any entry. If that first entry is missing, or the joining
    /// index is -1, or no entry has the joining index, the original
    /// leaves the entry without a target; the check (0x458319) then
    /// returns at once, before any side effect, so the entry is left out
    /// here. The rest of the entry is copied from the running action;
    /// the scan reads none of it but word +0 (see [`Entry`]).
    fn autonomy_entries(&self, chooser: &Chooser, target: u16) -> Result<Vec<Entry>, Trap> {
        let object = &self.world.objects[&target];
        let kind = self.definition_words(object.guid)?[TYPE];
        if kind == 7 && !chooser.dog && !chooser.cat {
            return Ok(Vec::new());
        }
        let Some(table) = self.table(target)? else {
            return Ok(Vec::new());
        };
        let room = object.data.get(data::ROOM).copied().unwrap_or(0);
        let mut entries = Vec::new();
        for interaction in table.interactions.iter().rev() {
            let flags = interaction.flags;
            let threshold = interaction.autonomy as u16 as i16;
            if threshold <= 0 {
                continue;
            }
            if (chooser.dog && flags & entry::DOGS == 0)
                || (chooser.cat && flags & entry::CATS == 0)
            {
                continue;
            }
            if chooser.child {
                if flags & entry::NOT_CHILD != 0 {
                    continue;
                }
            } else if !chooser.dog && !chooser.cat && flags & entry::NOT_ADULT != 0 {
                continue;
            }
            if chooser.autonomy < threshold || flags & entry::NOT_AUTONOMOUS != 0 {
                continue;
            }
            let community = !chooser.residential && flags & entry::COMMUNITY != 0;
            if !community && chooser.visitor && flags & entry::VISITORS == 0 {
                continue;
            }
            if chooser.residential
                && chooser.visitor
                && (chooser.dog || chooser.cat)
                && !chooser.pets_outside
                && room != 0
            {
                continue;
            }
            entries.push(self.autonomy_entry(chooser, target, table, interaction.string_index));
        }
        if kind == 2
            && let Some(person) = &object.person
            && person.active.target != 0
        {
            let active = &person.active;
            let joinable = u16::try_from(active.target)
                .ok()
                .filter(|id| self.world.objects.contains_key(id))
                .map(|id| self.table(id))
                .transpose()?
                .flatten()
                .and_then(|table| find(table, active.table_index as u32))
                .is_some_and(|record| record.flags & entry::JOINABLE != 0);
            if joinable {
                // `joinable` holds only if the target exists.
                let goes_to = self.interaction_target(active.target as u16)?;
                if let Some(table) = self.table(goes_to)?
                    && let Some(running) = find(table, active.table_index as u32)
                    && running.joining_index != -1
                    && find(table, running.joining_index as u32).is_some()
                {
                    let menu = running.joining_index as u32;
                    let mut entry = self.autonomy_entry(chooser, goes_to, table, menu);
                    entry.join = true;
                    entries.push(entry);
                }
            }
        }
        Ok(entries)
    }

    /// The entry the scan looks at for a menu number (0x41f00e): its
    /// attenuation and flag bit 8 come from the first entry of the table
    /// with that number (0x41f12c), which need not be the one enumerated.
    /// The attenuation by code (0x475ec6): 0 the entry's own, 1 none, 2
    /// to 4 the low, moderate and high constants (a visitor's own), and
    /// any other code 0.02.
    fn autonomy_entry(
        &self,
        chooser: &Chooser,
        target: u16,
        table: &InteractionTable,
        menu: u32,
    ) -> Entry {
        let (attenuation, first) = match find(table, menu) {
            None => (F32::ZERO, false),
            Some(record) => (
                attenuation(&chooser.constants, chooser.visitor, record),
                record.flags & entry::FIRST != 0,
            ),
        };
        Entry {
            target,
            menu,
            attenuation,
            first,
            join: false,
        }
    }

    /// Whether an entry is allowed now (0x458319), and if so its
    /// advertisements. The table entry is the first with the entry's menu
    /// number. If there is one, the person's object data word 50 is
    /// cleared first, whatever follows. It is ruled out if:
    ///
    /// - the target's data word 8 has bit 9 set, or its data word 25 is
    ///   above 0;
    /// - the person is a visitor, the entry lacks flag 1, and neither is
    ///   the lot not residential with the entry's flag 0x20 set (a target
    ///   of category 4 or 7 then asks the relationship, 0x41f660, which is
    ///   not ported and fails);
    /// - by the person's age and kind, as [`Machine::autonomy_entries`].
    ///
    /// An entry still allowed has its advertisements copied, and its check
    /// tree, if it has one, decides: the tree of the target's type runs
    /// synchronously on the person's stack (0x4749b5), with the target as
    /// its stack object and the arguments 1, 0, 0, 0 (1, 1, 0, 0 for a
    /// join entry) and the tree's ID
    /// (the word that follows the original's four in memory). The
    /// person's temps are saved before the first check tree of the scan.
    /// Last, if the target is in another room than the person, the room
    /// motive's delta becomes its minimum negated.
    fn autonomy_allows(
        &mut self,
        chooser: &Chooser,
        entry: Entry,
        saved: &mut Option<Vec<i16>>,
    ) -> Result<Option<[Advertisement; ADVERTISEMENTS]>, Trap> {
        let Some(table) = self.table(entry.target)? else {
            return Ok(None);
        };
        let Some(record) = find(table, entry.menu) else {
            return Ok(None);
        };
        let record = record.clone();
        let me = chooser.me;
        self.me_mut()?.data[data::CHECKED] = 0;
        let target = &self.world.objects[&entry.target];
        let target_data = |index: usize| target.data.get(index).copied().unwrap_or(0);
        let mut allowed = true;
        if target_data(data::FLAGS) & 0x200 != 0 || target_data(data::IN_USE) > 0 {
            allowed = false;
        } else if record.flags & entry::VISITORS == 0 && chooser.visitor {
            let community = !chooser.residential && record.flags & entry::COMMUNITY != 0;
            if !community {
                allowed = false;
                if matches!(target_data(data::CATEGORY), 4 | 7) {
                    return Err(Trap::Unsupported(
                        "a visitor's entry on an object of category 4 or 7, which asks the \
                         relationship (0x41f660)"
                            .into(),
                    ));
                }
            }
        }
        let flags = record.flags;
        let fits = if chooser.child {
            flags & entry::NOT_CHILD == 0
        } else if chooser.dog {
            flags & entry::DOGS != 0
        } else if chooser.cat {
            flags & entry::CATS != 0
        } else {
            flags & entry::NOT_ADULT == 0
        };
        if !fits || !allowed {
            return Ok(None);
        }
        let room = target_data(data::ROOM);
        if record.motives.len() < ADVERTISEMENTS {
            return Err(Trap::Unsupported(format!(
                "an interaction with {} advertisements: the original copies them over the \
                 last entry's, which are not kept",
                record.motives.len()
            )));
        }
        let mut ads = [Advertisement::default(); ADVERTISEMENTS];
        for (ad, motive) in ads.iter_mut().zip(&record.motives) {
            *ad = Advertisement {
                personality: motive.personality as i16,
                minimum: motive.minimum,
                delta: motive.delta,
            };
        }
        let resource = self
            .definition(self.world.objects[&entry.target].guid)?
            .resource;
        let found = record.test == 0
            || match self.program.tree(resource, record.test) {
                crate::program::Tree::Missing => false,
                crate::program::Tree::Behavior(..) => true,
                crate::program::Tree::Primitive(id) => {
                    return Err(Trap::Unsupported(format!(
                        "check tree {id:#06x}, which names a primitive"
                    )));
                }
            };
        let allowed = if record.test != 0 && !found {
            // The synchronous run fails to push a tree the file lacks
            // (0x47484a), and returns false (0x474ada).
            if saved.is_none() {
                *saved = Some(self.me()?.temps.clone());
            }
            false
        } else if record.test != 0 {
            if saved.is_none() {
                *saved = Some(self.me()?.temps.clone());
            }
            let tree = record.test;
            let args = check_arguments(entry.join, tree);
            let mut scratch = Some(ads);
            let result = self
                .interpreter()
                .call_as(
                    self.world,
                    me,
                    entry.target,
                    tree,
                    entry.target as i16,
                    &args,
                    args.len(),
                    TURN_BUDGET,
                    &mut scratch,
                )
                .map_err(|fault| {
                    Trap::Unsupported(format!(
                        "check tree {tree:#06x} of object {}: {fault}",
                        entry.target
                    ))
                })?;
            if let Some(scratch) = scratch {
                ads = scratch;
            }
            result
        } else {
            true
        };
        if room != chooser.room {
            ads[13].delta = ads[13].minimum.wrapping_neg();
        }
        Ok(allowed.then_some(ads))
    }
}

/// An entry's attenuation by its code (0x475ec6): 0 the entry's own, 1
/// none, 2 to 4 the low, moderate and high constants (a visitor's own),
/// and any other code 0.02.
pub(crate) fn attenuation(constants: &Constants, visitor: bool, record: &Interaction) -> F32 {
    let set = if visitor {
        &constants.visitor_attenuation
    } else {
        &constants.attenuation
    };
    match record.attenuation_code {
        0 => F32::from_f32(record.attenuation),
        1 => F32::ZERO,
        code @ 2..=4 => set[code as usize - 2],
        _ => F32::from_bits(DEFAULT_ATTENUATION),
    }
}

/// The first entry of a table with a menu number (0x41f12c).
pub(crate) fn find(table: &InteractionTable, menu: u32) -> Option<&Interaction> {
    table
        .interactions
        .iter()
        .find(|interaction| interaction.string_index == menu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_join_entry_runs_its_check_with_a_second_argument_of_1() {
        assert_eq!(check_arguments(false, 0x1234), [1, 0, 0, 0, 0x1234]);
        assert_eq!(check_arguments(true, 0x1234), [1, 1, 0, 0, 0x1234]);
    }
}
