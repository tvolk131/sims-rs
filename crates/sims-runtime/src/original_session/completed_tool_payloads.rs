//! Qualified cTool bases from actual factory-sized payload observations.
//! Undefined heap bytes beyond initialized fields are not promoted to state.
use super::{
    completed_lot::Memory,
    retained_tool_inputs::{Base, State},
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;

const ALLOCATIONS: [(u32, usize); 12] = [
    (0x484520, 0x60),
    (0x484554, 0x60),
    (0x484588, 0x74),
    (0x4845b2, 0x64),
    (0x484606, 0x58),
    (0x4845dc, 0x68),
    (0x48462c, 0x108),
    (0x484655, 0xb0),
    (0x484682, 0x44),
    (0x4846a8, 0x5c),
    (0x4846d2, 0x58),
    (0x4846fc, 0x48),
];
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("tool payload integer absent")?
        .try_into()?)
}
fn word(raw: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap())
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("tool payload bytes absent")?)
}
/// Pure enrichment. Every row is validated before the small tool state is
/// copied; no VM, World, constructor, getter or callback is invoked.
pub fn import(v: &Value, tools: &Value, state: &State) -> Result<State> {
    let payloads = actual_payloads(v, tools, state)?;
    state
        .clone()
        .with_bases(payloads.map(|payload| base(&payload)))
}
pub(super) fn base(payload: &[u8]) -> Base {
    Base {
        words_04_20: std::array::from_fn(|n| word(payload, 4 + 4 * n)),
        word_24: word(payload, 0x24),
        tile_overlay: payload[0x28],
    }
}
/// Authenticate the actual allocation bodies without changing any retained
/// owner. The initialized-field importer shares the same source checks.
pub(super) fn actual_payloads(v: &Value, tools: &Value, state: &State) -> Result<[Vec<u8>; 12]> {
    ensure!(
        v["schema"] == 2
            && matches!(v["tick"].as_u64(), Some(88448 | 88449))
            && v["tick"] == tools["tick"]
            && v["executes_target_code"] == false
            && v["allocation_sizes_inferred"] == false,
        "tool payload boundary/profile differs"
    );
    ensure!(
        v["source_observation_sha256"]
            == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c"
            && v["source_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e"
            && v["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && v["current"] == tools["current"],
        "tool payload source/current owner differs"
    );
    let memory = Memory::new(v)?;
    let rows = v["tools"].as_array().context("tool payload rows absent")?;
    ensure!(rows.len() == 12, "stock tool payload cohort differs");
    let mut payloads = Vec::with_capacity(12);
    for (i, (row, slot)) in rows.iter().zip(state.slots()).enumerate() {
        let (address, size) = ALLOCATIONS[i];
        ensure!(
            row["slot"] == i
                && number(&row["pointer"])? == slot.handle
                && number(&row["vtable"])? == slot.class
                && row["factory_allocation"]["address"] == address
                && row["factory_allocation"]["bytes"] == size,
            "stock tool allocation/alias differs"
        );
        let push = if size < 128 {
            vec![0x6a, size as u8]
        } else {
            let mut b = vec![0x68];
            b.extend_from_slice(&(size as u32).to_le_bytes());
            b
        };
        ensure!(
            memory.read(address, push.len())? == push
                && bytes(&row["factory_allocation"]["raw"])? == push,
            "actual tool factory allocation instruction differs"
        );
        let payload = memory.read(slot.handle, size)?;
        ensure!(
            payload == bytes(&row["payload"])?
                && row["payload_sha256"] == sims_fixtures::digest(&payload)
                && word(&payload, 0) == slot.class
                && payload[..0x2c] == bytes(&row["raw"])?
                && row["sha256"] == sims_fixtures::digest(&payload[..0x2c]),
            "actual tool payload/base receipt differs"
        );
        if v["current"] == slot.handle {
            let active = bytes(&tools["tool"]["raw"])?;
            ensure!(
                active.get(..size) == Some(payload.as_slice()),
                "current tool payload conflicts with qualified input owner"
            );
        }
        let base = base(&payload);
        ensure!(
            serde_json::to_value(base.words_04_20)? == row["words_04_20"]
                && row["word_24"] == base.word_24
                && row["byte_28"] == base.tile_overlay,
            "tool base projection differs from actual bytes"
        );
        payloads.push(payload);
    }
    payloads
        .try_into()
        .map_err(|_| anyhow::anyhow!("stock payload count differs"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private original twelve-tool payload observations"]
    fn actual_bases_preserve_inactive_overlay_and_do_not_touch_vm() -> Result<()> {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(run)?
            .join()
            .map_err(|_| anyhow::anyhow!("tool-base test worker panicked"))?
    }
    fn run() -> Result<()> {
        use super::super::{Saved, completed_tools};
        use sims_fixtures::Store;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let read = |name: &str| -> Result<Value> {
            Ok(serde_json::from_slice(&std::fs::read(root.join(name))?)?)
        };
        let b = "fixtures/private/goth-tool-inputs-20261029-first/session-second";
        let refs = read(&format!("{b}/refs.json"))?;
        let saved: Saved = Store::open(root.join(format!("{b}/store")))?
            .get_value(&serde_json::from_value(refs["session"].clone())?)?;
        let before_vm = postcard::to_allocvec(&saved.simulation.state.vm)?;
        let src = "fixtures/private/goth-supplemental-owners-20261029-first";
        let tools = read(&format!("{src}/input-tools-second/before.json"))?;
        let threads = read(&format!("{src}/frames-before-first.json"))?;
        let reentry = read(
            "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json",
        )?;
        let initial = completed_tools::import_retained_inputs(
            &tools,
            &reentry["source_closure"],
            &threads,
            &saved.simulation.state.vm.world,
        )?;
        let payloads = read(
            "fixtures/private/goth-input-ownership-20261029-second/tool-payloads-second/before.json",
        )?;
        let actual = import(&payloads, &tools, &initial)?;
        for (index, slot) in actual.slots().iter().enumerate() {
            let base = slot.base.as_ref().context("base not imported")?;
            ensure!(
                base.words_04_20 == [0; 8]
                    && base.word_24 == 7
                    && base.tile_overlay == u8::from(index == 3),
                "actual inactive-tool overlay or base scalars lost"
            );
        }
        let after = read(
            "fixtures/private/goth-input-ownership-20261029-second/tool-payloads-second/after.json",
        )?;
        let after_tools = read(&format!("{src}/input-tools-second/after.json"))?;
        ensure!(
            actual == import(&after, &after_tools, &initial)?,
            "actual native bases changed across recorded tick"
        );
        ensure!(
            actual.picker() == initial.picker() && actual.selected() == initial.selected(),
            "base binding changed another retained owner"
        );
        let wire = serde_json::to_value(&actual)?;
        ensure!(
            serde_json::from_value::<State>(wire.clone())? == actual,
            "base codec changed values"
        );
        let mut old = wire;
        for slot in old["slots"].as_array_mut().unwrap() {
            slot.as_object_mut().unwrap().remove("base");
        }
        let old: State = serde_json::from_value(old)?;
        ensure!(
            old == initial,
            "historical bases must stay unknown, not default-constructed"
        );
        ensure!(
            import(&payloads, &tools, &actual).is_err(),
            "base writer overlap accepted"
        );
        for fault in 0..5 {
            let mut bad = payloads.clone();
            match fault {
                0 => bad["tools"][3]["byte_28"] = 0.into(),
                1 => bad["tools"][3]["factory_allocation"]["bytes"] = 0x70.into(),
                2 => {
                    bad["tools"].as_array_mut().unwrap().pop();
                }
                3 => bad["source_observation_sha256"] = "wrong".into(),
                _ => bad["tools"][0]["payload_sha256"] = "wrong".into(),
            }
            ensure!(
                import(&bad, &tools, &initial).is_err(),
                "altered tool base/extent/source accepted"
            );
        }
        ensure!(
            postcard::to_allocvec(&saved.simulation.state.vm)? == before_vm,
            "pure base import changed VM/RNG"
        );
        Ok(())
    }
}
