use super::*;

fn matrix(token: u64, fill: u8) -> Matrix {
    Matrix {
        width: 64,
        height: 64,
        stride: 1,
        allocation: Allocation::Known(Backing {
            token,
            column_offsets: (0..64).map(|column| column * 64).collect(),
            data: vec![fill; 4096],
        }),
    }
}

fn data(matrix: &Matrix) -> &[u8] {
    let Allocation::Known(backing) = &matrix.allocation else {
        panic!("actual test allocation absent")
    };
    &backing.data
}

#[test]
fn zero_argument_uses_the_original_fixed_seed_and_restores_each_global_stream() {
    let mut expected = matrix(1, 0);
    let mut expected_target = matrix(2, 0);
    regenerate(&mut expected, &mut expected_target, DEFAULT_SEED, &mut 0).unwrap();
    for saved in [0, 1, 0x80000000, u32::MAX] {
        let mut grass = matrix(3, 0xa5);
        let mut target = matrix(4, 0x7f);
        let mut random = saved;
        regenerate(&mut grass, &mut target, 0, &mut random).unwrap();
        assert_eq!(random, saved);
        assert_eq!(data(&grass), data(&expected));
        assert_eq!(data(&target), data(&grass));
        assert_eq!(backing(&grass).unwrap().token, 3);
        assert_eq!(backing(&target).unwrap().token, 4);
    }
}

#[test]
fn supplied_seed_changes_the_complete_field_without_consuming_the_global_stream() {
    let mut first = matrix(1, 0);
    let mut first_target = matrix(2, 0);
    let mut second = matrix(3, 0);
    let mut second_target = matrix(4, 0);
    let mut random = 123;
    regenerate(&mut first, &mut first_target, 1, &mut random).unwrap();
    regenerate(&mut second, &mut second_target, 2, &mut random).unwrap();
    assert_ne!(data(&first), data(&second));
    assert_eq!(random, 123);
    assert_eq!(data(&first), data(&first_target));
    assert_eq!(data(&second), data(&second_target));
}

#[test]
fn signed_samples_wrap_in_both_directions_and_keep_negative_values() {
    let mut payload = vec![0; 4096];
    payload[63 * 64 + 1] = 0x80;
    payload[2 * 64 + 63] = 0xff;
    assert_eq!(sample(&payload, -1, 1), -128);
    assert_eq!(sample(&payload, 127, 65), -128);
    assert_eq!(sample(&payload, 2, -1), -1);
}

#[test]
fn unavailable_or_unsupported_input_stops_before_rng_or_matrix_effects() {
    for change in 0..7 {
        let mut grass = matrix(1, 0xa5);
        let mut target = matrix(2, 0x7f);
        match change {
            0 => grass.width = 63,
            1 => target.height = 63,
            2 => grass.stride = 2,
            3 => target.allocation = Allocation::Unavailable,
            4 => grass.allocation = Allocation::Null,
            5 => target.allocation = Allocation::Released { token: 2 },
            6 => target = matrix(1, 0x7f),
            _ => unreachable!(),
        }
        let before = (grass.clone(), target.clone());
        let mut random = 123;
        assert!(regenerate(&mut grass, &mut target, 0, &mut random).is_err());
        assert_eq!((grass, target), before);
        assert_eq!(random, 123);
    }
}

#[test]
fn malformed_constructor_backing_is_not_replaced_or_repacked() {
    for change in 0..4 {
        let mut grass = matrix(1, 0xa5);
        let mut target = matrix(2, 0);
        let Allocation::Known(allocation) = &mut grass.allocation else {
            unreachable!()
        };
        match change {
            0 => allocation.token = 0,
            1 => allocation.data.pop().map(|_| ()).unwrap(),
            2 => allocation.column_offsets[1] = 0,
            3 => allocation.column_offsets.clear(),
            _ => unreachable!(),
        }
        let before = (grass.clone(), target.clone());
        let mut random = 123;
        assert!(regenerate(&mut grass, &mut target, 0, &mut random).is_err());
        assert_eq!((grass, target), before);
        assert_eq!(random, 123);
    }
}
