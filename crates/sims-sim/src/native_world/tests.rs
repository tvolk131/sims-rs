use super::*;
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Services {
    replies: VecDeque<Allocated>,
    calls: Vec<Request>,
}

impl Host for Services {
    fn allocate(&mut self, request: Request) -> Result<Allocated, Stop> {
        request.bytes()?;
        let reply = self
            .replies
            .pop_front()
            .ok_or_else(|| Stop::Unavailable("world-prefix allocator unavailable".into()))?;
        self.calls.push(request);
        Ok(reply)
    }
}

fn allocation(token: u64, bytes: usize) -> Allocated {
    Allocated::Bytes {
        token,
        data: vec![0xa5; bytes],
    }
}

fn registry(seed: u32) -> Registry<u32> {
    let mut registry = Registry {
        generation: seed,
        ..Registry::default()
    };
    registry.construct(0x1000).unwrap();
    registry.bind(0x1000, 0x667ec0).unwrap();
    registry
}

fn services() -> Services {
    Services {
        replies: [
            allocation(0x2000, 136),
            allocation(0x3000, 72),
            allocation(0x4000, 96),
            allocation(0x5000, 12),
            allocation(0x6000, 12),
            allocation(0x7000, 12),
            allocation(0x8000, 12),
        ]
        .into(),
        calls: Vec::new(),
    }
}

fn finish(prefix: &mut Prefix, host: &mut Services) -> Cut {
    for _ in 0..100 {
        let cut = prefix.step(host).unwrap();
        let restored: (Prefix, Services) =
            serde_json::from_slice(&serde_json::to_vec(&(&prefix, &host)).unwrap()).unwrap();
        assert_eq!((&restored.0, &restored.1), (&*prefix, &*host));
        (*prefix, *host) = restored;
        if let Some(cut) = cut {
            return cut;
        }
    }
    panic!("world-prefix construction did not reach its bounded cut");
}

