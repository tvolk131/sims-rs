//! Retain observed initialized editor storage, shared catalogs and empty edit
//! owners. No native or replacement constructor, lookup, callback or VM runs.
use super::{
    completed_lot::Memory,
    completed_tool_payloads,
    retained_tool_inputs::{
        State,
        stock::{self, Alias, EmptyPreview, Stock},
    },
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_sim::object_catalog::move_tool::{Group, Groups};
#[cfg(test)]
mod tests;

fn word(raw: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(raw[off..off + 4].try_into().unwrap())
}
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64().context("stock integer absent")?.try_into()?)
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("stock source bytes absent")?)
}
fn source(v: &Value, tick: &Value) -> Result<()> {
    ensure!(
        v["tick"] == *tick
            && v["executes_target_code"] == false
            && v["source_observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c"
            && v["source_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e"
            && v["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f",
        "stock source epoch differs"
    );
    Ok(())
}
fn reasons(uitext: &[u8], id: u16, count: usize) -> Result<Vec<Vec<u8>>> {
    ensure!(
        sims_fixtures::digest(uitext)
            == "e73b8f48ddfbbb9c476854b131aa68d4715495a1b7b88f4b23c841f399d95f44",
        "stock immutable UIText differs"
    );
    let iff = sims_formats::iff::Iff::parse(uitext.to_vec())?;
    let chunks: Vec<_> = iff
        .all(b"STR#")
        .filter(|c| c.id == id && c.flags & 4 == 0)
        .collect();
    ensure!(chunks.len() == 1, "ambiguous stock UIText table");
    let strings = sims_formats::strings::StringTable::parse(iff.data(chunks[0]))?;
    ensure!(strings.version == -3, "stock UIText encoding differs");
    let rows: Vec<_> = strings
        .entries
        .into_iter()
        .filter(|e| e.language == 1)
        .take(count)
        .map(|e| e.raw_text)
        .collect();
    ensure!(
        rows.len() == count,
        "stock UIText English reason cohort differs"
    );
    Ok(rows)
}
fn strings(
    memory: &Memory,
    descriptors: &[u8],
    rows: &Value,
    expected: &[Vec<u8>],
) -> Result<Vec<Vec<u8>>> {
    let rows = rows.as_array().context("stock cached reason rows absent")?;
    ensure!(
        rows.len() == expected.len() && descriptors.len() == 12 * rows.len(),
        "stock reason descriptor count differs"
    );
    let mut result = vec![];
    for (i, (row, want)) in rows.iter().zip(expected).enumerate() {
        let [begin, end, capacity] = [0, 4, 8].map(|n| word(descriptors, i * 12 + n));
        ensure!(
            begin <= end
                && end <= capacity
                && capacity - begin <= 4096
                && row["index"] == i
                && row["begin"] == begin
                && row["end"] == end
                && row["capacity"] == capacity,
            "stock cached text extent/projection differs"
        );
        let actual = memory.read(begin, (end - begin) as usize)?;
        ensure!(
            actual == *want && actual == bytes(&row["logical_raw"])?,
            "stock reason cache differs from immutable UIText"
        );
        result.push(actual);
    }
    Ok(result)
}
pub fn import(
    payload: &Value,
    catalogs: &Value,
    tools: &Value,
    state: &State,
    uitext: &[u8],
) -> Result<State> {
    source(payload, &tools["tick"])?;
    source(catalogs, &tools["tick"])?;
    ensure!(
        catalogs["schema"] == 1
            && catalogs["code_sha256"]
                == "a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321",
        "stock catalog schema/code differs"
    );
    let raw = completed_tool_payloads::actual_payloads(payload, tools, state)?;
    for (bytes, slot) in raw.iter().zip(state.slots()) {
        ensure!(
            slot.base.as_ref() == Some(&completed_tool_payloads::base(bytes)),
            "stock binding conflicts with existing base"
        );
    }
    let m = Memory::new(payload)?;
    let c = Memory::new(catalogs)?;
    let history = number(&payload["history"]["pointer"])?;
    let h = m.read(history, 0x1c)?;
    ensure!(
        h == bytes(&payload["history"]["raw"])? && word(&h, 0) == 0x6681bc && h[4..] == [0; 24],
        "nonempty/unknown native edit history requires a complete command import"
    );
    ensure!(
        word(&raw[7], 0x34) == 1
            && [0x20, 0x30, 0x3c, 0x44]
                .iter()
                .all(|&o| word(&raw[7], o) == 0),
        "retained Move has pending gameplay work"
    );
    let mut aliases = vec![];
    for (slot, offset) in [
        (2, 0x68),
        (3, 0x34),
        (5, 0x2c),
        (7, 0x2c),
        (9, 0x30),
        (10, 0x50),
    ] {
        ensure!(
            word(&raw[slot], offset) == history,
            "native history/tool alias differs"
        );
        aliases.push(Alias {
            slot: slot as u8,
            offset: offset as u16,
            handle: history,
        });
    }
    let rows = payload["preview_commands"]
        .as_array()
        .context("native previews absent")?;
    ensure!(rows.len() == 3, "native preview cohort differs");
    let mut previews = vec![];
    for (i, (row, (slot, field, size, parent, push))) in rows
        .iter()
        .zip([
            (2, 0x5c, 0xe068, 0xe044, 0x48ae1a),
            (3, 0x30, 0x44, 0x3c, 0x489e6c),
            (5, 0x30, 0x34, 0x30, 0x4885bd),
        ])
        .enumerate()
    {
        let handle = word(&raw[slot], field);
        ensure!(
            row["slot"] == slot
                && row["pointer"] == handle
                && row["bytes"] == size
                && row["tool_field"] == field
                && row["parent_offset"] == parent
                && row["factory"] == push,
            "native preview allocation/alias profile differs"
        );
        let expected = if size < 128 {
            vec![0x6a, size as u8]
        } else {
            let mut b = vec![0x68];
            b.extend_from_slice(&(size as u32).to_le_bytes());
            b
        };
        ensure!(
            m.read(push, expected.len())? == expected,
            "preview factory size witness differs"
        );
        let p = m.read(handle, size)?;
        ensure!(
            p == bytes(&row["raw"])?
                && row["sha256"] == sims_fixtures::digest(&p)
                && word(&p, parent) == state.slots()[slot].handle
                && word(&p, 4) == 0
                && word(&p, 8) == 1
                && p[0xc..0x24] == [0; 24],
            "preview has unknown lifetime/state/dependencies"
        );
        let records = if slot == 2 {
            &p[0x24..0xe024]
        } else {
            &p[0x24..0x30]
        };
        ensure!(
            records.iter().all(|&b| b == 0),
            "nonempty native preview records require a complete edit import"
        );
        previews.push(EmptyPreview {
            slot: slot as u8,
            handle,
            class: word(&p, 0),
            references: word(&p, 8),
            fields: stock::project(&p, stock::PREVIEW_FIELDS[i]),
        });
    }
    let rows = catalogs["move_groups"]
        .as_array()
        .context("Move groups absent")?;
    ensure!(rows.len() == 7, "Move group cohort differs");
    let mut groups = vec![];
    for (i, row) in rows.iter().enumerate() {
        let address = 0x6e57a8 - 24 * i as u32;
        let raw = c.read(address, 24)?;
        ensure!(
            row["group"] == i + 1 && row["pointer"] == address && raw == bytes(&row["raw"])?,
            "Move category/address order differs"
        );
        let [begin, end, capacity] = [0, 4, 8].map(|o| word(&raw, o));
        ensure!(
            begin <= end
                && end <= capacity
                && (end - begin) % 4 == 0
                && (capacity - begin) % 4 == 0
                && capacity - begin <= 1 << 18
                && row["begin"] == begin
                && row["end"] == end
                && row["capacity"] == capacity,
            "Move source group extent differs"
        );
        let allocated = c.read(begin, (capacity - begin) as usize)?;
        ensure!(
            allocated == bytes(&row["allocated_raw"])?,
            "Move group allocation receipt differs"
        );
        let guids: Vec<_> = (0..(end - begin) as usize)
            .step_by(4)
            .map(|n| word(&allocated, n))
            .collect();
        ensure!(
            serde_json::to_value(&guids)? == row["logical_words"],
            "Move group logical projection differs"
        );
        groups.push(Group {
            guids,
            first: word(&raw, 12) as i32,
            last: word(&raw, 16) as i32,
        });
    }
    let picker_shared = c.read(0x6e57ec, 4)?;
    ensure!(
        picker_shared == [0; 4]
            && picker_shared == bytes(&catalogs["picker_global"]["raw"])?
            && catalogs["picker_global"]["pointer"] == 0x6e57ec,
        "non-null Picker shared owner requires an identity mapping"
    );
    let dirt = c.read(0x6e5814, 8)?;
    ensure!(
        dirt == bytes(&catalogs["dirt"]["raw"])?
            && catalogs["dirt"]["global_pointer"] == 0x6e5814
            && catalogs["dirt"]["pointer"] == word(&dirt, 0)
            && catalogs["dirt"]["references"] == word(&dirt, 4),
        "Dirt shared cache owner differs"
    );
    let descriptors = c.read(word(&dirt, 0), 84)?;
    ensure!(
        descriptors == bytes(&catalogs["dirt"]["descriptors_raw"])?,
        "Dirt cached descriptor bytes differ"
    );
    let stock = Stock {
        initialized: std::array::from_fn(|i| stock::project(&raw[i], stock::FIELDS[i])),
        empty_history: history,
        history_aliases: aliases,
        previews: previews
            .try_into()
            .map_err(|_| anyhow::anyhow!("preview cohort differs"))?,
        groups: Groups(
            groups
                .try_into()
                .map_err(|_| anyhow::anyhow!("group cohort differs"))?,
        ),
        picker_reasons: strings(
            &m,
            &raw[6][0x6c..0xd8],
            &payload["picker_strings"],
            &reasons(uitext, 159, 9)?,
        )?,
        dirt_reasons: strings(
            &c,
            &descriptors,
            &catalogs["dirt"]["strings"],
            &reasons(uitext, 149, 7)?,
        )?,
        dirt_references: word(&dirt, 4),
        picker_shared_null: true,
    };
    state.clone().with_stock(stock)
}
