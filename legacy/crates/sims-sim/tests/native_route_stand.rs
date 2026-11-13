#![cfg(feature = "private-fixtures")]

use serde_json::{Value, json};
use sims_sim::{
    Point,
    lot_route::departure::{self, Dispatch, Scan, Services},
};

fn corpus() -> Value {
    let report: Value = serde_json::from_str(include_str!("fixtures/route-stand.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    report
}
#[test]
fn departure_callbacks_and_serialized_scan_match_original() {
    for row in corpus()["departures"].as_array().unwrap() {
        let c = &row["input"];
        let point =
            |v: &Value| Point::new(v[0].as_i64().unwrap() as i32, v[1].as_i64().unwrap() as i32);
        let mask = if c["contained"] == true && c["sit_function"] == true {
            c["mask"].as_u64().unwrap() as u16
        } else {
            0
        };
        let mut scan = Scan::new(point(&c["position"]), point(&c["target"]), mask);
        let mut facing = c["facing"].as_i64().unwrap() as i16;
        let mut checks = vec![];
        while let Some(probe) = scan.next(facing) {
            scan.validate().unwrap();
            let mut saved: Scan =
                serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
            assert_eq!(
                saved.next(facing.wrapping_add(3)),
                Some(probe),
                "pending request changed {c}"
            );
            let classification = c["classification"][usize::from(probe.direction)]
                .as_u64()
                .unwrap() as u8;
            checks.push(json!({"point":probe.position,"direction":probe.direction,"classification":classification}));
            scan.resume(probe, classification).unwrap();
            saved.resume(probe, classification).unwrap();
            assert_eq!(scan, saved);
            if c["mutate"] == true {
                facing = ((c["facing"].as_i64().unwrap() + 3 * checks.len() as i64) & 7) as i16;
            }
        }
        scan.validate().unwrap();
        let selected = scan.outcome().unwrap();
        assert_eq!(
            json!({"position":[selected.position.x,selected.position.y],"flags":selected.flags,"checks":checks,"error":null}),
            row["original"],
            "{c}"
        );
    }
}
struct Host<'a> {
    c: &'a Value,
    events: Vec<Value>,
    person37: i16,
    person72: i16,
}
impl Services for Host<'_> {
    fn posture(&self) -> i16 {
        self.c["posture"].as_i64().unwrap() as i16
    }
    fn reuse(&mut self) -> Result<bool, String> {
        self.events.push(json!("reuse"));
        Ok(self.c["keep"] == true)
    }
    fn container(&mut self) -> Result<Option<i16>, String> {
        self.events.push(json!("container"));
        Ok((self.c["contained"] == true).then_some(2))
    }
    fn function(&mut self, _: i16, number: usize) -> Result<u16, String> {
        self.events.push(json!(format!("function{number}")));
        Ok(
            if self.c[if number == 26 { "sit" } else { "stand" }] == true {
                if number == 26 { 4096 } else { 4102 }
            } else {
                0
            },
        )
    }
    fn flags(&self) -> u16 {
        self.c["departure"].as_u64().unwrap() as u16
    }
    fn push(&mut self, seat: i16, bhav: u16, flags: u16) -> Result<bool, String> {
        self.events.push(json!({"push":{"target":seat,"args":[flags as i16,0,0,0],"bhav":bhav,"check":0,"person37":self.person37,"person72":self.person72}}));
        Ok(self.c["push"] == true)
    }
    fn publish(&mut self, flags: u16) -> Result<(), String> {
        self.person37 = flags as i16;
        self.person72 = 0;
        Ok(())
    }
}
#[test]
fn stand_dispatch_and_resumption_match_original() {
    let report = corpus();
    for row in report["dispatch"].as_array().unwrap() {
        let mut host = Host {
            c: &row["input"],
            events: vec![json!("prepare")],
            person37: 77,
            person72: 99,
        };
        let result = departure::dispatch(&mut host).unwrap();
        if result == Dispatch::NotSeated {
            host.events.push(json!("plan"));
        }
        let n = &row["original"];
        assert_eq!(json!(host.events), n["events"], "{}", row["input"]);
        assert_eq!(json!(host.person37), n["person37"]);
        assert_eq!(json!(host.person72), n["person72"]);
        let expected = if n["state"] == 5 {
            Dispatch::Started
        } else if n["state"] == 8 {
            Dispatch::Keep
        } else if n["state"] == 7 {
            Dispatch::Failed
        } else {
            Dispatch::NotSeated
        };
        assert_eq!(result, expected, "{row}");
        assert!(n["error"].is_null());
    }
    for row in report["resumes"].as_array().unwrap() {
        let c = &row["input"];
        let resumed =
            departure::returned(c["success"] == true, c["posture"].as_i64().unwrap() as i16);
        assert_eq!(
            resumed,
            row["original"]["events"] == json!(["prepare"]),
            "{row}"
        );
        assert_eq!(row["original"]["person72"], 0x321);
        assert_eq!(row["original"]["result"], if resumed { 0 } else { 6 });
    }
}
