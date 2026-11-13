use super::*;
use crate::{
    Index, Resource, avatar_catalog::Directories, house_startup::saved_arrays::File,
    object_resources::MountedStream,
};
use sims_sim::{
    native_world,
    native_world_children::Owner,
    object_catalog::iff::Iff,
    saved_matrix::{
        self, Allocated, Allocation, AllocationRequest, Backing, Matrix, Read, Resize, Scratch,
        Stop,
    },
};

fn reply(value: Allocated) -> (u64, Vec<u8>) {
    match value {
        Allocated::Bytes { token, data } => (token, data),
        Allocated::Null => panic!("real heap must not invent null allocator replies"),
    }
}

fn index(chunks: &[(u16, Vec<u8>)]) -> (Index, Directories) {
    let root = std::env::temp_dir().join("bootstrap-memory-source-test");
    let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1".to_vec();
    bytes.resize(64, 0);
    for (id, payload) in chunks {
        let mut chunk = b"Arry".to_vec();
        chunk.extend(((76 + payload.len()) as u32).to_be_bytes());
        chunk.extend(id.to_be_bytes());
        chunk.extend(0_u16.to_be_bytes());
        chunk.resize(76, 0);
        chunk.extend(payload);
        bytes.extend(chunk);
    }
    let size = bytes.len() as u64;
    let index = Index {
        root: root.clone(),
        resources: vec![Resource {
            locator: "Fixture/House.iff".into(),
            name: "House.iff".into(),
            source: root.join("House.iff"),
            offset: 0,
            size,
            unpacked_size: size,
            archived: false,
            owned: Some(bytes.into()),
        }],
        archives: 0,
        diagnostics: vec![],
    };
    let mut directories = Directories::default();
    directories
        .insert_first(b"./Fixture/", b"House.iff", 0)
        .unwrap();
    (index, directories)
}

fn opened<'a>(index: &'a Index, directories: &'a Directories) -> File<MountedStream<'a>> {
    let mut stream = MountedStream::new(index, directories);
    let mut iff = Iff::default();
    assert_eq!(iff.open(b"./Fixture/House.iff", 0, &mut stream).unwrap(), 0);
    File::from_returned(iff, stream)
}

