use super::*;
use std::collections::VecDeque;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Services {
    replies: VecDeque<Allocated>,
    calls: Vec<Request>,
}

impl Host for Services {
    fn allocate(&mut self, request: Request) -> Result<Allocated, Stop> {
        let reply = self
            .replies
            .pop_front()
            .ok_or_else(|| Stop::Unavailable("test allocation service unavailable".into()))?;
        self.calls.push(request);
        Ok(reply)
    }
}

fn patterned(token: u64, bytes: usize, seed: u8) -> Owner {
    Owner {
        token,
        data: (0..bytes)
            .map(|offset| seed.wrapping_add((offset as u8).wrapping_mul(19)))
            .collect(),
    }
}

fn reply(owner: Owner) -> Allocated {
    Allocated::Bytes {
        token: owner.token,
        data: owner.data,
    }
}

fn setup(seed: u8) -> (Construction, Services) {
    let construction = Construction::new(patterned(0x10000, 72, seed)).unwrap();
    let mut replies = VecDeque::from([reply(patterned(0x20000, 96, seed.wrapping_add(1)))]);
    for index in 0..4 {
        replies.push_back(reply(patterned(
            0x30000 + index * 16,
            12,
            seed.wrapping_add(2 + index as u8),
        )));
    }
    (
        construction,
        Services {
            replies,
            calls: Vec::new(),
        },
    )
}

fn finish(execution: &mut Construction, host: &mut Services) {
    for _ in 0..100 {
        let complete = execution.step(host).unwrap();
        let encoded = serde_json::to_vec(&(execution.clone(), host.clone())).unwrap();
        let restored: (Construction, Services) = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored, (execution.clone(), host.clone()));
        (*execution, *host) = restored;
        if complete {
            return;
        }
    }
    panic!("world child construction did not finish")
}

