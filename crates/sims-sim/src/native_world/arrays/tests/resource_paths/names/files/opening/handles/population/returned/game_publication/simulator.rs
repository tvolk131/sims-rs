use super::*;
use crate::native_game::simulator::{Construction, Cut, Host, State};

mod object_registry;

struct ListHost {
    reply: Allocated,
    calls: usize,
    deny: bool,
}

impl Host for ListHost {
    fn allocate_list(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        assert_eq!(bytes, 28);
        self.calls += 1;
        if self.deny {
            Err(Stop::Unsupported("test list allocator unavailable".into()))
        } else {
            Ok(self.reply.clone())
        }
    }
}

fn host() -> ListHost {
    ListHost {
        reply: Allocated::Bytes {
            token: 0x79000200,
            data: (0..28).map(|index| 0x41 + index).collect(),
        },
        calls: 0,
        deny: false,
    }
}

fn construction() -> Construction {
    Construction::new(
        super::finished(super::world_returned(0xd9))
            .into_state()
            .unwrap(),
        Owner {
            token: 0x79000000,
            data: (0..428).map(|index| (index % 251) as u8).collect(),
        },
    )
    .unwrap()
}

fn reach(execution: &mut Construction, host: &mut ListHost, cut: Cut) {
    for _ in 0..6 {
        if execution.cut() == cut {
            return;
        }
        execution.step(host).unwrap();
    }
    panic!("Simulator constructor did not reach {cut:?}");
}

