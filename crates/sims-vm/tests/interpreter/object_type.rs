//! Test Object Type: an object's own GUID or its multi-tile catalog
//! master's. The lamp runs; its stack object is the Sim.

use crate::support::*;
use sims_vm::{Fault, Trap};

const MASTER_GUID: u32 = 0xcafe_0001;

/// Runs Test Object Type for `guid` on the object in `scope` and `index`.
fn test(fx: &mut Fixture, guid: u32, scope: u8, index: i16) -> Result<bool, Fault> {
    let mut operands = [0; 8];
    operands[..4].copy_from_slice(&guid.to_le_bytes());
    operands[4..6].copy_from_slice(&index.to_le_bytes());
    operands[6] = scope;
    // Byte 7 is ignored.
    operands[7] = 0xee;
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(32, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run()
}

/// Puts the lamp in multi-tile group 5, and adds catalog definitions to
/// its file in the given order: (chunk, group, sub-index, GUID).
fn group(fx: &mut Fixture, definitions: &[(u16, i16, i16, u32)]) {
    let library = fx.program.libraries.get_mut(&LAMP_FILE).unwrap();
    let lamp = library.definitions.get_mut(&128).unwrap();
    lamp[10] = 5;
    lamp[11] = 0x0101;
    for &(chunk, group, sub, guid) in definitions {
        let mut words = definition(guid, 4);
        words[10] = group;
        words[11] = sub;
        library.definitions.insert(chunk, words);
        library.definition_order.push(chunk);
    }
}

#[test]
fn an_object_is_of_its_own_type() {
    let mut fx = Fixture::new();
    assert_eq!(test(&mut fx, SIM_GUID, scope::STACK_OBJECT, 0), Ok(true));
    assert_eq!(test(&mut fx, LAMP_GUID, scope::STACK_OBJECT, 0), Ok(false));
    assert_eq!(
        test(&mut fx, LAMP_GUID, scope::LITERAL, LAMP as i16),
        Ok(true)
    );
    fx.lamp_mut().temps[3] = SIM as i16;
    assert_eq!(test(&mut fx, SIM_GUID, scope::TEMPS, 3), Ok(true));
}

#[test]
fn a_tile_is_of_its_catalog_masters_type() {
    let mut fx = Fixture::new();
    // A definition in another group, and a tile of the same group, come
    // before the master.
    group(
        &mut fx,
        &[
            (130, 6, -1, 0xbad),
            (131, 5, 0x0102, 0xbad2),
            (129, 5, -1, MASTER_GUID),
            (132, 5, -1, 0xbad3),
        ],
    );
    let lamp = scope::LITERAL;
    assert_eq!(test(&mut fx, MASTER_GUID, lamp, LAMP as i16), Ok(true));
    assert_eq!(test(&mut fx, LAMP_GUID, lamp, LAMP as i16), Ok(true));
    // Only the first master in resource order counts.
    assert_eq!(test(&mut fx, 0xbad3, lamp, LAMP as i16), Ok(false));
    assert_eq!(test(&mut fx, 0xbad2, lamp, LAMP as i16), Ok(false));
}

#[test]
fn a_group_without_a_master_fails_loudly() {
    let mut fx = Fixture::new();
    group(&mut fx, &[(129, 6, -1, MASTER_GUID)]);
    assert!(matches!(
        trap(test(&mut fx, MASTER_GUID, scope::LITERAL, LAMP as i16)),
        Trap::Unsupported(_)
    ));
    // The object's own GUID is checked first.
    assert_eq!(
        test(&mut fx, LAMP_GUID, scope::LITERAL, LAMP as i16),
        Ok(true)
    );
}

#[test]
fn a_missing_object_is_error_0x17() {
    let mut fx = Fixture::new();
    for id in [0, 40, -1] {
        assert_eq!(
            code(test(&mut fx, SIM_GUID, scope::LITERAL, id)),
            Some(0x17)
        );
    }
    // The operand itself must read.
    assert_eq!(code(test(&mut fx, SIM_GUID, scope::TEMPS, 8)), Some(2));
}
