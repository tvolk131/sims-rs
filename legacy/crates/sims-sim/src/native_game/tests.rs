use super::*;
use sims_vm::commander::Registry;

fn incoming(fill: u8) -> Incoming {
    let mut registry = Registry::default();
    registry.construct(0x100010).unwrap();
    registry.bind(0x100010, 0x669804).unwrap();
    let mut application = Owner {
        token: 0x100000,
        data: vec![0x73; 64],
    };
    application.data[16..32].copy_from_slice(&header(&registry, 0x100010).unwrap());
    Incoming {
        owners: vec![
            application,
            Owner {
                token: 0x200000,
                data: vec![fill; 112],
            },
        ],
        registry,
        game: 0x200000,
        provider_word: 0x12345678,
    }
}

struct NameHost {
    calls: usize,
    reply: Allocated,
}

impl Host for NameHost {
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        assert_eq!(bytes, 8);
        self.calls += 1;
        Ok(self.reply.clone())
    }
}

fn construction(fill: u8) -> (Construction, NameHost) {
    (
        Construction::new(incoming(fill)).unwrap(),
        NameHost {
            calls: 0,
            reply: Allocated::Bytes {
                token: 0x300000,
                data: vec![0xb7; 8],
            },
        },
    )
}

fn finish(execution: &mut Construction, host: &mut NameHost) {
    while execution.step(host).unwrap().is_some() {}
}

#[test]
fn game_constructor_owns_entire_name_capacity_and_original_ordered_stores() {
    for fill in [0, 0xa5, 0xff] {
        let (mut execution, mut host) = construction(fill);
        finish(&mut execution, &mut host);
        execution.validate().unwrap();
        assert_eq!(host.calls, 1);
        assert_eq!(execution.cut(), Cut::Returned);
        let game = execution.owner();
        assert_eq!(game.data[0..4], 0x666714_u32.to_le_bytes());
        assert_eq!(game.data[0x34..0x38], 0x12345678_u32.to_le_bytes());
        assert_eq!(game.data[0x28..0x2c], 0x300000_u32.to_le_bytes());
        assert_eq!(game.data[0x30..0x34], 0x300008_u32.to_le_bytes());
        assert_eq!(
            execution.owners()[2].data,
            [0, 0xb7, 0xb7, 0xb7, 0xb7, 0xb7, 0xb7, 0xb7]
        );
        assert_eq!(execution.writes().len(), 33);
        assert_eq!(execution.writes()[10].instruction, 0x40163d);
        assert_eq!(execution.writes()[32].instruction, 0x415bed);
        assert_eq!(execution.registry().head, Some(0x200000));
        assert_eq!(execution.registry().generation, 2);
        assert_eq!(execution.owners()[0], execution.incoming().owners[0]);
        assert_eq!(
            execution.into_state().unwrap().construction().cut(),
            Cut::Returned
        );
    }
}

#[test]
fn every_game_host_reply_cut_roundtrips_without_reallocating() {
    let (mut execution, mut host) = construction(0xa5);
    for expected in [
        Cut::Entry,
        Cut::Registered,
        Cut::StringInitialized,
        Cut::NameAllocated,
        Cut::StringConstructed,
        Cut::Returned,
    ] {
        assert_eq!(execution.cut(), expected);
        execution = serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        execution.validate().unwrap();
        execution.step(&mut host).unwrap();
    }
    assert_eq!(host.calls, 1);
    assert!(execution.step(&mut host).unwrap().is_none());
    let state = execution.into_state().unwrap();
    let restored: State = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(state, restored);
}

#[test]
fn game_rejects_changed_bytes_cursor_reply_and_constructor_provenance() {
    let (mut execution, mut host) = construction(0xa5);
    finish(&mut execution, &mut host);
    let original = serde_json::to_value(&execution).unwrap();
    for path in ["owners", "writes", "cut", "name_reply", "incoming"] {
        let mut changed = original.clone();
        match path {
            "owners" => changed["owners"][0]["data"][0] = 0.into(),
            "writes" => changed["writes"][0]["before"][0] = 0.into(),
            "cut" => changed["cut"] = "NameAllocated".into(),
            "name_reply" => changed["name_reply"]["Bytes"]["data"][7] = 0.into(),
            _ => changed["incoming"]["provider_word"] = 0.into(),
        }
        assert!(
            serde_json::from_value::<Construction>(changed).is_err(),
            "{path}"
        );
    }
    let (partial, _) = construction(0xa5);
    let mut changed = serde_json::to_value(execution.into_state().unwrap()).unwrap();
    changed["construction"] = serde_json::to_value(partial).unwrap();
    assert!(serde_json::from_value::<State>(changed).is_err());
}

#[test]
fn game_rejects_unowned_registry_prefix_and_overlapping_source_allocations() {
    let mut source = incoming(0xa5);
    source.owners[0].data[16] ^= 1;
    assert!(Construction::new(source).is_err());
    let mut source = incoming(0xa5);
    source.owners[1].token = source.owners[0].token + 32;
    source.game = source.owners[1].token;
    assert!(Construction::new(source).is_err());
    let mut source = incoming(0xa5);
    source.owners[1].data.pop();
    assert!(Construction::new(source).is_err());
    let mut source = incoming(0xa5);
    source.owners[1].token = u64::MAX;
    source.game = u64::MAX;
    assert!(Construction::new(source).is_err());
}

