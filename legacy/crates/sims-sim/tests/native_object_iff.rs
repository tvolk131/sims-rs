use serde_json::{Value, json};
use sims_sim::object_catalog::iff::{Iff, Stream};

struct Input {
    bytes: Vec<u8>,
    position: usize,
    opened: bool,
    case: Value,
    events: Vec<Value>,
}
impl Stream for Input {
    fn open(&mut self, path: &[u8], writable: u32) -> Result<i32, String> {
        self.events.push(json!(["open", path, writable]));
        let code = self.case["open_error"].as_i64().unwrap_or(0) as i32;
        self.opened = code == 0;
        Ok(code)
    }
    fn seek(&mut self, offset: u32) -> Result<i32, String> {
        self.events.push(json!(["seek", offset]));
        self.position = offset as usize;
        Ok(if !self.opened {
            -49
        } else {
            self.case["seek_errors"][offset.to_string()]
                .as_i64()
                .unwrap_or(0) as i32
        })
    }
    fn read(&mut self, output: &mut [u8]) -> Result<i32, String> {
        self.events
            .push(json!(["read", self.position, output.len()]));
        let input = self.bytes.get(self.position..).unwrap_or_default();
        let n = input.len().min(output.len());
        output[..n].copy_from_slice(&input[..n]);
        self.position += n;
        Ok(if n == output.len() { 0 } else { -48 })
    }
    fn close(&mut self) -> Result<i32, String> {
        self.events.push(json!(["close"]));
        let code = if self.opened { 0 } else { -49 };
        self.opened = false;
        Ok(code)
    }
}
fn number(value: &Value) -> u32 {
    value.as_u64().unwrap() as u32
}
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::from_value(value.clone()).unwrap()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_iff_open_maps_lazy_queries_and_read_only_close() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/object-iff.json")).unwrap();
    let mut operations = 0;
    for (case_index, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let encoded = row["input"]["bytes"].as_str().unwrap();
        let data = (0..encoded.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16).unwrap())
            .collect();
        let mut stream = Input {
            bytes: data,
            position: 0,
            opened: false,
            case: row["input"].clone(),
            events: vec![],
        };
        let mut iff = Iff::default();
        let mut last = 0;
        for (op_index, op) in row["input"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let value = match op[0].as_str().unwrap() {
                "open" => json!(
                    iff.open(b"./sample.iff", number(&op[1]), &mut stream)
                        .unwrap()
                ),
                "count" => json!(iff.count(number(&op[1]))),
                "ordinal" | "by_id" => {
                    let callback = op.get(3).is_none_or(|v| number(v) != 0);
                    last = if op[0] == "ordinal" {
                        iff.ordinal(number(&op[1]), number(&op[2]), callback, &mut stream)
                    } else {
                        iff.by_id(number(&op[1]), number(&op[2]), callback, &mut stream)
                    }
                    .unwrap();
                    json!(iff.payload(last))
                }
                "name" => {
                    let handle = if op[1] == "last" {
                        last
                    } else {
                        number(&op[1])
                    };
                    let mut output = bytes(&op[2]);
                    iff.name(handle, &mut output);
                    json!(output)
                }
                "id" => {
                    let handle = if op[1] == "last" {
                        last
                    } else {
                        number(&op[1])
                    };
                    let mut output = number(&op[2]);
                    iff.id(handle, &mut output);
                    json!(output)
                }
                "release_chunk" => {
                    let handle = if op[1] == "last" {
                        last
                    } else {
                        number(&op[1])
                    };
                    iff.release_chunk(handle).unwrap();
                    Value::Null
                }
                "close" => json!(iff.close(&mut stream).unwrap()),
                _ => panic!("unknown IFF operation"),
            };
            let records = iff
                .records()
                .map(|r| {
                    json!({"tag":r.tag,"id":r.id,"flags":r.flags,
                "disk_flags":r.disk_flags,"ordinal":r.ordinal,"offset":r.offset,"name":r.name,
                "payload":r.payload.as_ref().map(|p| &p.bytes)})
                })
                .collect::<Vec<_>>();
            let state = json!({"value":value,"error":iff.error,"valid":iff.valid,"map_offset":iff.map_offset,"unused":iff.unused,"records":records});
            assert_eq!(
                state, row["original"]["results"][op_index],
                "case {case_index}, operation {op_index}: {op}"
            );
            operations += 1;
        }
        assert_eq!(
            json!(stream.events),
            row["original"]["events"],
            "case {case_index} stream calls"
        );
    }
    println!(
        "{} native IFF histories; {operations} operations",
        fixture["cases"].as_array().unwrap().len()
    );
}
