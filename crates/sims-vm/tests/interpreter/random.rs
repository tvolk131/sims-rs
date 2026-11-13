//! Random Number: which stream it draws from, its range, and where the
//! draw lands.

use crate::support::*;
use sims_core::GameRng;
use sims_vm::{Stream, Trap};

/// Random Number operands: destination scope and index, range scope and
/// index.
fn operands(scope: u16, index: i16, range_scope: u16, range: i16) -> [u8; 8] {
    let mut bytes = [0; 8];
    bytes[0..2].copy_from_slice(&index.to_le_bytes());
    bytes[2..4].copy_from_slice(&scope.to_le_bytes());
    bytes[4..6].copy_from_slice(&range.to_le_bytes());
    bytes[6..8].copy_from_slice(&range_scope.to_le_bytes());
    bytes
}

fn random(
    fx: &mut Fixture,
    scope: u16,
    index: i16,
    range_scope: u16,
    range: i16,
) -> Result<bool, sims_vm::Fault> {
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            8,
            RETURN_TRUE,
            RETURN_FALSE,
            operands(scope, index, range_scope, range),
        )]),
    );
    fx.run()
}

#[test]
fn draws_from_the_lot_stream_modulo_the_range() {
    let mut fx = Fixture::new();
    let mut expected = GameRng::from_state(1);
    for _ in 0..5 {
        assert_eq!(random(&mut fx, 8, 2, 7, 100), Ok(true));
        assert_eq!(fx.lamp().temps[2], (expected.next_u32() % 100) as i16);
    }
    assert_eq!(fx.world.lot_rng, expected);
    assert_eq!(fx.world.global_rng, GameRng::from_state(2));
    // The first draw from state 1 is 16838.
    let mut fx = Fixture::new();
    random(&mut fx, 8, 0, 7, 30000).unwrap();
    assert_eq!(fx.lamp().temps[0], 16838);
}

#[test]
fn draws_from_the_global_stream_when_it_is_active() {
    let mut fx = Fixture::new();
    fx.stream = Stream::Global;
    let mut expected = GameRng::from_state(2);
    assert_eq!(random(&mut fx, 8, 0, 7, 1000), Ok(true));
    assert_eq!(fx.lamp().temps[0], (expected.next_u32() % 1000) as i16);
    assert_eq!(fx.world.global_rng, expected);
    assert_eq!(fx.world.lot_rng, GameRng::from_state(1));
}

#[test]
fn the_range_can_come_from_any_readable_scope() {
    let mut fx = Fixture::new();
    // My attribute 2 is 12.
    let mut expected = GameRng::from_state(1);
    assert_eq!(random(&mut fx, 3, 40, 0, 2), Ok(true));
    assert_eq!(fx.lamp().data[40], (expected.next_u32() % 12) as i16);
}

#[test]
fn a_negative_range_is_unsigned() {
    let mut fx = Fixture::new();
    let mut expected = GameRng::from_state(1);
    random(&mut fx, 8, 0, 7, -2).unwrap();
    let range = -2i32 as u32;
    assert_eq!(fx.lamp().temps[0], (expected.next_u32() % range) as i16);
}

#[test]
fn a_zero_range_faults() {
    let mut fx = Fixture::new();
    assert_eq!(code(random(&mut fx, 8, 0, 7, 0)), Some(4));
    assert_eq!(fx.world.lot_rng, GameRng::from_state(1));
}

#[test]
fn faults_come_before_the_draw() {
    // Read-only destination.
    let mut fx = Fixture::new();
    assert_eq!(code(random(&mut fx, 7, 0, 7, 10)), Some(0x13));
    // Unreadable destination (the lamp has no motives), before the range
    // is even read.
    assert_eq!(code(random(&mut fx, 14, 0, 7, 0)), Some(0x1c));
    // A destination whose store is unsupported.
    assert!(matches!(
        trap(random(&mut fx, 3, 11, 7, 10)),
        Trap::Unsupported(_)
    ));
    // Out-of-bounds destination and range.
    assert_eq!(code(random(&mut fx, 8, 8, 7, 10)), Some(2));
    assert_eq!(code(random(&mut fx, 8, 0, 9, 4)), Some(8));
    // A scope word above 255.
    assert_eq!(code(random(&mut fx, 0x100, 0, 7, 10)), Some(9));
    assert_eq!(code(random(&mut fx, 8, 0, 0x107, 10)), Some(9));
    assert_eq!(fx.world.lot_rng, GameRng::from_state(1));
}

#[test]
fn a_range_in_global_34_takes_the_draw() {
    // Global 34 resolves with a word pointer even read-only, which replaces
    // the destination: the draw lands in global 34, not the temp.
    let mut fx = Fixture::new();
    fx.world.globals[34] = 50;
    let mut expected = GameRng::from_state(1);
    assert_eq!(random(&mut fx, 8, 0, 6, 34), Ok(true));
    assert_eq!(fx.world.globals[34], (expected.next_u32() % 50) as i16);
    assert_eq!(fx.lamp().temps[0], 0);
    // Even when the destination itself has no word pointer.
    let mut fx = Fixture::new();
    fx.world.globals[34] = 50;
    assert_eq!(random(&mut fx, 7, 0, 6, 34), Ok(true));
}

#[test]
fn a_motive_takes_the_draw_as_a_float() {
    let mut fx = Fixture::new();
    let mut expected = GameRng::from_state(1);
    assert_eq!(random(&mut fx, 15, 3, 7, 1000), Ok(true));
    assert_eq!(
        fx.sim().person.as_ref().unwrap().motives[3],
        (expected.next_u32() % 1000) as f32
    );
}
