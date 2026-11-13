use super::*;
use std::{fs, path::Path};
#[test]
#[ignore = "private retained Goth baseline, original completed caller proof and source header"]
fn actual_resident_epoch_rejects_latched_work_wrong_stack_and_active_native_profile() -> Result<()>
{
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("Resident epoch test worker panicked"))?
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(root.join(p))?)?) };
    let cp = root.join("fixtures/private/goth-label-owners-20261029-first/session-first");
    let refs: Value = serde_json::from_slice(&fs::read(cp.join("refs.json"))?)?;
    let saved: Saved = sims_fixtures::Store::open(cp.join("store"))?
        .get_value(&serde_json::from_value(refs["session"].clone())?)?;
    let source = read(
        "fixtures/private/goth-native-animation-rebase-20261028-first/native-before-complete-geometry-first.json",
    )?;
    let proof = read("fixtures/private/goth-caller-boundary-20261029-first/proof-second.json")?;
    let before = postcard::to_allocvec(&saved.simulation.state.vm)?;
    let actual = evaluate(&saved, &source, &proof)?;
    ensure!(
        actual["difference_count"] == 0
            && actual["inventory"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["fields"].as_array().unwrap().len() == 15),
        "actual Resident ownership classification differs"
    );
    let mut changed = saved.clone();
    changed
        .simulation
        .state
        .characters
        .get_mut(&16)
        .unwrap()
        .animation_yield = Some((16, 88448, 1));
    ensure!(
        evaluate(&changed, &source, &proof)?["difference_count"] == 1,
        "latched replacement request was ignored"
    );
    ensure!(
        postcard::to_allocvec(&changed.simulation.state.vm)? == before,
        "Resident observer changed VM/RNG"
    );
    for kind in 0..4 {
        let mut wrong = proof.clone();
        match kind {
            0 => wrong["cuts"][1]["anchors"][1]["raw"] = "74da4600".into(),
            1 => wrong["cuts"][1]["registers"]["eip"] = 1.into(),
            2 => wrong["code"][1]["sha256"] = "different".into(),
            3 => {
                wrong["native_object_ids"].as_array_mut().unwrap().pop();
            }
            _ => unreachable!(),
        }
        ensure!(
            evaluate(&saved, &source, &wrong).is_err(),
            "wrong caller qualification accepted: {kind}"
        );
    }
    let mut active = source;
    let r = &mut active["resident"]["actors"][0]["animator_raw"];
    let mut b = crate::original_session::completed_animations::bytes(r.as_str().unwrap())?;
    b[0x188] = 1;
    *r = b
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .into();
    ensure!(
        evaluate(&saved, &active, &proof).is_err(),
        "active original motion was classified as a closed synchronous caller"
    );
    Ok(())
}
