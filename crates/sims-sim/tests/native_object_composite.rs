use serde::Deserialize;
use serde_json::{Value, json};
use sims_sim::object_catalog::composite::{Chunk, Composite, Filter, Host, Reply};

#[derive(Default, Clone, Deserialize)]
#[serde(default)]
struct Row {
    open: i32,
    close: i32,
    filename: Option<Vec<u8>>,
    valid: u8,
    count: u32,
    count_error: i32,
    by_id: u32,
    by_id_error: i32,
    ordinal: u32,
    ordinal_error: i32,
    name: Option<Vec<u8>>,
    name_error: i32,
    id: Option<u32>,
    id_error: i32,
    release_error: i32,
    hits: Vec<u32>,
    flush_error: i32,
    write_error: i32,
    delete_error: i32,
    add_error: i32,
}
#[derive(Deserialize)]
struct Input {
    resources: Vec<Row>,
    slots: Vec<u32>,
    created: Vec<Row>,
    configs: Vec<(i32, Vec<u8>)>,
    operations: Vec<Vec<Value>>,
}
struct Services<'a> {
    rows: Vec<Row>,
    created: &'a [Row],
    constructed: usize,
    events: Vec<Value>,
}
impl Host for Services<'_> {
    fn construct(&mut self, kind: u8) -> Result<u32, String> {
        self.rows
            .push(self.created[self.constructed % self.created.len()].clone());
        self.constructed += 1;
        let id = self.rows.len() as u32;
        self.events.push(json!(["construct", id, kind]));
        Ok(id)
    }
    fn open(&mut self, resource: u32, path: &[u8], writable: u32) -> Result<i32, String> {
        let value = self.rows[resource as usize - 1].open;
        self.events
            .push(json!(["open", resource, path, writable, value]));
        Ok(value)
    }
    fn valid(&mut self, resource: u32) -> Result<bool, String> {
        self.events.push(json!(["valid", resource]));
        Ok(self.rows[resource as usize - 1].valid != 0)
    }
    fn writable(&mut self, resource: u32) -> Result<bool, String> {
        self.events.push(json!(["component", resource - 1]));
        Ok(self.rows[resource as usize - 1].valid != 0)
    }
    fn flush(&mut self, resource: u32) -> Result<i32, String> {
        self.events.push(json!(["flush", resource]));
        Ok(self.rows[resource as usize - 1].flush_error)
    }
    fn write(&mut self, resource: u32, handle: u32) -> Result<i32, String> {
        self.events.push(json!(["write", resource, handle]));
        Ok(self.rows[resource as usize - 1].write_error)
    }
    fn delete(&mut self, resource: u32, handle: u32) -> Result<i32, String> {
        self.events.push(json!(["delete", resource, handle]));
        Ok(self.rows[resource as usize - 1].delete_error)
    }
    fn add(&mut self, resource: u32, chunk: Chunk<'_>) -> Result<i32, String> {
        self.events.push(json!([
            "add",
            resource,
            chunk.handle,
            chunk.tag,
            chunk.id,
            chunk.name,
            chunk.variant,
            chunk.converted
        ]));
        Ok(self.rows[resource as usize - 1].add_error)
    }
    fn filename(&mut self, resource: u32, output: &mut Vec<u8>) -> Result<(), String> {
        self.events.push(json!(["filename", resource]));
        if let Some(value) = &self.rows[resource as usize - 1].filename {
            *output = value.clone();
        }
        Ok(())
    }
    fn count(&mut self, resource: u32, tag: u32) -> Result<Reply, String> {
        self.events.push(json!(["count", resource, tag]));
        let row = &self.rows[resource as usize - 1];
        Ok(Reply {
            value: row.count,
            error: row.count_error,
        })
    }
    fn by_id(
        &mut self,
        resource: u32,
        tag: u32,
        id: u32,
        options: u32,
        callback: u32,
    ) -> Result<Reply, String> {
        self.events
            .push(json!(["by_id", resource, tag, id, options, callback]));
        let row = &mut self.rows[resource as usize - 1];
        if !row.hits.is_empty() {
            row.by_id = row.hits.remove(0);
        }
        Ok(Reply {
            value: row.by_id,
            error: row.by_id_error,
        })
    }
    fn ordinal(
        &mut self,
        resource: u32,
        tag: u32,
        ordinal: u32,
        callback: u32,
    ) -> Result<Reply, String> {
        self.events
            .push(json!(["ordinal", resource, tag, ordinal, callback]));
        let row = &self.rows[resource as usize - 1];
        Ok(Reply {
            value: row.ordinal,
            error: row.ordinal_error,
        })
    }
    fn name(&mut self, resource: u32, handle: u32, output: &mut Vec<u8>) -> Result<i32, String> {
        self.events.push(json!(["name", resource, handle]));
        let row = &self.rows[resource as usize - 1];
        if let Some(value) = &row.name {
            *output = value.clone();
        }
        Ok(row.name_error)
    }
    fn id(&mut self, resource: u32, handle: u32, output: &mut u32) -> Result<i32, String> {
        self.events.push(json!(["id", resource, handle]));
        let row = &self.rows[resource as usize - 1];
        if let Some(value) = row.id {
            *output = value;
        }
        Ok(row.id_error)
    }
    fn release_chunk(&mut self, resource: u32, handle: u32) -> Result<i32, String> {
        self.events.push(json!(["release_chunk", resource, handle]));
        Ok(self.rows[resource as usize - 1].release_error)
    }
    fn close(&mut self, resource: u32) -> Result<i32, String> {
        let value = self.rows[resource as usize - 1].close;
        self.events.push(json!(["close", resource, value]));
        Ok(value)
    }
    fn destroy(&mut self, resource: u32) -> Result<(), String> {
        self.events.push(json!(["destroy", resource]));
        Ok(())
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_write_dispatch_filters_alias_replacement_and_errors_match() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/composite-write.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 84);
    for (n, case) in cases.iter().enumerate() {
        let input: Input = serde_json::from_value(case["input"].clone()).unwrap();
        let mut composite = Composite::empty();
        composite.error = -77;
        for (i, id) in input.slots.into_iter().enumerate() {
            composite.slots[i] = (id != 0).then_some(id);
        }
        for (i, f) in case["input"]["filters"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            composite.filters[i] = Filter {
                tags: serde_json::from_value(f["tags"].clone()).unwrap(),
                exclude: f["exclude"].as_u64().unwrap() as u16,
            };
        }
        let mut host = Services {
            rows: input.resources,
            created: &[],
            constructed: 0,
            events: vec![],
        };
        let mut results = vec![];
        for op in input.operations {
            let handle = op.get(1).and_then(Value::as_u64).unwrap_or(0) as u32;
            match op[0].as_str().unwrap() {
                "flush" => composite.flush(&mut host),
                "delete" => composite.delete(handle, &mut host),
                "write" => composite.write(handle, &mut host),
                "add" => composite.add(
                    Chunk {
                        handle,
                        tag: 0x424d505f,
                        id: 2002,
                        name: b"faces",
                        variant: 0,
                        converted: 1,
                    },
                    &mut host,
                ),
                _ => unreachable!(),
            }
            .unwrap();
            results.push(
                json!({"error":composite.error,"slots":composite.slots.map(|s|s.unwrap_or(0))}),
            );
        }
        assert_eq!(
            json!({"events":host.events,"results":results}),
            case["original"],
            "case {n}"
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_writable_query_stops_at_first_empty_or_capable_component() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/resource-access.json")).unwrap();
    let cases = report["queries"].as_array().unwrap();
    assert_eq!(cases.len(), 136);
    for (i, case) in cases.iter().enumerate() {
        let input: [Option<i16>; 8] = serde_json::from_value(case["input"].clone()).unwrap();
        let mut composite = Composite::empty();
        composite.error = -1;
        composite.slots = std::array::from_fn(|i| input[i].map(|_| i as u32 + 1));
        let mut host = Services {
            rows: input
                .into_iter()
                .map(|value| Row {
                    valid: value.unwrap_or(0).max(0) as u8,
                    ..Default::default()
                })
                .collect(),
            created: &[],
            constructed: 0,
            events: vec![],
        };
        let returned = composite.writable(&mut host).unwrap();
        assert_eq!(
            json!({"returned":returned,"error":composite.error,"events":host.events}),
            case["original"],
            "case {i}"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_composite_open_queries_errors_and_destruction_match() {
    #[derive(Deserialize)]
    struct Case {
        input: Input,
        original: Value,
    }
    #[derive(Deserialize)]
    struct Report {
        cases: Vec<Case>,
    }
    let report: Report =
        serde_json::from_str(include_str!("fixtures/object-composite.json")).unwrap();
    assert_eq!(report.cases.len(), 227);
    let mut operations = 0;
    for (i, case) in report.cases.into_iter().enumerate() {
        let input = case.input;
        let mut composite = Composite::empty();
        composite.error = -77;
        for (kind, extension) in &input.configs {
            composite.configure(*kind, extension);
        }
        for (slot, &id) in composite.slots.iter_mut().zip(&input.slots) {
            *slot = (id != 0).then_some(id);
        }
        let mut host = Services {
            rows: input.resources,
            created: &input.created,
            constructed: 0,
            events: vec![],
        };
        let mut results = vec![];
        for (step, op) in input.operations.iter().enumerate() {
            operations += 1;
            let n = |i: usize| op[i].as_u64().unwrap() as u32;
            let b = |i: usize| serde_json::from_value::<Vec<u8>>(op[i].clone()).unwrap();
            let value = match op[0].as_str().unwrap() {
                "open" => json!(composite.open(&b(1), n(2), &mut host).unwrap()),
                "release_chunk" => {
                    composite
                        .release_chunk(op[1].as_u64().unwrap() as u32, &mut host)
                        .unwrap();
                    Value::Null
                }
                "valid" => json!(composite.valid(&mut host).unwrap()),
                "filename" => {
                    let mut out = b(1);
                    composite.filename(&mut out, &mut host).unwrap();
                    json!(out)
                }
                "count" => json!(composite.count(n(1), &mut host).unwrap()),
                "by_id" => json!(composite.by_id(n(1), n(2), n(3), n(4), &mut host).unwrap()),
                "ordinal" => json!(composite.ordinal(n(1), n(2), n(3), &mut host).unwrap()),
                "name" => {
                    let mut out = b(2);
                    composite.name(n(1), &mut out, &mut host).unwrap();
                    json!(out)
                }
                "id" => {
                    let mut out = n(2);
                    composite.id(n(1), &mut out, &mut host).unwrap();
                    json!(out)
                }
                "destroy" => {
                    composite.destroy(&mut host).unwrap();
                    Value::Null
                }
                "close" => json!(composite.close(&mut host).unwrap()),
                _ => unreachable!(),
            };
            let result = json!({"value":value,"slots":composite.slots.map(|id|id.unwrap_or(0)),"error":composite.error});
            assert_eq!(
                result, case.original["results"][step],
                "case {i} step {step}"
            );
            results.push(result);
        }
        assert_eq!(
            json!({"events":host.events,"results":results}),
            case.original,
            "case {i} callbacks"
        );
    }
    assert_eq!(operations, 944);
}
