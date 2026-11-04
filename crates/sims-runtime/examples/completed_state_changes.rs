//! Exhaustive Rust-state change inventory. This is not a native parity gate.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::Saved;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
fn load(store: &str, refs: &str, field: &str) -> Result<Saved> {
    let refs: Value = serde_json::from_slice(&fs::read(refs)?)?;
    Store::open(store)?.get_value(&serde_json::from_value(refs[field].clone())?)
}
fn small(v: Option<&Value>) -> Value {
    let Some(v) = v else {
        return json!({"absent":true});
    };
    let raw = serde_json::to_vec(v).unwrap();
    if raw.len() <= 256 {
        v.clone()
    } else {
        json!({"json_bytes":raw.len(),"sha256":sims_fixtures::digest(&raw)})
    }
}
fn visit(a: Option<&Value>, b: Option<&Value>, path: &str, groups: &mut BTreeMap<String, Value>) {
    match (a, b) {
        (Some(Value::Object(a)), Some(Value::Object(b))) => {
            let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for k in keys {
                visit(
                    a.get(k),
                    b.get(k),
                    &format!("{path}/{}", k.replace('~', "~0").replace('/', "~1")),
                    groups,
                );
            }
            return;
        }
        (Some(Value::Array(a)), Some(Value::Array(b))) if a.len() == b.len() => {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                visit(Some(a), Some(b), &format!("{path}/{i}"), groups);
            }
            return;
        }
        // Compare encoded numeric scalars too: Value equality would hide -0.
        (Some(a), Some(b)) if serde_json::to_vec(a).unwrap() == serde_json::to_vec(b).unwrap() => {
            return;
        }
        (None, None) => return,
        _ => (),
    }
    let group = path
        .split('/')
        .map(|p| if p.parse::<i64>().is_ok() { "*" } else { p })
        .collect::<Vec<_>>()
        .join("/");
    let row = groups
        .entry(group)
        .or_insert_with(|| json!({"count":0,"examples":[]}));
    row["count"] = json!(row["count"].as_u64().unwrap() + 1);
    let examples = row["examples"].as_array_mut().unwrap();
    if examples.len() < 3 {
        examples.push(json!({"path":path,"before":small(a),"after":small(b)}));
    }
}
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 7,
        "usage: completed_state_changes BEFORE_STORE BEFORE_REFS FIELD AFTER_STORE AFTER_REFS FIELD NEW_DIR"
    );
    let before = load(&a[0], &a[1], &a[2])?;
    let after = load(&a[3], &a[4], &a[5])?;
    let mut groups = BTreeMap::new();
    visit(
        Some(&serde_json::to_value(&before)?),
        Some(&serde_json::to_value(&after)?),
        "",
        &mut groups,
    );
    let out = Path::new(&a[6]);
    fs::create_dir(out).context("new state-change output directory")?;
    let report = json!({"schema":1,"scope":"all serialized Rust Saved fields; missing and null differ; numeric encoding preserves signed-zero changes; native equality is not tested",
        "native_comparison_executed":false,"native_parity_ticks":0,"groups":groups,
        "before_tick":before.simulation.state.vm.tick,"after_tick":after.simulation.state.vm.tick});
    fs::write(
        out.join("changes.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", json!({"groups":groups.len(),"native_parity_ticks":0}));
    Ok(())
}
