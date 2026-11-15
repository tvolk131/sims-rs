//! Alter Budget (primitive 25, 0x44eade): whether the household can pay
//! an amount, and spending it (0x46d91c).

use crate::support::*;

/// The lamp's tree 4096 runs Alter Budget on the amount 6 (byte 0 is 0: a
/// literal), with
/// `flags` and budget category `category`, on a lot with `funds` and the
/// budget mode `mode` (global 32).
fn alter(funds: i32, mode: i16, flags: u8, category: u8) -> Result<bool, sims_vm::Fault> {
    spend_from(funds, mode, flags, category, 6).1
}

/// Six days of budget entries, today's first.
fn days() -> Vec<Option<[i32; 8]>> {
    (0..6)
        .map(|day| Some(std::array::from_fn(|c| day * 100 + c as i32)))
        .collect()
}

/// [`alter`] on the amount `amount` with [`days`], returning the lot.
fn spend_from(
    funds: i32,
    mode: i16,
    flags: u8,
    category: u8,
    amount: i16,
) -> (Fixture, Result<bool, sims_vm::Fault>) {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(25, RETURN_TRUE, RETURN_FALSE, {
            let [low, high] = amount.to_le_bytes();
            [0, 0, low, high, flags, 0, category, 0]
        })]),
    );
    fx.world.budget.funds = funds;
    fx.world.budget.days = days();
    fx.world.globals[32] = mode;
    let result = fx.run();
    (fx, result)
}

/// Globals 14 and 13.
fn funds_globals(fx: &Fixture) -> (i16, i16) {
    (fx.world.globals[14], fx.world.globals[13])
}

#[test]
fn an_amount_the_funds_cover_is_true() {
    // Bit 0: only test.
    assert_eq!(alter(6, 0, 1, 0), Ok(true));
    assert_eq!(alter(5, 0, 1, 8), Ok(false));
    // Bit 1: income, a negative amount, which any funds cover.
    assert_eq!(alter(-5, 0, 3, 0), Ok(true));
    // Budget mode 1: 999,999 is available.
    assert_eq!(alter(0, 1, 1, 0), Ok(true));
    // In budget mode 1 nothing is spent, so a call without bit 0 is true.
    assert_eq!(alter(0, 1, 0, 0), Ok(true));
}

#[test]
fn the_amount_may_be_an_argument_or_another_scope() {
    // Byte 0 = 3: the scope in byte 1, here the lamp's data word 3 (6).
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            25,
            RETURN_TRUE,
            RETURN_FALSE,
            [3, scope::MY_DATA, 3, 0, 1, 0, 0, 0],
        )]),
    );
    fx.world.budget.funds = 5;
    assert_eq!(fx.run(), Ok(false));
    fx.world.budget.funds = 6;
    assert_eq!(fx.run(), Ok(true));
    // Byte 0 = 1: argument 0 of the tree.
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            25,
            RETURN_TRUE,
            RETURN_FALSE,
            [1, 0, 0, 0, 1, 0, 0, 0],
        )]),
    );
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[7], 1000), Ok(false));
    assert_eq!(fx.run_with(LAMP, 4096, SIM as i16, &[6], 1000), Ok(true));
}

#[test]
fn a_bad_category_fails() {
    // Category 9: native error 0x38, before the amount is read.
    assert_eq!(code(alter(100, 0, 1, 9)), Some(0x38));
    // An amount the funds do not cover is false without spending.
    let (fx, result) = spend_from(5, 0, 0, 0, 6);
    assert_eq!(result, Ok(false));
    assert_eq!(fx.world.budget.funds, 5);
    assert_eq!(fx.world.budget.days, days());
}

#[test]
fn spending_takes_the_funds_into_todays_entry() {
    let (fx, result) = spend_from(123_456, 0, 0, 3, 6);
    assert_eq!(result, Ok(true));
    assert_eq!(fx.world.budget.funds, 123_450);
    // Today's entry is the first; the other days are as they were.
    let mut expected = days();
    expected[0].as_mut().unwrap()[3] += 6;
    assert_eq!(fx.world.budget.days, expected);
    // Globals 14 and 13: the funds split at 10,000.
    assert_eq!((fx.world.globals[14], fx.world.globals[13]), (12, 3450));
    // Only testing (bit 0) spends nothing.
    let (fx, result) = spend_from(123_456, 0, 1, 3, 6);
    assert_eq!(result, Ok(true));
    assert_eq!(fx.world.budget.funds, 123_456);
    assert_eq!(fx.world.budget.days, days());
}

#[test]
fn income_is_spent_as_a_negative_amount() {
    // Bit 1: the amount is negated, so the funds go up and today's entry
    // down. Each part of the split is truncated toward zero.
    let (fx, result) = spend_from(-20_005, 0, 2, 7, 6);
    assert_eq!(result, Ok(true));
    assert_eq!(fx.world.budget.funds, -19_999);
    assert_eq!(fx.world.budget.days[0].unwrap()[7], 1);
    assert_eq!((fx.world.globals[14], fx.world.globals[13]), (-1, -9_999));
    // The funds and the entry wrap in 32 bits, and the split is cut to 16:
    // -214,748 becomes -18,140.
    let (fx, result) = spend_from(i32::MAX, 0, 2, 0, 6);
    assert_eq!(result, Ok(true));
    assert_eq!(fx.world.budget.funds, i32::MIN + 5);
    assert_eq!(
        (fx.world.globals[14], fx.world.globals[13]),
        (-18_140, -3_643)
    );
}

#[test]
fn only_budget_mode_1_spends_nothing() {
    // Mode 1: nothing is spent, and the branch is true.
    let (fx, result) = spend_from(0, 1, 0, 0, 6);
    assert_eq!(result, Ok(true));
    assert_eq!(fx.world.budget.funds, 0);
    assert_eq!(fx.world.budget.days, days());
    assert_eq!(funds_globals(&fx), funds_globals(&Fixture::new()));
    // Mode 2 spends, though the funds setter (0x46d8dc) would not write.
    let (fx, result) = spend_from(100, 2, 0, 1, 6);
    assert_eq!(result, Ok(true));
    assert_eq!(fx.world.budget.funds, 94);
    assert_eq!(fx.world.budget.days[0].unwrap()[1], 7);
    assert_eq!(fx.world.globals[13], 94);
}

#[test]
fn spending_what_is_not_ported_fails_and_changes_nothing() {
    // Category 8 passes the check but is past today's eight entries.
    let (fx, result) = spend_from(100, 0, 0, 8, 6);
    assert!(trap(result).to_string().contains("+0x1a4"));
    assert_eq!(fx.world.budget.funds, 100);
    assert_eq!(fx.world.budget.days, days());
    assert_eq!(funds_globals(&fx), funds_globals(&Fixture::new()));
    // Category 8 only tested is fine.
    assert_eq!(alter(100, 0, 1, 8), Ok(true));
    // In mode 1 nothing is written, so category 8 does nothing.
    assert_eq!(alter(100, 1, 0, 8), Ok(true));

    let mut fx = spend_from(100, 0, 1, 0, 6).0;
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            25,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 6, 0, 0, 0, 0, 0],
        )]),
    );
    // A lot with no days read from a save.
    fx.world.budget.days.clear();
    assert!(trap(fx.run()).to_string().contains("0x46d1e4"));
    // A today the save marked with flag 0.
    fx.world.budget.days = days();
    fx.world.budget.days[0] = None;
    assert!(trap(fx.run()).to_string().contains("0x46d270"));
    assert_eq!(fx.world.budget.funds, 100);
    assert_eq!(funds_globals(&fx), funds_globals(&Fixture::new()));
}