#[test]
fn allocation_failure_preserves_accounting_and_old_tokens_are_never_reused() {
    let mut memory = Memory::new(64).unwrap();
    let first = memory.allocate_owner(40).unwrap();
    assert_eq!(first.data, vec![0; 40]);
    assert!(matches!(
        memory.allocate_owner(25),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!((memory.live_bytes(), memory.live_allocations()), (40, 1));
    assert!(memory.release_owner(first.token + 1).is_err());
    assert_eq!(memory.live_bytes(), 40);
    memory.release_owner(first.token).unwrap();
    assert!(memory.release_owner(first.token).is_err());
    let second = memory.allocate_owner(40).unwrap();
    assert!(second.token >= first.token + 40);
    assert!(memory.require_owner_view(first.token, 1).is_err());
    assert_eq!(second.token % 16, 0);
}

#[test]
fn derived_matrix_view_needs_one_complete_live_containing_owner() {
    let mut memory = Memory::new(128).unwrap();
    let owner = memory.allocate_owner(40).unwrap();
    assert!(memory.require_owner_view(owner.token + 4, 32).is_ok());
    for (offset, bytes) in [(9, 32), (0, 0), (40, 1)] {
        assert!(
            memory
                .require_owner_view(owner.token + offset, bytes)
                .is_err()
        );
    }
    assert!(memory.require_owner_view(u64::MAX, 32).is_err());
    memory.release_owner(owner.token).unwrap();
    assert!(memory.require_owner_view(owner.token + 4, 32).is_err());
}

#[test]
fn matrix_reserves_directory_and_returns_payload_only() {
    let mut memory = Memory::new(1024).unwrap();
    let request = AllocationRequest {
        columns: 3,
        data_bytes: 24,
        total_bytes: 36,
    };
    let (token, data) = reply(memory.allocate_matrix(request).unwrap());
    assert_eq!(data.len(), 24);
    assert_eq!(memory.allocation_bytes(token).unwrap().len(), 36);
    let next = memory.allocate_owner(16).unwrap();
    assert!(next.token >= token + 36);
    let backing = Backing {
        token,
        column_offsets: vec![0, 8, 16],
        data: (0..24).collect(),
    };
    memory.synchronize_matrix(&backing).unwrap();
    let full = memory.allocation_bytes(token).unwrap();
    let pointers: Vec<_> = full[..12]
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(
        pointers,
        vec![token as u32 + 12, token as u32 + 20, token as u32 + 28]
    );
    assert_eq!(&full[12..], backing.data);
    let before = full.to_vec();
    let mut bad = backing.clone();
    bad.column_offsets[2] = 24;
    assert!(memory.synchronize_matrix(&bad).is_err());
    assert_eq!(memory.allocation_bytes(token).unwrap(), before);
    assert!(
        memory
            .synchronize_owner(&Owner {
                token,
                data: before
            })
            .is_err()
    );
}

#[test]
fn zero_stride_matrix_still_owns_its_directory() {
    let mut memory = Memory::new(128).unwrap();
    let (token, data) = reply(
        memory
            .allocate_matrix(AllocationRequest {
                columns: 2,
                data_bytes: 0,
                total_bytes: 8,
            })
            .unwrap(),
    );
    assert!(data.is_empty());
    memory
        .synchronize_matrix(&Backing {
            token,
            column_offsets: vec![0, 0],
            data,
        })
        .unwrap();
    assert_eq!(
        memory.allocation_bytes(token).unwrap(),
        [(token as u32 + 8).to_le_bytes(); 2].concat()
    );
    memory.release_matrix(token).unwrap();
    assert_eq!(memory.live_bytes(), 0);
}

#[test]
fn malformed_matrix_requests_and_cross_kind_release_have_no_effect() {
    let mut memory = Memory::new(128).unwrap();
    for request in [
        AllocationRequest {
            columns: 0,
            data_bytes: 8,
            total_bytes: 8,
        },
        AllocationRequest {
            columns: 2,
            data_bytes: 8,
            total_bytes: 15,
        },
        AllocationRequest {
            columns: u32::MAX,
            data_bytes: 0,
            total_bytes: 4,
        },
        AllocationRequest {
            columns: 1,
            data_bytes: u32::MAX,
            total_bytes: 4,
        },
    ] {
        assert!(memory.allocate_matrix(request).is_err());
        assert_eq!((memory.live_bytes(), memory.live_allocations()), (0, 0));
    }
    let owner = memory.allocate_owner(32).unwrap();
    assert!(memory.release_matrix(owner.token).is_err());
    assert_eq!(memory.live_bytes(), 32);
    let (token, _) = reply(
        memory
            .allocate_matrix(AllocationRequest {
                columns: 2,
                data_bytes: 4,
                total_bytes: 12,
            })
            .unwrap(),
    );
    assert!(memory.release_owner(token).is_err());
    memory.release_matrix(token).unwrap();
    assert_eq!(memory.live_bytes(), 32);
}

#[test]
fn string_growth_preserves_both_actual_allocations_until_explicit_free() {
    use sims_sim::native_world::resources::strings::{self, Host as _};
    let mut memory = Memory::new(128).unwrap();
    let (old, _) = reply(
        memory
            .allocate_string(strings::Request::Construct(8))
            .unwrap(),
    );
    let (new, _) = reply(memory.allocate_string(strings::Request::Grow(20)).unwrap());
    assert_eq!((memory.live_bytes(), memory.live_allocations()), (28, 2));
    memory.free_string(old).unwrap();
    assert!(memory.free_string(old).is_err());
    assert_eq!(memory.allocation_bytes(new).unwrap().len(), 20);
    let owner = memory.allocate_owner(12).unwrap();
    assert!(memory.free_string(owner.token).is_err());
    assert!(
        memory
            .synchronize_owner(&Owner {
                token: new,
                data: vec![1; 19]
            })
            .is_err()
    );
    memory
        .synchronize_owner(&Owner {
            token: new,
            data: vec![7; 20],
        })
        .unwrap();
    assert_eq!(memory.allocation_bytes(new).unwrap(), vec![7; 20]);
}

#[test]
fn real_iff_read_resizes_the_same_matrix_and_releases_its_source_lease() {
    let mut payload = 1_i16.to_le_bytes().to_vec();
    payload.extend(2_i16.to_le_bytes());
    payload.extend(2_i16.to_le_bytes());
    payload.resize(16, 0);
    payload.extend([11, 22, 33, 44]);
    let (index, dirs) = index(&[(0, payload)]);
    let mut file = opened(&index, &dirs);
    let mut memory = Memory::new(1024).unwrap();
    let (old, data) = reply(
        memory
            .allocate_matrix(AllocationRequest {
                columns: 1,
                data_bytes: 1,
                total_bytes: 5,
            })
            .unwrap(),
    );
    let mut matrix = Matrix {
        width: 1,
        height: 1,
        stride: 1,
        allocation: Allocation::Known(Backing {
            token: old,
            column_offsets: vec![0],
            data,
        }),
    };
    let mut read = Read::new(u32::from_be_bytes(*b"Arry"), 0, 0);
    let mut scratch = Scratch {
        requested_converter: 91,
    };
    let mut complete = false;
    for _ in 0..128 {
        let status = read
            .step(
                &mut matrix,
                &mut scratch,
                &mut ArrayHost::new(&mut memory, &mut file),
            )
            .unwrap();
        if let Some(status) = status {
            assert_eq!(status, 0);
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!((matrix.width, matrix.height), (2, 2));
    assert!(memory.allocation_bytes(old).is_err());
    let Allocation::Known(backing) = &matrix.allocation else {
        panic!()
    };
    assert_eq!(backing.data, [11, 22, 33, 44]);
    assert!(read.held_lease.is_none());
    memory.synchronize_matrix(backing).unwrap();
    assert_eq!(memory.allocation_bytes(backing.token).unwrap().len(), 12);
    // A new lookup still uses this same resident IFF/stream, not a replacement
    // file or a manufactured Missing response.
    assert!(matches!(
        ResourceSource::lookup(
            &mut file,
            u32::from_be_bytes(*b"Arry"),
            0,
            saved_matrix::NOOP_RESOURCE_CALLBACK
        )
        .unwrap(),
        saved_matrix::Lookup::Found { .. }
    ));
}

#[test]
fn pending_resize_retries_same_owner_after_real_quota_failure() {
    let (index, dirs) = index(&[]);
    let mut file = opened(&index, &dirs);
    let mut memory = Memory::new(40).unwrap();
    let blocker = memory.allocate_owner(32).unwrap();
    let mut matrix = Matrix {
        width: -1,
        height: -1,
        stride: 1,
        allocation: Allocation::Null,
    };
    let mut resize = Resize::new(2, 2);
    let error = loop {
        match resize.step(&mut matrix, &mut ArrayHost::new(&mut memory, &mut file)) {
            Err(error) => break error,
            Ok(None) => (),
            Ok(Some(_)) => panic!("quota must stop before an allocation reply"),
        }
    };
    assert!(matches!(error, Stop::Unavailable(_)));
    assert_eq!(memory.live_allocations(), 1);
    memory.release_owner(blocker.token).unwrap();
    let mut complete = false;
    for _ in 0..8 {
        if resize
            .step(&mut matrix, &mut ArrayHost::new(&mut memory, &mut file))
            .unwrap()
            == Some(true)
        {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!(memory.live_allocations(), 1);
    assert_eq!(memory.live_bytes(), 12);
}

fn constructed_arrays() -> (native_world::arrays::State, Memory) {
    let (index, dirs) = index(&[]);
    let mut file = opened(&index, &dirs);
    let mut memory = Memory::new(1024 * 1024).unwrap();
    let mut prefix = native_world::Prefix::new(Default::default()).unwrap();
    for _ in 0..256 {
        if prefix.step(&mut memory).unwrap().is_some() {
            break;
        }
    }
    assert_eq!(
        prefix.cut(),
        Some(native_world::Cut::ArrayConstructionEntry)
    );
    let world_token = prefix.owner().unwrap().token;
    let mut arrays = native_world::arrays::Construction::new(
        prefix,
        native_world::arrays::Context {
            roof_owners: 0,
            random: 19,
        },
    )
    .unwrap();
    for _ in 0..20_000 {
        if arrays
            .step(&mut ArrayHost::new(&mut memory, &mut file))
            .unwrap()
            .is_some()
        {
            break;
        }
    }
    let state = arrays.into_state().unwrap();
    assert_eq!(state.prefix.owner.token, world_token);
    (state, memory)
}

#[test]
fn existing_world_prefix_and_all_arrays_use_one_real_heap() {
    let (state, mut memory) = constructed_arrays();
    let world_token = state.prefix.owner.token;
    assert_eq!(state.roots.len(), 20);
    assert_eq!(state.vectors.iter().flatten().count(), 2);
    memory.synchronize_owner(&state.prefix.owner).unwrap();
    for root in state.roots.values() {
        memory.synchronize_owner(root).unwrap();
    }
    for (identity, node) in &state.matrices.nodes {
        memory.require_owner_view(*identity, 32).unwrap();
        memory
            .synchronize_owner(&Owner {
                token: node.name.token,
                data: node.name.data.clone(),
            })
            .unwrap();
        let Allocation::Known(backing) = &node.matrix.allocation else {
            panic!()
        };
        memory.synchronize_matrix(backing).unwrap();
        assert_eq!(
            memory.allocation_bytes(backing.token).unwrap().len(),
            backing.column_offsets.len() * 4 + backing.data.len()
        );
    }
    for vector in state.vectors.iter().flatten() {
        memory.synchronize_owner(&vector.owner).unwrap();
        memory
            .synchronize_owner(vector.backing.as_ref().unwrap())
            .unwrap();
    }
    assert_eq!(
        &memory.allocation_bytes(world_token).unwrap()[16..20],
        &64_i32.to_le_bytes()
    );
    // This uses explicit synthetic constructor context for a source component
    // check; it does not produce a native-admitted World or a VM map owner.
}

struct NamingHost<'a, Source> {
    inner: ArrayHost<'a, Source>,
    allocations: usize,
    releases: usize,
    stop_after_release: bool,
    pending_release: Option<u64>,
    bad_name_reply: Option<(usize, u64)>,
}

impl<Source: ResourceSource> saved_matrix::Host for NamingHost<'_, Source> {
    fn lookup(&mut self, tag: u32, id: u32, callback: u32) -> Result<saved_matrix::Lookup, Stop> {
        saved_matrix::Host::lookup(&mut self.inner, tag, id, callback)
    }
    fn source(&mut self, lease: u64) -> Result<Vec<u8>, Stop> {
        saved_matrix::Host::source(&mut self.inner, lease)
    }
    fn release(&mut self, lease: u64) -> Result<(), Stop> {
        saved_matrix::Host::release(&mut self.inner, lease)
    }
    fn free(&mut self, token: u64) -> Result<(), Stop> {
        saved_matrix::Host::free(&mut self.inner, token)
    }
    fn allocate(&mut self, request: AllocationRequest) -> Result<Allocated, Stop> {
        saved_matrix::Host::allocate(&mut self.inner, request)
    }
}

impl<Source: ResourceSource> sims_sim::native_matrix::Host for NamingHost<'_, Source> {
    fn require_owner(&self, owner: u64, bytes: u32) -> Result<(), Stop> {
        sims_sim::native_matrix::Host::require_owner(&self.inner, owner, bytes)
    }
    fn allocate_name(&mut self, bytes: u32) -> Result<Allocated, Stop> {
        if let Some((ordinal, token)) = self.bad_name_reply {
            if ordinal == self.allocations + 1 {
                self.allocations += 1;
                return Ok(Allocated::Bytes {
                    token,
                    data: vec![0; bytes as usize],
                });
            }
        }
        let reply = sims_sim::native_matrix::Host::allocate_name(&mut self.inner, bytes)?;
        self.allocations += 1;
        Ok(reply)
    }
    fn release_name(&mut self, token: u64) -> Result<(), Stop> {
        if self.pending_release == Some(token) {
            self.pending_release = None;
            return Ok(());
        }
        assert!(self.pending_release.is_none());
        sims_sim::native_matrix::Host::release_name(&mut self.inner, token)?;
        self.releases += 1;
        if self.stop_after_release {
            self.stop_after_release = false;
            self.pending_release = Some(token);
            return Err(Stop::Unavailable(
                "actual release completed; return acknowledgment pending".into(),
            ));
        }
        Ok(())
    }
}

#[test]
fn all_eighteen_names_retain_one_array_graph_and_real_temporary_lifetimes() {
    use native_world::arrays::naming::Execution;
    let (source, mut memory) = constructed_arrays();
    let before = (memory.live_bytes(), memory.live_allocations());
    let (index, dirs) = index(&[]);
    let mut file = opened(&index, &dirs);
    let mut host = NamingHost {
        inner: ArrayHost::new(&mut memory, &mut file),
        allocations: 0,
        releases: 0,
        stop_after_release: false,
        pending_release: None,
        bad_name_reply: None,
    };
    let mut execution = Execution::new(source.clone()).unwrap();
    let mut complete = false;
    for _ in 0..256 {
        let result = execution.step(&mut host).unwrap();
        let restored: Execution =
            serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
        assert_eq!(restored, execution);
        execution = restored;
        if result {
            complete = true;
            break;
        }
    }
    assert!(complete);
    assert_eq!(execution.completed_names(), 18);
    assert_eq!((host.allocations, host.releases), (18, 18));
    assert!(execution.step(&mut host).unwrap());
    assert_eq!((host.allocations, host.releases), (18, 18));
    let returned = execution.into_returned().unwrap();
    returned.validate().unwrap();
    assert_eq!(returned.source(), &source);
    assert_eq!(returned.arrays(), &source);
    assert_eq!((memory.live_bytes(), memory.live_allocations()), before);
}

#[test]
fn naming_quota_stop_keeps_source_and_incomplete_extraction_keeps_the_child() {
    use native_world::arrays::naming::Execution;
    let (source, mut memory) = constructed_arrays();
    let blocker = memory
        .allocate_owner((1024 * 1024 - memory.live_bytes()) as u32)
        .unwrap();
    let (index, dirs) = index(&[]);
    let mut file = opened(&index, &dirs);
    let mut execution = Execution::new(source.clone()).unwrap();
    let before = execution.clone();
    assert!(matches!(
        execution.step(&mut ArrayHost::new(&mut memory, &mut file)),
        Err(Stop::Unavailable(_))
    ));
    assert_eq!(execution, before);
    execution = execution.into_returned().unwrap_err();
    assert_eq!(execution.source(), &source);
    memory.release_owner(blocker.token).unwrap();
    for _ in 0..256 {
        if execution
            .step(&mut ArrayHost::new(&mut memory, &mut file))
            .unwrap()
        {
            break;
        }
    }
    assert!(execution.into_returned().is_ok());
}

#[test]
fn naming_release_acknowledgment_resumes_without_a_second_real_free() {
    use native_world::arrays::naming::Execution;
    let (source, mut memory) = constructed_arrays();
    let before = (memory.live_bytes(), memory.live_allocations());
    let (index, dirs) = index(&[]);
    let mut file = opened(&index, &dirs);
    let mut host = NamingHost {
        inner: ArrayHost::new(&mut memory, &mut file),
        allocations: 0,
        releases: 0,
        stop_after_release: true,
        pending_release: None,
        bad_name_reply: None,
    };
    let mut execution = Execution::new(source.clone()).unwrap();
    let mut stopped = false;
    for _ in 0..32 {
        match execution.step(&mut host) {
            Err(Stop::Unavailable(_)) => {
                stopped = true;
                break;
            }
            Ok(false) => (),
            other => panic!("expected the retained release stop: {other:?}"),
        }
    }
    assert!(stopped);
    assert_eq!((host.allocations, host.releases), (1, 1));
    assert!(host.pending_release.is_some());
    execution.validate().unwrap();
    execution = serde_json::from_slice(&serde_json::to_vec(&execution).unwrap()).unwrap();
    for _ in 0..256 {
        if execution.step(&mut host).unwrap() {
            break;
        }
    }
    assert!(execution.complete());
    assert_eq!((host.allocations, host.releases), (18, 18));
    assert!(host.pending_release.is_none());
    assert_eq!(execution.arrays(), &source);
    assert_eq!((memory.live_bytes(), memory.live_allocations()), before);
}

#[test]
fn restored_naming_rejects_foreign_array_mutation_and_lost_service_history() {
    use native_world::arrays::naming::Execution;
    let (source, mut memory) = constructed_arrays();
    let (index, dirs) = index(&[]);
    let mut file = opened(&index, &dirs);
    let mut execution = Execution::new(source).unwrap();
    execution
        .step(&mut ArrayHost::new(&mut memory, &mut file))
        .unwrap();
    let value = serde_json::to_value(&execution).unwrap();
    let mut changed = value.clone();
    changed["progress"]["arrays"]["context"]["random"] = 20.into();
    assert!(serde_json::from_value::<Execution>(changed).is_err());
    let mut changed = value.clone();
    changed["effects"] = serde_json::json!([]);
    assert!(serde_json::from_value::<Execution>(changed).is_err());
    let mut changed = value;
    changed["steps"] = 0.into();
    assert!(serde_json::from_value::<Execution>(changed).is_err());
}

#[test]
fn invalid_nested_name_pointer_is_rejected_before_freeing_the_old_real_name() {
    use native_world::arrays::naming::Execution;
    use sims_sim::native_matrix::{Host as _, Name};
    for bad_token in [u64::from(u32::MAX) + 16, u64::from(u32::MAX) - 4, u64::MAX] {
        let (mut source, mut memory) = constructed_arrays();
        let (index, dirs) = index(&[]);
        let mut file = opened(&index, &dirs);
        let identity = source.roots[&0x14].token;
        // A real eight-byte initial name forces the nested assignment to grow
        // for "altitude". The invalid reply is a deliberate negative control.
        let mut adapter = ArrayHost::new(&mut memory, &mut file);
        let (old, data) = reply(adapter.allocate_name(8).unwrap());
        adapter
            .release_name(source.matrices.nodes[&identity].name.token)
            .unwrap();
        source.matrices.nodes.get_mut(&identity).unwrap().name = Name {
            token: old,
            length: 0,
            data,
        };
        let root = source.roots.get_mut(&0x14).unwrap();
        for (offset, word) in [(16, old as u32), (20, old as u32), (24, old as u32 + 8)] {
            root.data[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        let mut execution = Execution::new(source).unwrap();
        let mut host = NamingHost {
            inner: adapter,
            allocations: 0,
            releases: 0,
            stop_after_release: false,
            pending_release: None,
            bad_name_reply: Some((2, bad_token)),
        };
        let mut rejected = false;
        for _ in 0..32 {
            if execution.step(&mut host).is_err() {
                rejected = true;
                break;
            }
        }
        assert!(rejected);
        assert_eq!(host.allocations, 2);
        assert_eq!(
            host.releases, 0,
            "invalid pointer must stop before releasing the old name"
        );
        assert!(execution.step(&mut host).is_err());
        assert_eq!((host.allocations, host.releases), (2, 0));
        assert!(memory.allocation_bytes(old).is_ok());
    }
}

#[test]
fn naming_preserves_actual_game_to_world_source_ownership() {
    use native_world::arrays::naming::Execution;
    use sims_sim::native_game;
    let mut memory = Memory::new(1024 * 1024).unwrap();
    let game = memory.allocate_owner(112).unwrap();
    // Explicit synthetic caller input, followed by the actual modeled Game and
    // World constructors; no native application/provider admission is claimed.
    let mut construction = native_game::Construction::new(native_game::Incoming {
        game: game.token,
        owners: vec![game],
        registry: Default::default(),
        provider_word: 0x1234,
    })
    .unwrap();
    for _ in 0..32 {
        if construction.cut() == native_game::Cut::Returned {
            break;
        }
        construction.step(&mut memory).unwrap();
    }
    let mut entry = construction.into_state().unwrap().begin_world().unwrap();
    for _ in 0..256 {
        if entry.step(&mut memory).unwrap().is_some() {
            break;
        }
    }
    let mut arrays = entry
        .into_arrays(native_world::arrays::Context {
            roof_owners: 0,
            random: 19,
        })
        .unwrap();
    let (index, dirs) = index(&[]);
    let mut file = opened(&index, &dirs);
    for _ in 0..20_000 {
        if arrays
            .step(&mut ArrayHost::new(&mut memory, &mut file))
            .unwrap()
            .is_some()
        {
            break;
        }
    }
    let source = arrays.into_state().unwrap();
    assert!(source.prefix.game().is_some());
    let mut execution = Execution::new(source.clone()).unwrap();
    for _ in 0..256 {
        if execution
            .step(&mut ArrayHost::new(&mut memory, &mut file))
            .unwrap()
        {
            break;
        }
    }
    let returned = execution.into_returned().unwrap();
    assert_eq!(returned.source().prefix.game(), source.prefix.game());
    assert_eq!(returned.arrays(), &source);
}
