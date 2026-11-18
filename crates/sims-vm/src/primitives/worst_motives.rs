//! Find 5 Worst Motives (opcode 33, 0x44d077): the five motives a person
//! is lowest in, into the running object's temps.
//!
//! The general table (0x45246a) sends opcode 33 here. What it does:
//!
//! 1. **Whose motives.** Operand word 2 (bytes 4 and 5, unsigned) names
//!    the person:
//!    - 1: the stack object (0x474f39, then 0x40743b);
//!    - 2: no one;
//!    - anything else: the running object.
//!
//!    No object there, or one whose type (definition word 9) is not 2, a
//!    person, throws native error 0xa (0x44d167).
//! 2. **Operand word 3** (bytes 6 and 7, unsigned) must be 0, 1 or 2, or
//!    native error 0x1e is thrown (0x44d10f). Nothing else reads it.
//! 3. **The sort.** The person's eight motives that are not moods (5, 6,
//!    7, 8, 9, 13, 14 and 15, in that order) are sorted by the C
//!    runtime's `qsort` (0x6356ba), which for eight elements is a
//!    selection sort that moves the greatest to the end (0x63580e). The
//!    comparison (0x44d03b) takes the second motive from the first: below
//!    -0.01 the first goes before, above 0.01 after, and otherwise the two
//!    are equal. A difference that is not a number puts the first before.
//!    The FPU runs in single precision (as Find Best Interaction's
//!    comparison takes it, `autonomy.rs`), so the difference is rounded
//!    to 24 bits before it is compared.
//! 4. **The temps.** The first five motive numbers of the sorted order,
//!    the lowest first, become the running object's temps 0 to 4 (+0x3a),
//!    whoever's motives were sorted. The branch is true.
//!
//! The original also keeps a pointer to the person's motives in a global
//! (0x6e50a4) for the comparison; nothing else reads it, so it is not
//! modelled.

use crate::autonomy::qsort;
use crate::fault::Trap;
use crate::interpreter::{Machine, word};
use core::cmp::Ordering;
use sims_core::float::{Ext, F32};

/// The motives that are sorted, in the order they start in (0x44d087).
const MOTIVES: [usize; 8] = [5, 6, 7, 8, 9, 13, 14, 15];

/// How many motive numbers are written.
const WORST: usize = 5;

/// The comparison's tolerances, the doubles at 0x667268 and 0x667260.
const BELOW: u64 = 0xbf84_7ae1_47ae_147b; // -0.01
const ABOVE: u64 = 0x3f84_7ae1_47ae_147b; // 0.01

/// Definition word 9's value for a person.
const PERSON: i16 = 2;

impl Machine<'_, '_> {
    /// Find 5 Worst Motives (0x44d077). See the module's documentation.
    pub(crate) fn find_worst_motives(&mut self, operands: &[u8; 8]) -> Result<bool, Trap> {
        let whose = match word(operands, 4) as u16 {
            1 => u16::try_from(self.frame()?.stack_object).ok(),
            2 => None,
            _ => Some(self.me),
        }
        .filter(|id| self.world.objects.contains_key(id));
        let Some(id) = whose else {
            return Err(Trap::native(0xa, "Find 5 Worst Motives: no person there"));
        };
        if self.interpreter().kind(self.world, id)? != PERSON {
            return Err(Trap::native(0xa, "Find 5 Worst Motives: not a person"));
        }
        if word(operands, 6) as u16 > 2 {
            return Err(Trap::native(
                0x1e,
                "Find 5 Worst Motives: operand word 3 is past 2",
            ));
        }
        let motives = &self.world.objects[&id]
            .person
            .as_ref()
            .ok_or_else(|| Trap::Invalid(format!("person {id} has no person data")))?
            .motives;
        let mut sorted = MOTIVES
            .into_iter()
            .map(|index| {
                motives
                    .get(index)
                    .map(|&value| (index, F32::from_f32(value)))
                    .ok_or_else(|| Trap::Invalid(format!("person {id} has no motive {index}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        qsort(&mut sorted, &|a, b| compare(a.1, b.1));
        let temps = &mut self.me_mut()?.temps;
        if temps.len() < WORST {
            return Err(Trap::Invalid("object has fewer than five temps".into()));
        }
        for (temp, (index, _)) in temps.iter_mut().zip(sorted.into_iter().take(WORST)) {
            // At most 15, so the word holds it.
            *temp = index as i16;
        }
        Ok(true)
    }
}

/// The comparison (0x44d03b): `a` less `b`, rounded to single precision;
/// below -0.01 (or not a number) `a` goes first, above 0.01 after, and
/// otherwise they are equal.
fn compare(a: F32, b: F32) -> i32 {
    let difference = (a.ext() - b.ext()).single();
    if !matches!(
        difference.partial_cmp(&Ext::from_f64_bits(BELOW)),
        Some(Ordering::Greater | Ordering::Equal)
    ) {
        -1
    } else if difference.partial_cmp(&Ext::from_f64_bits(ABOVE)) == Some(Ordering::Greater) {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(value: f32) -> F32 {
        F32::from_f32(value)
    }

    #[test]
    fn the_comparison_has_a_hundredth_of_slack() {
        assert_eq!(compare(f(1.0), f(2.0)), -1);
        assert_eq!(compare(f(2.0), f(1.0)), 1);
        assert_eq!(compare(f(1.0), f(1.005)), 0);
        assert_eq!(compare(f(1.005), f(1.0)), 0);
        assert_eq!(compare(f(1.0), f(1.0)), 0);
        assert_eq!(compare(F32::from_bits(0x7fc0_0000), f(1.0)), -1);
        assert_eq!(compare(f(1.0), F32::from_bits(0x7fc0_0000)), -1);
    }
}
