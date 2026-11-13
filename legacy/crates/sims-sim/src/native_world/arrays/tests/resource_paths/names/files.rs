use super::*;
use crate::native_world::resources::file;

mod opening;

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileCall {
    File(u32),
    String(strings::Request),
    Free(Owner),
}

struct FileHost {
    next: u64,
    calls: Vec<FileCall>,
    live: BTreeMap<u64, usize>,
    reply: Option<Allocated>,
    unavailable: bool,
    deny_free: bool,
}

impl FileHost {
    fn new() -> Self {
        Self {
            next: 0x73000000,
            calls: vec![],
            live: BTreeMap::new(),
            reply: None,
            unavailable: false,
            deny_free: false,
        }
    }

    fn allocate(&mut self, bytes: u32) -> Allocated {
        let token = self.next;
        self.next += u64::from((bytes + 15) & !15);
        let reply = self.reply.take().unwrap_or_else(|| Allocated::Bytes {
            token,
            data: vec![0x93; bytes as usize],
        });
        if let Allocated::Bytes { token, data } = &reply {
            self.live.insert(*token, data.len());
        }
        reply
    }
}

impl file::Host for FileHost {
    fn allocate_file(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        if self.unavailable {
            return Err(Stop::Unavailable("file allocator pending".into()));
        }
        self.calls.push(FileCall::File(bytes));
        Ok(self.allocate(bytes))
    }

    fn allocate_string(&mut self, request: strings::Request) -> Result<Allocated, Stop> {
        if self.unavailable {
            return Err(Stop::Unavailable("file string allocator pending".into()));
        }
        self.calls.push(FileCall::String(request));
        Ok(self.allocate(request.bytes()))
    }

    fn free_string(&mut self, backing: &Owner) -> Result<(), Stop> {
        if self.deny_free {
            return Err(Stop::Unavailable("file string release pending".into()));
        }
        assert_eq!(self.live.remove(&backing.token), Some(backing.data.len()));
        self.calls.push(FileCall::Free(backing.clone()));
        Ok(())
    }
}

fn reached_names() -> names::State {
    let mut input = incoming();
    input.directory_descriptor.token = input.basename_descriptor.token + 12;
    let mut execution =
        names::Construction::new(path_state(path::Inputs::lightmap()), input).unwrap();
    let mut host = NameHost::new();
    for _ in 0..200 {
        if execution.step(&mut host).unwrap() == Some(names::Cut::FileAllocationEntry) {
            return execution.into_state().unwrap();
        }
    }
    panic!("manager name did not reach file allocation");
}

fn file_construction() -> (file::Construction, FileHost) {
    (
        file::Construction::new(reached_names()).unwrap(),
        FileHost::new(),
    )
}

fn finish_file(execution: &mut file::Construction, host: &mut FileHost) {
    for _ in 0..240 {
        let serialized = serde_json::to_vec(execution).unwrap();
        let restored: file::Construction = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(*execution, restored);
        restored.validate().unwrap();
        if execution.step(host).unwrap() == Some(file::Cut::Returned) {
            return;
        }
    }
    panic!("file construction did not return");
}

#[test]
fn file_constructor_preserves_complete_owners_and_actual_service_order() {
    let (mut execution, mut host) = file_construction();
    let context = execution.context().clone();
    finish_file(&mut execution, &mut host);
    let calls = host.calls.clone();
    assert_eq!(
        execution.step(&mut host).unwrap(),
        Some(file::Cut::Returned)
    );
    assert_eq!(host.calls, calls);
    assert_eq!(
        calls[..5],
        [
            FileCall::File(128),
            FileCall::String(strings::Request::Construct(8)),
            FileCall::String(strings::Request::Construct(8)),
            FileCall::String(strings::Request::Construct(24)),
            FileCall::String(strings::Request::Grow(24)),
        ]
    );
    assert!(matches!(&calls[5], FileCall::Free(owner) if owner.data.len() == 8));
    assert!(
        matches!(&calls[6], FileCall::Free(owner) if owner.data == b"./GameData/lightmap.dat\0")
    );
    let state = execution.into_state().unwrap();
    assert_eq!(state.context, context);
    assert_eq!(
        state.filename().unwrap().bytes().unwrap(),
        b"./GameData/lightmap.dat"
    );
    assert_eq!(
        state.secondary_backing.data,
        [vec![0], vec![0x93; 7]].concat()
    );
    assert_eq!(state.owner.data[25..28], [0x93; 3]);
    assert_eq!(host.live.len(), 3);
    assert_eq!(host.live.values().sum::<usize>(), 160);
    let restored: file::State =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(restored, state);
    restored.validate().unwrap();
}

