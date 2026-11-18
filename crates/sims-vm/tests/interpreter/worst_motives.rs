//! Find 5 Worst Motives: the lowest five of the eight motives that are not
//! moods, into the running object's temps, ties broken by the C runtime's
//! selection sort. Synthetic; no game data.

use crate::support::*;

/// Find 5 Worst Motives with operand word 2 `whose` and word 3 `option`.
fn worst(whose: u16, option: u16) -> [u8; 8] {
    let [w0, w1] = whose.to_le_bytes();
    let [o0, o1] = option.to_le_bytes();
    [0, 0, 0, 0, w0, w1, o0, o1]
}

/// `runner` runs one Find 5 Worst Motives with `stack_object`.
fn run(
    fx: &mut Fixture,
    runner: u16,
    stack_object: i16,
    operands: [u8; 8],
) -> Result<bool, sims_vm::Fault> {
    let file = if runner == SIM { SIM_FILE } else { LAMP_FILE };
    fx.add(
        file,
        4096,
        tree(vec![ins(33, RETURN_TRUE, RETURN_FALSE, operands)]),
    );
    fx.run_with(runner, 4096, stack_object, &[], 100)
}

/// Sets the Sim's eight sorted motives to 10, then `values` over them.
fn motives(fx: &mut Fixture, values: &[(usize, f32)]) {
    let person = fx.sim_mut().person.as_mut().unwrap();
    for index in [5, 6, 7, 8, 9, 13, 14, 15] {
        person.motives[index] = 10.0;
    }
    for &(index, value) in values {
        person.motives[index] = value;
    }
}

fn temps(object: &sims_sim::Object) -> Vec<i16> {
    object.temps.clone()
}

#[test]
fn the_stack_objects_lowest_five_go_into_my_temps() {
    let mut fx = Fixture::new();
    motives(
        &mut fx,
        &[
            (15, -90.0),
            (13, -80.0),
            (7, -70.0),
            (9, 0.0),
            (5, 50.0),
            (6, 60.0),
            (8, 70.0),
            (14, 80.0),
        ],
    );
    // The moods do not count, however low.
    fx.sim_mut().person.as_mut().unwrap().motives[3] = -100.0;
    fx.lamp_mut().temps = vec![-1; 8];
    let sim_temps = temps(fx.sim());
    assert_eq!(run(&mut fx, LAMP, SIM as i16, worst(1, 0)), Ok(true));
    assert_eq!(temps(fx.lamp()), vec![15, 13, 7, 9, 5, -1, -1, -1]);
    assert_eq!(temps(fx.sim()), sim_temps);
}

#[test]
fn a_person_sorts_their_own_motives_unless_word_2_is_1() {
    for whose in [0, 3, 0xffff] {
        let mut fx = Fixture::new();
        motives(&mut fx, &[(14, -5.0), (6, -4.0), (8, -3.0)]);
        // The stack object is the lamp, which is not read.
        assert_eq!(run(&mut fx, SIM, LAMP as i16, worst(whose, 2)), Ok(true));
        assert_eq!(&temps(fx.sim())[..5], &[14, 6, 8, 9, 13], "word 2 {whose}");
    }
}

#[test]
fn equal_motives_come_out_in_the_selection_sorts_order() {
    // All eight equal: each pass swaps the first to the end, so motive 5,
    // which starts first, ends last.
    let mut fx = Fixture::new();
    motives(&mut fx, &[]);
    assert_eq!(run(&mut fx, LAMP, SIM as i16, worst(1, 0)), Ok(true));
    assert_eq!(&temps(fx.lamp())[..5], &[6, 7, 8, 9, 13]);
}

#[test]
fn a_hundredth_apart_is_equal() {
    let mut fx = Fixture::new();
    motives(&mut fx, &[(5, 9.995)]);
    assert_eq!(run(&mut fx, LAMP, SIM as i16, worst(1, 0)), Ok(true));
    assert_eq!(&temps(fx.lamp())[..5], &[6, 7, 8, 9, 13]);

    // Two hundredths below the rest, motive 5 comes first, and the
    // others' order is still the selection sort's.
    let mut fx = Fixture::new();
    motives(&mut fx, &[(5, 9.98)]);
    assert_eq!(run(&mut fx, LAMP, SIM as i16, worst(1, 0)), Ok(true));
    assert_eq!(&temps(fx.lamp())[..5], &[5, 7, 8, 9, 13]);
}

#[test]
fn no_person_there_is_native_error_0xa() {
    // Word 2 at 2 names no one.
    let mut fx = Fixture::new();
    assert_eq!(code(run(&mut fx, LAMP, SIM as i16, worst(2, 0))), Some(0xa));
    // A stack object that is not a person, or not there.
    let mut fx = Fixture::new();
    assert_eq!(code(run(&mut fx, SIM, LAMP as i16, worst(1, 0))), Some(0xa));
    let mut fx = Fixture::new();
    assert_eq!(code(run(&mut fx, LAMP, 99, worst(1, 0))), Some(0xa));
    let mut fx = Fixture::new();
    assert_eq!(code(run(&mut fx, LAMP, -1, worst(1, 0))), Some(0xa));
    // The running object, when it is not a person; checked before word 3.
    let mut fx = Fixture::new();
    assert_eq!(code(run(&mut fx, LAMP, SIM as i16, worst(0, 3))), Some(0xa));
}

#[test]
fn word_3_past_2_is_native_error_0x1e() {
    for option in [3, 0x8000] {
        let mut fx = Fixture::new();
        fx.lamp_mut().temps = vec![-1; 8];
        assert_eq!(
            code(run(&mut fx, LAMP, SIM as i16, worst(1, option))),
            Some(0x1e)
        );
        assert_eq!(temps(fx.lamp()), vec![-1; 8]);
    }
}
