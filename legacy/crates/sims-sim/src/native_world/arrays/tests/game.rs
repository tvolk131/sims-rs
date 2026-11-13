use super::*;
use crate::native_game;
use crate::native_world::resources::{path, strings};

const GAME: u64 = 0x800000;
const GAME_NAME: u64 = 0x900000;

fn manager_buckets() -> [Owner; 2] {
    [0x4000, 0x5000].map(|token| Owner {
        token,
        data: vec![0; 772],
    })
}

struct NameHost;

impl native_game::Host for NameHost {
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        assert_eq!(bytes, 8);
        Ok(Allocated::Bytes {
            token: GAME_NAME,
            data: vec![0x57; 8],
        })
    }
}

fn application() -> (Owner, Owner) {
    let mut owner = Owner {
        token: 0x1000,
        data: vec![0x73; 324],
    };
    for (offset, value) in [
        (0x68, 0x669804_u32),
        (0x6c, 0),
        (0x70, 0),
        (0x74, 0),
        (180, 0x3000),
        (184, 0x3000),
        (188, 0x3008),
    ] {
        owner.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut backing = Owner {
        token: 0x3000,
        data: vec![0x61; 8],
    };
    backing.data[0] = 0;
    (owner, backing)
}

fn arrays() -> (Construction, Services) {
    let (application, backing) = application();
    let mut registry = sims_vm::commander::Registry::default();
    registry.construct(0x1068).unwrap();
    registry.bind(0x1068, 0x669804).unwrap();
    let mut game = native_game::Construction::new(native_game::Incoming {
        owners: vec![
            application,
            backing,
            Owner {
                token: GAME,
                data: vec![0xa5; 112],
            },
        ]
        .into_iter()
        .chain(manager_buckets())
        .collect(),
        registry,
        game: GAME,
        provider_word: 0,
    })
    .unwrap();
    while game.step(&mut NameHost).unwrap().is_some() {}
    let mut entry = game.into_state().unwrap().begin_world().unwrap();
    let mut host = Services::new();
    host.live.extend([
        (0x3000, 8),
        (0x4000, 772),
        (0x5000, 772),
        (GAME, 112),
        (GAME_NAME, 8),
    ]);
    for _ in 0..100 {
        if entry.step(&mut host).unwrap() == Some(native_world::Cut::ArrayConstructionEntry) {
            break;
        }
    }
    (
        entry
            .into_arrays(Context {
                roof_owners: 0,
                random: 0x12345678,
            })
            .unwrap(),
        host,
    )
}

fn resources() -> (native_world::resources::Construction, Services) {
    let (mut arrays, mut host) = arrays();
    finish(&mut arrays, &mut host, false);
    let mut fixed = native_world::fixed::Construction::new(
        arrays,
        native_world::fixed::Context {
            requested: 64,
            current_world: 0,
        },
    )
    .unwrap();
    for _ in 0..5 {
        fixed.step().unwrap();
    }
    let mut lighting = native_world::lighting::Construction::new(
        fixed,
        native_world::lighting::Tables::new(vec![0x53; 4320], 3).unwrap(),
        native_world::lighting::Context {
            application_mode: 2,
        },
    )
    .unwrap();
    finish_lighting(&mut lighting);
    let mut resources = native_world::resources::Construction::new(
        lighting,
        Owner {
            token: 0x6f01a0,
            data: vec![0; 100],
        },
        native_world::resources::Context {
            level_bits: 12.0_f32.to_bits(),
        },
    )
    .unwrap();
    for _ in 0..80 {
        if resources.step(&mut host).unwrap() == Some(native_world::resources::Cut::VectorsReserved)
        {
            return (resources, host);
        }
    }
    panic!("Game-containing resource construction did not finish");
}

fn path_incoming() -> path::Incoming {
    let (owner, backing) = application();
    path::Incoming {
        application: path::Application::new(owner, backing, vec![]).unwrap(),
        buffer: Owner {
            token: 0x60000000,
            data: vec![0x72; 272],
        },
        output_descriptor: Owner {
            token: 0x60000120,
            data: vec![0x31; 12],
        },
        scenario_descriptor: Owner {
            token: 0x60000140,
            data: vec![0x42; 12],
        },
        retained: manager_buckets().into_iter().collect(),
    }
}

pub(super) fn completed_path() -> path::State {
    let (resources, mut host) = resources();
    let mut execution =
        path::Construction::new(resources, path_incoming(), path::Inputs::lightmap()).unwrap();
    for _ in 0..100 {
        if execution.step(&mut host).unwrap() == Some(path::Cut::ComposedPath) {
            return execution.into_state().unwrap();
        }
    }
    panic!("Game-containing path construction did not finish");
}

#[test]
fn restored_path_rejects_missing_or_foreign_published_array_owners() {
    let original = serde_json::to_value(completed_path()).unwrap();
    for mutation in 0..5 {
        let mut changed = original.clone();
        let arrays = &mut changed["resources"]["lighting"]["fixed"]["arrays"];
        match mutation {
            0 => arrays["vectors"][0] = serde_json::Value::Null,
            1 => arrays["vectors"][0]["owner"]["token"] = (0x70000000_u64).into(),
            2 => {
                arrays["roots"].as_object_mut().unwrap().remove("20");
            }
            3 => {
                let identity = arrays["roots"]["20"]["token"].as_u64().unwrap();
                arrays["matrices"]["nodes"]
                    .as_object_mut()
                    .unwrap()
                    .remove(&identity.to_string());
            }
            _ => arrays["prefix"]["owner"]["data"][0x68] = 0xff.into(),
        }
        let restored: path::State = serde_json::from_value(changed).unwrap();
        assert!(
            restored.validate().is_err(),
            "array ownership mutation {mutation}"
        );
    }
}

#[test]
fn restored_path_rejects_truncated_application_without_panicking() {
    let original = serde_json::to_value(completed_path()).unwrap();
    for extent in [0, 0x68, 0x77, 179] {
        let mut changed = original.clone();
        changed["application"]["prefix"]["data"]
            .as_array_mut()
            .unwrap()
            .truncate(extent);
        let restored: path::State = serde_json::from_value(changed).unwrap();
        assert!(restored.validate().is_err(), "application extent {extent}");
    }
}

#[test]
fn retained_array_validation_allows_legitimate_payload_changes() {
    let (mut execution, mut host) = arrays();
    finish(&mut execution, &mut host, false);
    let mut state = execution.into_state().unwrap();
    state.vectors[0]
        .as_mut()
        .unwrap()
        .backing
        .as_mut()
        .unwrap()
        .data[0] ^= 1;
    let identity = state.identity(0x14).unwrap();
    let Allocation::Known(backing) = &mut state
        .matrices
        .nodes
        .get_mut(&identity)
        .unwrap()
        .matrix
        .allocation
    else {
        panic!("completed matrix backing absent");
    };
    backing.data[0] ^= 1;
    state.validate_retained().unwrap();
    state.validate().unwrap();
}

#[test]
fn restored_array_publication_requires_the_current_constructed_owner() {
    for sequence in [0, 12] {
        let (mut execution, mut host) = arrays();
        while execution.sequence != sequence || execution.phase != Phase::AllocateRoot {
            execution.step(&mut host).unwrap();
        }
        let mut changed = serde_json::to_value(&execution).unwrap();
        changed["phase"] = "PublishRoot".into();
        let mut restored: Construction = serde_json::from_value(changed).unwrap();
        let before = restored.clone();
        let events = host.events.clone();
        assert!(restored.step(&mut host).is_err());
        assert_eq!(restored, before);
        assert_eq!(host.events, events);
    }
}

#[test]
fn restored_nested_matrix_cursors_reject_foreign_free_and_allocation_replay() {
    for mutation in 0..5 {
        let (mut execution, mut host) = arrays();
        let mut exercised = false;
        for _ in 0..5000 {
            let mut changed = serde_json::to_value(&execution).unwrap();
            let selected = match mutation {
                0 => {
                    changed["matrix"]["phase"] == "Resize"
                        && changed["matrix"]["resize"]["phase"] == "Inspect"
                }
                1 => changed["matrix"]["phase"] == "AcceptName",
                2 => changed["matrix"]["resize"]["phase"] == "Accept",
                3 => changed["naming"]["assignment"]["phase"] == "Accept",
                _ => changed["naming"]["assignment"]["phase"] == "Release",
            };
            if selected {
                match mutation {
                    0 => changed["matrix"]["resize"]["phase"] = serde_json::json!({"Free": GAME}),
                    1 => changed["matrix"]["phase"] = "ReserveName".into(),
                    2 => changed["matrix"]["resize"]["phase"] = "Allocate".into(),
                    _ => changed["naming"]["assignment"]["phase"] = "Allocate".into(),
                }
                let mut restored: Construction = serde_json::from_value(changed).unwrap();
                let before = restored.clone();
                let events = host.events.clone();
                assert!(
                    restored.step(&mut host).is_err(),
                    "nested mutation {mutation}"
                );
                assert_eq!(restored, before, "nested mutation {mutation}");
                assert_eq!(host.events, events, "nested mutation {mutation}");
                assert!(host.live.contains_key(&GAME));
                assert!(host.live.contains_key(&GAME_NAME));
                exercised = true;
                break;
            }
            execution.step(&mut host).unwrap();
        }
        assert!(exercised, "nested mutation {mutation} was not reached");
    }
}

impl path::Host for Services {
    fn allocate_string(
        &mut self,
        _: path::Scope,
        request: strings::Request,
    ) -> Result<Allocated, Stop> {
        Ok(self.allocation(request.bytes()))
    }

    fn free_string(&mut self, _: path::Scope, backing: &Owner) -> Result<(), Stop> {
        assert_eq!(
            self.live.remove(&backing.token),
            Some(backing.data.len() as u32)
        );
        self.events.push(Event::Free(backing.token));
        Ok(())
    }

    fn open_scenario(&mut self, filename: &[u8], mode: &[u8]) -> Result<path::Opened, Stop> {
        assert_eq!(filename, b"./GameData/Scenario.tds");
        assert_eq!(mode, b"rb");
        self.events.push(Event::ScenarioOpen(0x4242));
        Ok(path::Opened::File(0x4242))
    }

    fn close_scenario(&mut self, capability: u64) -> Result<(), Stop> {
        assert_eq!(self.events.last(), Some(&Event::ScenarioOpen(capability)));
        self.events.push(Event::ScenarioClose(capability));
        Ok(())
    }
}

#[test]
fn game_source_survives_array_construction_and_application_path_transfer() {
    let (resources, mut host) = resources();
    let game = resources
        .lighting()
        .fixed
        .arrays
        .prefix
        .game()
        .unwrap()
        .clone();
    let mut execution =
        path::Construction::new(resources, path_incoming(), path::Inputs::lightmap()).unwrap();
    let mut complete = false;
    for _ in 0..100 {
        let cut = execution.step(&mut host).unwrap();
        let restored = serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        assert_eq!(execution, restored);
        execution = restored;
        if cut == Some(path::Cut::ComposedPath) {
            complete = true;
            break;
        }
    }
    assert!(complete);
    let state = execution.into_state().unwrap();
    state.validate().unwrap();
    assert_eq!(
        state.resources.lighting.fixed.arrays.prefix.game(),
        Some(&game)
    );
    assert_eq!(state.application.path().unwrap().bytes().unwrap(), b"./");
    assert_eq!(game.construction().incoming().owners[1].data[0], 0);
    assert_eq!(
        state.application.path().unwrap().backing.data[3..],
        [0x61; 5]
    );
    assert!(host.live.contains_key(&GAME));
    assert!(host.live.contains_key(&GAME_NAME));
    assert_eq!(
        host.events
            .iter()
            .filter(|event| matches!(event, Event::Prefix(_)))
            .count(),
        7
    );
}

#[test]
fn game_path_transfer_rejects_changed_source_bytes_and_foreign_epoch() {
    let (resources, _) = resources();
    for mutation in 0..3 {
        let mut incoming = path_incoming();
        let (mut owner, mut backing) = application();
        match mutation {
            0 => owner.data[0] ^= 1,
            1 => backing.data[7] ^= 1,
            _ => {
                owner.token += 0x10000;
                backing.token += 0x10000;
                for (offset, value) in [
                    (180, backing.token),
                    (184, backing.token),
                    (188, backing.token + 8),
                ] {
                    owner.data[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
                }
            }
        }
        incoming.application = path::Application::new(owner, backing, vec![]).unwrap();
        assert!(
            path::Construction::new(resources.clone(), incoming, path::Inputs::lightmap()).is_err()
        );
    }
}

#[test]
fn game_resource_transfer_rejects_changed_or_repeated_source_buckets() {
    let (resources, _) = resources();
    for mutation in 0..3 {
        let mut incoming = path_incoming();
        match mutation {
            0 => incoming.retained[0].data[771] ^= 1,
            1 => incoming.retained.push(incoming.retained[0].clone()),
            _ => {
                incoming.retained.push(Owner {
                    token: GAME,
                    data: resources
                        .lighting()
                        .fixed
                        .arrays
                        .prefix
                        .game()
                        .unwrap()
                        .construction()
                        .owner()
                        .data
                        .clone(),
                });
            }
        }
        assert!(
            path::Construction::new(resources.clone(), incoming, path::Inputs::lightmap()).is_err()
        );
    }
}

#[test]
fn nested_array_allocations_cannot_alias_full_game_or_source_capacity() {
    for target in 0..6 {
        let (mut execution, mut host) = arrays();
        let mut injected = false;
        let mut stopped = false;
        for _ in 0..5000 {
            let state = serde_json::to_value(&execution).unwrap();
            let selected = match target {
                0 => state["phase"] == "AllocateRoot",
                1 => state["phase"].get("AllocateTemporary").is_some(),
                2 => state["matrix"]["phase"] == "ReserveName",
                3 => state["matrix"]["resize"]["phase"] == "Allocate",
                4 => {
                    state["matrix"]["naming"]["assignment"]["phase"] == "Allocate"
                        || state["naming"]["assignment"]["phase"] == "Allocate"
                }
                _ => state["vector"]["phase"] == "Allocate",
            };
            if !injected && selected {
                match target {
                    0 => {
                        host.root_reply = Some(Allocated::Bytes {
                            token: GAME + 96,
                            data: vec![0xa5; 32],
                        })
                    }
                    1 | 2 | 4 => host.name_alias = Some(GAME + 96),
                    3 => host.backing_alias = Some(GAME - 16384 - 128),
                    _ => {
                        host.vector_reply = Some(Allocated::Bytes {
                            token: GAME + 96,
                            data: vec![0xa5; 16384],
                        })
                    }
                }
                injected = true;
            }
            match execution.step(&mut host) {
                Ok(_) => {}
                Err(_) => {
                    assert!(injected);
                    let events = host.events.clone();
                    assert!(execution.step(&mut host).is_err());
                    assert_eq!(host.events, events);
                    stopped = true;
                    break;
                }
            }
        }
        assert!(injected && stopped, "nested allocation target{target}");
        assert!(host.live.contains_key(&GAME));
        assert!(host.live.contains_key(&GAME_NAME));
    }
}

#[test]
fn array_root_replies_reserve_application_tail_and_complete_game_name_capacity() {
    for token in [0x1000 + 300, GAME_NAME + 7] {
        let (mut execution, mut host) = arrays();
        while execution.phase != Phase::AllocateRoot {
            execution.step(&mut host).unwrap();
        }
        host.root_reply = Some(Allocated::Bytes {
            token,
            data: vec![0xa5; 32],
        });
        assert!(execution.step(&mut host).is_err());
        let events = host.events.clone();
        assert!(execution.step(&mut host).is_err());
        assert_eq!(host.events, events);
        assert!(host.live.contains_key(&GAME));
        assert!(host.live.contains_key(&GAME_NAME));
    }
}
