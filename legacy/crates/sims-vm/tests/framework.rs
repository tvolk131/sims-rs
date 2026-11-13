use serde_json::{Value, json};
use sims_vm::scheduler::clock::draw::framework::{self, Fields, Frame, Query, Reply, Request};
use std::collections::BTreeMap;
#[derive(serde::Deserialize)]
struct Window {
    flags: u32,
    parent: u32,
}
struct Queries {
    windows: BTreeMap<u32, Window>,
    events: Vec<Value>,
}
impl Query for Queries {
    fn flags(&mut self, window: u32, mask: u32) -> Result<u32, String> {
        self.events.push(json!(Request::Flags { window, mask }));
        Ok(u32::from(self.windows[&window].flags & mask != 0))
    }
    fn parent(&mut self, window: u32) -> Result<u32, String> {
        self.events.push(json!(Request::Parent { window }));
        Ok(self.windows[&window].parent)
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_framework_queries_and_each_suspended_caller() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/native-framework-queries.json")).unwrap();
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 160);
    assert_eq!(
        corpus["source_sha256"],
        "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321"
    );
    for (i, row) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let fields = Fields {
            owner: 0x10000000,
            root: c["root"].as_u64().unwrap() as u32,
            registered: serde_json::from_value(c["registered"].clone()).unwrap(),
        };
        let mut q = Queries {
            windows: serde_json::from_value(c["windows"].clone()).unwrap(),
            events: vec![],
        };
        let mut frame = Frame::new(fields.owner);
        let mut events = vec![];
        while let Some(r) = frame.advance(&fields).unwrap() {
            let restored: Frame =
                serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            assert_eq!(restored, frame);
            let mut restored = restored;
            assert_eq!(restored.advance(&fields).unwrap(), Some(r.clone()));
            events.push(json!(r));
            let reply = match r {
                Request::Flags { window, mask } => Reply::Value(q.flags(window, mask).unwrap()),
                Request::Visible { window } => {
                    Reply::Value(framework::visible(window, &mut q).unwrap())
                }
                Request::Descendant { window, ancestor } => {
                    Reply::Value(framework::descendant(window, ancestor, &mut q).unwrap())
                }
                Request::Parent { window } => Reply::Value(q.parent(window).unwrap()),
                _ => Reply::Done,
            };
            frame.acknowledge(&fields, &r, reply).unwrap();
        }
        assert!(frame.complete());
        assert_eq!(
            json!({"requests":events,"queries":q.events}),
            row["expected"],
            "case{i}"
        );
    }
}
