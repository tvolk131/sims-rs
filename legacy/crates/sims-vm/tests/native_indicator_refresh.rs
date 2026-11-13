use serde_json::{Value, json};
use sims_vm::{ObjectId, indicator_refresh::*, indicator_unregister::Queues};

fn id(v: &Value) -> Option<ObjectId> {
    let n = v.as_i64().unwrap() as i16;
    (n != 0).then_some(n)
}
fn ids(v: &Value) -> Vec<Option<ObjectId>> {
    v.as_array().unwrap().iter().map(id).collect()
}
fn state(q: &Queues) -> Value {
    json!({"dates": q.dates.iter().map(|v|v.unwrap_or(0)).collect::<Vec<_>>(),
        "stars": q.stars.iter().map(|v|v.unwrap_or(0)).collect::<Vec<_>>(), "dirty":q.dirty})
}
struct Host<'a>(&'a Value);
impl View for Host<'_> {
    fn date_target(&self, person: ObjectId) -> Result<i16, String> {
        Ok(self.0["targets"][person as usize - 1].as_i64().unwrap() as i16)
    }
    fn script_id(&self, person: ObjectId) -> Result<i16, String> {
        Ok(self.0["ids"][person as usize - 1].as_i64().unwrap() as i16)
    }
}
fn restore(d: &mut Drain, q: &mut Queues) {
    let bytes = serde_json::to_vec(&(&*d, &*q)).unwrap();
    let (r, s): (Drain, Queues) = serde_json::from_slice(&bytes).unwrap();
    r.validate().unwrap();
    assert_eq!((&r, &s), (&*d, &*q));
    (*d, *q) = (r, s);
}

#[cfg(feature = "private-fixtures")]
#[test]
fn producers_and_drain_match_native_order_mutations_and_dirty_state() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/indicator-refresh.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 99);
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let mut queues = Queues {
            dates: ids(&c["dates"]),
            stars: ids(&c["stars"]),
            dirty: c["dirty"] != 0,
        };
        let mut events = vec![];
        let mut applied = vec![];
        for op in c["operations"].as_array().unwrap() {
            match op[0].as_str().unwrap() {
                "date" => date(&mut queues, id(&op[1]), &Host(c)).unwrap(),
                "star" => star(&mut queues, id(&op[1])).unwrap(),
                "drain" => {
                    let mut drain = Drain::default();
                    while let Some(request) = drain.advance(&mut queues).unwrap() {
                        restore(&mut drain, &mut queues);
                        let before = (drain.clone(), queues.clone());
                        assert_eq!(drain.advance(&mut queues).unwrap(), Some(request));
                        assert_eq!((drain.clone(), queues.clone()), before);
                        let (name, person) = match request {
                            Request::Date(id) => ("date", id),
                            Request::Star(id) => ("star", id),
                        };
                        let wrong = match request {
                            Request::Date(id) => Request::Star(id),
                            Request::Star(id) => Request::Date(id),
                        };
                        assert!(drain.acknowledge(wrong).is_err());
                        assert_eq!((drain.clone(), queues.clone()), before);
                        events.push(json!([name, person.unwrap_or(0), state(&queues)]));
                        for (i, change) in c["changes"].as_array().into_iter().flatten().enumerate()
                        {
                            if applied.contains(&i)
                                || change["after"] != json!([name, person.unwrap_or(0)])
                            {
                                continue;
                            }
                            applied.push(i);
                            if !change["dates"].is_null() {
                                queues.dates = ids(&change["dates"]);
                            }
                            if !change["stars"].is_null() {
                                queues.stars = ids(&change["stars"]);
                            }
                            if !change["dirty"].is_null() {
                                queues.dirty = change["dirty"] != 0;
                            }
                        }
                        restore(&mut drain, &mut queues);
                        assert_eq!(drain.advance(&mut queues).unwrap(), Some(request));
                        drain.acknowledge(request).unwrap();
                        let before = drain.clone();
                        assert!(drain.acknowledge(request).is_err());
                        assert_eq!(drain, before);
                        restore(&mut drain, &mut queues);
                    }
                    restore(&mut drain, &mut queues);
                }
                _ => panic!("unknown native fixture operation"),
            }
        }
        assert_eq!(
            json!({"state":state(&queues),"events":events,"applied":applied}),
            row["original"],
            "{c}"
        );
    }
}

#[test]
fn invalidated_iterator_stops_and_empty_dirty_state_survives_save() {
    let mut q = Queues::default();
    star(&mut q, None).unwrap();
    assert!(
        !q.is_empty(),
        "dirty-only queue must not be omitted from saved state"
    );
    let saved = serde_json::to_vec(&q).unwrap();
    assert_eq!(serde_json::from_slice::<Queues>(&saved).unwrap(), q);
    let mut d = Drain::default();
    assert_eq!(d.advance(&mut q).unwrap(), None);
    assert!(q.is_empty());
    q.dates = vec![Some(1), Some(2)];
    q.dirty = true;
    d = Drain::default();
    let request = d.advance(&mut q).unwrap().unwrap();
    q.dates.pop();
    d.acknowledge(request).unwrap();
    let before = (d.clone(), q.clone());
    assert!(
        d.advance(&mut q)
            .unwrap_err()
            .contains("invalidated queue iterators")
    );
    assert_eq!((d, q), before);
    let mut bad = serde_json::to_value(Drain::default()).unwrap();
    bad["pending"] = json!({"Date":1});
    assert!(
        serde_json::from_value::<Drain>(bad)
            .unwrap()
            .validate()
            .is_err()
    );
}
