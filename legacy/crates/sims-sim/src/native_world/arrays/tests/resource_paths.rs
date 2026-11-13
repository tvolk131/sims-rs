use super::*;
use crate::native_world::resources::{path, strings};

mod names;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Allocate(path::Scope, strings::Request),
    Free(path::Scope, Owner),
    Open(Vec<u8>, Vec<u8>),
    Close(u64),
}

struct PathHost {
    next: u64,
    calls: Vec<Call>,
    live: BTreeMap<u64, usize>,
    reply: Option<Allocated>,
    opened: bool,
    open_reply: path::Opened,
    deny_allocation: bool,
    deny_free: bool,
    deny_close: bool,
}

impl PathHost {
    fn new() -> Self {
        Self {
            next: 0x70000000,
            calls: vec![],
            live: BTreeMap::new(),
            reply: None,
            opened: false,
            open_reply: path::Opened::File(0x40000000),
            deny_allocation: false,
            deny_free: false,
            deny_close: false,
        }
    }
}

impl path::Host for PathHost {
    fn allocate_string(
        &mut self,
        scope: path::Scope,
        request: strings::Request,
    ) -> Result<Allocated, Stop> {
        if self.deny_allocation {
            return Err(Stop::Unavailable("test allocator unavailable".into()));
        }
        self.calls.push(Call::Allocate(scope, request));
        let token = self.next;
        self.next += 32;
        let reply = self.reply.take().unwrap_or_else(|| Allocated::Bytes {
            token,
            data: vec![0xa5; request.bytes() as usize],
        });
        if let Allocated::Bytes { token, data } = &reply {
            self.live.insert(*token, data.len());
        }
        Ok(reply)
    }

    fn free_string(&mut self, scope: path::Scope, backing: &Owner) -> Result<(), Stop> {
        if self.deny_free {
            return Err(Stop::Unavailable("test free unavailable".into()));
        }
        assert_eq!(self.live.remove(&backing.token), Some(backing.data.len()));
        self.calls.push(Call::Free(scope, backing.clone()));
        Ok(())
    }

    fn open_scenario(&mut self, filename: &[u8], mode: &[u8]) -> Result<path::Opened, Stop> {
        assert!(!self.opened);
        self.calls
            .push(Call::Open(filename.to_vec(), mode.to_vec()));
        self.opened = matches!(self.open_reply, path::Opened::File(token) if token != 0);
        Ok(self.open_reply)
    }

    fn close_scenario(&mut self, capability: u64) -> Result<(), Stop> {
        if self.deny_close {
            return Err(Stop::Unavailable("test close unavailable".into()));
        }
        assert!(self.opened);
        assert_eq!(self.open_reply, path::Opened::File(capability));
        self.opened = false;
        self.calls.push(Call::Close(capability));
        Ok(())
    }
}