fn word(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

#[test]
fn simulator_matches_ordered_native_initialization_and_preserves_all_untouched_bytes() {
    let mut execution = construction();
    let original = execution.incoming().clone();
    let source = execution.source().clone();
    let mut host = host();
    reach(&mut execution, &mut host, Cut::Returned);
    assert_eq!(execution.source(), &source);
    assert_eq!(host.calls, 1);
    assert_eq!(execution.owners().len(), 2);
    assert_eq!(execution.writes().len(), 104);
    let data = &execution.owner().data;
    assert_eq!(word(data, 0), 0x667714);
    assert_eq!(word(data, 4), source.registry().head.unwrap_or(0));
    assert_eq!(word(data, 12), source.registry().generation);
    for offset in [0xc8, 0xd0, 0xd8, 0xdc] {
        assert_eq!(
            word(data, offset),
            match offset {
                0xc8 => 100,
                0xd0 => 30,
                0xd8 => 60,
                _ => 90,
            }
        );
    }
    let list = &execution.owners()[1];
    assert_eq!(word(data, 0x1a4), list.token as u32);
    assert_eq!(word(&list.data, 0), list.token as u32);
    assert_eq!(word(&list.data, 4), list.token as u32);
    let Allocated::Bytes { data: initial, .. } = &host.reply else {
        unreachable!()
    };
    assert_eq!(&list.data[8..], &initial[8..]);
    for (offset, bytes) in [(0x90, 12), (0xa8, 8), (0xd5, 3), (0xe0, 4), (0x1a8, 4)] {
        assert_eq!(
            &data[offset..offset + bytes],
            &original.data[offset..offset + bytes]
        );
    }
    let mut expected = original.data.clone();
    let mut list_bytes = initial.clone();
    for write in execution.writes() {
        let target = if write.owner == original.token {
            &mut expected
        } else {
            assert_eq!(write.owner, list.token);
            &mut list_bytes
        };
        assert_eq!(
            &target[write.offset..write.offset + write.before.len()],
            &write.before
        );
        target[write.offset..write.offset + write.after.len()].copy_from_slice(&write.after);
    }
    assert_eq!(expected, *data);
    assert_eq!(list_bytes, list.data);
    assert_eq!(execution.writes()[56].instruction, 0x46ce17);
    assert_eq!(execution.writes()[57].instruction, 0x46ce19);
    assert_eq!(execution.writes()[58].instruction, 0x46ce1e);
    assert_eq!(execution.writes()[59].instruction, 0x46ce29);
    assert_eq!(
        execution.registry().nodes.len(),
        source.registry().nodes.len() + 1
    );
    assert_eq!(execution.registry().head, Some(original.token as u32));
    assert_eq!(
        execution.registry().nodes[&(original.token as u32)].handler,
        0x667714
    );
}

#[test]
fn simulator_all_six_cuts_restore_without_repeating_accepted_allocation() {
    let mut execution = construction();
    let mut original_host = host();
    let mut expected = execution.clone();
    reach(&mut expected, &mut host(), Cut::Returned);
    for (index, cut) in [
        Cut::Entry,
        Cut::Registered,
        Cut::BuffersInitialized,
        Cut::ListAllocated,
        Cut::ListLinked,
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
    assert_eq!(execution.step(&mut original_host).unwrap(), None);
    let state = execution.into_state().unwrap();
    assert_eq!(
        serde_json::from_slice::<State>(&serde_json::to_vec(&state).unwrap()).unwrap(),
        state
    );
}

#[test]
fn simulator_refuses_outer_allocation_aliases_and_incomplete_extents() {
    let source = construction().source().clone();
    for (token, bytes) in [
        (0, 428),
        (0x79000000, 427),
        (0x79000000, 429),
        (u64::from(u32::MAX) - 100, 428),
        (u64::MAX, 428),
        (0x6de850, 428),
        (0x6e5500, 428),
        (source.owner().token + 64, 428),
        (source.world().owner().token + 100, 428),
    ] {
        assert!(
            Construction::new(
                source.clone(),
                Owner {
                    token,
                    data: vec![0x59; bytes]
                }
            )
            .is_err()
        );
    }
}

#[test]
fn simulator_rejects_bad_list_replies_without_repeating_host_effects() {
    let base = construction();
    let replies = [
        Allocated::Null,
        Allocated::Bytes {
            token: base.incoming().token + 400,
            data: vec![0; 28],
        },
        Allocated::Bytes {
            token: base.source().owner().token + 40,
            data: vec![0; 28],
        },
        Allocated::Bytes {
            token: 0x6de850,
            data: vec![0; 28],
        },
        Allocated::Bytes {
            token: 0x6e5520,
            data: vec![0; 28],
        },
        Allocated::Bytes {
            token: 0x79000200,
            data: vec![0; 27],
        },
        Allocated::Bytes {
            token: 0,
            data: vec![0; 28],
        },
        Allocated::Bytes {
            token: u64::from(u32::MAX) - 20,
            data: vec![0; 28],
        },
    ];
    for reply in replies {
        let mut execution = base.clone();
        let mut host = ListHost {
            reply,
            calls: 0,
            deny: false,
        };
        reach(&mut execution, &mut host, Cut::BuffersInitialized);
        assert!(execution.step(&mut host).is_err());
        assert_eq!(execution.cut(), Cut::ListAllocated);
        assert!(execution.step(&mut host).is_err());
        assert_eq!(host.calls, 1);
        assert!(
            serde_json::from_value::<Construction>(serde_json::to_value(execution).unwrap())
                .is_err()
        );
    }
}

#[test]
fn simulator_unavailable_allocator_has_no_accepted_reply_or_partial_store() {
    let mut execution = construction();
    let mut host = host();
    reach(&mut execution, &mut host, Cut::BuffersInitialized);
    let before = execution.clone();
    host.deny = true;
    assert!(execution.step(&mut host).is_err());
    assert_eq!(execution, before);
    host.deny = false;
    reach(&mut execution, &mut host, Cut::Returned);
    assert_eq!(host.calls, 2);
}

#[test]
fn simulator_restore_rejects_forged_cursor_owner_registry_reply_and_ordered_stores() {
    let mut execution = construction();
    reach(&mut execution, &mut host(), Cut::Returned);
    let original = serde_json::to_value(execution).unwrap();
    for field in [
        "Entry",
        "Registered",
        "BuffersInitialized",
        "ListAllocated",
        "ListLinked",
    ] {
        let mut broken = original.clone();
        broken["cut"] = serde_json::json!(field);
        assert!(serde_json::from_value::<Construction>(broken).is_err());
    }
    for pointer in [
        "/owners/0/data/0",
        "/owners/1/data/0",
        "/registry/generation",
        "/writes/0/instruction",
        "/writes/0/before/0",
        "/writes/57/owner",
        "/list_reply/Bytes/token",
    ] {
        let mut broken = original.clone();
        let value = broken.pointer_mut(pointer).unwrap();
        *value = serde_json::json!(value.as_u64().unwrap() ^ 1);
        assert!(
            serde_json::from_value::<Construction>(broken).is_err(),
            "{pointer}"
        );
    }
    let mut broken = original.clone();
    broken["owners"].as_array_mut().unwrap().remove(1);
    assert!(serde_json::from_value::<Construction>(broken).is_err());
    let mut broken = original;
    broken["writes"].as_array_mut().unwrap().swap(57, 58);
    assert!(serde_json::from_value::<Construction>(broken).is_err());
}

#[test]
fn simulator_completed_state_cannot_claim_an_earlier_constructor_cut() {
    let mut execution = construction();
    assert!(execution.clone().into_state().is_err());
    for cut in [
        Cut::Entry,
        Cut::Registered,
        Cut::BuffersInitialized,
        Cut::ListAllocated,
        Cut::ListLinked,
    ] {
        reach(&mut execution, &mut host(), cut);
        assert!(
            serde_json::from_value::<State>(serde_json::json!({"construction": execution}))
                .is_err()
        );
    }
}

#[test]
fn simulator_guard_covers_accepted_unlinked_list_and_complete_prior_graph() {
    let mut execution = construction();
    reach(&mut execution, &mut host(), Cut::ListAllocated);
    for (token, bytes) in [
        (0x79000000, 428),
        (0x79000200, 28),
        (0x790001ff, 2),
        (execution.source().owner().token + 100, 1),
        (execution.source().world().owner().token + 130, 1),
        (0x6de858, 8),
        (0x6e5528, 4),
        (u64::MAX, 28),
    ] {
        assert!(execution.guard_span(token, bytes).is_err());
    }
    assert!(execution.guard_span(0x7900021c, 28).is_ok());
    assert!(execution.guard_span(0x790001ac, 28).is_ok());
}