#[test]
fn native_sparse_fields_preserve_every_unwritten_allocation_byte() {
    for seed in [0, 0x37, 0xa5, 0xff] {
        let (mut execution, mut host) = setup(seed);
        let mut expected_owner = execution.state.owner.data.clone();
        let Allocated::Bytes {
            data: mut expected_child,
            ..
        } = host.replies[0].clone()
        else {
            unreachable!()
        };
        finish(&mut execution, &mut host);
        for offset in [0, 1, 2, 8, 9, 10, 11, 12, 0x24, 0x25, 0x2a, 0x3e, 0x50] {
            expected_child[offset] = 0;
        }
        for offset in [4, 0x10, 0x14, 0x18, 0x1c, 0x20] {
            expected_child[offset..offset + 4].fill(0);
        }
        expected_child[0x20] = 1;
        assert_eq!(execution.state.child.as_ref().unwrap().data, expected_child);
        for (offset, value) in [
            (0, 0x66f3d8u32),
            (4, 0),
            (8, 0x30000),
            (12, 0x30020),
            (16, 0x30010),
            (20, 0x30030),
            (0x28, 0x20000),
            (0x2c, 0),
            (0x30, 0),
            (0x34, 0),
            (0x38, 1),
            (0x3c, 1),
            (0x40, 1),
            (0x44, 1),
        ] {
            expected_owner[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        assert_eq!(execution.state.owner.data, expected_owner);
        assert!(
            execution
                .state
                .vectors
                .iter()
                .flatten()
                .all(|owner| owner.data == vec![0; 12])
        );
        assert_eq!(
            host.calls,
            [
                Request::Child,
                Request::Vector(0),
                Request::Vector(1),
                Request::Vector(2),
                Request::Vector(3)
            ]
        );
        assert!(host.replies.is_empty());
        let completed = execution.clone();
        let calls = host.calls.clone();
        assert!(execution.step(&mut host).unwrap());
        assert_eq!(execution, completed);
        assert_eq!(host.calls, calls);
    }
}

#[test]
fn child_constructor_return_precedes_parent_reference_increment() {
    let (mut execution, mut host) = setup(0xa5);
    while execution.phase != Phase::PublishChild {
        assert!(!execution.step(&mut host).unwrap());
    }
    assert_eq!(
        &execution.state.child.as_ref().unwrap().data[0x20..0x24],
        &[0; 4]
    );
    assert_eq!(
        &execution.state.owner.data[0x28..0x2c],
        &[0x9d, 0xb0, 0xc3, 0xd6]
    );
    execution.step(&mut host).unwrap();
    assert_eq!(
        &execution.state.child.as_ref().unwrap().data[0x20..0x24],
        &[1, 0, 0, 0]
    );
    assert_eq!(
        &execution.state.owner.data[0x28..0x2c],
        &0x20000u32.to_le_bytes()
    );
}

#[test]
fn original_nullable_child_and_vector_branches_publish_null_without_fake_storage() {
    for absent in 0..5 {
        let (mut execution, mut host) = setup(0x37);
        host.replies[absent] = Allocated::Null;
        finish(&mut execution, &mut host);
        assert_eq!(execution.state.child.is_none(), absent == 0);
        for index in 0..4 {
            assert_eq!(
                execution.state.vectors[index].is_none(),
                absent == index + 1
            );
        }
        let offset = [0x28, 8, 16, 12, 20][absent];
        assert_eq!(&execution.state.owner.data[offset..offset + 4], &[0; 4]);
        assert_eq!(host.calls.len(), 5);
    }
}

#[test]
fn invalid_reply_is_retained_across_retries_and_metadata_roundtrip() {
    for owner in [
        patterned(0, 96, 0xa5),
        patterned(0x20000, 95, 0xa5),
        patterned(u32::MAX as u64 - 2, 96, 0xa5),
    ] {
        let (mut execution, mut host) = setup(0x37);
        host.replies[0] = reply(owner);
        execution.step(&mut host).unwrap();
        execution.step(&mut host).unwrap();
        let before = execution.clone();
        assert!(matches!(
            execution.step(&mut host),
            Err(Stop::Unsupported(_))
        ));
        assert_eq!(execution, before);
        let mut restored: Construction =
            serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        assert!(restored.step(&mut host).is_err());
        assert_eq!(restored, before);
        assert_eq!(host.calls, [Request::Child]);
        assert!(execution.state.child.is_none());
    }
}

#[test]
fn overlapping_returned_allocations_stop_before_native_pointer_publication() {
    for (index, token, bytes) in [
        (0, 0x10000, 96),
        (0, 0xffc0, 96),
        (1, 0x20010, 12),
        (2, 0x30004, 12),
    ] {
        let (mut execution, mut host) = setup(0xa5);
        host.replies[index] = reply(patterned(token, bytes, 0xa5));
        loop {
            match execution.step(&mut host) {
                Ok(false) => {}
                Err(Stop::Unsupported(message)) => {
                    assert!(message.contains("aliases"));
                    let before = execution.clone();
                    assert!(execution.step(&mut host).is_err());
                    assert_eq!(execution, before);
                    assert_eq!(host.calls.len(), index + 1);
                    break;
                }
                result => panic!("unexpected overlapping constructor result {result:?}"),
            }
        }
    }
}

#[test]
fn caller_owner_requires_the_actual_complete_native_width_span() {
    for owner in [
        patterned(0, 72, 0),
        patterned(0x10000, 71, 0),
        patterned(u32::MAX as u64 + 1, 72, 0),
    ] {
        assert!(Construction::new(owner).is_err());
    }
    assert!(Request::Vector(4).bytes().is_err());
    let (mut execution, mut host) = setup(0xa5);
    execution.state.owner.data.pop();
    assert!(execution.step(&mut host).is_err());
    assert!(host.calls.is_empty());
}

fn rejects_without_effects(execution: &mut Construction, host: &mut Services) {
    let before = execution.clone();
    let services = host.clone();
    assert!(
        serde_json::from_slice::<Construction>(&serde_json::to_vec(execution).unwrap()).is_err()
    );
    assert!(execution.returned().is_err());
    assert!(execution.step(host).is_err());
    assert_eq!(*execution, before);
    assert_eq!(*host, services);
}

#[test]
fn malformed_nested_cursors_stop_before_indexing_or_allocator_reentry() {
    for phase in [
        Phase::Allocate(Request::Vector(4)),
        Phase::Accept(Request::Vector(255)),
        Phase::InitializeVector(255),
        Phase::PublishVector(255),
        Phase::Accept(Request::Child),
    ] {
        let (mut execution, mut host) = setup(0xa5);
        execution.phase = phase;
        rejects_without_effects(&mut execution, &mut host);
    }
    let (mut execution, mut host) = setup(0xa5);
    execution.step(&mut host).unwrap();
    execution.step(&mut host).unwrap();
    execution.phase = Phase::Allocate(Request::Child);
    rejects_without_effects(&mut execution, &mut host);
    assert_eq!(host.calls, [Request::Child]);
}

#[test]
fn retained_cursor_requires_matching_child_publication_and_initialized_fields() {
    for mutation in 0..6 {
        let (mut execution, mut host) = setup(0xa5);
        match mutation {
            0 => execution.state.child = Some(patterned(0x20000, 96, 0xa5)),
            1 => execution.state.vectors[3] = Some(patterned(0x30030, 12, 0xa5)),
            2 => {
                while execution.phase != Phase::PublishChild {
                    execution.step(&mut host).unwrap();
                }
                execution.state.child.as_mut().unwrap().data[0] = 1;
            }
            3 => {
                while execution.phase != Phase::Allocate(Request::Vector(0)) {
                    execution.step(&mut host).unwrap();
                }
                execution.state.child = None;
            }
            4 => {
                while execution.phase != Phase::PublishVector(0) {
                    execution.step(&mut host).unwrap();
                }
                execution.state.vectors[0].as_mut().unwrap().data[0] = 1;
            }
            _ => {
                finish(&mut execution, &mut host);
                execution.state.vectors[0] = None;
            }
        }
        rejects_without_effects(&mut execution, &mut host);
    }
}
