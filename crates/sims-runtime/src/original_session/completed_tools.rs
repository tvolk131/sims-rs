//! Qualified current-tool/listener and picker fields at the actual Goth cuts.
//! No constructor, callback, resource lookup, scheduler, or VM mutation occurs.
use super::{completed_lot::Memory, current_tool};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_vm::{ObjectId, interface::current_tool::Listeners};
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private actual original tool readsets"]
    fn actual_tools_reject_missing_owners_and_false_aliases() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let load = |p: &str| -> Result<Value> {
            Ok(serde_json::from_slice(&std::fs::read(root.join(p))?)?)
        };
        let reentry = load(
            "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json",
        )?;
        let before = load(
            "fixtures/private/goth-supplemental-owners-20261029-first/input-tools-second/before.json",
        )?;
        let after = load(
            "fixtures/private/goth-supplemental-owners-20261029-first/input-tools-second/after.json",
        )?;
        let a = import(&before, &reentry["source_closure"])?;
        let b = import(&after, &reentry["source_closure"])?;
        ensure!(
            a.current_tool.current() == Some(0x09e20d78)
                && a.picker_target.is_none()
                && a.picker_activation_word == 0
                && a.current_tool == b.current_tool,
            "actual retained tool fields differ"
        );
        let encoded = postcard::to_allocvec(&a.current_tool)?;
        ensure!(
            postcard::from_bytes::<current_tool::State>(&encoded)? == a.current_tool,
            "retained current-tool cold codec differs"
        );
        for i in 0..6 {
            let mut bad = before.clone();
            match i {
                0 => bad["listeners"].as_array_mut().unwrap().clear(),
                1 => bad["listeners"][0]["next"] = 1.into(),
                2 => bad["tool"]["picker"]["activation"] = 1.into(),
                3 => bad["CPState"]["selected_tool"] = 0.into(),
                4 => bad["registry"]["slots"][6]["pointer"] = 0.into(),
                _ => bad["provenance"]["replay_report_sha256"] = "wrong".into(),
            }
            ensure!(
                import(&bad, &reentry["source_closure"]).is_err(),
                "bad tool source {i} admitted"
            );
        }
        Ok(())
    }
    #[test]
    #[ignore = "private original tool inputs, pointer registry and managed World"]
    fn actual_retained_inputs_preserve_scalars_original_actor_and_vm() -> Result<()> {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(retained_actual)?
            .join()
            .map_err(|_| anyhow::anyhow!("retained input test worker panicked"))?
    }
    fn retained_actual() -> Result<()> {
        use crate::original_session::Saved;
        use sims_fixtures::Store;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let load = |path: &str| -> Result<Value> {
            Ok(serde_json::from_slice(&std::fs::read(root.join(path))?)?)
        };
        let p = "fixtures/private/goth-supplemental-owners-20261029-first";
        let before = load(&format!("{p}/input-tools-second/before.json"))?;
        let after = load(&format!("{p}/input-tools-second/after.json"))?;
        let threads = load(&format!("{p}/frames-before-first.json"))?;
        let after_threads = load(&format!("{p}/frames-after-first.json"))?;
        let reentry = load(
            "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json",
        )?;
        let refs = load(&format!("{p}/session-tools-first/refs.json"))?;
        let saved: Saved = Store::open(root.join(format!("{p}/session-tools-first/store")))?
            .get_value(&serde_json::from_value(refs["session"].clone())?)?;
        let vm = &saved.simulation.state.vm;
        let original = postcard::to_allocvec(vm)?;
        let a = import_retained_inputs(&before, &reentry["source_closure"], &threads, &vm.world)?;
        let b = import_retained_inputs(
            &after,
            &reentry["source_closure"],
            &after_threads,
            &vm.world,
        )?;
        ensure!(
            a == b
                && a.slots().len() == 12
                && a.selected().active == Some(0x09e20d78)
                && a.selected().argument == 0
                && a.picker().mode == Some(0)
                && a.picker().cursor == Some(28)
                && a.picker().actor == Some(21),
            "actual retained input values differ"
        );
        let wire = serde_json::to_vec(&a)?;
        ensure!(
            serde_json::from_slice::<super::super::retained_tool_inputs::State>(&wire)? == a,
            "retained input codec differs"
        );
        let mut legacy = serde_json::to_value(&a)?;
        legacy["picker"].as_object_mut().unwrap().remove("cursor");
        let legacy: super::super::retained_tool_inputs::State = serde_json::from_value(legacy)?;
        ensure!(
            legacy.picker().cursor.is_none()
                && legacy.picker().mode == a.picker().mode
                && legacy.picker().actor == a.picker().actor,
            "historical missing cursor must remain unknown, not constructor5 or captured28"
        );
        let mut changed_selection = vm.world.clone();
        *changed_selection
            .objects
            .get_mut(&21)
            .unwrap()
            .native_flags
            .as_mut()
            .unwrap() &= !2;
        *changed_selection
            .objects
            .get_mut(&16)
            .unwrap()
            .native_flags
            .as_mut()
            .unwrap() |= 2;
        ensure!(
            import_retained_inputs(
                &before,
                &reentry["source_closure"],
                &threads,
                &changed_selection
            )?
            .picker()
            .actor
                == Some(21),
            "Picker cache inferred from Rust selection instead of original pointer registry"
        );
        for i in 0..4 {
            let mut bad = threads.clone();
            match i {
                0 => bad["threads"]["objects"][0]["pointer"] = 0.into(),
                1 => bad["threads"]["registry"]["pointer_vector"]["allocated"]["raw"] = "00".into(),
                2 => bad["tick"] = 88449.into(),
                _ => bad["source_observation_sha256"] = "wrong".into(),
            }
            ensure!(
                import_retained_inputs(&before, &reentry["source_closure"], &bad, &vm.world)
                    .is_err(),
                "false original actor registry {i} admitted"
            );
        }
        let mut activated = a.clone();
        activated.activate_picker(0x09e20d78, 1)?;
        ensure!(
            activated.picker().mode == Some(2) && activated.selected() == a.selected(),
            "global Picker activation changed CPState fields"
        );
        activated.set_actor(None);
        ensure!(
            activated.picker().actor.is_none(),
            "native no-person gate retained stale actor"
        );
        ensure!(
            postcard::to_allocvec(vm)? == original,
            "retained tool input import changed VM or RNG"
        );
        Ok(())
    }
}

