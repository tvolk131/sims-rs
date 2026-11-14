//! Alter Budget (primitive 25, 0x44eade): whether the household can pay
//! an amount. Only the test is ported; spending fails loudly.

use crate::support::*;

/// The lamp's tree 4096 runs Alter Budget on the amount 6 (byte 0 is 0: a
/// literal), with
/// `flags` and budget category `category`, on a lot with `funds` and the
/// budget mode `mode` (global 32).
fn alter(funds: i32, mode: i16, flags: u8, category: u8) -> Result<bool, sims_vm::Fault> {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            25,
            RETURN_TRUE,
            RETURN_FALSE,
            [0, 0, 6, 0, flags, 0, category, 0],
        )]),
    );
    fx.world.budget.funds = funds;
    fx.world.globals[32] = mode;
    fx.run()
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
fn a_bad_category_or_spending_fails() {
    // Category 9: native error 0x38, before the amount is read.
    assert_eq!(code(alter(100, 0, 1, 9)), Some(0x38));
    // Spending writes the day's budget entry, which is not ported.
    let trap = trap(alter(100, 0, 0, 0));
    assert!(trap.to_string().contains("0x46d91c"), "{trap}");
    // An amount the funds do not cover is false without spending.
    assert_eq!(alter(5, 0, 0, 0), Ok(false));
}
