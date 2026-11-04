use serde_json::{Value, json};
use sims_vm::scheduler::clock::draw::actions::record::{self, Caption, Strings, Text};
use std::collections::BTreeMap;
const DATA: u32 = 0x10001000;
fn bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect()
}
struct Host {
    header: [u32; 3],
    next: u32,
    allocations: BTreeMap<u32, Vec<u8>>,
    events: Vec<Value>,
}
impl Host {
    fn state(&self) -> Value {
        json!({"header":self.header,"allocations":self.allocations.iter().map(|(p,b)|(p.to_string(),hex(b))).collect::<BTreeMap<_,_>>()})
    }
    fn allocation(&self, p: u32, n: usize) -> Result<(u32, usize), String> {
        self.allocations
            .range(..=p)
            .next_back()
            .and_then(|(&begin, b)| {
                let i = (p - begin) as usize;
                (i <= b.len() && n <= b.len() - i).then_some((begin, i))
            })
            .ok_or_else(|| "caption pointer outside owned bytes".into())
    }
}
impl Strings for Host {
    fn allocate(&mut self, n: u32) -> Result<u32, String> {
        let p = self.next;
        self.next += 0x400;
        self.events
            .push(json!({"Allocate":{"pointer":p,"bytes":n,"state":self.state()}}));
        self.allocations.insert(p, vec![0xab; n as usize]);
        Ok(p)
    }
    fn write_nul(&mut self, p: u32) -> Result<(), String> {
        self.write(p, &[0])
    }
}
impl Caption for Host {
    fn read(&self, p: u32, n: u32) -> Result<Vec<u8>, String> {
        let (begin, i) = self.allocation(p, n as usize)?;
        Ok(self.allocations[&begin][i..i + n as usize].to_vec())
    }
    fn write(&mut self, p: u32, bytes: &[u8]) -> Result<(), String> {
        let (begin, i) = self.allocation(p, bytes.len())?;
        self.allocations.get_mut(&begin).unwrap()[i..i + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
    fn free(&mut self, p: u32, n: u32) -> Result<(), String> {
        assert_eq!(self.allocations[&p].len(), n as usize);
        self.events
            .push(json!({"Free":{"pointer":p,"bytes":n,"state":self.state()}}));
        self.allocations.remove(&p);
        Ok(())
    }
    fn header(&mut self, h: [u32; 3]) {
        self.header = h;
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_caption_growth_shrink_and_allocation_boundaries() {
    let v: Value =
        serde_json::from_str(include_str!("fixtures/native-caption-assignment.json")).unwrap();
    assert_eq!(
        v["source_sha256"],
        "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321"
    );
    assert_eq!(v["cases"].as_array().unwrap().len(), 231);
    for (i, row) in v["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let text = bytes(c["text"].as_str().unwrap());
        let source = bytes(c["source"].as_str().unwrap());
        let cap = c["capacity"].as_u64().unwrap() as u32;
        let mut allocation = text.clone();
        allocation.push(0);
        allocation.resize(cap as usize, 0xa5);
        let mut header = [DATA, DATA + text.len() as u32, DATA + cap];
        let mut host = Host {
            header,
            next: 0x10003000,
            allocations: BTreeMap::from([(DATA, allocation)]),
            events: vec![],
        };
        let source = if c["self"] == true {
            Text::Same
        } else {
            Text::Bytes(&source)
        };
        record::assign_caption(&mut header, source, &mut host).unwrap();
        assert_eq!(host.header, header);
        let mut actual = host.state();
        actual["events"] = json!(host.events);
        assert_eq!(actual, row["expected"], "case {i}");
    }
}
