//! Breakpoint (opcode 15, 0x451e34) and Show String (opcode 21,
//! 0x451ab7). Synthetic; no game data.

use crate::support::*;

/// Runs one instruction of `opcode` as the lamp's tree 4096.
fn one(fx: &mut Fixture, opcode: u16, operands: [u8; 8]) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(opcode, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run()
}

/// Breakpoint operands: the index in word 0, the scope in word 1.
fn breakpoint(scope: i16, index: i16) -> [u8; 8] {
    let [i0, i1] = index.to_le_bytes();
    let [s0, s1] = scope.to_le_bytes();
    [i0, i1, s0, s1, 0, 0, 0, 0]
}

#[test]
fn a_breakpoint_on_a_word_of_0_goes_on() {
    let mut fx = Fixture::new();
    assert_eq!(
        one(&mut fx, 15, breakpoint(scope::LITERAL.into(), 0)),
        Ok(true)
    );
    fx.world.globals[7] = 0;
    assert_eq!(
        one(&mut fx, 15, breakpoint(scope::GLOBAL.into(), 7)),
        Ok(true)
    );
}

#[test]
fn a_breakpoint_on_a_word_that_is_set_stops() {
    let mut fx = Fixture::new();
    let fault = one(&mut fx, 15, breakpoint(scope::LITERAL.into(), 5)).unwrap_err();
    assert_eq!(fault.trap.code(), Some(0x3e), "{fault}");
    // Any word: global 7.
    let mut fx = Fixture::new();
    fx.world.globals[7] = -1;
    let fault = one(&mut fx, 15, breakpoint(scope::GLOBAL.into(), 7)).unwrap_err();
    assert_eq!(fault.trap.code(), Some(0x3e), "{fault}");
}

#[test]
fn a_breakpoint_with_no_such_scope_is_native_error_9() {
    for scope in [39, 255, 256, -1] {
        let mut fx = Fixture::new();
        let fault = one(&mut fx, 15, breakpoint(scope, 0)).unwrap_err();
        assert_eq!(fault.trap.code(), Some(9), "scope {scope}: {fault}");
    }
}

#[test]
fn show_string_changes_nothing() {
    let mut fx = Fixture::new();
    let mut expected = fx.world.clone();
    expected.objects.get_mut(&LAMP).unwrap().last_result = true;
    assert_eq!(one(&mut fx, 21, [1, 0x80, 2, 0, 9, 9, 9, 9]), Ok(true));
    assert_eq!(hash_without_frames(&fx.world), expected.hash());
}
