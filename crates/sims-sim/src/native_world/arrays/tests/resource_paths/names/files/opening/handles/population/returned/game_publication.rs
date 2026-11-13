use super::*;
use crate::native_game::{
    WorldReturned,
    publication::{Construction, Cut, State},
};
use crate::native_world::arrays::tests::{game, resource_paths::names as name_fixtures};
use file_fixtures::opening as open_fixtures;
use name_fixtures::files as file_fixtures;

mod simulator;

struct GameManagerHost;

impl manager::Host for GameManagerHost {
    fn allocate_buckets(&mut self, map: u8, bytes: u32) -> Result<Allocated, Stop> {
        assert_eq!(bytes, 772);
        Ok(Allocated::Bytes {
            token: [0x4000, 0x5000][usize::from(map)],
            data: vec![0xa5; bytes as usize],
        })
    }

    fn initialize_section(&mut self, address: u64, bytes: u32) -> Result<u64, Stop> {
        assert_eq!((address, bytes), (0x6fe578, 24));
        Ok(1)
    }
}

fn world_returned(corner_tail: u8) -> WorldReturned {
    let mut path = game::completed_path();
    let mut manager = manager::Construction::new(
        Owner {
            token: 0x6fe548,
            data: vec![0x61; 72],
        },
        [0, 0],
        vec![],
    )
    .unwrap();
    while !manager.step(&mut GameManagerHost).unwrap() {}
    let manager = manager.into_state().unwrap();
    for bucket in &manager.buckets {
        assert!(path.retained.contains(bucket));
    }
    path.retained.push(manager.owner);
    let mut incoming = name_fixtures::incoming();
    incoming.directory_descriptor.token = incoming.basename_descriptor.token + 12;
    let mut names = names::Construction::new(path, incoming).unwrap();
    let mut name_host = name_fixtures::NameHost::new();
    for _ in 0..200 {
        if names.step(&mut name_host).unwrap() == Some(names::Cut::FileAllocationEntry) {
            break;
        }
    }
    let mut file = file::Construction::new(names.into_state().unwrap()).unwrap();
    file_fixtures::finish_file(&mut file, &mut file_fixtures::FileHost::new());
    let file = file.into_state().unwrap();
    let incoming = open_fixtures::open_incoming(&file);
    let mut opened = opening::Execution::new(file, incoming).unwrap();
    let mut file_host = open_fixtures::OpenHost::new();
    open_fixtures::reach(&mut opened, &mut file_host, opening::Cut::Returned);
    let opened = opened.into_state().unwrap();
    let canonical = opened.context().name.backing.clone();
    let mut handle_host = HandleHost {
        file: file_host,
        live: BTreeMap::from([(canonical.token, canonical.data.len())]),
        canonical,
        next: 0x75000000,
        calls: vec![],
        handle_reply: None,
        string_reply: None,
        string_allocations: 0,
        size_reply: handle::FileSize::Bytes(2271104),
        deny: false,
        deny_free: false,
    };
    let mut handle = handle::Construction::new(opened).unwrap();
    reach_handle(&mut handle, &mut handle_host, handle::Cut::Published);
    let mut population = population::Construction::new(handle.into_state().unwrap()).unwrap();
    let mut population_host = PopulationHost {
        next: 0x76000000,
        calls: vec![],
        reply: None,
        deny_allocate: false,
        deny_free: false,
    };
    super::super::reach(&mut population, &mut population_host, 99);
    let mut incoming = super::incoming(0x78000000);
    incoming.corners[0].data[3] = corner_tail;
    let mut returned =
        returned::Construction::new(population.into_state().unwrap(), incoming).unwrap();
    super::reach_return(&mut returned, returned::Cut::Returned);
    WorldReturned::new(returned.into_state().unwrap()).unwrap()
}

fn finished(source: WorldReturned) -> Construction {
    let mut execution = Construction::new(source).unwrap();
    assert_eq!(execution.step().unwrap(), Cut::WorldPublished);
    assert_eq!(execution.step().unwrap(), Cut::ReadyForSimulator);
    execution
}

#[test]
fn publication_preserves_complete_source_and_exact_two_native_stores() {
    let source = world_returned(0xc7);
    let mut expected = source.game_constructor().construction().owner().clone();
    let original = expected.clone();
    let world = (source.world().owner().token as u32).to_le_bytes();
    let mut execution = Construction::new(source.clone()).unwrap();
    assert_eq!(execution.owner(), &expected);
    assert_eq!(execution.cut(), Cut::Entry);
    for (index, (cut, offset, instruction)) in [
        (Cut::WorldPublished, 0x18, 0x415ca4),
        (Cut::ReadyForSimulator, 0x1c, 0x415ca7),
    ]
    .into_iter()
    .enumerate()
    {
        let before = expected.data[offset..offset + 4].to_vec();
        expected.data[offset..offset + 4].copy_from_slice(&world);
        assert_eq!(execution.step().unwrap(), cut);
        assert_eq!(execution.owner(), &expected);
        assert_eq!(execution.writes().len(), index + 1);
        assert_eq!(execution.writes()[index].instruction, instruction);
        assert_eq!(execution.writes()[index].offset, offset);
        assert_eq!(execution.writes()[index].before, before);
        assert_eq!(execution.writes()[index].after, world);
        assert_eq!(execution.source(), &source);
        assert_eq!(execution.registry(), source.world().registry());
    }
    assert_eq!(
        execution.source().game_constructor().construction().owner(),
        &original
    );
    let completed = execution.clone();
    assert_eq!(execution.step().unwrap(), Cut::ReadyForSimulator);
    assert_eq!(execution, completed);
}