#[test]
fn composes_original_wrapper_commander_and_companion_ownership() {
    let original = registry(0);
    let mut prefix = Prefix::new(original.clone()).unwrap();
    let mut host = services();
    assert_eq!(finish(&mut prefix, &mut host), Cut::ArrayConstructionEntry);
    assert!(host.replies.is_empty());
    assert_eq!(
        host.calls,
        [
            Request::World,
            Request::Companion,
            Request::CompanionChild,
            Request::CompanionVector(0),
            Request::CompanionVector(1),
            Request::CompanionVector(2),
            Request::CompanionVector(3),
        ]
    );
    let mut expected = vec![0xa5; 136];
    for (offset, word) in [(0, 0x667a88_u32), (4, 0x1000), (8, 0), (12, 1)] {
        expected[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
    }
    for offset in cleared_slots() {
        expected[offset..offset + 4].fill(0);
    }
    expected[0x54..0x58].copy_from_slice(&0x3000_u32.to_le_bytes());
    assert_eq!(prefix.owner().unwrap().data, expected);
    assert_eq!(prefix.registry().head, Some(0x2000));
    assert_eq!(prefix.registry().generation, 2);
    assert_eq!(prefix.registry().nodes[&0x1000], original.nodes[&0x1000]);
    let retained_calls = host.calls.clone();
    assert_eq!(
        prefix.step(&mut host).unwrap(),
        Some(Cut::ArrayConstructionEntry)
    );
    assert_eq!(host.calls, retained_calls);
    let parts = prefix.into_parts().unwrap();
    assert_eq!(parts.owner.data, expected);
    let companion = parts.companion.unwrap();
    assert_eq!(companion.owner.token, 0x3000);
    assert_eq!(companion.child.unwrap().token, 0x4000);
    assert!(companion.vectors.iter().all(Option::is_some));
}

#[test]
fn typed_domain_generation_wraps_without_native_static_assumption() {
    for seed in [0xffff_fffe, 0xffff_ffff] {
        let mut prefix = Prefix::new(registry(seed)).unwrap();
        assert_eq!(
            finish(&mut prefix, &mut services()),
            Cut::ArrayConstructionEntry
        );
        assert_eq!(prefix.registry().generation, seed.wrapping_add(2));
        assert_eq!(
            prefix.registry().nodes[&0x2000].serial,
            seed.wrapping_add(1)
        );
    }
}

#[test]
fn null_world_preserves_domain_and_never_constructs_an_owner() {
    let original = registry(7);
    let mut prefix = Prefix::new(original.clone()).unwrap();
    let mut host = services();
    host.replies = [Allocated::Null].into();
    assert_eq!(finish(&mut prefix, &mut host), Cut::NullWorld);
    assert_eq!(prefix.registry(), &original);
    assert_eq!(host.calls, [Request::World]);
    assert!(prefix.owner().is_none());
    assert!(prefix.into_parts().is_err());
}

#[test]
fn null_companion_publishes_null_without_fabricating_child_storage() {
    let mut prefix = Prefix::new(registry(0)).unwrap();
    let mut host = services();
    host.replies = [allocation(0x2000, 136), Allocated::Null].into();
    assert_eq!(finish(&mut prefix, &mut host), Cut::ArrayConstructionEntry);
    assert_eq!(host.calls, [Request::World, Request::Companion]);
    assert!(prefix.companion().is_none());
    assert_eq!(&prefix.owner().unwrap().data[0x54..0x58], &[0; 4]);
}

#[test]
fn invalid_world_replies_are_retained_without_reallocating() {
    for reply in [
        allocation(0, 136),
        allocation(0x2000, 135),
        allocation(0x2000, 137),
        allocation(0xffff_fff0, 136),
        allocation(0x1004, 136),
    ] {
        let mut prefix = Prefix::new(registry(0)).unwrap();
        let mut host = services();
        host.replies = [reply].into();
        assert_eq!(prefix.step(&mut host).unwrap(), None);
        let suspended = prefix.clone();
        assert!(prefix.step(&mut host).is_err());
        assert_eq!(prefix, suspended);
        assert!(prefix.step(&mut host).is_err());
        assert_eq!(host.calls, [Request::World]);
    }
}

#[test]
fn allocator_unavailability_preserves_cursor_and_retries_only_the_missing_call() {
    let mut prefix = Prefix::new(registry(0)).unwrap();
    let mut host = Services {
        replies: VecDeque::new(),
        calls: Vec::new(),
    };
    let before = prefix.clone();
    assert!(prefix.step(&mut host).is_err());
    assert_eq!(prefix, before);
    host = services();
    assert_eq!(finish(&mut prefix, &mut host), Cut::ArrayConstructionEntry);
    assert_eq!(host.calls.len(), 7);
}

#[test]
fn corrupted_restored_prefix_ownership_never_reaches_array_entry() {
    let mut prefix = Prefix::new(registry(0)).unwrap();
    let mut host = services();
    finish(&mut prefix, &mut host);
    let mut variants = Vec::new();
    let mut changed = prefix.clone();
    changed.owner.as_mut().unwrap().data[0x54] ^= 1;
    variants.push(changed);
    let mut changed = prefix.clone();
    changed.owner.as_mut().unwrap().data[0x14] = 1;
    variants.push(changed);
    let mut changed = prefix.clone();
    changed.owner.as_mut().unwrap().data[0] ^= 1;
    variants.push(changed);
    let mut changed = prefix.clone();
    changed.registry.nodes.get_mut(&0x2000).unwrap().serial += 1;
    variants.push(changed);
    let mut changed = prefix.clone();
    changed
        .companion
        .as_mut()
        .unwrap()
        .state
        .child
        .as_mut()
        .unwrap()
        .token = 0x2000;
    variants.push(changed);
    let mut changed = prefix.clone();
    changed.pending = Some(Allocated::Null);
    variants.push(changed);
    let mut changed = prefix.clone();
    changed
        .companion
        .as_mut()
        .unwrap()
        .state
        .child
        .as_mut()
        .unwrap()
        .data[0x20] = 0;
    variants.push(changed);
    let mut changed = prefix.clone();
    changed.companion.as_mut().unwrap().state.owner.data[8] ^= 1;
    variants.push(changed);
    for mut changed in variants {
        let encoded = serde_json::to_vec(&changed).unwrap();
        let before = changed.clone();
        assert!(changed.step(&mut host).is_err());
        assert_eq!(changed, before);
        assert!(changed.into_parts().is_err());
        if let Ok(mut restored) = serde_json::from_slice::<Prefix>(&encoded) {
            assert!(restored.step(&mut host).is_err());
            assert!(restored.into_parts().is_err());
        }
    }
    assert_eq!(host.calls.len(), 7);
}
