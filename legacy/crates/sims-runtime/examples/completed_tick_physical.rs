//! All current physical owners and the full used registry word backing.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_animations, completed_lot};
use sims_vm::scheduler::storage::Storage;
use std::{collections::BTreeMap, fs, path::Path};
fn bytes(v: &Value) -> Result<Vec<u8>> {
    completed_animations::bytes(v.as_str().context("physical bytes absent")?)
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn same<T: Serialize>(name: &str, a: &T, b: &T, rows: &mut Vec<Value>) -> Result<()> {
    let x = postcard::to_allocvec(a)?;
    let y = postcard::to_allocvec(b)?;
    let mut r = json!({"field":name,"matched":x==y,"native_sha256":sims_fixtures::digest(&x),"rust_sha256":sims_fixtures::digest(&y)});
    if x != y {
        r["native"] = serde_json::to_value(a)?;
        r["rust"] = serde_json::to_value(b)?;
    }
    rows.push(r);
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 5,
        "usage: completed_tick_physical STORE REFS FIELD NATIVE_JSON NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved =
        Store::open(&args[0])?.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let input: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    let simulation = &saved.simulation.state;
    let world = &simulation.vm.world;
    ensure!(
        input["schema"] == 1
            && input["tick"] == simulation.vm.tick
            && input["executes_target_code"] == false
            && input["provenance"]["replay_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e",
        "physical source boundary differs"
    );
    let before = postcard::to_allocvec(world)?;
    let lot = completed_lot::import(&input["lot"])?;
    let physical = completed_lot::physical::import(&lot, world)?;
    let actual = simulation
        .native_lot
        .as_ref()
        .context("retained physical owner absent")?;
    let mut rows = Vec::new();
    same(
        "placement_all_links_bounds_flags",
        &physical.placement,
        &actual.placement,
        &mut rows,
    )?;
    same(
        "cached_room_membership",
        &physical.membership,
        &actual.membership,
        &mut rows,
    )?;
    same(
        "indicators",
        &physical.indicators,
        &actual.indicators,
        &mut rows,
    )?;
    same(
        "indicator_pools",
        &physical.pools,
        &simulation.native_indicator_pools,
        &mut rows,
    )?;
    same(
        "selected_person",
        &physical.selected_person,
        &actual.selected_person,
        &mut rows,
    )?;
    let registry_raw = bytes(&input["lot"]["registry_raw"])?;
    let registry = input["lot"]["registry"]
        .as_u64()
        .context("registry pointer absent")?;
    let mut arrays = Vec::new();
    for (name, off) in [("pointer_vector", 0x18), ("word_vector", 0x24)] {
        let v = &input["registry"][name];
        let h = bytes(&v["header"]["raw"])?;
        ensure!(
            h.len() == 12
                && v["header"]["address"] == registry + off as u64
                && h == registry_raw[off..off + 12]
                && v["header"]["sha256"] == sims_fixtures::digest(&h),
            "registry vector/header alias differs"
        );
        let (a, b, c) = (word(&h, 0), word(&h, 4), word(&h, 8));
        let body = bytes(&v["allocated"]["raw"])?;
        ensure!(
            a >= 0x10000
                && a <= b
                && b <= c
                && (b - a) % 4 == 0
                && (c - a) % 4 == 0
                && body.len() == (c - a) as usize
                && v["begin"] == a
                && v["end"] == b
                && v["capacity"] == c
                && v["allocated"]["address"] == a
                && v["allocated"]["sha256"] == sims_fixtures::digest(&body),
            "registry allocated backing differs"
        );
        arrays.push(
            body[..(b - a) as usize]
                .chunks_exact(4)
                .map(|b| word(b, 0))
                .collect::<Vec<_>>(),
        );
    }
    let mut pointers = BTreeMap::new();
    for row in input["lot"]["objects"]
        .as_array()
        .context("physical object inventory absent")?
    {
        pointers.insert(
            row["pointer"].as_u64().context("physical pointer absent")? as u32,
            row["object"].as_i64().context("physical id absent")? as u32,
        );
    }
    let handles = arrays[0]
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if *p == 0 {
                Ok(None)
            } else {
                let id = *pointers
                    .get(p)
                    .context("registry pointer has no physical owner")?;
                ensure!(id == i as u32 + 1, "registry identity/slot differs");
                Ok(Some(id))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let last = handles
        .iter()
        .flatten()
        .copied()
        .max()
        .context("empty registry")?;
    let expected = Storage::from_retained(world, handles, arrays[1].clone(), last)
        .map_err(anyhow::Error::msg)?;
    let actual_registry = world
        .scheduler_storage
        .as_ref()
        .context("retained registry backing absent")?;
    same(
        "registry_full_used_words_holes_tail_and_lifetime_mapping",
        &expected,
        actual_registry,
        &mut rows,
    )?;
    ensure!(
        postcard::to_allocvec(world)? == before,
        "physical comparison changed World/VM"
    );
    let differences = rows.iter().filter(|r| r["matched"] != true).count();
    let report = json!({"schema":1,"tick":simulation.vm.tick,"comparisons":rows,"difference_count":differences,
        "pointer_extent":arrays[0].len(),"word_extent":arrays[1].len(),"word_tail":arrays[1].len()-arrays[0].len(),
        "native_parity_ticks":0,"complete_native_Session_admitted":false,"vm_world_unchanged":true,
        "normalization":"Fresh Rust registry lifetimes use source-qualified occupied ID handles; native pointer identities and unused allocation capacity bytes are outside semantic equality. All used words, including holes and the complete separate tail, remain compared.",
        "scope":"Physical/registry owners only; full Session consumer/continuation admission remains separate"});
    let out = Path::new(&args[4]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"comparisons":rows.len(),"differences":differences})
    );
    ensure!(differences == 0, "retained physical/registry owners differ");
    Ok(())
}
