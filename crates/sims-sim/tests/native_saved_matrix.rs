#![cfg(feature = "private-fixtures")]

//! Original component cases use authored source/allocation services. These are
//! not current saved payloads, world startup qualification or gameplay parity.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_sim::saved_matrix::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Service {
    Lookup,
    Source,
    Release,
    Free,
    Allocate,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Services {
    source: Vec<u8>,
    missing: bool,
    allocation_null: bool,
    block: Option<Service>,
    events: Vec<Value>,
    lookups: usize,
    sources: usize,
    releases: usize,
    frees: usize,
    allocations: usize,
    references: usize,
    next_token: u64,
}
impl Services {
    fn ready(&self, service: Service) -> Result<(), Stop> {
        if self.block == Some(service) {
            Err(Stop::Unavailable(format!("{service:?} pending")))
        } else {
            Ok(())
        }
    }
}
impl Host for Services {
    fn lookup(&mut self, tag: u32, id: u32, callback: u32) -> Result<Lookup, Stop> {
        self.ready(Service::Lookup)?;
        self.lookups += 1;
        self.events.push(json!(["lookup", [tag, id, callback]]));
        if self.missing {
            Ok(Lookup::Missing { native_status: -98 })
        } else {
            self.references += 1;
            Ok(Lookup::Found { lease: 7 })
        }
    }
    fn source(&mut self, lease: u64) -> Result<Vec<u8>, Stop> {
        self.ready(Service::Source)?;
        assert_eq!(lease, 7);
        self.sources += 1;
        Ok(self.source.clone())
    }
    fn release(&mut self, lease: u64) -> Result<(), Stop> {
        self.ready(Service::Release)?;
        assert_eq!(lease, 7);
        self.releases += 1;
        self.references -= 1;
        self.events.push(json!(["release"]));
        Ok(())
    }
    fn free(&mut self, token: u64) -> Result<(), Stop> {
        self.ready(Service::Free)?;
        self.frees += 1;
        self.events.push(json!(["free_old", token == 1]));
        Ok(())
    }
    fn allocate(&mut self, request: AllocationRequest) -> Result<Allocated, Stop> {
        self.ready(Service::Allocate)?;
        self.allocations += 1;
        self.events.push(json!(["allocate", request.total_bytes]));
        if self.allocation_null {
            Ok(Allocated::Null)
        } else {
            let token = self.next_token;
            self.next_token += 1;
            Ok(Allocated::Bytes {
                token,
                data: vec![0; request.data_bytes as usize],
            })
        }
    }
}
fn case_source(case: &Value) -> Vec<u8> {
    let width = case["width"].as_u64().unwrap() as u32;
    let height = case["height"].as_u64().unwrap() as u32;
    let stride = case["stride"].as_u64().unwrap() as u32;
    let body: Vec<u8> = serde_json::from_value(case["body"].clone()).unwrap();
    if case["parsed"].as_bool().unwrap() {
        let mut result = [stride as i16, width as i16, height as i16, 0]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
        result.extend(vec![0xcc; width as usize * 4]);
        result.extend(body);
        result
    } else {
        let rle = case["rle"].as_bool().unwrap();
        let mut result = [0, width, height, stride, u32::from(rle)]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        if rle {
            result.extend_from_slice(&(body.len() as u16).to_le_bytes())
        }
        result.extend(body);
        result
    }
}
fn setup(case: &Value) -> (Matrix, Scratch, Services) {
    let width = if case["same_size"].as_bool().unwrap() {
        case["width"].as_i64().unwrap() as i32
    } else {
        2
    };
    let height = if case["same_size"].as_bool().unwrap() {
        case["height"].as_i64().unwrap() as i32
    } else {
        2
    };
    let stride = case["stride"].as_i64().unwrap() as i32;
    let matrix = Matrix {
        width,
        height,
        stride,
        allocation: Allocation::Known(Backing {
            token: 1,
            column_offsets: (0..width).map(|x| (x * height * stride) as u32).collect(),
            data: vec![0xa5; (width * height * stride) as usize],
        }),
    };
    let host = Services {
        source: case_source(case),
        missing: case["missing"].as_bool().unwrap(),
        allocation_null: case["allocation_fail"].as_bool().unwrap(),
        block: None,
        events: vec![],
        lookups: 0,
        sources: 0,
        releases: 0,
        frees: 0,
        allocations: 0,
        references: 1,
        next_token: 2,
    };
    (
        matrix,
        Scratch {
            requested_converter: 0,
        },
        host,
    )
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/saved-matrix-native.json")).unwrap()
}
fn finish(read: &mut Read, matrix: &mut Matrix, scratch: &mut Scratch, host: &mut Services) -> i32 {
    for _ in 0..1000 {
        if let Some(status) = read.step(matrix, scratch, host).unwrap() {
            return status;
        }
    }
    panic!("authored array read exceeded budget")
}

