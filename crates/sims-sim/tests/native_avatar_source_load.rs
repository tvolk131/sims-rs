use serde_json::{Value, json};
use sims_sim::pose::clothing::{
    catalog_files::Resource,
    catalog_index::Lazy,
    catalog_load::{self, Format, Host},
    resource_name::Name,
};

struct Services {
    input: Value,
    flags: u32,
    events: Vec<Value>,
}
struct Stream {
    source: usize,
    mode: u32,
}
fn bytes(v: &Value) -> Vec<u8> {
    serde_json::from_value(v.clone()).unwrap()
}
impl Host for Services {
    type Source = usize;
    type Stream = Stream;
    fn enumerate(&mut self, kind: u32, context: u32) -> Result<Vec<Resource<usize>>, String> {
        self.events.push(json!(["enumerate", kind, context]));
        Ok(self.input["sources"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(source, r)| {
                let mut kind = [0; 4];
                kind[..3].copy_from_slice(r["type"].as_str().unwrap().as_bytes());
                Resource {
                    source,
                    kind: u32::from_be_bytes(kind),
                    directory: bytes(&r["directory"]),
                    name: Name {
                        identity: 0x80000000,
                        bytes: bytes(&r["stem"]),
                    },
                }
            })
            .collect())
    }
    fn flags(&self) -> u32 {
        self.flags
    }
    fn set_flags(&mut self, flags: u32) -> Result<(), String> {
        self.flags = flags;
        self.events.push(json!(["flags", flags]));
        Ok(())
    }
    fn create(&mut self, resource: &Resource<usize>) -> Result<Stream, String> {
        self.events.push(json!(["create", resource.source]));
        Ok(Stream {
            source: resource.source,
            mode: 0,
        })
    }
    fn binary(&mut self, stream: &mut Stream) -> Result<(), String> {
        stream.mode = 1;
        self.events.push(json!(["mode", stream.source, 1]));
        Ok(())
    }
    fn open(&mut self, stream: &mut Stream, _: &Resource<usize>) -> Result<bool, String> {
        let opened = self.input["sources"][stream.source]["open"]
            .as_bool()
            .unwrap();
        self.events.push(json!([
            "open",
            stream.source,
            stream.mode,
            self.flags,
            opened
        ]));
        Ok(opened)
    }
    fn load(
        &mut self,
        stream: &mut Stream,
        format: Format,
        location: &Lazy,
    ) -> Result<u32, String> {
        let status = self.input["sources"][stream.source]["status"]
            .as_u64()
            .unwrap() as u32;
        self.events.push(json!([
            "load",
            stream.source,
            match format {
                Format::Text => "cmx",
                Format::Binary => "bcf",
                Format::Index => "ndx",
            },
            location.directory,
            location.catalog,
            location.context,
            status
        ]));
        Ok(status)
    }
    fn close(&mut self, stream: Stream) -> Result<(), String> {
        self.events.push(json!(["close", stream.source]));
        Ok(())
    }
    fn finish(&mut self) -> Result<(), String> {
        self.events.push(json!(["finish"]));
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_startup_preserves_source_order_modes_context_and_failure_continuation() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/avatar-source-load.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 46);
    for (i, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let mut host = Services {
            input: input.clone(),
            flags: input["flags"].as_u64().unwrap() as u32,
            events: vec![],
        };
        catalog_load::run(
            input["indexed"].as_bool().unwrap(),
            input["context"].as_u64().unwrap() as u32,
            &mut host,
        )
        .unwrap();
        assert_eq!(
            json!({"returned":0,"flags":host.flags,"events":host.events}),
            row["original"],
            "case {i}"
        );
    }
}
