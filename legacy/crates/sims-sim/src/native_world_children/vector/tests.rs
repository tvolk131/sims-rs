use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Services {
    reply: Option<Allocated>,
    allocations: Vec<u32>,
    blocked: bool,
}

impl Host for Services {
    fn allocate_vector_elements(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        if self.blocked {
            return Err(Stop::Unavailable(
                "element-vector allocator is unavailable".into(),
            ));
        }
        let reply = self
            .reply
            .take()
            .ok_or_else(|| Stop::Unsupported("repeated element-vector allocation".into()))?;
        self.allocations.push(bytes);
        Ok(reply)
    }
}

fn setup(seed: u8) -> (Construction, Services) {
    (
        Construction::new(Owner {
            token: 0x10000,
            data: vec![seed; 12],
        })
        .unwrap(),
        Services {
            reply: Some(Allocated::Bytes {
                token: 0x20000,
                data: (0..BYTES)
                    .map(|offset| seed.wrapping_add(offset as u8))
                    .collect(),
            }),
            allocations: Vec::new(),
            blocked: false,
        },
    )
}

fn finish(execution: &mut Construction, host: &mut Services) -> usize {
    for transitions in 1..=20 {
        let complete = execution.step(host).unwrap();
        let encoded = serde_json::to_vec(&(execution.clone(), host.clone())).unwrap();
        let restored: (Construction, Services) = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored, (execution.clone(), host.clone()));
        (*execution, *host) = restored;
        if complete {
            return transitions;
        }
    }
    panic!("element-vector construction failed to complete")
}

#[test]
fn genuine_fresh_vector_initialization_growth_fill_and_bounds_roundtrip_every_stage() {
    for seed in [0, 0x37, 0xa5, 0xff] {
        let (mut execution, mut host) = setup(seed);
        assert_eq!(finish(&mut execution, &mut host), 5);
        assert_eq!(
            execution.state.backing.as_ref().unwrap().data,
            vec![0x7f; 16384]
        );
        let expected: Vec<_> = [0x20000u32, 0x24000, 0x24000]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        assert_eq!(execution.state.owner.data, expected);
        assert_eq!(host.allocations, [16384]);
        let before = (execution.clone(), host.clone());
        assert!(execution.step(&mut host).unwrap());
        assert_eq!((execution, host), before);
    }
}

#[test]
fn initial_empty_header_return_is_distinct_from_resized_publication() {
    let (mut execution, mut host) = setup(0xa5);
    assert_eq!(execution.state.owner.data, [0xa5; 12]);
    assert!(!execution.step(&mut host).unwrap());
    assert_eq!(execution.state.owner.data, [0; 12]);
    assert_eq!(execution.phase, Phase::Allocate);
    assert!(execution.state.backing.is_none());
    assert!(host.allocations.is_empty());
    while execution.phase != Phase::Publish {
        assert!(!execution.step(&mut host).unwrap());
    }
    assert_eq!(execution.state.owner.data, [0; 12]);
    assert_eq!(
        execution.state.backing.as_ref().unwrap().data,
        vec![0x7f; BYTES]
    );
    assert!(execution.step(&mut host).unwrap());
}

#[test]
fn absent_allocator_cannot_turn_into_constructor_success_or_placeholder_storage() {
    let (mut execution, mut host) = setup(0xa5);
    host.blocked = true;
    execution.step(&mut host).unwrap();
    let before = execution.clone();
    assert!(matches!(
        execution.step(&mut host),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(execution, before);
    assert!(host.allocations.is_empty());
    host.blocked = false;
    finish(&mut execution, &mut host);
    assert_eq!(host.allocations, [16384]);
}

#[test]
fn null_allocator_reply_remains_pending_without_reallocation_or_positive_size_success() {
    let (mut execution, mut host) = setup(0xa5);
    host.reply = Some(Allocated::Null);
    execution.step(&mut host).unwrap();
    execution.step(&mut host).unwrap();
    for _ in 0..3 {
        let before = execution.clone();
        assert!(matches!(
            execution.step(&mut host),
            Err(Stop::Unavailable(_))
        ));
        assert_eq!(execution, before);
        execution = serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
    }
    assert_eq!(host.allocations, [16384]);
    assert!(execution.state.backing.is_none());
    assert_eq!(execution.state.owner.data, [0; 12]);
}

#[test]
fn malformed_or_overlapping_allocator_replies_remain_owned_without_publication() {
    for (token, bytes) in [
        (0, BYTES),
        (0x20000, BYTES - 1),
        (0x20000, BYTES + 1),
        (0x10000, BYTES),
        (0xc008, BYTES),
        (u32::MAX as u64 - BYTES as u64 + 1, BYTES),
        (u32::MAX as u64 + 1, BYTES),
    ] {
        let (mut execution, mut host) = setup(0xa5);
        host.reply = Some(Allocated::Bytes {
            token,
            data: vec![0xa5; bytes],
        });
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
        assert_eq!(host.allocations, [16384]);
        assert!(execution.state.backing.is_none());
    }
}

#[test]
fn restored_stage_ownership_header_and_fill_validation_fail_before_new_effects() {
    let (mut execution, mut host) = setup(0xa5);
    let mut corrupted = execution.clone();
    corrupted.phase = Phase::Complete;
    assert!(corrupted.step(&mut host).is_err());
    execution.step(&mut host).unwrap();
    let mut corrupted = execution.clone();
    corrupted.state.owner.data[0] = 1;
    assert!(corrupted.step(&mut host).is_err());
    assert!(host.allocations.is_empty());
    while execution.phase != Phase::Publish {
        execution.step(&mut host).unwrap();
    }
    let mut corrupted = execution.clone();
    corrupted.state.backing.as_mut().unwrap().data[100] = 0;
    assert!(corrupted.step(&mut host).is_err());
    execution.step(&mut host).unwrap();
    execution.state.owner.data[8] ^= 1;
    assert!(execution.step(&mut host).is_err());
    assert_eq!(host.allocations, [16384]);
}
