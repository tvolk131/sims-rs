use serde_json::{Value, json};
use sims_sim::lot_route::completion::{Completion, Services};
use std::collections::BTreeMap;
struct Host<'a> {
    input: &'a Value,
    person: BTreeMap<i16, i16>,
    data: BTreeMap<i16, i16>,
    events: Vec<Value>,
    success: Option<bool>,
    suspend: bool,
    accepts: usize,
    commits: usize,
}
impl Host<'_> {
    fn mutate(&mut self, key: &str) {
        if let Some(m) = self.input[key].as_object() {
            for (name, v) in m {
                let v = v.as_i64().unwrap() as i16;
                if let Some(i) = name.strip_prefix("person") {
                    self.person.insert(i.parse().unwrap(), v);
                } else {
                    self.data
                        .insert(name.strip_prefix("data").unwrap().parse().unwrap(), v);
                }
            }
        }
    }
}
impl Services for Host<'_> {
    fn person(&self, i: i16) -> Result<i16, String> {
        Ok(self.person[&i])
    }
    fn set_person(&mut self, i: i16, v: i16) -> Result<(), String> {
        self.person.insert(i, v);
        Ok(())
    }
    fn data(&self, i: i16) -> Result<i16, String> {
        Ok(self.data[&i])
    }
    fn set_data(&mut self, i: i16, v: i16) -> Result<(), String> {
        self.data.insert(i, v);
        Ok(())
    }
    fn accept(&mut self) -> Result<Option<bool>, String> {
        self.accepts += 1;
        if self.suspend && self.accepts == 1 {
            return Ok(None);
        }
        self.events.push(json!({"accept":[self.person[&73]]}));
        self.mutate("accept_mutations");
        Ok(Some(self.input["accepted"].as_bool().unwrap()))
    }
    fn commit(&mut self) -> Result<bool, String> {
        self.commits += 1;
        if self.suspend && self.commits == 1 {
            return Ok(false);
        }
        self.events.push(json!({"commit":[40,56]}));
        self.mutate("commit_mutations");
        Ok(true)
    }
    fn path(&self) -> Vec<[i32; 2]> {
        serde_json::from_value(self.input["path"].clone()).unwrap()
    }
    fn orient(&mut self, d: u8) -> Result<(), String> {
        self.data.insert(1, i16::from(d));
        self.events.push(json!("orientation"));
        Ok(())
    }
    fn moving(&self) -> bool {
        self.input["animating"].as_bool().unwrap()
    }
    fn movement_diagnostic(&mut self) -> Result<(), String> {
        self.events.push(json!("movement_diagnostic"));
        Ok(())
    }
    fn update_movement(&mut self) -> Result<(), String> {
        self.events.push(json!("animator_update"));
        Ok(())
    }
    fn depth(&self) -> usize {
        self.input["depth"].as_u64().unwrap() as usize
    }
    fn finish(&mut self, s: bool) -> Result<(), String> {
        self.success = Some(s);
        self.events.push(json!("context_pop"));
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_final_cleanup_matches_fields_callbacks_and_resumable_order() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/route-completion.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        for suspend in [false, true] {
            let mut h = Host {
                input: c,
                person: BTreeMap::from([
                    (0, c["posture"].as_i64().unwrap() as i16),
                    (38, c["person38"].as_i64().unwrap() as i16),
                    (39, 0),
                    (62, 0),
                    (72, c["person72"].as_i64().unwrap() as i16),
                    (73, 23),
                ]),
                data: BTreeMap::from([(1, 3), (9, c["data9"].as_i64().unwrap() as i16)]),
                events: vec![],
                success: None,
                suspend,
                accepts: 0,
                commits: 0,
            };
            let mut machine =
                Completion::new(c["state"] != 7, c["result"].as_i64().unwrap() as i32);
            let mut attempts = 0;
            while !machine.drive(&mut h).unwrap() {
                attempts += 1;
                assert!(attempts <= 2);
                machine = serde_json::from_slice(&serde_json::to_vec(&machine).unwrap()).unwrap();
            }
            let (success, result) = machine.outcome().unwrap();
            assert_eq!(h.success, Some(success));
            let rust = json!({"returned":i32::from(success),"state":if success{8}else{7},"context_result":result as u32,"person62":h.person[&62],"person72":h.person[&72],"person73":h.person[&73],"person39":h.person[&39],"data9":h.data[&9],"direction":h.data[&1],"events":h.events});
            assert_eq!(rust, row["original"], "{c}, suspended {suspend}");
        }
    }
}
