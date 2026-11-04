use serde_json::Value;
use sims_sim::lot_links::Links;
use sims_vm::{Object, World};
#[cfg(feature = "private-fixtures")]
#[test]
fn native_tile_seat_lookup_preserves_physical_chain_and_flag_order() {
    let report: Value = serde_json::from_str(include_str!("fixtures/seat-lookup.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let f = &report["fixture"];
        let mut w = World::default();
        let mut links = Links {
            size: 6,
            heads: serde_json::from_value(f["heads"].clone()).unwrap(),
            next: Default::default(),
        };
        for o in f["objects"].as_array().unwrap() {
            let id = o["id"].as_i64().unwrap() as i16;
            let slots: Vec<i16> = serde_json::from_value(o["slots"].clone()).unwrap();
            links.next.insert(id, slots[0]);
            w.objects.insert(
                id,
                Object {
                    data: serde_json::from_value(o["data"].clone()).unwrap(),
                    slots: slots[1..].iter().map(|i| (*i != 0).then_some(*i)).collect(),
                    ..Default::default()
                },
            );
        }
        links.heads[0][14] = c["head"].as_i64().unwrap() as i16;
        links.next.insert(3, c["next"].as_i64().unwrap() as i16);
        for (id, i) in [(3, 0), (5, 1)] {
            w.objects
                .get_mut(&id)
                .unwrap()
                .data
                .insert(8, c["flags"][i].as_i64().unwrap() as i16);
        }
        let found = links
            .first_blocker(
                &w,
                serde_json::from_value(c["point"].clone()).unwrap(),
                c["level"].as_i64().unwrap() as i32,
            )
            .unwrap()
            .unwrap_or(0);
        assert_eq!(i64::from(found), row["original"].as_i64().unwrap(), "{c}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn native_selected_seat_dispatch_matches_guards_and_call_order() {
    use serde_json::json;
    use sims_sim::lot_route::seat::{self, Dispatch, Services};
    struct Host<'a> {
        input: &'a Value,
        calls: Vec<Value>,
    }
    impl Services for Host<'_> {
        fn exists(&self, seat: i16) -> bool {
            seat == 2
        }
        fn function(&mut self, _: i16) -> Result<u16, String> {
            self.calls.push(json!(["function", 26]));
            Ok(self.input["function"].as_u64().unwrap() as u16)
        }
        fn container(&mut self) -> Result<Option<i16>, String> {
            self.calls.push(json!(["container"]));
            Ok((self.input["container"] == 2).then_some(2))
        }
        fn species(&self) -> Result<i16, String> {
            Ok(self.input["species"].as_i64().unwrap() as i16)
        }
        fn push(&mut self, id: i16, bhav: u16) -> Result<bool, String> {
            self.calls.push(json!(["push", id, 0, bhav, 0]));
            Ok(self.input["push"].as_bool().unwrap())
        }
    }
    let report: Value = serde_json::from_str(include_str!("fixtures/seat-dispatch.json")).unwrap();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let n = &row["original"];
        let mut h = Host {
            input: c,
            calls: vec![],
        };
        let action = seat::dispatch(c["seat"].as_i64().unwrap() as i16, &mut h).unwrap();
        h.calls.push(json!(["clear"]));
        assert_eq!(json!(h.calls), n["calls"], "{c}");
        assert_eq!(
            json!(match action {
                Dispatch::Failed => 0,
                Dispatch::AlreadySeated => 1,
                Dispatch::Started => 3,
            }),
            n["returned"],
            "{c}"
        );
        assert_eq!(
            json!(if action == Dispatch::Failed { 5 } else { 77 }),
            n["result"],
            "{c}"
        );
        assert_eq!(n["look"], Value::Null);
        assert_eq!(n["entry_flags"], 85);
        assert_eq!(n["error"], Value::Null);
    }
}
