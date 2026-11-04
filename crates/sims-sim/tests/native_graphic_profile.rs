use serde_json::{Value, json};
use sims_sim::pose::graphic_profile::*;

struct Host {
    length: i32,
    loaded: u32,
    written: u32,
    reads: Vec<Metadata>,
    counts: Vec<i32>,
    appends: Vec<i32>,
    writes: Vec<Value>,
}
impl Strings for Host {
    fn read(&mut self, metadata: &Metadata) -> Result<u32, String> {
        self.reads.push(metadata.clone());
        Ok(self.loaded)
    }
    fn count(&mut self) -> Result<i32, String> {
        self.counts.push(self.length);
        Ok(self.length)
    }
    fn append_empty(&mut self, index: i32) -> Result<(), String> {
        assert_eq!(index, self.length + 1);
        self.appends.push(index);
        self.length += 1;
        Ok(())
    }
    fn write(&mut self, file: u64, id: u16) -> Result<u32, String> {
        self.writes.push(json!({"file":file,"id":id}));
        Ok(self.written)
    }
}
fn mutate(state: &mut State, case: &Value, event: &str) {
    for mutation in case["mutations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["after"] == event)
    {
        let mut value = serde_json::to_value(&state).unwrap();
        for (key, v) in mutation["state"].as_object().unwrap() {
            value[key] = v.clone();
        }
        *state = serde_json::from_value(value).unwrap();
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn ordinary_profile_cache_and_load_match_original_including_padding_and_writeback() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-profile.json")).unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 169);
    for (case, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut host = Host {
            length: c["length"].as_i64().unwrap() as i32,
            loaded: c["loaded"].as_u64().unwrap() as u32,
            written: c["write_result"].as_u64().unwrap() as u32,
            reads: vec![],
            counts: vec![],
            appends: vec![],
            writes: vec![],
        };
        let mut metadata = Metadata::default();
        let mut constructed = Value::Null;
        let mut events = vec![];
        let mut failed = false;
        let result;
        if c["entry"] == "Load" {
            result = json!(load(&mut metadata, state.file, state.id, &mut host).unwrap());
        } else {
            let mut update = Update::default();
            loop {
                let before = (update.clone(), state.clone());
                (update, state) =
                    serde_json::from_slice(&serde_json::to_vec(&before).unwrap()).unwrap();
                assert_eq!((update.clone(), state.clone()), before);
                let request = match update.advance(&state) {
                    Ok(Some(request)) => request,
                    Ok(None) => break,
                    Err(e) if c["allocation_fails"] == true => {
                        assert!(e.contains("dereference null"));
                        failed = true;
                        break;
                    }
                    Err(e) => panic!("case {case}: {e}"),
                };
                let pending = update.clone();
                assert_eq!(update.advance(&state).unwrap(), Some(request.clone()));
                assert_eq!(update, pending);
                events.push(json!({"request":request,"state":state}));
                let reply = match request {
                    Request::Construct => {
                        mutate(&mut state, c, "Construct");
                        let id = if c["allocation_fails"] == true {
                            None
                        } else {
                            Some(1)
                        };
                        if id.is_some() {
                            constructed = json!({"metadata":metadata,"vector":[0,0,0],"name_length":0,"name_capacity":64,"first_name_byte":0});
                        }
                        Reply::Constructed(id)
                    }
                    Request::Load { profile, file, id } => {
                        assert_eq!(profile, 1);
                        mutate(&mut state, c, "Load");
                        let value = load(&mut metadata, file, id, &mut host).unwrap();
                        mutate(&mut state, c, "Loaded");
                        Reply::Loaded(value)
                    }
                };
                update = serde_json::from_slice(&serde_json::to_vec(&update).unwrap()).unwrap();
                update.acknowledge(&mut state, &request, reply).unwrap();
                assert!(
                    update
                        .acknowledge(&mut state, &request, Reply::Loaded(0))
                        .is_err()
                );
            }
            result = if failed {
                Value::Null
            } else {
                json!(update.result().unwrap())
            };
        }
        assert_eq!(
            json!({"state":state,"result":result,"failed":failed,"events":events,"constructed":constructed,
            "length":host.length,"reads":host.reads,"counts":host.counts,"appends":host.appends,"writes":host.writes}),
            row["original"],
            "case {case}"
        );
    }
}

#[test]
fn unavailable_string_backend_is_an_error_not_a_successful_empty_profile() {
    struct Unavailable;
    impl Strings for Unavailable {
        fn read(&mut self, _: &Metadata) -> Result<u32, String> {
            Err("source resource unavailable".into())
        }
        fn count(&mut self) -> Result<i32, String> {
            panic!("count after missing service")
        }
        fn append_empty(&mut self, _: i32) -> Result<(), String> {
            unreachable!()
        }
        fn write(&mut self, _: u64, _: u16) -> Result<u32, String> {
            unreachable!()
        }
    }
    let mut metadata = Metadata::default();
    assert_eq!(
        load(&mut metadata, 71, 200, &mut Unavailable).unwrap_err(),
        "source resource unavailable"
    );
    assert_eq!(metadata.file, 71);
    assert_eq!(metadata.id, 200);
}
