use sims_content::{Index, vm::load_program};
use sims_vm::{Definition, Object, Snapshot, Start, Status, Vm, World, action};
use std::{collections::BTreeMap, fs, path::PathBuf};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn iff(chunks: Vec<([u8; 4], u16, Vec<u8>)>) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    for (tag, id, data) in chunks {
        bytes.extend(tag);
        bytes.extend((76 + data.len() as u32).to_be_bytes());
        bytes.extend(id.to_be_bytes());
        bytes.extend([0; 66]);
        bytes.extend(data);
    }
    bytes
}
fn bhav(opcode: u16, table: u16, global: bool) -> Vec<u8> {
    let mut bytes = 0x8002u16.to_le_bytes().to_vec();
    bytes.extend(1u16.to_le_bytes());
    bytes.extend([0; 8]);
    bytes.extend(opcode.to_le_bytes());
    bytes.extend([254, 255]);
    bytes.extend(table.to_le_bytes());
    bytes.extend([u8::from(global), 0, 1, 0, 0, 0]);
    bytes
}
fn string(value: &str) -> Vec<u8> {
    let mut bytes = (-1i16).to_le_bytes().to_vec();
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(value.bytes());
    bytes.push(0);
    bytes
}

#[test]
fn shared_code_loads_strings_in_each_private_frame_context() {
    let root = Fixture(std::env::temp_dir().join(format!(
        "sims-frame-strings-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    )));
    fs::create_dir(&root.0).unwrap();
    for label in ["First", "Second"] {
        fs::write(
            root.0.join(format!("{label}.iff")),
            iff(vec![
                (*b"GLOB", 128, b"\x06shared".to_vec()),
                (*b"BHAV", 4096, bhav(8192, 0, false)),
                (*b"STR#", 302, string(label)),
                (*b"STR#", 400, string("Global code private text")),
                (*b"STR#", 401, string("Private named tree")),
                // This unrelated obsolete chunk must not be parsed eagerly.
                (*b"STR#", 999, vec![0xff]),
            ]),
        )
        .unwrap();
    }
    fs::write(
        root.0.join("shared.iff"),
        iff(vec![
            (*b"BHAV", 8192, bhav(50, 302, false)),
            (*b"BHAV", 8193, bhav(28, 401, false)),
            (*b"BHAV", 8194, bhav(28, 402, true)),
            (*b"STR#", 302, string("Wrong shared label")),
        ]),
    )
    .unwrap();
    fs::write(
        root.0.join("global.iff"),
        iff(vec![
            (*b"BHAV", 256, bhav(50, 400, false)),
            (*b"STR#", 302, string("Wrong global label")),
            (*b"STR#", 402, string("Explicit global named tree")),
        ]),
    )
    .unwrap();
    let index = Index::scan(&root.0).unwrap();
    for label in ["First", "Second"] {
        let owner = format!("{label}.iff");
        let mut p = load_program(&index, &owner, "global.iff").unwrap();
        let module = &p.modules[&owner];
        assert_eq!(module.strings[&302], [label]);
        assert_eq!(module.strings[&400], ["Global code private text"]);
        assert_eq!(module.strings[&401], ["Private named tree"]);
        assert!(!module.strings.contains_key(&402));
        assert!(!module.strings.contains_key(&999));
        p.modules
            .get_mut(&owner)
            .unwrap()
            .object_interactions
            .insert(
                1,
                vec![sims_vm::program::MenuEntry {
                    id: 0,
                    action: 4096,
                    ..Default::default()
                }],
            );
        let mut world = World::default();
        world.objects.insert(
            1,
            Object {
                person: Some(BTreeMap::new()),
                definition: Some(Definition {
                    source: owner.clone(),
                    object: 1,
                    guid: 1,
                    slots: 0,
                }),
                ..Default::default()
            },
        );
        world.actions = Some(action::WorldState {
            actors: BTreeMap::from([(
                1,
                action::Actor {
                    active: Some(action::Action {
                        routine: 4096,
                        target: 1,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        });
        for check in [false, true] {
            let mut vm = Vm::new(world.clone(), 1);
            let id = vm
                .spawn(
                    &p,
                    Start {
                        owner: &owner,
                        bhav: 4096,
                        caller: 1,
                        callee: 1,
                        stack_object: 0,
                        args: vec![],
                        check,
                    },
                )
                .unwrap();
            vm.run_slice(&p, id, 16).unwrap();
            assert_eq!(vm.threads[&id].status, Status::Returned(true));
            let action = vm.world.actions.as_ref().unwrap().actors[&1]
                .active
                .as_ref()
                .unwrap();
            assert_eq!(
                action.label.as_deref(),
                if check { None } else { Some(label) }
            );
            assert_eq!(Snapshot::capture(&p, &vm).restore(&p).unwrap(), vm);
        }
    }
}
