use serde_json::{Value, json};
use sims_sim::object_catalog::iff::{Add, Iff, Stream, WriteStream};

struct Memory {
    bytes: Vec<u8>,
    position: usize,
    opened: bool,
    writable: bool,
    writes: usize,
    flushes: usize,
    case: Value,
    events: Vec<Value>,
}
impl Stream for Memory {
    fn open(&mut self, path: &[u8], request: u32) -> Result<i32, String> {
        self.events.push(json!(["open", path, request]));
        self.opened = true;
        Ok(0)
    }
    fn seek(&mut self, position: u32) -> Result<i32, String> {
        self.events.push(json!(["seek", position]));
        self.position = position as usize;
        Ok(if self.opened { 0 } else { -49 })
    }
    fn read(&mut self, output: &mut [u8]) -> Result<i32, String> {
        self.events
            .push(json!(["read", self.position, output.len()]));
        let available = self.bytes.get(self.position..).unwrap_or_default();
        let n = available.len().min(output.len());
        output[..n].copy_from_slice(&available[..n]);
        self.position += n;
        Ok(if n == output.len() { 0 } else { -48 })
    }
    fn close(&mut self) -> Result<i32, String> {
        unreachable!()
    }
}
impl WriteStream for Memory {
    fn writable(&self) -> bool {
        self.writable
    }
    fn write(&mut self, bytes: &[u8]) -> Result<i32, String> {
        self.writes += 1;
        let code = self.case["write_errors"][self.writes.to_string()]
            .as_i64()
            .unwrap_or(0) as i32;
        self.events
            .push(json!(["write", self.position, bytes, code]));
        if code == 0 {
            let end = self.position + bytes.len();
            self.bytes.resize(self.bytes.len().max(end), 0);
            self.bytes[self.position..end].copy_from_slice(bytes);
            self.position = end;
        }
        Ok(code)
    }
    fn position(&mut self) -> Result<Result<u32, i32>, String> {
        self.events.push(json!(["position", self.position]));
        Ok(Ok(self.position as u32))
    }
    fn flush(&mut self) -> Result<i32, String> {
        self.flushes += 1;
        let code = self.case["flush_errors"][self.flushes.to_string()]
            .as_i64()
            .unwrap_or(0) as i32;
        self.events.push(json!(["flush", code]));
        Ok(code)
    }
}
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::from_value(value.clone()).unwrap()
}
fn unhex(value: &Value) -> Vec<u8> {
    let s = value.as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn disk(mut bytes: Vec<u8>) -> Vec<u8> {
    let mut offset = 64;
    while offset + 76 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        if size < 76 {
            break;
        }
        let flags = u16::from_be_bytes(bytes[offset + 10..offset + 12].try_into().unwrap());
        if flags & 4 != 0 {
            bytes[offset + 8..offset + 10].fill(0);
            let end = bytes.len().min(offset + size);
            bytes[offset + 12..end].fill(0);
        }
        offset += size;
    }
    bytes
}
fn events(values: Vec<Value>) -> Vec<Value> {
    values
        .into_iter()
        .map(|mut v| {
            if v[0] == "write" {
                let mut b = bytes(&v[2]);
                if b.len() == 76 && &b[..4] == b"XXXX" {
                    b[8..10].fill(0);
                    b[12..].fill(0);
                    v[2] = json!(b);
                }
            }
            v
        })
        .collect()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_iff_writeback_matches_metadata_active_bytes_and_io_order() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/iff-write.json")).unwrap();
    check(&fixture, 101, 543);
}
fn check(fixture: &Value, histories: usize, operations: usize) {
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), histories);
    let mut total = 0;
    for (n, case) in cases.iter().enumerate() {
        let input = &case["input"];
        let mut io = Memory {
            bytes: unhex(&input["bytes"]),
            position: 0,
            opened: false,
            writable: input["writable"].as_bool().unwrap_or(true),
            writes: 0,
            flushes: 0,
            case: input.clone(),
            events: vec![],
        };
        let mut iff = Iff::default();
        let mut last = 0;
        if input["opened"].as_bool().unwrap_or(true) {
            assert_eq!(iff.open(b"./sample.iff", 1, &mut io).unwrap(), 0);
        }
        for (step, op) in input["operations"].as_array().unwrap().iter().enumerate() {
            let num = |i: usize| op[i].as_u64().unwrap() as u32;
            let kind = op[0].as_str().unwrap();
            match kind {
                "add" | "add_last" | "add_null" => {
                    let payload = match kind {
                        "add" => {
                            let p = iff.allocate_payload(bytes(&op[6])).unwrap();
                            last = p.handle;
                            Some(p)
                        }
                        "add_last" => {
                            iff.payload(last)
                                .map(|b| sims_sim::object_catalog::iff::Payload {
                                    handle: last,
                                    bytes: b.to_vec(),
                                })
                        }
                        _ => {
                            last = 0;
                            None
                        }
                    };
                    iff.add(
                        Add {
                            payload,
                            tag: num(1),
                            id: num(2),
                            variant: num(3),
                            converted: num(4),
                            name: &bytes(&op[5]),
                        },
                        &mut io,
                    )
                    .unwrap();
                }
                "by_id" => last = iff.by_id(num(1), num(2), false, &mut io).unwrap(),
                "delete_found" => {
                    if last != 0 {
                        iff.delete(last, &mut io).unwrap();
                    }
                }
                "flush" => iff.flush(&mut io).unwrap(),
                "write" | "delete" | "detach" | "release" => {
                    let handle = if kind == "release" || op[1] == "last" {
                        last
                    } else {
                        num(1)
                    };
                    match kind {
                        "write" => iff.write_chunk(handle, &mut io),
                        "delete" => iff.delete(handle, &mut io),
                        _ => iff.detach(handle, &mut io),
                    }
                    .unwrap();
                }
                _ => panic!("unknown operation"),
            }
            let records=iff.records().map(|r|json!({"tag":r.tag,"id":r.id,"flags":r.flags,"disk_flags":r.disk_flags,"ordinal":r.ordinal,"offset":r.offset,"name":r.name,"payload":r.payload.as_ref().map(|p|&p.bytes)})).collect::<Vec<_>>();
            let actual = json!({"error":iff.error,"valid":iff.valid,"map_offset":iff.map_offset,"cursor":iff.write_cursor,"modified":iff.modified,"unused":iff.unused,"records":records});
            let mut expected = case["original"]["results"][step].clone();
            let original = unhex(&expected["bytes"]);
            expected.as_object_mut().unwrap().remove("bytes");
            assert_eq!(actual, expected, "case {n} step {step} {op}");
            assert_eq!(
                disk(io.bytes.clone()),
                disk(original),
                "disk case {n} step {step} {op}"
            );
            total += 1;
        }
        assert_eq!(
            events(io.events),
            events(serde_json::from_value(case["original"]["events"].clone()).unwrap()),
            "I/O case {n}"
        );
    }
    assert_eq!(total, operations);
}

#[test]
#[ignore = "requires native write of the original Mashuga character and completed persistent portrait job"]
fn original_character_write_matches_native_and_concrete_restored_overlay() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/iff-write-source.json")).unwrap(),
    )
    .unwrap();
    check(&fixture, 1, 6);
    let original = unhex(&fixture["cases"][0]["original"]["results"][5]["bytes"]);
    let overlay: Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/original-portrait-persist-job-overlay.json")).unwrap(),
    )
    .unwrap();
    let actual = bytes(&overlay["entries"]["UserData2/Characters/User00000.iff"]["bytes"]);
    assert_eq!(
        disk(actual.clone()),
        disk(original.clone()),
        "production overlay versus original writer"
    );
    println!(
        "original/overlay bytes: {} / {}; raw bytes identical: {}",
        original.len(),
        actual.len(),
        original == actual
    );
}
