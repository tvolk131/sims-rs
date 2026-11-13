use super::*;
use crate::native_game::object_registry::{
    Construction, Cut, Host, Incoming, PublishedSimulator, State,
};
use std::sync::OnceLock;

mod simulator_binding;

fn published() -> PublishedSimulator {
    static SOURCE: OnceLock<crate::native_game::simulator::State> = OnceLock::new();
    let source = SOURCE.get_or_init(|| {
        let mut execution = super::construction();
        super::reach(&mut execution, &mut super::host(), super::Cut::Returned);
        execution.into_state().unwrap()
    });
    PublishedSimulator::new(source.clone()).unwrap()
}

fn incoming(byte: u8) -> Incoming {
    Incoming {
        owner: Owner {
            token: 0x7a000000,
            data: (0..272).map(|index| (index % 251) as u8).collect(),
        },
        comparator: Owner {
            token: 0x7b000000,
            data: vec![byte],
        },
    }
}

struct SentinelHost {
    reply: Allocated,
    calls: usize,
    deny: bool,
}

impl Host for SentinelHost {
    fn allocate_sentinel(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        assert_eq!(bytes, 24);
        self.calls += 1;
        if self.deny {
            Err(Stop::Unsupported(
                "test sentinel allocator unavailable".into(),
            ))
        } else {
            Ok(self.reply.clone())
        }
    }
}

fn host() -> SentinelHost {
    SentinelHost {
        reply: Allocated::Bytes {
            token: 0x7a000200,
            data: (0..24).map(|index| 0x71 + index).collect(),
        },
        calls: 0,
        deny: false,
    }
}

fn construction(byte: u8) -> Construction {
    Construction::new(published(), incoming(byte)).unwrap()
}

fn reach(execution: &mut Construction, host: &mut SentinelHost, cut: Cut) {
    for _ in 0..5 {
        if execution.cut() == cut {
            return;
        }
        execution.step(host).unwrap();
    }
    panic!("Object-registry construction did not reach {cut:?}");
}

#[test]
fn registry_publication_changes_only_actual_game_simulator_word() {
    let publication = published();
    let source = publication.source();
    let original = source.construction().source().owner();
    let simulator = source.construction().owner().token as u32;
    let mut expected = original.clone();
    expected.data[0x20..0x24].copy_from_slice(&simulator.to_le_bytes());
    assert_eq!(publication.game(), &expected);
    assert_eq!(publication.registry(), source.construction().registry());
    assert_eq!(publication.writes().len(), 1);
    let write = &publication.writes()[0];
    assert_eq!(
        (write.owner, write.offset, write.instruction),
        (original.token, 0x20, 0x415cd1)
    );
    assert_eq!(write.before, original.data[0x20..0x24]);
    assert_eq!(write.after, simulator.to_le_bytes());
    assert_eq!(
        serde_json::from_value::<PublishedSimulator>(serde_json::to_value(&publication).unwrap())
            .unwrap(),
        publication
    );
}

#[test]
fn registry_publication_restore_rejects_owner_source_store_and_preimage_changes() {
    let original = serde_json::to_value(published()).unwrap();
    for pointer in [
        "/game/token",
        "/game/data/32",
        "/writes/0/offset",
        "/writes/0/instruction",
        "/writes/0/before/0",
        "/writes/0/after/0",
    ] {
        let mut broken = original.clone();
        let value = broken.pointer_mut(pointer).unwrap();
        *value = serde_json::json!(value.as_u64().unwrap() ^ 1);
        assert!(
            serde_json::from_value::<PublishedSimulator>(broken).is_err(),
            "{pointer}"
        );
    }
    let mut broken = original;
    broken["writes"] = serde_json::json!([]);
    assert!(serde_json::from_value::<PublishedSimulator>(broken).is_err());
}