pub struct Imported {
    pub current_tool: current_tool::State,
    pub picker_target: Option<ObjectId>,
    pub picker_activation_word: u16,
    /// Source audit only. Unrepresented receiver/cache fields are not silently
    /// declared absent or certified by importing these three retained owners.
    pub source: Value,
}
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("retained tool number absent")?
        .try_into()?)
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("retained tool bytes absent")?)
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
pub fn import(v: &Value, closure: &Value) -> Result<Imported> {
    ensure!(
        v["schema"] == 1
            && matches!(v["tick"].as_u64(), Some(88448 | 88449))
            && v["executes_target_code"] == false
            && v["native_parity_ticks"] == 0,
        "retained tool cut profile differs"
    );
    ensure!(
        v["provenance"]["image_sha256"]
            == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && v["provenance"]["replay_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e"
            && closure["image_sha256"] == v["provenance"]["image_sha256"]
            && closure["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c"
            && closure["source_frame_unwind"]["completed_tick_ebp"] == 8518784u32
            && closure["source_frame_unwind"]["returns"]
                == json!(["46da75", "4d1ab8", "4f7563", "5d5e7c"]),
        "retained tool source/caller epoch differs"
    );
    let m = Memory::new(v)?;
    for (a, n, hash) in [
        (
            0x476a35,
            76,
            "9ca88c0479089b0b8b8cdc74b17534f667d68b3d2b38d7def987bfb26a23ff9a",
        ),
        (
            0x48c834,
            176,
            "1a39691424e954a66d1f43e21e011796b9c42a7fe68f7e8821b2d10b06ae4a92",
        ),
        (
            0x48c8e4,
            96,
            "fb0ac9929fbd5f73ab3e630181cdddd888ac9ef8ed255fae9af261800a87275c",
        ),
        (
            0x4cee3d,
            13,
            "87c8ddde4d9ed6baf877e483bf86f55dfe62244e156360d96f2ff300c2651599",
        ),
        (
            0x51d372,
            3,
            "e598d0c3ba86d917b177d7adde0556aa99bc355543c57aee0a3e50b684dd7e99",
        ),
        (
            0x4cef9b,
            90,
            "683b2f20e05021d625ac4a636ab762ac3ed8e60462b99527c8d1cebc52f0080e",
        ),
        (
            0x4847cf,
            30,
            "29ac38e8647217c3bce12314f2d8931094f957b8414c76126c632f9bece8140a",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&m.read(a, n)?) == hash,
            "retained tool instruction profile differs"
        );
    }
    let process = word(&m.read(0x6e48bc, 4)?, 0);
    let pr = m.read(process, 0x20)?;
    ensure!(
        number(&v["process"]["pointer"])? == process
            && bytes(&v["process"]["raw"])? == pr
            && number(&v["process"]["viewer"])? == word(&pr, 4),
        "retained tool process/viewer alias differs"
    );
    let current = word(&m.read(0x6e54fc, 4)?, 0);
    ensure!(
        current != 0 && v["current"] == current && v["tool"]["pointer"] == current,
        "retained current tool alias differs"
    );
    let tr = m.read(current, 0x110)?;
    let vt = m.read(0x667f1c, 0x6c)?;
    ensure!(
        tr == bytes(&v["tool"]["raw"])?
            && word(&tr, 0) == 0x667f1c
            && v["tool"]["vtable"] == 0x667f1cu32
            && vt == bytes(&v["tool"]["vtable_raw"])?,
        "retained current Picker class/storage differs"
    );
    let app = word(&m.read(0x6f2a58, 4)?, 0);
    let cp = word(&m.read(app + 0xb0, 4)?, 0);
    let cpraw = m.read(cp, 0x220)?;
    ensure!(
        v["CPState"]["pointer"] == cp
            && bytes(&v["CPState"]["raw"])? == cpraw
            && word(&cpraw, 0) == 0x668d34
            && word(&cpraw, 0x4c) == current
            && v["CPState"]["selected_tool"] == current,
        "retained CPState/tool alias differs"
    );
    let head = word(&m.read(0x6e5500, 4)?, 0);
    let rows = v["listeners"].as_array().context("tool listeners absent")?;
    ensure!(
        rows.len() == 1 && head == word(&pr, 4) && v["listener_head"] == head,
        "retained tool viewer/listener cohort differs"
    );
    let row = &rows[0];
    let lr = m.read(head, 8)?;
    let lv = m.read(0x668744, 0x34)?;
    ensure!(
        row["pointer"] == head
            && lr == bytes(&row["raw"])?
            && word(&lr, 0) == 0x668744
            && word(&lr, 4) == 0
            && row["next"] == 0
            && row["vtable"] == 0x668744u32
            && lv == bytes(&row["vtable_raw"])?
            && word(&lv, 0x2c) == 0x51d372
            && word(&lv, 0x30) == 0x51d372
            && row["old"] == 0x51d372u32
            && row["new"] == 0x51d372u32,
        "retained tool listener order/callbacks differ"
    );
    // Native viewer pointers map to the one portable household listener ID.
    // No listener registration is replayed; the actual chain is singleton.
    let listeners = Listeners {
        head: Some(1),
        next: [(1, None)].into(),
    };
    let target = i16::from_le_bytes(tr[0x2e..0x30].try_into()?);
    let activation = u16::from_le_bytes(tr[0x40..0x42].try_into()?);
    ensure!(
        v["tool"]["picker"]["target"] == target
            && v["tool"]["picker"]["activation"] == activation
            && v["tool"]["picker"]["actor_pointer"] == word(&tr, 0x100),
        "retained picker decoded fields differ"
    );
    let registry = word(&pr, 0x18);
    let rr = m.read(registry, 4)?;
    let slots_pointer = word(&rr, 0);
    let slots_raw = m.read(slots_pointer, 48)?;
    let slots = v["registry"]["slots"]
        .as_array()
        .context("stock tools absent")?;
    ensure!(
        v["registry"]["pointer"] == registry
            && bytes(&v["registry"]["raw"])? == rr
            && v["registry"]["slots_pointer"] == slots_pointer
            && bytes(&v["registry"]["slots_raw"])? == slots_raw
            && slots.len() == 12
            && word(&slots_raw, 6 * 4) == current,
        "retained stock-tool aliases differ"
    );
    for (i, row) in slots.iter().enumerate() {
        let p = word(&slots_raw, i * 4);
        ensure!(
            p != 0
                && row["index"] == i
                && row["pointer"] == p
                && row["vtable"] == word(&m.read(p, 4)?, 0),
            "retained stock-tool class differs"
        );
    }
    Ok(Imported {
        current_tool: current_tool::State::from_completed_inputs(Some(current), listeners),
        picker_target: (target != 0).then_some(target),
        picker_activation_word: activation,
        source: json!({"schema":1,"tick":v["tick"],"represented":["current_tool_and_ordered_listeners","picker_target","picker_activation_word"],
            "native_current_tool":current,"native_listener":head,"native_picker_mode":word(&tr,0x3c),
            "native_picker_cached_actor":word(&tr,0x100),"stock_tools":slots,
            "unrepresented":["stock_tool_receiver_state","picker_cached_actor","picker_mode"],
            "no_constructors_callbacks_or_VM_changes":true,"complete_native_Session_admitted":false}),
    })
}

/// Retain actual process inputs without constructing the indexed-tool/Move
/// driver. Actor identity comes from the original pointer registry, never the
/// currently selected Rust person. The source map is the qualified raw arena
/// observation used by the full frame comparison at this same boundary.
pub fn import_retained_inputs(
    v: &Value,
    closure: &Value,
    threads: &Value,
    world: &sims_vm::World,
) -> Result<super::retained_tool_inputs::State> {
    use super::retained_tool_inputs::{Picker, Slot, State};
    let _qualified = import(v, closure)?;
    ensure!(
        threads["schema"] == 1
            && threads["tick"] == v["tick"]
            && threads["executes_target_code"] == false
            && threads["source_observation_sha256"]
                == if v["tick"] == 88448 {
                    "949d6e08125f8af0a374147ff01d430863e0f65c69972dba4e1406b178cfddcb"
                } else {
                    "eccec60015b2c9d706a0cac9fa78e4f158464bc8f51a5429fb227d0b89ee2e41"
                }
            && threads["threads"]["boundary"]["tick"] == v["tick"],
        "retained tools and original actor registry cuts differ"
    );
    let registry = &threads["threads"]["registry"]["pointer_vector"];
    let header = bytes(&registry["header"]["raw"])?;
    ensure!(
        header.len() == 12
            && registry["header"]["bytes"] == 12
            && registry["header"]["sha256"] == sims_fixtures::digest(&header),
        "original actor pointer-vector header differs"
    );
    let (a, b, c) = (word(&header, 0), word(&header, 4), word(&header, 8));
    let pointers = bytes(&registry["allocated"]["raw"])?;
    ensure!(
        a >= 0x10000
            && a <= b
            && b <= c
            && (b - a) % 4 == 0
            && (c - a) % 4 == 0
            && (c - a) / 4 <= 32767
            && registry["begin"] == a
            && registry["end"] == b
            && registry["capacity"] == c
            && registry["allocated"]["address"] == a
            && registry["allocated"]["bytes"] == pointers.len()
            && pointers.len() == (c - a) as usize
            && registry["allocated"]["sha256"] == sims_fixtures::digest(&pointers),
        "original actor pointer registry extent/hash differs"
    );
    let mut actor_map = std::collections::BTreeMap::new();
    let mut ids = std::collections::BTreeSet::new();
    for row in threads["threads"]["objects"]
        .as_array()
        .context("original actor map absent")?
    {
        let id: ObjectId = row["id"]
            .as_i64()
            .context("original actor id absent")?
            .try_into()?;
        let pointer = number(&row["pointer"])?;
        let h = bytes(&row["thread_header"]["raw"])?;
        ensure!(
            id > 0
                && (id as u32) * 4 <= b - a
                && word(&pointers, (id as usize - 1) * 4) == pointer
                && pointer >= 0x10000
                && row["thread_header"]["address"] == pointer
                && h.len() == 44
                && row["thread_header"]["bytes"] == h.len()
                && row["thread_header"]["sha256"] == sims_fixtures::digest(&h)
                && actor_map.insert(pointer, id).is_none()
                && ids.insert(id),
            "original actor/lifetime registry alias differs"
        );
    }
    ensure!(
        ids.iter().eq(world.objects.keys()),
        "original actor registry cohort differs from supplied World"
    );
    let m = Memory::new(v)?;
    let pointer = number(&v["current"])?;
    let raw = m.read(pointer, 0x110)?;
    ensure!(
        raw[0x30..0x3c] == [0; 12],
        "nonempty captured Picker menu vector requires its retained publication owner"
    );
    let actor_pointer = word(&raw, 0x100);
    let actor = if actor_pointer == 0 {
        None
    } else {
        let id = *actor_map
            .get(&actor_pointer)
            .context("cached Picker actor is outside actual original registry")?;
        ensure!(
            world.objects[&id].person.is_some(),
            "cached Picker actor is not an original person"
        );
        Some(id)
    };
    let cp = m.read(number(&v["CPState"]["pointer"])?, 0x220)?;
    let slots: Vec<_> = v["registry"]["slots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            Ok(Slot {
                handle: number(&r["pointer"])?,
                class: number(&r["vtable"])?,
                base: None,
            })
        })
        .collect::<Result<_>>()?;
    let slots: [Slot; 12] = slots
        .try_into()
        .map_err(|_| anyhow::anyhow!("retained stock-tool count differs"))?;
    State::from_completed_inputs(
        slots,
        sims_vm::interface::tool::Fields {
            active: (word(&cp, 0x4c) != 0).then_some(word(&cp, 0x4c)),
            argument: word(&cp, 0x50),
        },
        Picker {
            handle: pointer,
            mode: Some(word(&raw, 0x3c) as i32),
            cursor: Some(word(&raw, 0x68) as i32),
            actor,
        },
    )
}
