use serde_json::{Value, json};
use sims_vm::scheduler::clock::draw::actions::{
    Record,
    record::{self, Build, Copy, Images, Strings},
};
use std::collections::BTreeMap;
const NODE: u32 = 0x10000400;
const PERSON: u32 = 0x10000600;
const OBJECT: u32 = 0x10000c00;
const RESOURCE: u32 = 0x10000e00;
const MASTER: u32 = 0x10001200;
const DESCRIPTOR: u32 = 0x10001400;
const IMAGES: u32 = 0x10002000;
fn image(i: u32) -> u32 {
    if i == 0 { 0 } else { IMAGES + (i - 1) * 256 }
}
fn id(p: u32) -> usize {
    if p == 0 {
        0
    } else {
        ((p - IMAGES) / 256 + 1) as usize
    }
}
fn bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}
fn number(v: &Value) -> u32 {
    v.as_u64().unwrap() as u32
}
fn set(r: &mut Record, off: usize, v: u32) {
    r[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
struct Host<'a> {
    c: &'a Value,
    events: Vec<Value>,
    refs: [u32; 5],
    allocations: BTreeMap<u32, Vec<u8>>,
    next: u32,
    flags: u32,
}
impl Host<'_> {
    fn put(&mut self, n: u32) -> u32 {
        let p = self.next;
        self.next += 0x80;
        self.allocations.insert(p, vec![0xab; n as usize]);
        p
    }
    fn decrement(&mut self, image: u32) {
        let i = id(image);
        self.refs[i - 1] = self.refs[i - 1].wrapping_sub(1);
        if self.refs[i - 1] == 0 {
            self.events.push(json!({"Destroy":{"image":i}}));
        }
    }
}
impl Strings for Host<'_> {
    fn allocate(&mut self, n: u32) -> Result<u32, String> {
        self.events.push(json!({"Allocate":{"bytes":n}}));
        Ok(self.put(n))
    }
    fn write_nul(&mut self, p: u32) -> Result<(), String> {
        self.events.push(json!({"Nul":{"pointer":p}}));
        self.allocations.get_mut(&p).unwrap()[0] = 0;
        Ok(())
    }
}
impl Images for Host<'_> {
    fn assign(&mut self, slot: &mut u32, incoming: u32) -> Result<(), String> {
        self.events
            .push(json!({"Assign":{"old":id(*slot),"image":id(incoming)}}));
        if incoming != 0 {
            self.refs[id(incoming) - 1] += 1;
        }
        if *slot != 0 {
            self.decrement(*slot);
        }
        *slot = incoming;
        Ok(())
    }
    fn release(&mut self, p: u32) -> Result<(), String> {
        self.events.push(json!({"Release":{"image":id(p)}}));
        self.decrement(p);
        Ok(())
    }
}
impl Copy for Host<'_> {
    fn action_label(&mut self, node: u32) -> Result<Vec<u8>, String> {
        self.events.push(json!({"Label":{"node":node}}));
        self.flags &= !128;
        Ok(bytes(self.c["label"].as_str().unwrap()))
    }
    fn caption(&mut self, h: &mut [u32; 3], label: &[u8]) -> Result<(), String> {
        self.events
            .push(json!({"Caption":{"header":h,"bytes":hex(label)}}));
        if label.len() + 1 > (h[2] - h[0]) as usize {
            self.allocations.remove(&h[0]);
            h[0] = self.put(label.len() as u32 + 1);
            h[2] = h[0] + label.len() as u32 + 1;
        }
        h[1] = h[0] + label.len() as u32;
        let b = self.allocations.get_mut(&h[0]).unwrap();
        b[..label.len()].copy_from_slice(label);
        b[label.len()] = 0;
        Ok(())
    }
}
impl Build for Host<'_> {
    fn icon_owner(&self, node: u32) -> Result<u32, String> {
        assert_eq!(node, NODE);
        Ok(if self.c["object"].as_bool().unwrap() {
            OBJECT
        } else {
            0
        })
    }
    fn kind(&self, o: u32) -> Result<u16, String> {
        assert_eq!(o, OBJECT);
        Ok(number(&self.c["kind"]) as u16)
    }
    fn resource(&self, o: u32) -> Result<u32, String> {
        assert_eq!(o, OBJECT);
        Ok(RESOURCE)
    }
    fn guid(&self, r: u32) -> Result<u32, String> {
        assert_eq!(r, RESOURCE);
        Ok(number(&self.c["guid"]))
    }
    fn cache_index(&self, g: u32) -> i32 {
        self.c["cache"]
            .as_array()
            .unwrap()
            .iter()
            .position(|v| number(v) == g)
            .map_or(-1, |i| i as i32)
    }
    fn portrait(&mut self, index: i32, temporary: &mut u32) -> Result<bool, String> {
        self.events.push(json!({"Portrait":{"index":index}}));
        *temporary = image(number(&self.c["portrait_image"]));
        if *temporary != 0 {
            self.refs[id(*temporary) - 1] += 1;
        }
        Ok(self.c["portrait_success"].as_bool().unwrap())
    }
    fn master(&mut self, r: u32) -> Result<Option<u32>, String> {
        self.events.push(json!({"Master":{"resource":r}}));
        Ok(self.c["master"].as_bool().unwrap().then_some(MASTER))
    }
    fn descriptor(&self, m: u32) -> Result<Option<u32>, String> {
        assert_eq!(m, MASTER);
        Ok(self.c["descriptor"]
            .as_bool()
            .unwrap()
            .then_some(DESCRIPTOR))
    }
    fn thumbnail(&mut self, d: u32, temporary: &mut u32) -> Result<bool, String> {
        self.events.push(json!({"Thumbnail":{"descriptor":d}}));
        *temporary = image(number(&self.c["thumbnail_image"]));
        if *temporary != 0 {
            self.refs[id(*temporary) - 1] += 1;
        }
        Ok(self.c["thumbnail_success"].as_bool().unwrap())
    }
    fn active_counter(&self, p: u32) -> Result<u16, String> {
        assert_eq!(p, PERSON);
        Ok(number(&self.c["counter"]) as u16)
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_record_constructor_copy_and_image_builder() {
    let data: Value =
        serde_json::from_str(include_str!("fixtures/native-action-record-build.json")).unwrap();
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 352);
    for (n, row) in cases.iter().enumerate() {
        let c = &row["input"];
        let mut raw: Record = bytes(c["initial"].as_str().unwrap()).try_into().unwrap();
        let initial_image = record::word(&raw, 4);
        set(&mut raw, 4, image(initial_image));
        let mut source: Record = bytes(c["source"].as_str().unwrap()).try_into().unwrap();
        let source_image = number(&c["source_image"]);
        set(&mut source, 4, image(source_image));
        set(&mut source, 12, NODE);
        let mut host = Host {
            c,
            events: vec![],
            refs: [1; 5],
            allocations: BTreeMap::new(),
            next: 0x10004000,
            flags: 128,
        };
        for i in [initial_image, source_image] {
            if i != 0 {
                host.refs[i as usize - 1] += 1;
            }
        }
        let returned = match c["mode"].as_str().unwrap() {
            "construct" => {
                record::construct(&mut raw, &mut host).unwrap();
                None
            }
            "copy" => {
                record::copy(&mut raw, &source, &mut host).unwrap();
                None
            }
            "build" => Some(
                record::build(
                    &mut raw,
                    326,
                    PERSON,
                    NODE,
                    number(&c["current"]) as u8,
                    &mut host,
                )
                .unwrap(),
            ),
            _ => unreachable!(),
        };
        let actual = json!({"events":host.events,"record":hex(&raw),"refs":host.refs,"allocations":host.allocations.iter().map(|(p,b)|(p.to_string(),hex(b))).collect::<BTreeMap<_,_>>(),"action_flags":host.flags,"returned":returned});
        let mut expected = row["expected"].clone();
        let written = expected
            .as_object_mut()
            .unwrap()
            .remove("written_bytes")
            .unwrap();
        if c["mode"] == "construct" {
            assert_eq!(
                number(&written),
                record::CONSTRUCTED_BYTES,
                "native initialized-byte mask {n}"
            );
        }
        assert_eq!(actual, expected, "case {n}");
    }
}
