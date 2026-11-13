use super::*;
use crate::{Index, Resource, avatar_catalog::Directories, object_resources::MountedStream};
use serde::{Deserialize, Serialize};
use sims_sim::saved_matrix::{Allocation, Backing, Host, Matrix, Read, Scratch};
use std::path::Path;

const ARRY: u32 = u32::from_be_bytes(*b"Arry");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct Services {
    unavailable_free: bool,
    freed: Vec<u64>,
    allocated: Vec<AllocationRequest>,
    named: usize,
}

impl saved_matrix::Host for Services {
    fn lookup(&mut self, _: u32, _: u32, _: u32) -> Result<Lookup, Stop> {
        panic!("source lookup must use the retained IFF")
    }
    fn source(&mut self, _: u64) -> Result<Vec<u8>, Stop> {
        panic!("source bytes must use the retained IFF")
    }
    fn release(&mut self, _: u64) -> Result<(), Stop> {
        panic!("source release must use the retained IFF")
    }
    fn free(&mut self, token: u64) -> Result<(), Stop> {
        if self.unavailable_free {
            return Err(Stop::Unavailable("allocation owner is pending".into()));
        }
        self.freed.push(token);
        Ok(())
    }
    fn allocate(&mut self, request: AllocationRequest) -> Result<Allocated, Stop> {
        self.allocated.push(request);
        Ok(Allocated::Bytes {
            token: 100 + self.allocated.len() as u64,
            data: vec![0xa5; request.data_bytes as usize],
        })
    }
}

impl saved_world::Host for Services {
    fn construct_temporary(&mut self, _: Legacy) -> Result<Temporary, Stop> {
        Err(Stop::Unavailable("legacy constructor is pending".into()))
    }
    fn destroy_temporary(&mut self, _: &Temporary) -> Result<(), Stop> {
        Err(Stop::Unavailable("legacy destructor is pending".into()))
    }
    fn name_arrays(&mut self) -> Result<(), Stop> {
        self.named += 1;
        Ok(())
    }
}

fn iff_bytes(chunks: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1".to_vec();
    bytes.resize(64, 0);
    for (id, payload) in chunks {
        let mut chunk = b"Arry".to_vec();
        chunk.extend(((76 + payload.len()) as u32).to_be_bytes());
        chunk.extend(id.to_be_bytes());
        chunk.extend(0u16.to_be_bytes());
        chunk.resize(76, 0);
        chunk.extend(payload);
        bytes.extend(chunk);
    }
    bytes
}

fn parsed(stride: i16, seed: u8) -> Vec<u8> {
    let mut bytes = stride.to_le_bytes().to_vec();
    bytes.extend(2i16.to_le_bytes());
    bytes.extend(2i16.to_le_bytes());
    bytes.resize(16, 0);
    bytes.extend((0..4 * stride as usize).map(|offset| seed.wrapping_add(offset as u8)));
    bytes
}