#[test]
fn file_temporary_transfers_real_retired_stack_storage_without_aliasing() {
    let names = reached_names();
    let expected = [
        names.basename_descriptor.data[4..].to_vec(),
        names.directory_descriptor.data.clone(),
    ]
    .concat();
    let execution = file::Construction::new(names.clone()).unwrap();
    assert_eq!(
        execution.temporary().token,
        names.basename_descriptor.token + 4
    );
    assert_eq!(execution.temporary().data, expected);
    assert_eq!(
        execution.context().retired_prefix.data,
        names.basename_descriptor.data[..4]
    );
    let mut nonadjacent = names;
    nonadjacent.directory_descriptor.token += 4;
    assert!(file::Construction::new(nonadjacent).is_err());
}

#[test]
fn unavailable_file_allocator_preserves_the_exact_continuation() {
    let (mut execution, mut host) = file_construction();
    host.unavailable = true;
    let before = execution.clone();
    assert!(matches!(
        execution.step(&mut host),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(execution, before);
    assert!(host.calls.is_empty());
    host.unavailable = false;
    finish_file(&mut execution, &mut host);
}

#[test]
fn null_or_aliased_file_allocation_remains_pending_without_reallocation() {
    for reply in [
        Allocated::Null,
        Allocated::Bytes {
            token: 0x6fe548,
            data: vec![0x93; 128],
        },
    ] {
        let (mut execution, mut host) = file_construction();
        host.reply = Some(reply);
        let _ = execution.step(&mut host);
        let before = execution.clone();
        for _ in 0..3 {
            assert!(execution.step(&mut host).is_err());
            assert_eq!(execution, before);
        }
        assert_eq!(host.calls, [FileCall::File(128)]);
    }
}

#[test]
fn failed_primary_growth_release_retains_both_actual_backings() {
    let (mut execution, mut host) = file_construction();
    for _ in 0..160 {
        if execution.cut() == Some(file::Cut::TemporaryReady) {
            break;
        }
        execution.step(&mut host).unwrap();
    }
    assert_eq!(execution.cut(), Some(file::Cut::TemporaryReady));
    host.deny_free = true;
    let mut stopped = false;
    for _ in 0..40 {
        if let Err(Stop::Unavailable(_)) = execution.step(&mut host) {
            stopped = true;
            break;
        }
    }
    assert!(stopped);
    assert_eq!(host.live.len(), 5);
    execution.validate().unwrap();
    let before = execution.clone();
    let calls = host.calls.clone();
    assert!(matches!(
        execution.step(&mut host),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(execution, before);
    assert_eq!(host.calls, calls);
    host.deny_free = false;
    finish_file(&mut execution, &mut host);
}

#[test]
fn failed_file_temporary_destruction_cannot_return_or_drop_ownership() {
    let (mut execution, mut host) = file_construction();
    for _ in 0..200 {
        if execution.cut() == Some(file::Cut::FilenameAssigned) {
            break;
        }
        execution.step(&mut host).unwrap();
    }
    assert_eq!(execution.cut(), Some(file::Cut::FilenameAssigned));
    host.deny_free = true;
    let before = execution.clone();
    assert!(matches!(
        execution.step(&mut host),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(execution, before);
    assert_eq!(host.live.len(), 4);
    assert!(execution.clone().into_state().is_err());
    host.deny_free = false;
    finish_file(&mut execution, &mut host);
}

#[test]
fn file_constructor_rejects_tampered_embedded_descriptor_without_panicking() {
    let (mut execution, mut host) = file_construction();
    finish_file(&mut execution, &mut host);
    let mut serialized = serde_json::to_value(&execution).unwrap();
    serialized["primary"]["Live"]["owner"]["data"] = serde_json::json!([0]);
    let broken: file::Construction = serde_json::from_value(serialized).unwrap();
    assert!(broken.validate().is_err());
    let mut state = execution.into_state().unwrap();
    state.owner.data.truncate(8);
    assert!(state.filename().is_err());
    assert!(state.validate().is_err());
}
