use super::*;

fn fixture(width: i32, height: i32, stride: i32) -> Matrix {
    Matrix {
        width,
        height,
        stride,
        allocation: Allocation::Known(Backing {
            token: 0x70000000,
            column_offsets: (0..width as u32)
                .map(|column| column * height as u32 * stride as u32)
                .collect(),
            data: (0..width * height * stride)
                .map(|index| (index * 73 + 19) as u8)
                .collect(),
        }),
    }
}

fn backing_mut(matrix: &mut Matrix) -> &mut Backing {
    let Allocation::Known(backing) = &mut matrix.allocation else {
        panic!()
    };
    backing
}

#[test]
fn primary_fill_uses_dword_stores_and_exact_odd_word_tail() {
    let mut primary = fixture(3, 3, 2);
    let flags = fixture(3, 3, 1);
    let plan = Plan::for_matrices(&primary, &flags, [0x2c, 0x34]).unwrap();
    assert_eq!(plan.primary_stores, 5);
    let mut writes = vec![];
    for store in &plan.stores[..plan.primary_stores] {
        writes.push(store.apply(backing_mut(&mut primary)).unwrap());
    }
    assert_eq!(backing_mut(&mut primary).data, vec![0xff; 18]);
    assert_eq!(
        writes.iter().map(|write| write.offset).collect::<Vec<_>>(),
        [0, 4, 8, 12, 16]
    );
    assert!(
        writes[..4]
            .iter()
            .all(|write| write.instruction == 0x47c5e4 && write.after.len() == 4)
    );
    assert_eq!(writes[4].instruction, 0x47c5e8);
    assert_eq!(writes[4].after, [0xff, 0xff]);
}

#[test]
fn flag_mask_follows_actual_columns_in_row_outer_order() {
    let primary = fixture(3, 3, 2);
    let mut flags = fixture(3, 3, 1);
    backing_mut(&mut flags).column_offsets = vec![6, 0, 3];
    let before = backing_mut(&mut flags).data.clone();
    let plan = Plan::for_matrices(&primary, &flags, [0x30, 0x38]).unwrap();
    let mut writes = vec![];
    for store in &plan.stores[plan.primary_stores..] {
        writes.push(store.apply(backing_mut(&mut flags)).unwrap());
    }
    assert_eq!(
        writes.iter().map(|write| write.offset).collect::<Vec<_>>(),
        [6, 0, 3, 7, 1, 4, 8, 2, 5]
    );
    assert!(
        writes
            .iter()
            .all(|write| write.slot == 0x38 && write.instruction == 0x47c945)
    );
    assert_eq!(
        backing_mut(&mut flags).data,
        before.iter().map(|byte| byte & 0x28).collect::<Vec<_>>()
    );
}

#[test]
fn same_value_primary_and_flag_stores_are_not_elided() {
    let mut primary = fixture(2, 2, 2);
    let mut flags = fixture(2, 2, 1);
    backing_mut(&mut primary).data.fill(0xff);
    backing_mut(&mut flags).data.fill(0x28);
    let plan = Plan::for_matrices(&primary, &flags, [0x2c, 0x34]).unwrap();
    assert_eq!(plan.stores.len(), 6);
    for store in plan.stores {
        let target = if store.slot == 0x2c {
            &mut primary
        } else {
            &mut flags
        };
        let write = store.apply(backing_mut(target)).unwrap();
        assert_eq!(write.before, write.after);
    }
}

#[test]
fn malformed_backings_and_signed_coordinate_overflow_are_rejected() {
    let primary = fixture(3, 3, 2);
    let flags = fixture(3, 3, 1);
    for fault in 0..9 {
        let mut changed = flags.clone();
        match fault {
            0 => changed.width = 2,
            1 => changed.stride = 2,
            2 => changed.allocation = Allocation::Null,
            3 => {
                backing_mut(&mut changed).data.pop();
            }
            4 => backing_mut(&mut changed).column_offsets[1] = 0,
            5 => backing_mut(&mut changed).column_offsets[2] = u32::MAX,
            6 => backing_mut(&mut changed).token = u64::MAX,
            7 => changed.height = 129,
            8 => changed.height = -1,
            _ => unreachable!(),
        }
        assert!(
            Plan::for_matrices(&primary, &changed, [0x2c, 0x34]).is_err(),
            "fault {fault}"
        );
    }
    let mut changed = primary.clone();
    backing_mut(&mut changed).column_offsets.swap(0, 1);
    assert!(Plan::for_matrices(&changed, &flags, [0x2c, 0x34]).is_err());
}
