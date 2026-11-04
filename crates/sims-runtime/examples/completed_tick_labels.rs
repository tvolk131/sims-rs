use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_labels};
use std::{fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 6 || (a.len() == 7 && a[6] == "--attach-before"),
        "usage: completed_tick_labels STORE REFS FIELD LABELS REENTRY NEW_DIR [--attach-before]"
    );
    let read = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(p)?)?) };
    let refs = read(&a[1])?;
    let mut saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let source = read(&a[3])?;
    let reentry = read(&a[4])?;
    let out = Path::new(&a[5]);
    fs::create_dir(out)?;
    let original = postcard::to_allocvec(&saved)?;
    let before = completed_labels::compare(&saved.simulation.state, &source, &reentry)?;
    fs::write(
        out.join("before-report.json"),
        serde_json::to_vec_pretty(&before)?,
    )?;
    if a.len() == 7 {
        let old: Vec<_> = saved
            .simulation
            .state
            .characters
            .iter()
            .map(|(&id, r)| (id, r.displayed_animation.clone()))
            .collect();
        completed_labels::attach_before(&mut saved.simulation.state, &source, &reentry)?;
        let mut reverse = saved.clone();
        for (id, value) in old {
            reverse
                .simulation
                .state
                .characters
                .get_mut(&id)
                .unwrap()
                .displayed_animation = value;
        }
        ensure!(
            postcard::to_allocvec(&reverse)? == original,
            "native label join changed other Session state or RNG"
        );
        let blob = Store::create(out.join("store"))?.put_value(&saved)?;
        fs::write(
            out.join("refs.json"),
            serde_json::to_vec_pretty(&json!({"session":blob}))?,
        )?;
    }
    let report = completed_labels::compare(&saved.simulation.state, &source, &reentry)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    ensure!(
        report["difference_count"] == 0,
        "native persistent person label cache differs"
    );
    Ok(())
}
