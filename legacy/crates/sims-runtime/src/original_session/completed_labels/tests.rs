use super::*;
use crate::original_session::Saved;
use sims_fixtures::Store;
use std::{fs, path::Path};
#[test]
#[ignore = "private actual Goth label source and managed baseline"]
fn actual_star_labels_reject_empty_state_and_preserve_all_other_owners() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("label source worker panicked"))?
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(root.join(p))?)?) };
    let source = read("fixtures/private/goth-label-owners-20261029-first/labels-first.json")?;
    let reentry = read(
        "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json",
    )?;
    let refs =
        read("fixtures/private/goth-editor-settings-20261029-first/session-first/refs.json")?;
    let mut saved: Saved = Store::open(
        root.join("fixtures/private/goth-editor-settings-20261029-first/session-first/store"),
    )?
    .get_value(&serde_json::from_value(refs["session"].clone())?)?;
    let original = postcard::to_allocvec(&saved)?;
    let vm = postcard::to_allocvec(&saved.simulation.state.vm)?;
    ensure!(
        compare(&saved.simulation.state, &source, &reentry)?["difference_count"] == 3,
        "missing labels were silently accepted"
    );
    attach_before(&mut saved.simulation.state, &source, &reentry)?;
    ensure!(
        compare(&saved.simulation.state, &source, &reentry)?["difference_count"] == 0,
        "actual labels differ"
    );
    ensure!(
        saved
            .simulation
            .state
            .characters
            .values()
            .all(|r| r.displayed_animation == b"*"),
        "original star label not retained"
    );
    ensure!(
        postcard::to_allocvec(&saved.simulation.state.vm)? == vm,
        "label retention changed VM/RNG"
    );
    let temporary = std::env::temp_dir().join(format!(
        "sims-person-labels-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let cold_store = Store::create(&temporary)?;
    let cold: Saved = cold_store.get_value(&cold_store.put_value(&saved)?)?;
    std::fs::remove_dir_all(&temporary)?;
    ensure!(
        compare(&cold.simulation.state, &source, &reentry)?["difference_count"] == 0,
        "cold codec lost labels"
    );
    let mut restored = saved.clone();
    for r in restored.simulation.state.characters.values_mut() {
        r.displayed_animation.clear();
    }
    ensure!(
        postcard::to_allocvec(&restored)? == original,
        "label import changed other owners"
    );
    let mut bad = source.clone();
    bad["cuts"][0]["objects"][0]["raw"] = "2a".into();
    bad["cuts"][0]["objects"][0]["bytes"] = 1.into();
    bad["cuts"][0]["objects"][0]["sha256"] = sims_fixtures::digest(b"*").into();
    ensure!(
        import(&bad, &reentry, 88448).is_err(),
        "coherent unterminated native label accepted"
    );
    let mut bad = source;
    bad["cuts"][0]["objects"][0]["address"] = 0x10000.into();
    ensure!(
        import(&bad, &reentry, 88448).is_err(),
        "wrong native label owner accepted"
    );
    Ok(())
}