fn index(bytes: Vec<u8>) -> (Index, Directories) {
    let root = std::env::temp_dir().join("sims-saved-array-source-fixture");
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

fn matrix(stride: i32, token: u64) -> Matrix {
    Matrix {
        width: 2,
        height: 2,
        stride,
        allocation: Allocation::Known(Backing {
            token,
            column_offsets: vec![0, (2 * stride) as u32],
            data: vec![0x3c; 4 * stride as usize],
        }),
    }
}

#[test]
fn mounted_source_matches_direct_native_iff_queries_and_release() {
    let (index, directories) = index(iff_bytes(&[(0, parsed(1, 40)), (103, parsed(2, 90))]));
    compare_source(&index, &directories);
}

fn compare_source(index: &Index, directories: &Directories) {
    let mut stream = MountedStream::new(index, directories);
    let mut direct_stream = MountedStream::new(index, directories);
    let mut iff = Iff::default();
    let mut direct = Iff::default();
    assert_eq!(iff.open(b"./Fixture/House.iff", 0, &mut stream).unwrap(), 0);
    let mut file = File::from_returned(iff, stream);
    assert_eq!(
        direct
            .open(b"./Fixture/House.iff", 0, &mut direct_stream)
            .unwrap(),
        0
    );
    let mut services = Services::default();
    for id in [
        0,
        4,
        11,
        12,
        3,
        8,
        111,
        112,
        103,
        108,
        6,
        7,
        9,
        10,
        u32::MAX,
    ] {
        let expected = direct.by_id(ARRY, id, true, &mut direct_stream).unwrap();
        let result = Source {
            file: &mut file,
            services: &mut services,
        }
        .lookup(ARRY, id, saved_matrix::NOOP_RESOURCE_CALLBACK)
        .unwrap();
        assert_eq!(file.iff, direct);
        if expected == 0 {
            assert_eq!(
                result,
                Lookup::Missing {
                    native_status: direct.error
                }
            );
        } else {
            assert_eq!(
                result,
                Lookup::Found {
                    lease: u64::from(expected)
                }
            );
            let mut source = Source {
                file: &mut file,
                services: &mut services,
            };
            assert_eq!(
                source.source(u64::from(expected)).unwrap(),
                direct.payload(expected).unwrap()
            );
            assert_eq!(
                source
                    .lookup(ARRY, id, saved_matrix::NOOP_RESOURCE_CALLBACK)
                    .unwrap(),
                result
            );
            source.release(u64::from(expected)).unwrap();
            direct.release_chunk(expected).unwrap();
            assert_eq!(file.iff, direct);
        }
    }
    assert_eq!(services, Services::default());
    assert_eq!(
        file.with_iff(|iff, stream| iff.close(stream)).unwrap(),
        direct.close(&mut direct_stream).unwrap()
    );
    assert_eq!(file.iff, direct);
}

#[test]
fn native_missing_and_short_read_statuses_are_not_host_successes() {
    let mut bytes = iff_bytes(&[(0, parsed(1, 40))]);
    bytes.pop();
    let (index, directories) = index(bytes);
    let mut stream = MountedStream::new(&index, &directories);
    let mut iff = Iff::default();
    assert_eq!(iff.open(b"./Fixture/House.iff", 0, &mut stream).unwrap(), 0);
    let mut file = File::from_returned(iff, stream);
    let mut services = Services::default();
    let mut source = Source {
        file: &mut file,
        services: &mut services,
    };
    assert_eq!(
        source
            .lookup(ARRY, 9, saved_matrix::NOOP_RESOURCE_CALLBACK)
            .unwrap(),
        Lookup::Missing { native_status: -98 }
    );
    assert_eq!(
        source
            .lookup(ARRY, 0, saved_matrix::NOOP_RESOURCE_CALLBACK)
            .unwrap(),
        Lookup::Missing { native_status: -48 }
    );
    assert_eq!(source.file.iff.error, -48);
    assert_eq!(source.file.iff.resident_payloads().count(), 0);
    assert_eq!(services, Services::default());
}

#[test]
fn unsupported_callback_and_wide_lease_preserve_source_owners() {
    let (index, directories) = index(iff_bytes(&[(0, parsed(1, 40))]));
    let mut stream = MountedStream::new(&index, &directories);
    let mut iff = Iff::default();
    assert_eq!(iff.open(b"./Fixture/House.iff", 0, &mut stream).unwrap(), 0);
    let mut file = File::from_returned(iff, stream);
    let before = file.iff.clone();
    let mut services = Services::default();
    let mut source = Source {
        file: &mut file,
        services: &mut services,
    };
    assert!(matches!(
        source.lookup(ARRY, 0, 1),
        Err(Stop::Unsupported(_))
    ));
    assert!(matches!(source.source(1 << 32), Err(Stop::Unsupported(_))));
    assert!(matches!(source.release(1 << 32), Err(Stop::Unsupported(_))));
    assert_eq!(source.file.iff, before);
    assert_eq!(services, Services::default());
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MemoryStream {
    bytes: Vec<u8>,
    position: usize,
    opened: bool,
    reads: usize,
    unavailable_payload: bool,
    failed_payload_seek: bool,
}

impl Stream for MemoryStream {
    fn open(&mut self, _: &[u8], _: u32) -> Result<i32, String> {
        self.position = 0;
        self.opened = true;
        Ok(0)
    }
    fn seek(&mut self, offset: u32) -> Result<i32, String> {
        if self.failed_payload_seek && offset == 140 {
            return Ok(-48);
        }
        self.position = offset as usize;
        Ok(if self.opened { 0 } else { -49 })
    }
    fn read(&mut self, output: &mut [u8]) -> Result<i32, String> {
        if !self.opened {
            return Ok(-49);
        }
        if self.unavailable_payload && self.position == 140 {
            return Err("payload service has not returned".into());
        }
        self.reads += 1;
        let available = self.bytes.get(self.position..).unwrap_or_default();
        let count = output.len().min(available.len());
        output[..count].copy_from_slice(&available[..count]);
        self.position += count;
        Ok(if count == output.len() { 0 } else { -48 })
    }
    fn close(&mut self) -> Result<i32, String> {
        self.opened = false;
        Ok(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Owners {
    file: File<MemoryStream>,
    services: Services,
    matrix: Matrix,
    scratch: Scratch,
    read: Read,
}

impl Owners {
    fn new() -> Self {
        let mut stream = MemoryStream {
            bytes: iff_bytes(&[(0, parsed(2, 40))]),
            position: 0,
            opened: false,
            reads: 0,
            unavailable_payload: false,
            failed_payload_seek: false,
        };
        let mut iff = Iff::default();
        assert_eq!(iff.open(b"fixture", 0, &mut stream).unwrap(), 0);
        let mut matrix = matrix(1, 71);
        matrix.width = 1;
        Self {
            file: File::from_returned(iff, stream),
            services: Services::default(),
            matrix,
            scratch: Scratch {
                requested_converter: 99,
            },
            read: Read::new(ARRY, 0, saved_matrix::HEAD_CONVERTER),
        }
    }

    fn step(&mut self) -> Result<Option<i32>, Stop> {
        self.read.step(
            &mut self.matrix,
            &mut self.scratch,
            &mut Source {
                file: &mut self.file,
                services: &mut self.services,
            },
        )
    }

    fn finish(&mut self) {
        for _ in 0..50 {
            if let Some(status) = self.step().unwrap() {
                assert_eq!(status, 0);
                return;
            }
        }
        panic!("array read did not finish")
    }
}

#[test]
fn read_and_all_actual_source_owners_roundtrip_at_every_transition() {
    let initial = Owners::new();
    let mut expected = initial.clone();
    expected.finish();
    let mut actual = initial;
    for _ in 0..50 {
        actual = serde_json::from_slice(&serde_json::to_vec(&actual).unwrap()).unwrap();
        if let Some(status) = actual.step().unwrap() {
            assert_eq!(status, 0);
            assert_eq!(actual, expected);
            assert_eq!(actual.services.freed, [71]);
            assert_eq!(actual.services.allocated.len(), 1);
            assert_eq!(actual.file.iff.resident_payloads().count(), 0);
            let Allocation::Known(backing) = &actual.matrix.allocation else {
                panic!()
            };
            assert_eq!(backing.data, (40..48).collect::<Vec<_>>());
            return;
        }
    }
    panic!("cold-cut read did not finish")
}

#[test]
fn pending_allocator_retains_source_without_reopening_or_reloading() {
    let mut owners = Owners::new();
    owners.services.unavailable_free = true;
    for _ in 0..50 {
        if let Err(Stop::Unavailable(message)) = owners.step() {
            assert_eq!(message, "allocation owner is pending");
            let reads = owners.file.stream.reads;
            assert_eq!(owners.file.iff.resident_payloads().count(), 1);
            let retained = owners.clone();
            assert!(matches!(owners.step(), Err(Stop::Unavailable(_))));
            assert_eq!(owners, retained);
            owners = serde_json::from_slice(&serde_json::to_vec(&owners).unwrap()).unwrap();
            owners.services.unavailable_free = false;
            owners.finish();
            assert_eq!(owners.file.stream.reads, reads);
            assert_eq!(owners.services.freed, [71]);
            assert_eq!(owners.services.allocated.len(), 1);
            assert_eq!(owners.file.iff.resident_payloads().count(), 0);
            return;
        }
    }
    panic!("pending allocation boundary was not reached")
}

#[test]
fn world_array_read_uses_one_source_and_preserves_primary_and_unused_lights() {
    let specifications = [
        (0, 1),
        (4, 1),
        (11, 2),
        (12, 14),
        (3, 2),
        (8, 4),
        (111, 2),
        (112, 14),
        (103, 2),
        (108, 4),
        (6, 1),
        (7, 1),
    ];
    let chunks = specifications.map(|(id, stride)| (id, parsed(stride, id as u8 + 1)));
    let (index, directories) = index(iff_bytes(&chunks));
    let mut stream = MountedStream::new(&index, &directories);
    let mut iff = Iff::default();
    assert_eq!(iff.open(b"./Fixture/House.iff", 0, &mut stream).unwrap(), 0);
    let mut file = File::from_returned(iff, stream);
    let mut token = 0;
    let mut next_matrix = |stride| {
        token += 1;
        matrix(stride, token)
    };
    let floors = [0, 1].map(|_| saved_world::Floor {
        covering: next_matrix(2),
        wall: next_matrix(14),
        head: next_matrix(2),
        flags: next_matrix(4),
        primary: next_matrix(1),
        lights: saved_world::Lights {
            used: 1,
            capacity: 2,
            values: vec![[1, 2, 3, 4], [5, 6, 7, 8]],
        },
    });
    let mut state = saved_world::State {
        active_size: 2,
        altitude: next_matrix(1),
        ground: next_matrix(1),
        grass: next_matrix(1),
        target_grass: next_matrix(1),
        pool: next_matrix(1),
        water: next_matrix(1),
        floors,
    };
    let before = state.clone();
    let mut execution = saved_world::Execution::new(63).unwrap();
    let mut scratch = Scratch {
        requested_converter: 77,
    };
    let mut services = Services::default();
    for _ in 0..500 {
        let status = execution
            .step(
                &mut state,
                &mut scratch,
                &mut Source {
                    file: &mut file,
                    services: &mut services,
                },
            )
            .unwrap();
        if let Some(status) = status {
            assert_eq!(status, 0);
            break;
        }
    }
    assert_eq!(execution.status(), Some(0));
    assert_eq!(services.named, 1);
    assert!(services.allocated.is_empty());
    assert!(services.freed.is_empty());
    assert_eq!(file.iff.resident_payloads().count(), 0);
    assert_eq!(file.iff.error, -98);
    assert_eq!(state.active_size, before.active_size);
    assert_eq!(state.pool, before.pool);
    assert_eq!(state.water, before.water);
    assert_eq!(scratch.requested_converter, 0);
    for (floor, original) in state.floors.iter().zip(&before.floors) {
        assert_eq!(floor.primary, original.primary);
        assert_eq!(floor.lights.used, original.lights.used);
        assert_eq!(floor.lights.capacity, original.lights.capacity);
        assert_eq!(floor.lights.values, [[0x7f; 4], [5, 6, 7, 8]]);
    }
    let matrices = [
        &state.altitude,
        &state.ground,
        &state.floors[0].covering,
        &state.floors[0].wall,
        &state.floors[0].head,
        &state.floors[0].flags,
        &state.floors[1].covering,
        &state.floors[1].wall,
        &state.floors[1].head,
        &state.floors[1].flags,
        &state.grass,
        &state.target_grass,
    ];
    for (matrix, (id, stride)) in matrices.into_iter().zip(specifications) {
        let Allocation::Known(backing) = &matrix.allocation else {
            panic!()
        };
        assert_eq!(backing.data, parsed(stride, id as u8 + 1)[16..]);
    }
}

#[test]
fn unreturned_source_operation_cannot_become_a_cached_success_after_restore() {
    let mut owners = Owners::new();
    owners.file.stream.unavailable_payload = true;
    let error = Source {
        file: &mut owners.file,
        services: &mut owners.services,
    }
    .lookup(ARRY, 0, saved_matrix::NOOP_RESOURCE_CALLBACK)
    .unwrap_err();
    assert!(
        matches!(&error, Stop::Unavailable(message) if message.contains("payload service has not returned"))
    );
    assert_eq!(owners.file.iff.resident_payloads().count(), 1);
    owners = serde_json::from_slice(&serde_json::to_vec(&owners).unwrap()).unwrap();
    owners.file.stream.unavailable_payload = false;
    let before = owners.clone();
    let lease = u64::from(owners.file.iff.resident_payloads().next().unwrap().handle);
    let mut source = Source {
        file: &mut owners.file,
        services: &mut owners.services,
    };
    assert_eq!(
        source.lookup(ARRY, 0, saved_matrix::NOOP_RESOURCE_CALLBACK),
        Err(error.clone())
    );
    assert_eq!(source.source(lease), Err(error.clone()));
    assert_eq!(source.release(lease), Err(error));
    assert_eq!(owners, before);
}

#[test]
fn retained_file_requires_its_fault_word_even_when_the_last_call_returned() {
    let owners = Owners::new();
    let mut value = serde_json::to_value(&owners.file).unwrap();
    let file = value.as_object_mut().unwrap();
    assert_eq!(file.remove("fault"), Some(serde_json::Value::Null));
    assert!(serde_json::from_value::<File<MemoryStream>>(value).is_err());
    let mut value = serde_json::to_value(&owners.file).unwrap();
    value["unowned_source_cursor"] = serde_json::json!(0);
    assert!(serde_json::from_value::<File<MemoryStream>>(value).is_err());
}

#[test]
fn returned_native_seek_failure_preserves_its_nonzero_handle_and_is_not_a_fault() {
    let mut owners = Owners::new();
    owners.file.stream.failed_payload_seek = true;
    let mut direct_iff = owners.file.iff.clone();
    let mut direct_stream = owners.file.stream.clone();
    let handle = direct_iff.by_id(ARRY, 0, true, &mut direct_stream).unwrap();
    assert_ne!(handle, 0);
    let expected = Lookup::Found {
        lease: u64::from(handle),
    };
    let mut source = Source {
        file: &mut owners.file,
        services: &mut owners.services,
    };
    assert_eq!(
        source
            .lookup(ARRY, 0, saved_matrix::NOOP_RESOURCE_CALLBACK)
            .unwrap(),
        expected
    );
    assert_eq!(source.file.iff, direct_iff);
    assert_eq!(source.file.stream, direct_stream);
    assert_eq!(source.file.iff.error, -48);
    assert_eq!(source.file.fault, None);
    assert_eq!(
        direct_iff.by_id(ARRY, 0, true, &mut direct_stream).unwrap(),
        handle
    );
    assert_eq!(
        source
            .lookup(ARRY, 0, saved_matrix::NOOP_RESOURCE_CALLBACK)
            .unwrap(),
        expected
    );
    assert_eq!(source.file.iff, direct_iff);
    assert_eq!(source.file.stream, direct_stream);
    assert_eq!(source.file.iff.error, 0);
    source.release(u64::from(handle)).unwrap();
    direct_iff.release_chunk(handle).unwrap();
    assert_eq!(source.file.iff, direct_iff);
}

#[test]
#[ignore = "requires an explicitly selected original house file"]
fn installed_house_array_sources_match_direct_native_iff() {
    let path = std::env::var_os("SIMS_SAVED_ARRAY_TEST_HOUSE").expect("select original House.iff");
    let path = Path::new(&path);
    let bytes = std::fs::read(path).unwrap();
    let before = bytes.clone();
    let (mut index, directories) = index(bytes);
    let resource = &mut index.resources[0];
    resource.source = path.to_path_buf();
    resource.owned = None;
    compare_source(&index, &directories);
    assert_eq!(std::fs::read(path).unwrap(), before);
}