#[test]
fn invalid_game_allocation_reply_is_retained_instead_of_requested_again() {
    for reply in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0x200060,
            data: vec![0; 8],
        },
        Allocated::Bytes {
            token: 0x300000,
            data: vec![0; 7],
        },
    ] {
        let (mut execution, mut host) = construction(0xa5);
        host.reply = reply;
        execution.step(&mut host).unwrap();
        execution.step(&mut host).unwrap();
        assert!(execution.step(&mut host).is_err());
        assert!(execution.step(&mut host).is_err());
        assert_eq!(host.calls, 1);
    }
}

struct WorldHost {
    next: u64,
    calls: usize,
    alias_call: Option<usize>,
}

impl native_world::Host for WorldHost {
    fn allocate(&mut self, request: native_world::Request) -> Result<Allocated, Stop> {
        self.calls += 1;
        let token = if self.alias_call == Some(self.calls) {
            0x200060
        } else {
            self.next
        };
        self.next += 0x1000;
        Ok(Allocated::Bytes {
            token,
            data: vec![0xa5; request.bytes()? as usize],
        })
    }
}

fn world_entry() -> WorldEntry {
    let (mut execution, mut host) = construction(0xa5);
    finish(&mut execution, &mut host);
    execution.into_state().unwrap().begin_world().unwrap()
}

#[test]
fn world_prefix_retains_game_owner_and_single_authoritative_current_registry() {
    let mut entry = world_entry();
    let game = entry.game().clone();
    let mut host = WorldHost {
        next: 0x400000,
        calls: 0,
        alias_call: None,
    };
    for _ in 0..80 {
        entry = serde_json::from_slice(&serde_json::to_vec(&entry).unwrap()).unwrap();
        entry.validate().unwrap();
        if entry.step(&mut host).unwrap() == Some(native_world::Cut::ArrayConstructionEntry) {
            break;
        }
    }
    assert_eq!(
        entry.prefix().cut(),
        Some(native_world::Cut::ArrayConstructionEntry)
    );
    assert_eq!(entry.game(), &game);
    assert_eq!(host.calls, 7);
    assert_eq!(entry.prefix().registry().head, Some(0x400000));
    assert_eq!(
        entry.prefix().registry().nodes[&0x400000].next,
        Some(0x200000)
    );
    assert_eq!(entry.prefix().registry().nodes[&0x200000].handler, 0x666714);
    assert_eq!(entry.prefix().registry().generation, 3);
    let mut changed = serde_json::to_value(&entry).unwrap();
    changed["prefix"]["registry"]["nodes"]["2097152"]["handler"] = 0.into();
    assert!(serde_json::from_value::<WorldEntry>(changed).is_err());
}

#[test]
fn world_and_nested_child_replies_cannot_alias_game_tail_or_repeat_allocator() {
    for alias_call in 1..=7 {
        let mut entry = world_entry();
        let mut host = WorldHost {
            next: 0x400000,
            calls: 0,
            alias_call: Some(alias_call),
        };
        let mut stopped = false;
        for _ in 0..80 {
            if entry.step(&mut host).is_err() {
                stopped = true;
                break;
            }
        }
        assert!(
            stopped,
            "allocation{alias_call} escaped full Game ownership"
        );
        assert_eq!(host.calls, alias_call);
        assert!(entry.step(&mut host).is_err());
        assert_eq!(host.calls, alias_call);
        assert!(
            serde_json::from_slice::<WorldEntry>(&serde_json::to_vec(&entry).unwrap()).is_err()
        );
    }
}

#[test]
fn retained_world_rejects_invalid_nested_cursor_and_pending_reply_reassignment() {
    let mut entry = world_entry();
    let mut host = WorldHost {
        next: 0x400000,
        calls: 0,
        alias_call: None,
    };
    while entry.prefix().companion().is_none() {
        entry.step(&mut host).unwrap();
    }
    let initial = serde_json::to_value(&entry).unwrap();
    for phase in [
        serde_json::json!({"InitializeVector": 255}),
        serde_json::json!({"PublishVector": 255}),
        serde_json::json!({"Allocate": {"Vector": 255}}),
        serde_json::json!({"Accept": "Child"}),
    ] {
        let mut changed = initial.clone();
        changed["prefix"]["companion"]["phase"] = phase.clone();
        assert!(
            serde_json::from_value::<WorldEntry>(changed).is_err(),
            "{phase}"
        );
    }
    while host.calls != 3 {
        entry.step(&mut host).unwrap();
    }
    let mut changed = serde_json::to_value(&entry).unwrap();
    changed["prefix"]["companion"]["phase"] = serde_json::json!({"Allocate": "Child"});
    assert!(serde_json::from_value::<WorldEntry>(changed).is_err());
    let mut restored: WorldEntry =
        serde_json::from_slice(&serde_json::to_vec(&entry).unwrap()).unwrap();
    restored.step(&mut host).unwrap();
    assert_eq!(host.calls, 3);
}

#[test]
fn retained_game_rejects_overflowing_source_extent_before_name_alias_checks() {
    let (mut execution, mut host) = construction(0xa5);
    finish(&mut execution, &mut host);
    let mut changed = serde_json::to_value(&execution).unwrap();
    changed["incoming"]["owners"][0]["token"] = u64::MAX.into();
    assert!(serde_json::from_value::<Construction>(changed).is_err());
}