#[test]
fn every_publication_boundary_roundtrips_and_continues_to_identical_state() {
    let source = world_returned(0x83);
    let expected = finished(source.clone()).into_state().unwrap();
    let mut execution = Construction::new(source).unwrap();
    for cut in [Cut::Entry, Cut::WorldPublished, Cut::ReadyForSimulator] {
        assert_eq!(execution.cut(), cut);
        let mut restored: Construction =
            serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        while restored.cut() != Cut::ReadyForSimulator {
            restored.step().unwrap();
        }
        assert_eq!(restored.into_state().unwrap(), expected);
        execution.step().unwrap();
    }
    let restored: State = serde_json::from_slice(&serde_json::to_vec(&expected).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, expected);
}

#[test]
fn publication_restore_rejects_changed_owner_cursor_store_or_preimage() {
    let original = serde_json::to_value(finished(world_returned(0x19))).unwrap();
    for mutation in 0..10 {
        let mut changed = original.clone();
        match mutation {
            0 => changed["owner"]["data"] = serde_json::json!([]),
            1 => changed["owner"]["token"] = 0.into(),
            2 => {
                let before = changed["owner"]["data"][0x6f].as_u64().unwrap();
                changed["owner"]["data"][0x6f] = (before ^ 1).into();
            }
            3 => changed["cut"] = "Entry".into(),
            4 => changed["writes"] = serde_json::json!([]),
            5 => changed["writes"][0]["instruction"] = 0x415ca7.into(),
            6 => changed["writes"][0]["offset"] = 0x1c.into(),
            7 => changed["writes"][0]["before"][0] = 0xff.into(),
            8 => changed["writes"][1]["after"][0] = 0xff.into(),
            _ => {
                let duplicate = changed["writes"][0].clone();
                changed["writes"].as_array_mut().unwrap().push(duplicate);
            }
        }
        assert!(
            serde_json::from_value::<Construction>(changed).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn publication_rejects_incomplete_completed_state_and_missing_graph() {
    let source = world_returned(0x27);
    let mut execution = Construction::new(source).unwrap();
    for _ in 0..2 {
        assert!(execution.clone().into_state().is_err());
        let encoded = serde_json::json!({"publication": execution});
        assert!(serde_json::from_value::<State>(encoded).is_err());
        execution.step().unwrap();
    }
    let original = serde_json::to_value(execution).unwrap();
    for field in ["source", "owner", "writes", "cut"] {
        let mut changed = original.clone();
        changed.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<Construction>(changed).is_err());
    }
    let mut changed = original;
    changed["invented_provenance"] = true.into();
    assert!(serde_json::from_value::<Construction>(changed).is_err());
}

#[test]
fn publication_rejects_foreign_current_world_and_registry() {
    let original = serde_json::to_value(finished(world_returned(0x39))).unwrap();
    for mutation in 0..3 {
        let mut changed = original.clone();
        let returned = &mut changed["source"]["world"]["execution"];
        assert!(returned.is_object());
        match mutation {
            0 => returned["owner"]["token"] = 0x800000.into(),
            1 => returned["registry"]["generation"] = 0.into(),
            _ => returned["registry"]["nodes"] = serde_json::json!({}),
        }
        assert!(
            serde_json::from_value::<Construction>(changed).is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn publication_guards_every_retained_allocation_and_native_global() {
    let execution = finished(world_returned(0x61));
    let mut spans = execution.world().population().retained_spans().unwrap();
    spans.extend(
        execution
            .world()
            .corners()
            .iter()
            .map(|owner| (owner.token, owner.data.len())),
    );
    spans.extend([(0x6de858, 8), (0x6e5528, 4)]);
    for (token, bytes) in spans {
        assert!(execution.guard_span(token, 1).is_err());
        assert!(execution.guard_span(token + bytes as u64 - 1, 1).is_err());
        assert!(execution.guard_span(token - 1, 2).is_err());
    }
    for (token, bytes) in [
        (0, 1),
        (0x90000000, 0),
        (u64::MAX, 1),
        (u64::from(u32::MAX), 2),
    ] {
        assert!(execution.guard_span(token, bytes).is_err());
    }
    let state = execution.into_state().unwrap();
    state.guard_span(0x90000000, 64).unwrap();
}

#[test]
fn publication_preserves_legitimate_untouched_corner_payload() {
    for tail in [0, 0xa5, 0xff] {
        let source = world_returned(tail);
        let state = finished(source.clone()).into_state().unwrap();
        assert_eq!(state.world(), source.world());
        assert_eq!(state.world().corners()[0].data[3], tail);
    }
}
