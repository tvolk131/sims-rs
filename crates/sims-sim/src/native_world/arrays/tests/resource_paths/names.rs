use super::*;

mod files;
use crate::native_world::resources::{manager, names};

#[derive(Debug, Clone, PartialEq, Eq)]
enum NameCall {
    Allocate(strings::Request),
    Free(Owner),
}

struct NameHost {
    next: u64,
    calls: Vec<NameCall>,
    live: BTreeMap<u64, usize>,
    reply: Option<Allocated>,
    deny_allocation: bool,
    deny_free: bool,
}

impl NameHost {
    fn new() -> Self {
        Self {
            next: 0x72000000,
            calls: vec![],
            live: BTreeMap::new(),
            reply: None,
            deny_allocation: false,
            deny_free: false,
        }
    }
}

impl names::Host for NameHost {
    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        if self.deny_allocation {
            return Err(Stop::Unavailable("name allocation pending".into()));
        }
        self.calls.push(NameCall::Allocate(request));
        let token = self.next;
        self.next += u64::from((request.bytes() + 15) & !15);
        let reply = self.reply.take().unwrap_or_else(|| Allocated::Bytes {
            token,
            data: vec![0xa5; request.bytes() as usize],
        });
        if let Allocated::Bytes { token, data } = &reply {
            self.live.insert(*token, data.len());
        }
        Ok(reply)
    }

    fn free_string(&mut self, backing: &Owner) -> Result<(), Stop> {
        if self.deny_free {
            return Err(Stop::Unavailable("name release pending".into()));
        }
        assert_eq!(self.live.remove(&backing.token), Some(backing.data.len()));
        self.calls.push(NameCall::Free(backing.clone()));
        Ok(())
    }
}

struct ManagerHost;

impl manager::Host for ManagerHost {
    fn allocate_buckets(&mut self, map: u8, bytes: u32) -> Result<Allocated, Stop> {
        Ok(Allocated::Bytes {
            token: 0x71000000 + u64::from(map) * 0x400,
            data: vec![0xa5; bytes as usize],
        })
    }

    fn initialize_section(&mut self, address: u64, bytes: u32) -> Result<u64, Stop> {
        assert_eq!((address, bytes), (0x6fe578, 24));
        Ok(1)
    }
}

fn path_state(inputs: path::Inputs) -> path::State {
    let (mut execution, mut host) = construction(b"", inputs);
    finish(&mut execution, &mut host, false);
    let mut state = execution.into_state().unwrap();
    let mut manager = manager::Construction::new(
        Owner {
            token: 0x6fe548,
            data: vec![0x61; 72],
        },
        [0, 0],
        vec![],
    )
    .unwrap();
    while !manager.step(&mut ManagerHost).unwrap() {}
    let manager = manager.into_state().unwrap();
    state
        .retained
        .extend(std::iter::once(manager.owner).chain(manager.buckets));
    state
}

fn incoming() -> names::Incoming {
    names::Incoming {
        basename_descriptor: Owner {
            token: 0x60000160,
            data: vec![0x53; 12],
        },
        directory_descriptor: Owner {
            token: 0x60000180,
            data: vec![0x71; 12],
        },
        synchronization: 1,
        manager_retained: vec![],
    }
}

fn name_construction() -> (names::Construction, NameHost) {
    (
        names::Construction::new(path_state(path::Inputs::lightmap()), incoming()).unwrap(),
        NameHost::new(),
    )
}

