use super::*;
use crate::original_session::{Saved, completed_tools};
use sims_fixtures::Store;
use std::{fs, path::Path};

fn change_readset(value: &mut Value, address: u32, replacement: u8) -> Result<()> {
    let mut touched = false;
    for row in value["reads"].as_array_mut().context("readset absent")? {
        let start = number(&row["address"])?;
        let mut raw = bytes(&row["raw"])?;
        if address >= start && ((address - start) as usize) < raw.len() {
            raw[(address - start) as usize] = replacement;
            row["raw"] = raw
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
                .into();
            row["sha256"] = sims_fixtures::digest(&raw).into();
            touched = true;
        }
    }
    ensure!(touched, "coherent source mutation outside readsets");
    Ok(())
}
fn change_tool(value: &mut Value, slot: usize, offset: usize, replacement: u8) -> Result<()> {
    let row = &mut value["tools"][slot];
    let pointer = number(&row["pointer"])?;
    let mut raw = bytes(&row["payload"])?;
    raw[offset] = replacement;
    row["payload"] = raw
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .into();
    row["payload_sha256"] = sims_fixtures::digest(&raw).into();
    change_readset(value, pointer + offset as u32, replacement)
}
#[test]
#[ignore = "private actual stock bodies, seven Move groups and immutable UIText"]
fn initialized_stock_owns_real_settings_without_heap_junk_or_vm_changes() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("stock test worker panicked"))?
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let load = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&fs::read(root.join(p))?)?) };
    let b = "fixtures/private/goth-input-ownership-20261029-second/session-first";
    let refs = load(&format!("{b}/refs.json"))?;
    let saved: Saved = Store::open(root.join(format!("{b}/store")))?
        .get_value(&serde_json::from_value(refs["session"].clone())?)?;
    let original = postcard::to_allocvec(&saved.simulation.state.vm)?;
    let p = "fixtures/private/goth-input-ownership-20261029-second/tool-payloads-sixth";
    let q = "fixtures/private/goth-editor-settings-20261029-first/catalogs-second";
    let t = "fixtures/private/goth-supplemental-owners-20261029-first/input-tools-second";
    let before = load(&format!("{p}/before.json"))?;
    let catalogs = load(&format!("{q}/before.json"))?;
    let tools = load(&format!("{t}/before.json"))?;
    let state = saved
        .view
        .retained_tool_inputs
        .as_ref()
        .context("native bases absent")?;
    let ui=fs::read(root.join("cache/managed-independent-20261022-third/immutable-assets/GameData_ALL_Recursive/UIText.iff"))?;
    let actual = import(&before, &catalogs, &tools, state, &ui)?;
    let stock = actual.stock().context("stock missing")?;
    ensure!(
        stock
            .groups
            .0
            .iter()
            .map(|g| g.guids.len())
            .eq([61, 47, 18, 73, 6, 46, 3]),
        "actual Move category order lost"
    );
    ensure!(
        stock.dirt_references == 3
            && stock.dirt_reasons.len() == 7
            && stock.picker_reasons.len() == 9,
        "actual shared cache owners differ"
    );
    let after = import(
        &load(&format!("{p}/after.json"))?,
        &load(&format!("{q}/after.json"))?,
        &load(&format!("{t}/after.json"))?,
        state,
        &ui,
    )?;
    ensure!(
        actual == after,
        "native initialized stock changed across tick"
    );
    ensure!(
        postcard::from_bytes::<State>(&postcard::to_allocvec(&actual)?)? == actual,
        "stock cold codec differs"
    );
    let wire = serde_json::to_value(&actual)?;
    let mut historical = wire.clone();
    historical.as_object_mut().unwrap().remove("stock");
    ensure!(
        serde_json::from_value::<State>(historical)? == *state,
        "historical stock owner fabricated"
    );
    ensure!(
        import(&before, &catalogs, &tools, &actual, &ui).is_err(),
        "overlapping stock writer accepted"
    );
    let mut changed = before.clone();
    change_tool(&mut changed, 0, 0x2d, 0x7f)?;
    ensure!(
        import(&changed, &catalogs, &tools, state, &ui)? == actual,
        "uninitialized SimpleAlt heap byte became logical state"
    );
    let mut changed = before.clone();
    change_tool(&mut changed, 5, 0x3d, 0)?;
    ensure!(
        import(&changed, &catalogs, &tools, state, &ui)? != actual,
        "initialized Floor byte was discarded"
    );
    let mut changed = before.clone();
    let row = &mut changed["preview_commands"][0];
    let pointer = number(&row["pointer"])?;
    let mut raw = bytes(&row["raw"])?;
    raw[0x24] = 1;
    row["raw"] = raw
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .into();
    row["sha256"] = sims_fixtures::digest(&raw).into();
    change_readset(&mut changed, pointer + 0x24, 1)?;
    ensure!(
        import(&changed, &catalogs, &tools, state, &ui).is_err(),
        "pending native edit was discarded"
    );
    let mut changed = stock.clone();
    changed.initialized[0].pop();
    ensure!(
        changed.validate().is_err(),
        "missing initialized source field accepted"
    );
    let mut changed = wire;
    for slot in changed["slots"].as_array_mut().unwrap() {
        slot.as_object_mut().unwrap().remove("base");
    }
    let invalid: State = serde_json::from_value(changed)?;
    ensure!(invalid.stock().is_some(), "invalid test lost stock owner");
    // The Session validator invokes State::validate_fields; exercise that
    // through the same private-module boundary without accepting a bad save.
    ensure!(
        invalid.validate_fields().is_err(),
        "orphaned stock/base cohort accepted"
    );
    ensure!(
        postcard::to_allocvec(&saved.simulation.state.vm)? == original,
        "stock import changed VM/RNG"
    );
    // Resolve the independent original actor mapping too, so source state is
    // not reconstructed from a selected-Sim convenience value.
    let source =
        "fixtures/private/goth-supplemental-owners-20261029-first/frames-before-first.json";
    let closure = load(
        "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json",
    )?;
    let initial = completed_tools::import_retained_inputs(
        &tools,
        &closure["source_closure"],
        &load(source)?,
        &saved.simulation.state.vm.world,
    )?;
    ensure!(
        completed_tool_payloads::import(&before, &tools, &initial)? == *state,
        "stock source has a different existing input owner"
    );
    Ok(())
}