#[test]
fn registry_complete_stores_preserve_nonzero_comparator_and_untouched_owner_bytes() {
    let mut execution = construction(0xd3);
    let original = execution.incoming().clone();
    let source = execution.source().clone();
    let mut host = host();
    reach(&mut execution, &mut host, Cut::Returned);
    assert_eq!(execution.source(), &source);
    assert_eq!(execution.incoming(), &original);
    assert_eq!(execution.owners().len(), 2);
    assert_eq!(execution.writes().len(), 76);
    assert_eq!(host.calls, 1);
    let root = execution.owner();
    let sentinel = &execution.owners()[1];
    assert_eq!(super::word(&root.data, 0), 0x667054);
    assert_eq!(
        super::word(&root.data, 0x7c),
        source.source().construction().owner().token as u32
    );
    assert_eq!(root.data[0x78], 0xd3);
    let read = execution.comparator_read().unwrap();
    assert_eq!(
        (read.source, read.instruction, read.byte),
        (original.comparator.token, 0x448a3e, 0xd3)
    );
    assert_eq!(root.data[0x44], 1);
    assert_eq!(&root.data[0x100..0x102], &[0xff, 0xff]);
    assert_eq!(&root.data[0x102..0x104], &original.owner.data[0x102..0x104]);
    assert_eq!(super::word(&sentinel.data, 8), sentinel.token as u32);
    assert_eq!(super::word(&sentinel.data, 12), sentinel.token as u32);
    let Allocated::Bytes { data, .. } = &host.reply else {
        unreachable!()
    };
    let mut expected = BTreeMap::from([
        (root.token, original.owner.data.clone()),
        (sentinel.token, data.clone()),
    ]);
    let mut written = BTreeMap::from([
        (root.token, vec![false; 272]),
        (sentinel.token, vec![false; 24]),
    ]);
    for write in execution.writes() {
        let bytes = expected.get_mut(&write.owner).unwrap();
        assert_eq!(
            &bytes[write.offset..write.offset + write.before.len()],
            &write.before
        );
        bytes[write.offset..write.offset + write.after.len()].copy_from_slice(&write.after);
        written.get_mut(&write.owner).unwrap()[write.offset..write.offset + write.after.len()]
            .fill(true);
    }
    assert_eq!(expected[&root.token], root.data);
    assert_eq!(expected[&sentinel.token], sentinel.data);
    assert_eq!(
        written[&root.token].iter().filter(|byte| **byte).count(),
        247
    );
    assert_eq!(
        written[&sentinel.token]
            .iter()
            .filter(|byte| **byte)
            .count(),
        13
    );
    for (owner, initial) in [(root, &original.owner.data), (sentinel, data)] {
        for (index, was_written) in written[&owner.token].iter().enumerate() {
            if !was_written {
                assert_eq!(owner.data[index], initial[index]);
            }
        }
    }
    assert_eq!(execution.registry().head, Some(root.token as u32));
    assert_eq!(
        execution.registry().generation,
        source.registry().generation + 1
    );
    for (identity, node) in &source.registry().nodes {
        assert_eq!(&execution.registry().nodes[identity], node);
    }
}

#[test]
fn registry_all_five_cuts_restore_without_repeating_accepted_sentinel_allocation() {
    let mut execution = construction(0x91);
    let mut original_host = host();
    let mut expected = execution.clone();
    reach(&mut expected, &mut host(), Cut::Returned);
    for (index, cut) in [
        Cut::Entry,
        Cut::Registered,
        Cut::TreeAllocationEntry,
        Cut::TreeAllocated,
        Cut::Returned,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(execution.cut(), cut);
        let mut restored: Construction =
            serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        let mut restored_host = host();
        restored_host.deny = index >= 3;
        reach(&mut restored, &mut restored_host, Cut::Returned);
        assert_eq!(restored, expected);
        assert_eq!(restored_host.calls, usize::from(index < 3));
        execution.step(&mut original_host).unwrap();
    }
    assert_eq!(original_host.calls, 1);
    let state = execution.into_state().unwrap();
    assert_eq!(
        serde_json::from_value::<State>(serde_json::to_value(&state).unwrap()).unwrap(),
        state
    );
}

#[test]
fn registry_rejects_incomplete_or_aliasing_outer_owner_and_missing_caller_byte() {
    let source = published();
    for (token, bytes) in [
        (0, 272),
        (0x7a000000, 271),
        (0x7a000000, 273),
        (u64::MAX, 272),
        (0x6de850, 272),
        (0x6e5500, 272),
        (source.game().token + 64, 272),
        (source.source().construction().owner().token + 100, 272),
    ] {
        let mut input = incoming(0xb9);
        input.owner = Owner {
            token,
            data: vec![0; bytes],
        };
        assert!(Construction::new(source.clone(), input).is_err());
    }
    for (token, bytes) in [
        (0, 1),
        (0x7b000000, 0),
        (0x7b000000, 2),
        (0x7a000010, 1),
        (source.game().token, 1),
        (0x6de858, 1),
        (u64::MAX, 1),
    ] {
        let mut input = incoming(0xb9);
        input.comparator = Owner {
            token,
            data: vec![0; bytes],
        };
        assert!(Construction::new(source.clone(), input).is_err());
    }
    let mut missing = serde_json::to_value(incoming(0xb9)).unwrap();
    missing.as_object_mut().unwrap().remove("comparator");
    assert!(serde_json::from_value::<Incoming>(missing).is_err());
}