fn phase(execution: &names::Construction) -> String {
    serde_json::to_value(execution).unwrap()["phase"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn until_phase(execution: &mut names::Construction, host: &mut NameHost, desired: &str) {
    for _ in 0..200 {
        if phase(execution) == desired {
            return;
        }
        execution.step(host).unwrap();
    }
    panic!("manager names did not reach {desired}");
}

#[test]
fn manager_names_compose_nine_services_four_cuts_and_reuse_actual_output_descriptor() {
    let (mut execution, mut host) = name_construction();
    let earlier = execution.context().clone();
    let manager = execution.manager().clone();
    let mut cuts = Vec::new();
    for _ in 0..200 {
        if let Some(cut) = execution.step(&mut host).unwrap() {
            cuts.push(cut);
        }
        let restored: names::Construction =
            serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        assert_eq!(execution, restored);
        execution = restored;
        assert_eq!(execution.context(), &earlier);
        assert_eq!(execution.manager(), &manager);
        if execution.cut() == Some(names::Cut::FileAllocationEntry) {
            break;
        }
    }
    assert_eq!(
        cuts,
        vec![
            names::Cut::NameCopied,
            names::Cut::NameNormalized,
            names::Cut::LookupReturn,
            names::Cut::FileAllocationEntry
        ]
    );
    assert_eq!(execution.lookup(), Some(manager::Lookup::Missing));
    let state = execution.into_state().unwrap();
    assert_eq!(state.name.owner.token, 0x60000120);
    assert_eq!(state.name.bytes().unwrap(), b".\\gamedata\\lightmap.dat");
    assert_eq!(host.calls.len(), 9);
    let allocations: Vec<_> = host
        .calls
        .iter()
        .filter_map(|call| {
            if let NameCall::Allocate(request) = call {
                Some(*request)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        allocations,
        vec![
            strings::Request::Construct(24),
            strings::Request::Construct(8),
            strings::Request::Construct(8),
            strings::Request::Grow(12),
            strings::Request::Grow(13)
        ]
    );
    let releases: Vec<_> = host
        .calls
        .iter()
        .filter_map(|call| {
            if let NameCall::Free(owner) = call {
                Some(owner)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        releases
            .iter()
            .map(|owner| owner.data.len())
            .collect::<Vec<_>>(),
        vec![8, 8, 12, 13]
    );
    assert_eq!(&releases[2].data, b".\\gamedata\\\0");
    assert_eq!(&releases[3].data, b"lightmap.dat\0");
    assert_eq!(host.live, BTreeMap::from([(state.name.backing.token, 24)]));
    assert_eq!(
        &state.directory_descriptor.data[..4],
        &(releases[2].token as u32).to_le_bytes()
    );
    assert_eq!(
        &state.basename_descriptor.data[..4],
        &(releases[3].token as u32).to_le_bytes()
    );
}

#[test]
fn manager_names_unavailable_allocation_keeps_the_real_pending_constructor() {
    let (mut execution, mut host) = name_construction();
    host.deny_allocation = true;
    loop {
        let before = execution.clone();
        if let Err(Stop::Unavailable(_)) = execution.step(&mut host) {
            assert_eq!(execution, before);
            break;
        }
    }
    assert!(host.calls.is_empty());
    host.deny_allocation = false;
    until_phase(&mut execution, &mut host, "FileAllocationEntry");
    assert_eq!(host.calls.len(), 9);
}

#[test]
fn manager_names_rejected_null_and_global_alias_replies_do_not_reallocate() {
    for reply in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0x60000010,
            data: vec![0xa5; 24],
        },
        Allocated::Bytes {
            token: 0x71000000,
            data: vec![0xa5; 24],
        },
    ] {
        let (mut execution, mut host) = name_construction();
        host.reply = Some(reply);
        let mut stopped = false;
        for _ in 0..20 {
            if execution.step(&mut host).is_err() {
                stopped = true;
                break;
            }
        }
        assert!(stopped);
        let before = execution.clone();
        assert!(execution.step(&mut host).is_err());
        assert_eq!(execution, before);
        assert_eq!(host.calls.len(), 1);
        assert!(execution.into_state().is_err());
    }
}

#[test]
fn manager_names_failed_growth_release_retains_both_actual_backings() {
    let (mut execution, mut host) = name_construction();
    until_phase(&mut execution, &mut host, "AssignDirectory");
    host.deny_free = true;
    let mut stopped = false;
    for _ in 0..30 {
        if execution.step(&mut host).is_err() {
            stopped = true;
            break;
        }
    }
    assert!(stopped);
    assert_eq!(host.live.len(), 4);
    let before = execution.clone();
    let calls = host.calls.clone();
    assert!(execution.step(&mut host).is_err());
    assert_eq!(execution, before);
    assert_eq!(host.calls, calls);
    host.deny_free = false;
    until_phase(&mut execution, &mut host, "FileAllocationEntry");
    assert_eq!(host.calls.len(), 9);
}

#[test]
fn manager_names_failed_destructor_keeps_result_and_does_not_zero_descriptor() {
    let (mut execution, mut host) = name_construction();
    until_phase(&mut execution, &mut host, "ReleaseDirectory");
    let directory = execution.directory().unwrap().clone();
    host.deny_free = true;
    let before = execution.clone();
    assert!(execution.step(&mut host).is_err());
    assert_eq!(execution, before);
    assert_eq!(execution.lookup(), Some(manager::Lookup::Missing));
    host.deny_free = false;
    until_phase(&mut execution, &mut host, "FileAllocationEntry");
    assert_eq!(
        execution.into_state().unwrap().directory_descriptor,
        directory.owner
    );
}

#[test]
fn manager_names_missing_manager_capability_or_aliased_locals_fail_before_services() {
    let state = path_state(path::Inputs::lightmap());
    let mut locals = incoming();
    locals.synchronization = 0;
    assert!(names::Construction::new(state.clone(), locals).is_err());
    let mut locals = incoming();
    locals.basename_descriptor.token = state.output_descriptor.token;
    assert!(names::Construction::new(state.clone(), locals).is_err());
    let mut missing = state;
    missing.retained.retain(|owner| owner.token != 0x6fe548);
    assert!(names::Construction::new(missing, incoming()).is_err());
}

#[test]
fn manager_names_non_ascii_normalization_stops_without_fabricating_lookup() {
    let state = path_state(path::Inputs {
        directory: Some(b"GameData".to_vec()),
        filename: Some(vec![0xff]),
    });
    let mut execution = names::Construction::new(state, incoming()).unwrap();
    let mut host = NameHost::new();
    let mut stopped = false;
    for _ in 0..30 {
        if execution.step(&mut host).is_err() {
            stopped = true;
            break;
        }
    }
    assert!(stopped);
    assert_eq!(execution.lookup(), None);
    assert!(execution.into_state().is_err());
    assert_eq!(host.calls.len(), 1);
}

#[test]
fn manager_names_corrupt_restored_buffer_descriptor_source_or_result_fails_closed() {
    let (mut execution, mut host) = name_construction();
    until_phase(&mut execution, &mut host, "LookupReturn");
    let encoded = serde_json::to_value(&execution).unwrap();
    for field in ["buffer", "scenario_descriptor"] {
        let mut corrupt = encoded.clone();
        corrupt["context"][field]["token"] = serde_json::json!(0x71000000_u64);
        let restored: names::Construction = serde_json::from_value(corrupt).unwrap();
        assert!(restored.validate().is_err());
    }
    let mut corrupt = encoded;
    corrupt["lookup"] = serde_json::json!({"Resource": 0x73000010_u64});
    let restored: names::Construction = serde_json::from_value(corrupt).unwrap();
    assert!(restored.validate().is_err());
    let state = execution.clone();
    let mut corrupt = serde_json::to_value(execution).unwrap();
    corrupt["name"]["Live"]["backing"]["data"][1] = serde_json::json!(b'/');
    let restored: names::Construction = serde_json::from_value(corrupt).unwrap();
    assert!(restored.validate().is_err());
    assert_eq!(state.lookup(), Some(manager::Lookup::Missing));
}