#[test]
fn original_nine_array_controls_match_status_header_bytes_columns_and_lifetime_order() {
    let fixture = fixture();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 9);
    for (i, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let expected = &row["original"];
        let (mut matrix, mut scratch, mut host) = setup(input);
        let original_source = host.source.clone();
        let mut status = 0;
        let mut read = Read::new(u32::from_be_bytes(*b"Arry"), 3, HEAD_CONVERTER);
        for _ in 0..input["repeat"].as_u64().unwrap_or(1) {
            read = Read::new(u32::from_be_bytes(*b"Arry"), 3, HEAD_CONVERTER);
            status = finish(&mut read, &mut matrix, &mut scratch, &mut host);
        }
        let (body, columns) = match &matrix.allocation {
            Allocation::Known(b) => {
                let first = b.column_offsets[0];
                let count = (matrix.width * matrix.height * matrix.stride) as usize;
                (
                    json!(b.data[first as usize..first as usize + count]),
                    json!(
                        b.column_offsets
                            .iter()
                            .map(|o| o - first)
                            .collect::<Vec<_>>()
                    ),
                )
            }
            Allocation::Null => (Value::Null, Value::Null),
            other => panic!("unexpected final allocation {other:?}"),
        };
        assert_eq!(
            json!({"status":status,"width":matrix.width,"height":matrix.height,
            "stride":matrix.stride,"body":body,"columns_relative_to_data":columns,
            "events":host.events,"converter_calls":0,"converter_scratch":scratch.requested_converter,
            "decoder_calls":if input["parsed"].as_bool().unwrap()||input["missing"].as_bool().unwrap(){0}else{host.lookups},
            "raw_stream_cursor":if input["parsed"].as_bool().unwrap()||input["missing"].as_bool().unwrap(){Value::Null}else{json!(read.source_cursor)},
            "source_unchanged":host.source==original_source}),
            *expected,
            "original array case{i}"
        );
        assert_eq!(host.references, if status == -1 { 2 } else { 1 });
        assert_eq!(read.held_lease, if status == -1 { Some(7) } else { None });
    }
}

#[test]
fn original_three_lighting_controls_preserve_capacity_tail() {
    for row in fixture()["lighting_cases"].as_array().unwrap() {
        let input: Vec<u32> = serde_json::from_value(row["input"].clone()).unwrap();
        let mut data: Vec<[u8; 4]> = input.into_iter().map(u32::to_le_bytes).collect();
        reset_lights(row["used"].as_u64().unwrap() as usize, &mut data).unwrap();
        assert_eq!(
            json!(data.into_iter().map(u32::from_le_bytes).collect::<Vec<_>>()),
            row["output"]
        );
    }
}

#[test]
fn pending_services_resume_cold_without_reacquiring_freeing_or_releasing_twice() {
    let fixture = fixture();
    for service in [
        Service::Lookup,
        Service::Source,
        Service::Free,
        Service::Allocate,
        Service::Release,
    ] {
        let input = &fixture["cases"][2]["input"]; // parsed3x2 into2x2: a real resize
        let (mut matrix, mut scratch, mut host) = setup(input);
        host.block = Some(service);
        let mut read = Read::new(u32::from_be_bytes(*b"Arry"), 3, HEAD_CONVERTER);
        loop {
            match read.step(&mut matrix, &mut scratch, &mut host) {
                Ok(None) => {}
                Err(Stop::Unavailable(_)) => break,
                other => panic!("unexpected{other:?}"),
            }
        }
        assert_eq!(read.status(), None);
        assert_eq!(scratch.requested_converter, HEAD_CONVERTER);
        if service == Service::Allocate {
            assert_eq!(matrix.width, 3);
            assert!(matches!(
                matrix.allocation,
                Allocation::Released { token: 1 }
            ));
            assert_eq!(host.frees, 1)
        }
        if service == Service::Source {
            assert_eq!(host.lookups, 1);
            assert_eq!(read.held_lease, Some(7));
        }
        let cold = serde_json::to_vec(&(read, matrix, scratch, host)).unwrap();
        let (mut read, mut matrix, mut scratch, mut host): (Read, Matrix, Scratch, Services) =
            serde_json::from_slice(&cold).unwrap();
        host.block = None;
        assert_eq!(finish(&mut read, &mut matrix, &mut scratch, &mut host), 0);
        let (mut m, mut s, mut h) = setup(input);
        let mut r = Read::new(u32::from_be_bytes(*b"Arry"), 3, HEAD_CONVERTER);
        assert_eq!(finish(&mut r, &mut m, &mut s, &mut h), 0);
        assert_eq!(
            (read, matrix, scratch, host),
            (r, m, s, h),
            "pending{service:?}"
        );
    }
}

