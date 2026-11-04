use serde_json::{Value, json};
use sims_vm::{
    ObjectId, indicator_pools::Pools, indicator_unregister::*, load_indicators::Indicators,
};

fn id(v: &Value) -> Option<ObjectId> {
    let n = v.as_i64().unwrap() as i16;
    (n != 0).then_some(n)
}
fn ids(v: &Value) -> Vec<Option<ObjectId>> {
    v.as_array().unwrap().iter().map(id).collect()
}
fn numbers(v: &[Option<ObjectId>]) -> Vec<ObjectId> {
    v.iter().map(|v| v.unwrap_or(0)).collect()
}
struct Fixture {
    input: Value,
    enabled: bool,
    kind: i16,
    guid: u32,
    preview: Option<ObjectId>,
    held: Option<ObjectId>,
    cast: Option<ObjectId>,
    light: i16,
    room: i16,
    indicators: Indicators,
    pools: Pools,
    queues: Queues,
    events: Vec<Value>,
    applied: Vec<usize>,
}
impl Fixture {
    fn new(v: &Value) -> Self {
        Self {
            input: v.clone(),
            enabled: v["enabled"] == 1,
            kind: v["kind"].as_i64().unwrap() as i16,
            guid: v["guid"].as_u64().unwrap() as u32,
            preview: id(&v["preview"]),
            held: id(&v["held"]),
            cast: id(&v["cast"]),
            light: v["light"].as_i64().unwrap() as i16,
            room: v["room"].as_i64().unwrap() as i16,
            indicators: Indicators {
                primary: id(&v["primary"]),
                secondary: id(&v["secondary"]),
            },
            pools: Pools {
                active: ids(&v["active"]),
                spare: ids(&v["pool"]),
            },
            queues: Queues {
                dates: ids(&v["queue_a"]),
                stars: ids(&v["queue_b"]),
                dirty: false,
            },
            events: vec![],
            applied: vec![],
        }
    }
    fn state(&self) -> Value {
        json!({"primary": self.indicators.primary.unwrap_or(0), "secondary": self.indicators.secondary.unwrap_or(0),
            "active": numbers(&self.pools.active), "spare": numbers(&self.pools.spare),
            "queue_a": numbers(&self.queues.dates), "queue_b": numbers(&self.queues.stars)})
    }
    fn record(&mut self, event: Value) {
        self.events.push(event.clone());
        for (i, change) in self.input["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            if self.applied.contains(&i) || change["after"] != event[0] {
                continue;
            }
            self.applied.push(i);
            for (name, value) in [
                ("queue_a", &mut self.queues.dates),
                ("queue_b", &mut self.queues.stars),
                ("active", &mut self.pools.active),
                ("pool", &mut self.pools.spare),
            ] {
                if !change[name].is_null() {
                    *value = ids(&change[name]);
                }
            }
            for (name, value) in [
                ("preview", &mut self.preview),
                ("held", &mut self.held),
                ("cast", &mut self.cast),
            ] {
                if !change[name].is_null() {
                    *value = id(&change[name]);
                }
            }
            for (name, value) in [
                ("kind", &mut self.kind),
                ("light", &mut self.light),
                ("room", &mut self.room),
            ] {
                if let Some(n) = change[name].as_i64() {
                    *value = n as i16;
                }
            }
            if let Some(n) = change["guid"].as_u64() {
                self.guid = n as u32;
            }
            if let Some(n) = change["enabled"].as_i64() {
                self.enabled = n != 0;
            }
        }
    }
    fn getter(&mut self, name: &str, value: Option<ObjectId>) -> Result<Option<ObjectId>, String> {
        self.record(json!([name, value.unwrap_or(0)]));
        Ok(value)
    }
}
impl Host for Fixture {
    fn enabled(&self) -> bool {
        self.enabled
    }
    fn kind(&self, _: ObjectId) -> Result<i16, String> {
        Ok(self.kind)
    }
    fn guid(&self, _: ObjectId) -> Result<u32, String> {
        Ok(self.guid)
    }
    fn preview_person(&mut self) -> Result<Option<ObjectId>, String> {
        self.getter("preview", self.preview)
    }
    fn held_object(&mut self) -> Result<Option<ObjectId>, String> {
        self.getter("held", self.held)
    }
    fn cast_person(&mut self, _: ObjectId) -> Result<Option<ObjectId>, String> {
        self.getter("cast", self.cast)
    }
    fn clear_ui(&mut self) -> Result<(), String> {
        self.record(json!(["ui", self.state()]));
        Ok(())
    }
    fn indicators(&mut self) -> (&mut Indicators, &mut Pools) {
        (&mut self.indicators, &mut self.pools)
    }
    fn queues(&mut self) -> &mut Queues {
        &mut self.queues
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn unregister_and_immediate_capture_order_match_original_instructions() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/indicator-unregister.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for (index, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input = &row["input"];
        let mut f = Fixture::new(input);
        let target = id(&input["object"]);
        match input["mode"].as_str().unwrap() {
            "indicator" => indicator(&mut f, target),
            "unregister" => unregister(&mut f, target).unwrap(),
            "immediate" => {
                if let Some(target) = target {
                    if f.enabled {
                        unregister(&mut f, Some(target)).unwrap();
                    }
                    // 44123f captures both words after unregistration and before
                    // virtual RemoveFromLot. Its service bodies are not modeled here.
                    let (light, room) = (f.light, f.room);
                    f.record(json!(["detach", target, 1, f.state()]));
                    f.record(json!(["destroy", target, 0]));
                    if light != 0 {
                        f.record(json!(["signal", 0xf3, room]));
                    }
                }
            }
            _ => unreachable!(),
        }
        assert_eq!(
            json!({"events": f.events, "state": f.state(), "applied": f.applied}),
            row["original"],
            "native case {index}"
        );
        // Ordered nullable queues persist exactly, including retained duplicates.
        let saved = serde_json::to_vec(&(&f.indicators, &f.pools, &f.queues)).unwrap();
        let restored: (Indicators, Pools, Queues) = serde_json::from_slice(&saved).unwrap();
        assert_eq!(restored, (f.indicators, f.pools, f.queues));
    }
    assert_eq!(report["cases"].as_array().unwrap().len(), 137);
}