fn construction(root: &[u8], inputs: path::Inputs) -> (path::Construction, PathHost) {
    let (mut resources, mut host) = resource_construction();
    for _ in 0..80 {
        if resources.step(&mut host).unwrap() == Some(native_world::resources::Cut::VectorsReserved)
        {
            break;
        }
    }
    let capacity = root.len().max(7) + 1;
    let mut owner = Owner {
        token: 0xf98,
        data: vec![0x37; 324],
    };
    for (index, word) in [0x667ec0_u32, 0, 0, 0].into_iter().enumerate() {
        owner.data[104 + index * 4..108 + index * 4].copy_from_slice(&word.to_le_bytes());
    }
    let mut backing = Owner {
        token: 0x50001000,
        data: vec![0x6a; capacity],
    };
    backing.data[..root.len()].copy_from_slice(root);
    backing.data[root.len()] = 0;
    for (offset, value) in [
        (180, backing.token),
        (184, backing.token + root.len() as u64),
        (188, backing.token + capacity as u64),
    ] {
        owner.data[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
    }
    let application = path::Application::new(
        owner,
        backing,
        vec![Owner {
            token: 0x50002000,
            data: vec![0x53; 17],
        }],
    )
    .unwrap();
    let incoming = path::Incoming {
        application,
        buffer: Owner {
            token: 0x60000000,
            data: (0..272).map(|index| (index % 253) as u8).collect(),
        },
        output_descriptor: Owner {
            token: 0x60000120,
            data: vec![0x71; 12],
        },
        scenario_descriptor: Owner {
            token: 0x60000140,
            data: vec![0x42; 12],
        },
        retained: vec![Owner {
            token: 0x50003000,
            data: vec![0x67; 72],
        }],
    };
    (
        path::Construction::new(resources, incoming, inputs).unwrap(),
        PathHost::new(),
    )
}

fn finish(execution: &mut path::Construction, host: &mut PathHost, roundtrip: bool) {
    for _ in 0..100 {
        let cut = execution.step(host).unwrap();
        if roundtrip {
            let saved = serde_json::to_vec(execution).unwrap();
            let restored = serde_json::from_slice(&saved).unwrap();
            assert_eq!(*execution, restored);
            *execution = restored;
        }
        if cut == Some(path::Cut::ComposedPath) {
            return;
        }
    }
    panic!("path construction did not finish");
}

fn until(execution: &mut path::Construction, host: &mut PathHost, phase: &str) {
    for _ in 0..100 {
        if serde_json::to_value(&*execution).unwrap()["phase"] == phase {
            return;
        }
        execution.step(host).unwrap();
    }
    panic!("path never reached{phase}");
}

#[test]
fn path_composes_reached_resources_and_preserves_every_live_owner_and_untouched_byte() {
    let (mut execution, mut host) = construction(b"", path::Inputs::lightmap());
    let resources = execution.resources().clone();
    let before = execution.application().owner().unwrap();
    let buffer = execution.buffer().clone();
    for _ in 0..5 {
        execution.step(&mut host).unwrap();
    }
    assert_eq!(execution.cut(), Some(path::Cut::BufferConstructed));
    assert_eq!(&execution.buffer().data[..4], &0x667858_u32.to_le_bytes());
    assert_eq!(&execution.buffer().data[13..], &buffer.data[13..]);
    finish(&mut execution, &mut host, true);
    let state = execution.into_state().unwrap();
    state.validate().unwrap();
    assert_eq!(state.resources, resources);
    assert_eq!(state.bytes().unwrap(), b"./GameData/lightmap.dat");
    let after = state.application.owner().unwrap();
    assert_eq!(&before.data[..180], &after.data[..180]);
    assert_eq!(&before.data[192..], &after.data[192..]);
    assert_eq!(state.application.path().unwrap().backing.data, b"./\0jjjjj");
    assert_eq!(&state.buffer.data[36..], &buffer.data[36..]);
    assert!(!host.opened && host.live.is_empty());
    assert_eq!(host.calls.len(), 8);
    assert_eq!(
        host.calls[0],
        Call::Allocate(path::Scope::World, strings::Request::Construct(8))
    );
    assert_eq!(
        host.calls[1],
        Call::Allocate(path::Scope::Path, strings::Request::Construct(3))
    );
    assert_eq!(
        host.calls[2],
        Call::Allocate(path::Scope::Path, strings::Request::Grow(24))
    );
    assert_eq!(
        host.calls[3],
        Call::Free(
            path::Scope::Path,
            Owner {
                token: 0x70000020,
                data: b"./\0".to_vec()
            }
        )
    );
    assert_eq!(
        host.calls[4],
        Call::Open(b"./GameData/Scenario.tds".to_vec(), b"rb".to_vec())
    );
    assert_eq!(host.calls[5], Call::Close(0x40000000));
    assert_eq!(
        host.calls[6],
        Call::Free(
            path::Scope::Path,
            Owner {
                token: 0x70000040,
                data: b"./GameData/Scenario.tds\0".to_vec()
            }
        )
    );
    assert_eq!(
        host.calls[7],
        Call::Free(
            path::Scope::World,
            Owner {
                token: 0x70000000,
                data: b"./\0\xa5\xa5\xa5\xa5\xa5".to_vec()
            }
        )
    );
    assert_eq!(
        &state.output_descriptor.data[..4],
        &0x70000000_u32.to_le_bytes()
    );
    assert_eq!(
        &state.scenario_descriptor.data[..4],
        &0x70000040_u32.to_le_bytes()
    );
}

#[test]
fn cached_path_skips_scenario_and_does_not_insert_a_separator_before_directory() {
    let (mut execution, mut host) = construction(b"assets", path::Inputs::lightmap());
    let before = execution.application().clone();
    finish(&mut execution, &mut host, false);
    let state = execution.into_state().unwrap();
    state.validate().unwrap();
    assert_eq!(state.application, before);
    assert_eq!(state.bytes().unwrap(), b"assetsGameData/lightmap.dat");
    assert_eq!(host.calls.len(), 2);
    assert_eq!(state.scenario_descriptor.data, vec![0x42; 12]);
}

#[test]
fn cached_long_root_grows_only_the_world_temporary_and_preserves_cstring_semantics() {
    let inputs = path::Inputs {
        directory: Some(b"Data\0ignored".to_vec()),
        filename: Some(b"f\0ignored".to_vec()),
    };
    let (mut execution, mut host) = construction(b"a/\0cached-bytes", inputs);
    finish(&mut execution, &mut host, true);
    let state = execution.into_state().unwrap();
    assert_eq!(state.bytes().unwrap(), b"a/Data/f");
    assert_eq!(host.calls.len(), 4);
    assert!(host.calls.iter().all(|call| matches!(
        call,
        Call::Allocate(path::Scope::World, _) | Call::Free(path::Scope::World, _)
    )));
}

#[test]
fn bounded_appends_truncate_without_fabricating_space_for_separator_or_filename() {
    let inputs = path::Inputs {
        directory: Some(vec![b'x'; 300]),
        filename: Some(b"discarded".to_vec()),
    };
    let (mut execution, mut host) = construction(b"", inputs);
    finish(&mut execution, &mut host, false);
    let state = execution.into_state().unwrap();
    assert_eq!(state.bytes().unwrap().len(), 259);
    assert_eq!(&state.bytes().unwrap()[..2], b"./");
    assert!(state.bytes().unwrap()[2..].iter().all(|byte| *byte == b'x'));
    assert_eq!(state.buffer.data[271], 0);
}

#[test]
fn null_or_aliased_allocator_responses_remain_pending_and_are_never_reissued() {
    for reply in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0x1000,
            data: vec![0xa5; 8],
        },
    ] {
        let (mut execution, mut host) = construction(b"", path::Inputs::lightmap());
        host.reply = Some(reply);
        for _ in 0..30 {
            if execution.step(&mut host).is_err() {
                break;
            }
        }
        let saved = serde_json::to_vec(&execution).unwrap();
        assert!(execution.step(&mut host).is_err());
        assert_eq!(serde_json::to_vec(&execution).unwrap(), saved);
        assert_eq!(host.calls.len(), 1);
        assert!(execution.into_state().is_err());
    }
}

