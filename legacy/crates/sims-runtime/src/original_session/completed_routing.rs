//! Actual object-owned route banks, not a reconstruction from SLOT resources.
use super::completed_lot::Memory;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_vm::{ObjectId, World, routing::Descriptor};
use std::collections::{BTreeMap, BTreeSet};
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("routing source bytes absent")?)
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("routing source pointer absent")?
        .try_into()?)
}
pub fn import(v: &Value, world: &World) -> Result<BTreeMap<ObjectId, Vec<Descriptor>>> {
    ensure!(
        v["schema"] == 1
            && matches!(v["tick"].as_u64(), Some(88448 | 88449))
            && v["executes_target_code"] == false
            && v["native_parity_ticks"] == 0,
        "routing owner source profile differs"
    );
    ensure!(
        v["provenance"]["image_sha256"]
            == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && v["provenance"]["replay_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e",
        "routing owner source epoch differs"
    );
    let m = Memory::new(v)?;
    for (a, n, h) in [
        (
            0x431aaf,
            192,
            "d3d851ef630d01d780632daef5c105a6ff0deec2449a735abb81ee08cb3949a0",
        ),
        (
            0x46e2d1,
            512,
            "123f37421719b65caee79a612e4d166e8727422145067ef8670a9fb6efa2801a",
        ),
        (
            0x45c634,
            320,
            "f4c1a05320978b48902b3e43d17879ee24049aad847843dfc752086760b753c3",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&m.read(a, n)?) == h,
            "routing vector/descriptor instruction mapping differs"
        );
    }
    let rows = v["objects"]
        .as_array()
        .context("routing source cohort absent")?;
    ensure!(
        rows.len() == world.objects.len(),
        "routing owner cohort extent differs"
    );
    let mut out = BTreeMap::new();
    let mut pointers = BTreeSet::new();
    for row in rows {
        let id: ObjectId = row["object"]
            .as_i64()
            .context("routing object id absent")?
            .try_into()?;
        let object = world
            .objects
            .get(&id)
            .context("routing source object absent from World")?;
        let p = number(&row["pointer"])?;
        let raw = m.read(p, 0x188)?;
        ensure!(
            pointers.insert(p)
                && raw == bytes(&row["raw"])?
                && i16::from_le_bytes(raw[0xf0..0xf2].try_into()?) == id,
            "routing object alias/readset differs"
        );
        let (a, b, c) = (word(&raw, 0x13c), word(&raw, 0x140), word(&raw, 0x144));
        ensure!(
            (a >= 0x10000 || a == 0 && b == 0 && c == 0)
                && a <= b
                && b <= c
                && (b - a) % 60 == 0
                && (c - a) % 60 == 0
                && (c - a) / 60 <= 4096,
            "routing vector extent differs"
        );
        let body = if b == a {
            vec![]
        } else {
            m.read(a, (b - a) as usize)?
        };
        ensure!(
            body == bytes(&row["descriptors_raw"])?,
            "routing descriptor body/readset differs"
        );
        let descriptors = body
            .chunks_exact(60)
            .map(|b| {
                let d = Descriptor(std::array::from_fn(|i| word(b, 4 * i)));
                d.validate().map_err(anyhow::Error::msg)?;
                Ok(d)
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            object
                .routing_slots
                .as_ref()
                .is_none_or(|old| *old == descriptors),
            "routing source contradicts already retained descriptors"
        );
        ensure!(
            out.insert(id, descriptors).is_none(),
            "duplicate routing source object"
        );
    }
    ensure!(
        out.keys().eq(world.objects.keys()),
        "routing owner cohort membership differs"
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_fixtures::Store;
    #[test]
    #[ignore = "private actual routing banks and constructorless current World"]
    fn actual_routing_vectors_preserve_every_word_without_touching_vm() -> Result<()> {
        // Full Saved decoding exceeds libtest's small worker stack. This is
        // fixture setup only; no engine stack size or VM behavior changes.
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(actual)?
            .join()
            .map_err(|_| anyhow::anyhow!("routing fixture worker panicked"))?
    }
    fn actual() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let capsule = root.join("fixtures/private/goth-elapsed-parent-20261029-first");
        let v: Value =
            serde_json::from_slice(&std::fs::read(capsule.join("routing-before-first.json"))?)?;
        let refs: Value =
            serde_json::from_slice(&std::fs::read(capsule.join("session-third/refs.json"))?)?;
        let saved: super::super::Saved = Store::open(capsule.join("session-third/store"))?
            .get_value(&serde_json::from_value(refs["session"].clone())?)?;
        let vm = &saved.simulation.state.vm;
        let before = postcard::to_allocvec(vm)?;
        let owners = import(&v, &vm.world)?;
        assert_eq!(owners.len(), 344);
        assert_eq!(owners.values().filter(|r| !r.is_empty()).count(), 119);
        assert_eq!(owners.values().map(Vec::len).sum::<usize>(), 424);
        assert_eq!(before, postcard::to_allocvec(vm)?);
        let mut x = v.clone();
        x["objects"].as_array_mut().unwrap().pop();
        assert!(import(&x, &vm.world).is_err());
        let mut x = v.clone();
        x["objects"][0]["pointer"] = serde_json::json!(0);
        assert!(import(&x, &vm.world).is_err());
        let mut x = v.clone();
        let row = x["objects"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|r| r["descriptors_raw"].as_str().unwrap().len() > 0)
            .unwrap();
        row["descriptors_raw"] = serde_json::json!("00".repeat(60));
        assert!(import(&x, &vm.world).is_err());
        let mut x = v;
        x["provenance"]["image_sha256"] = serde_json::json!("wrong");
        assert!(import(&x, &vm.world).is_err());
        Ok(())
    }
}