#[test]
fn raw_incompatible_backing_is_a_profile_stop_after_header_writes_not_a_native_error() {
    let mut input = fixture()["cases"][0]["input"].clone();
    input["same_size"] = json!(false);
    let (mut matrix, mut scratch, mut host) = setup(&input);
    let old = matrix.allocation.clone();
    let mut read = Read::new(u32::from_be_bytes(*b"Arry"), 3, HEAD_CONVERTER);
    loop {
        match read.step(&mut matrix, &mut scratch, &mut host) {
            Ok(None) => {}
            Err(Stop::Unsupported(_)) => break,
            other => panic!("unexpected{other:?}"),
        }
    }
    assert_eq!((matrix.width, matrix.height, matrix.stride), (3, 2, 2));
    assert_eq!(matrix.allocation, old);
    assert_eq!(read.status(), None);
    assert_eq!(read.held_lease, Some(7));
    assert_eq!((host.frees, host.allocations, host.releases), (0, 0, 0));
}

#[test]
fn raw_rle_pending_release_and_between_runs_cold_cuts_keep_exact_offsets() {
    let input = fixture()["cases"][1]["input"].clone();
    // Three authored repeat runs: each consumes command2+value1+padding1.
    let (mut matrix, mut scratch, mut host) = setup(&input);
    host.source.truncate(20);
    for value in [0x12, 0x34, 0x56] {
        host.source.extend_from_slice(&[4, 0x80, value, 0xa5]);
    }
    let original_host = host.clone();
    let original_matrix = matrix.clone();
    let mut read = Read::new(u32::from_be_bytes(*b"Arry"), 3, HEAD_CONVERTER);
    while read.source_cursor < 24 {
        read.step(&mut matrix, &mut scratch, &mut host).unwrap();
    }
    let cold = serde_json::to_vec(&(read, matrix, scratch, host)).unwrap();
    let (mut read, mut matrix, mut scratch, mut host): (Read, Matrix, Scratch, Services) =
        serde_json::from_slice(&cold).unwrap();
    host.block = Some(Service::Release);
    loop {
        match read.step(&mut matrix, &mut scratch, &mut host) {
            Ok(None) => {}
            Err(Stop::Unavailable(_)) => break,
            other => panic!("unexpected{other:?}"),
        }
    }
    assert_eq!(read.source_cursor, 32);
    assert_eq!(host.sources, 1);
    host.block = None;
    finish(&mut read, &mut matrix, &mut scratch, &mut host);
    let mut r = Read::new(u32::from_be_bytes(*b"Arry"), 3, HEAD_CONVERTER);
    let mut m = original_matrix;
    let mut h = original_host;
    let mut s = Scratch {
        requested_converter: 0,
    };
    finish(&mut r, &mut m, &mut s, &mut h);
    assert_eq!((read, matrix, scratch, host), (r, m, s, h));
}

#[test]
fn known_missing_resource_does_not_require_or_normalize_unconsumed_backing() {
    let mut case = fixture()["cases"][6]["input"].clone();
    case["missing"] = json!(true);
    let (mut matrix, mut scratch, mut host) = setup(&case);
    matrix.allocation = Allocation::Unavailable;
    let before = matrix.clone();
    let mut read = Read::new(u32::from_be_bytes(*b"Arry"), 3, HEAD_CONVERTER);
    assert_eq!(finish(&mut read, &mut matrix, &mut scratch, &mut host), -98);
    assert_eq!(matrix, before);
    assert_eq!(
        (host.sources, host.frees, host.allocations, host.releases),
        (0, 0, 0, 0)
    );
    assert_eq!(scratch.requested_converter, HEAD_CONVERTER);
}

#[test]
fn unsafe_allocation_extent_is_a_profile_stop_after_native_old_free_and_dimension_writes() {
    let (mut matrix, _, mut host) = setup(&fixture()["cases"][0]["input"]);
    matrix.stride = i32::MAX;
    let mut resize = Resize::new(i32::MAX, i32::MAX);
    loop {
        match resize.step(&mut matrix, &mut host) {
            Ok(None) => {}
            Err(Stop::Unsupported(_)) => break,
            other => panic!("unexpected{other:?}"),
        }
    }
    assert_eq!((matrix.width, matrix.height), (i32::MAX, i32::MAX));
    assert!(matches!(
        matrix.allocation,
        Allocation::Released { token: 1 }
    ));
    assert_eq!((host.frees, host.allocations), (1, 0));
}

#[test]
fn resize_same_dimensions_does_not_repair_null_or_changed_stride_backing() {
    let mut host = setup(&fixture()["cases"][0]["input"]).2;
    let mut null = Matrix {
        width: 3,
        height: 2,
        stride: 2,
        allocation: Allocation::Null,
    };
    let mut resize = Resize::new(3, 2);
    assert_eq!(resize.step(&mut null, &mut host).unwrap(), Some(true));
    let (mut matrix, _, _) = setup(&fixture()["cases"][0]["input"]);
    let old = matrix.allocation.clone();
    matrix.stride = 4;
    let mut resize = Resize::new(3, 2);
    assert_eq!(resize.step(&mut matrix, &mut host).unwrap(), Some(true));
    assert_eq!(matrix.allocation, old);
    assert_eq!((host.frees, host.allocations), (0, 0));
}