#[test]
fn failed_free_retains_old_and_replacement_until_acknowledged_without_reallocation() {
    let (mut execution, mut host) = construction(b"", path::Inputs::lightmap());
    host.deny_free = true;
    for _ in 0..60 {
        if execution.step(&mut host).is_err() {
            break;
        }
    }
    assert_eq!(host.calls.len(), 3);
    assert_eq!(host.live.len(), 3);
    let saved = serde_json::to_vec(&execution).unwrap();
    assert!(execution.step(&mut host).is_err());
    assert_eq!(serde_json::to_vec(&execution).unwrap(), saved);
    host.deny_free = false;
    finish(&mut execution, &mut host, false);
    assert!(host.live.is_empty());
}

#[test]
fn missing_null_and_unrepresentable_open_responses_never_become_path_success_or_reopen() {
    for reply in [
        path::Opened::Missing,
        path::Opened::File(0),
        path::Opened::File(u64::from(u32::MAX) + 1),
    ] {
        let (mut execution, mut host) = construction(b"", path::Inputs::lightmap());
        host.open_reply = reply;
        until(&mut execution, &mut host, "AcceptScenario");
        let saved = serde_json::to_vec(&execution).unwrap();
        assert!(execution.step(&mut host).is_err());
        assert!(execution.step(&mut host).is_err());
        assert_eq!(serde_json::to_vec(&execution).unwrap(), saved);
        assert_eq!(
            host.calls
                .iter()
                .filter(|call| matches!(call, Call::Open(..)))
                .count(),
            1
        );
        assert!(execution.into_state().is_err());
    }
}

#[test]
fn unavailable_close_retains_the_actual_file_and_retries_only_its_close() {
    let (mut execution, mut host) = construction(b"", path::Inputs::lightmap());
    until(&mut execution, &mut host, "CloseScenario");
    host.deny_close = true;
    assert!(host.opened);
    let calls = host.calls.clone();
    assert!(execution.step(&mut host).is_err());
    assert_eq!(host.calls, calls);
    host.deny_close = false;
    finish(&mut execution, &mut host, true);
    assert!(!host.opened);
    assert_eq!(
        host.calls
            .iter()
            .filter(|call| matches!(call, Call::Open(..)))
            .count(),
        1
    );
    assert_eq!(
        host.calls
            .iter()
            .filter(|call| matches!(call, Call::Close(..)))
            .count(),
        1
    );
}

#[test]
fn corrupted_restored_buffer_phase_source_and_retained_dependencies_fail_before_host_calls() {
    let (execution, _) = construction(b"cached/", path::Inputs::lightmap());
    let initial = serde_json::to_value(&execution).unwrap();
    for field in ["buffer", "phase", "retained", "commander"] {
        let mut changed = initial.clone();
        match field {
            "buffer" => changed["buffer"]["data"][271] = serde_json::json!(0),
            "phase" => changed["phase"] = serde_json::json!("OpenScenario"),
            "retained" => changed["retained"][0]["token"] = changed["buffer"]["token"].clone(),
            "commander" => changed["application"]["prefix"]["data"][104] = serde_json::json!(0),
            _ => unreachable!(),
        }
        let mut restored: path::Construction = serde_json::from_value(changed).unwrap();
        let mut host = PathHost::new();
        assert!(restored.step(&mut host).is_err());
        assert!(host.calls.is_empty());
    }
}