#[test]
fn registry_invalid_accepted_sentinel_reply_stays_failed_without_new_host_calls() {
    let base = construction(0x63);
    for reply in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0x7a000100,
            data: vec![0; 24],
        },
        Allocated::Bytes {
            token: 0x7b000000,
            data: vec![0; 24],
        },
        Allocated::Bytes {
            token: base.source().game().token + 96,
            data: vec![0; 24],
        },
        Allocated::Bytes {
            token: 0x6de858,
            data: vec![0; 24],
        },
        Allocated::Bytes {
            token: 0x7a000200,
            data: vec![0; 23],
        },
        Allocated::Bytes {
            token: u64::from(u32::MAX) - 10,
            data: vec![0; 24],
        },
    ] {
        let mut execution = base.clone();
        let mut host = SentinelHost {
            reply,
            calls: 0,
            deny: false,
        };
        reach(&mut execution, &mut host, Cut::TreeAllocationEntry);
        assert!(execution.step(&mut host).is_err());
        assert_eq!(execution.cut(), Cut::TreeAllocated);
        assert!(execution.step(&mut host).is_err());
        assert_eq!(host.calls, 1);
        assert!(
            serde_json::from_value::<Construction>(serde_json::to_value(execution).unwrap())
                .is_err()
        );
    }
}

#[test]
fn registry_unavailable_allocator_preserves_pending_cursor_and_bytes() {
    let mut execution = construction(0x55);
    let mut host = host();
    reach(&mut execution, &mut host, Cut::TreeAllocationEntry);
    let before = execution.clone();
    host.deny = true;
    assert!(execution.step(&mut host).is_err());
    assert_eq!(execution, before);
    host.deny = false;
    reach(&mut execution, &mut host, Cut::Returned);
    assert_eq!(host.calls, 2);
}

#[test]
fn registry_restore_rejects_changed_caller_source_owners_registry_reply_and_stores() {
    let mut execution = construction(0x83);
    reach(&mut execution, &mut host(), Cut::Returned);
    let original = serde_json::to_value(execution).unwrap();
    for pointer in [
        "/incoming/comparator/data/0",
        "/incoming/comparator/token",
        "/comparator_read/source",
        "/comparator_read/instruction",
        "/comparator_read/byte",
        "/incoming/owner/token",
        "/owners/0/data/120",
        "/owners/1/data/0",
        "/registry/generation",
        "/sentinel_reply/Bytes/token",
        "/writes/0/instruction",
        "/writes/25/before/0",
    ] {
        let mut broken = original.clone();
        let value = broken.pointer_mut(pointer).unwrap();
        *value = serde_json::json!(value.as_u64().unwrap() ^ 1);
        assert!(
            serde_json::from_value::<Construction>(broken).is_err(),
            "{pointer}"
        );
    }
    for cut in [
        "Entry",
        "Registered",
        "TreeAllocationEntry",
        "TreeAllocated",
    ] {
        let mut broken = original.clone();
        broken["cut"] = serde_json::json!(cut);
        assert!(serde_json::from_value::<Construction>(broken).is_err());
    }
    let mut broken = original.clone();
    broken["writes"].as_array_mut().unwrap().swap(27, 28);
    assert!(serde_json::from_value::<Construction>(broken).is_err());
    let mut broken = original;
    broken["owners"].as_array_mut().unwrap().remove(1);
    assert!(serde_json::from_value::<Construction>(broken).is_err());
}

#[test]
fn registry_guards_all_prior_owners_caller_input_and_unlinked_accepted_sentinel() {
    let mut execution = construction(0x47);
    reach(&mut execution, &mut host(), Cut::TreeAllocated);
    for (token, bytes) in [
        (0x7a000000, 272),
        (0x7a000200, 24),
        (0x7a0001ff, 2),
        (0x7b000000, 1),
        (execution.source().game().token + 100, 1),
        (0x79000080, 28),
        (0x79000200, 1),
        (0x6de858, 8),
        (0x6e5528, 4),
        (u64::MAX, 1),
    ] {
        assert!(execution.guard_span(token, bytes).is_err());
    }
    assert!(execution.guard_span(0x7a000110, 24).is_ok());
    assert!(execution.guard_span(0x7a000218, 24).is_ok());
    assert!(execution.guard_span(0x7b000001, 24).is_ok());
}

#[test]
fn registry_completed_state_cannot_deserialize_before_the_actual_return() {
    let mut execution = construction(0xe7);
    assert!(execution.clone().into_state().is_err());
    for cut in [
        Cut::Entry,
        Cut::Registered,
        Cut::TreeAllocationEntry,
        Cut::TreeAllocated,
    ] {
        reach(&mut execution, &mut host(), cut);
        assert!(
            serde_json::from_value::<State>(serde_json::json!({"construction": execution}))
                .is_err()
        );
    }
}
