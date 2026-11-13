use serde_json::{Value, json};
use sims_vm::{
    Definition,
    global_load::{self, Instance, Resource, Service},
};
use std::{fs, path::PathBuf};

fn definition(key: u16, guid: u32) -> Definition {
    Definition {
        source: "synthetic.iff".into(),
        object: key,
        guid,
        slots: 0,
    }
}

#[test]
fn reconciliation_keeps_registry_order_and_resource_identity() {
    let resources = [
        Resource {
            definition: definition(1, 3),
            kind: 7,
            global: 1,
        },
        Resource {
            definition: definition(2, 3),
            kind: 12,
            global: 1,
        },
    ];
    let instances = [93, 17, 2].map(|id| Instance {
        id,
        definition: definition(1, 3),
        global: 1,
    });
    let mut events = vec![];
    let got = global_load::reconcile(&resources, &instances, |service| {
        match service {
            Service::Create(d) => events.push(json!(["create", d.object])),
            Service::Delete(id) => events.push(json!(["delete", id])),
        }
        Ok(200)
    })
    .unwrap();
    assert_eq!(got, None);
    assert_eq!(
        events,
        [
            json!(["delete", 17]),
            json!(["delete", 2]),
            json!(["create", 2])
        ]
    );
}

#[test]
#[ignore = "requires private original-code global load oracle"]
fn global_reconciliation_matches_original_calls() {
    let report: Value = serde_json::from_slice(
        &fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../cache/research/sims-exe/oracle/global-load.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let resource = |key: usize| {
            let r = &c["resources"][key];
            Resource {
                definition: definition(key as u16, r["guid"].as_u64().unwrap() as u32),
                kind: r["kind"].as_i64().unwrap() as i16,
                global: r["global"].as_i64().unwrap() as i16,
            }
        };
        let catalog: Vec<_> = c["catalog"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| resource(key.as_u64().unwrap() as usize))
            .collect();
        let instances: Vec<_> = c["instances"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                let r = resource(i["resource"].as_u64().unwrap() as usize);
                Instance {
                    id: i["id"].as_i64().unwrap() as i16,
                    definition: r.definition,
                    global: r.global,
                }
            })
            .collect();
        let mut events = vec![];
        let mut created = c["created"].as_array().unwrap().iter();
        let selected = global_load::reconcile(&catalog, &instances, |s| {
            Ok(match s {
                Service::Delete(id) => {
                    events.push(json!(["delete", id]));
                    0
                }
                Service::Create(d) => {
                    let result = created.next().unwrap().as_i64().unwrap() as i16;
                    events.push(json!(["create", d.object, result]));
                    result
                }
            })
        })
        .unwrap();
        assert_eq!(
            json!({"events":events,"animation_tester":selected.unwrap_or(777)}),
            row["original"],
            "{c}"
        );
    }
}
