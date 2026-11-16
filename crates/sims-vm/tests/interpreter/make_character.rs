//! Make New Character: its locals, and the definition it cannot make yet.

use crate::support::*;
use sims_vm::Trap;

/// Make New Character with locals `skin`, `age` and `sex` and template
/// `species`, in a tree with four locals.
fn make(skin: u8, age: u8, sex: u8, species: u8) -> Result<bool, sims_vm::Fault> {
    let mut fx = Fixture::new();
    fx.add(
        LAMP_FILE,
        4096,
        tree(vec![ins(
            19,
            RETURN_TRUE,
            RETURN_FALSE,
            [skin, age, sex, species, 0, 0, 0, 0],
        )]),
    );
    fx.run()
}

#[test]
fn a_local_past_the_trees_locals_throws_native_error_8() {
    for (skin, age, sex) in [(4, 0, 0), (0, 4, 0), (0, 0, 4), (255, 255, 255)] {
        assert_eq!(code(make(skin, age, sex, 0)), Some(8), "{skin} {age} {sex}");
    }
}

#[test]
fn with_its_locals_in_range_it_stops_at_the_definition_it_cannot_make() {
    for species in [0, 1, 2, 3] {
        let trap = trap(make(0, 1, 2, species));
        assert!(
            matches!(&trap, Trap::Unsupported(m) if m.contains("0x4260e1")
                && m.contains(&format!("template {species},"))),
            "{trap:?}"
        );
    }
}
