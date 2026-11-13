use super::*;
use crate::original_session::{completed_geometry::bytes, completed_lot::Memory};
const CODE: &[(u32, usize, &str)] = &[
    (
        0x40922f,
        38,
        "59a56c853a6a25ff23f701868dd0703a503db6ac3dfd4cafbfeaafd0c7696d85",
    ),
    (
        0x409255,
        192,
        "3fb1e95ef93ddd88dfccdceffad41b23ada56d1c59cb3698b097891de66d2a3c",
    ),
    (
        0x40895a,
        418,
        "33bef89e54b19221e35facdc0f76ff129fb50a68044ab4672ad71f0dde1c2519",
    ),
];
pub(in crate::original_session) fn import(v: &Value) -> Result<sims_content::clothing_catalog::Cache> {
    ensure!(
        v["schema"] == 1
            && v["tick"] == 88448
            && v["executes_target_code"] == false
            && v["populates_caches"] == false
            && v["provenance"]["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && v["provenance"]["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
        "clothing source profile differs"
    );
    let m = Memory::new(v)?;
    let code = v["code"].as_array().context("clothing code absent")?;
    ensure!(code.len() == CODE.len(), "clothing code coverage differs");
    for &(p, n, hash) in CODE {
        let rows: Vec<_> = code.iter().filter(|c| c["address"] == p).collect();
        ensure!(rows.len() == 1, "clothing code identity differs");
        let c = rows[0];
        let raw = bytes(c, "raw")?;
        ensure!(
            c["bytes"] == n
                && c["sha256"] == hash
                && sims_fixtures::digest(&raw) == hash
                && m.read(p, n)? == raw,
            "clothing code witness differs"
        );
    }
    let mut lists = vec![];
    for (key, p, count) in [("bodies", 0x6e48a0, 2186), ("textures", 0x6e48b0, 4431)] {
        let c = &v["caches"][key];
        let h = &c["header"];
        let raw = bytes(c, "raw")?;
        let header = m.read(p, 12)?;
        let words: Vec<_> = header
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        ensure!(
            c["address"] == p
                && h["begin"] == words[0]
                && h["end"] == words[1]
                && h["capacity"] == words[2]
                && words[1].checked_sub(words[0]) == Some(raw.len().try_into()?)
                && raw.len() == count * 76
                && words[0] <= words[1]
                && words[1] <= words[2]
                && m.read(words[0], raw.len())? == raw,
            "clothing current vector ownership differs"
        );
        let values = c["values"]
            .as_array()
            .context("clothing cached names absent")?;
        ensure!(values.len() == count, "clothing name coverage differs");
        let mut names = vec![];
        for (i, row) in raw.chunks_exact(76).enumerate() {
            let pointer = u32::from_le_bytes(row[4..8].try_into()?);
            let length = usize::from(u16::from_le_bytes(row[8..10].try_into()?));
            ensure!(
                pointer == words[0] + (i * 76 + 12) as u32
                    && row[10..12] == [64, 0]
                    && length < 64
                    && row[12 + length] == 0,
                "clothing fixed string ownership differs"
            );
            let name = row[12..12 + length].to_vec();
            ensure!(
                crate::original_session::completed_animations::bytes(
                    values[i].as_str().context("clothing narrow bytes absent")?
                )? == name,
                "clothing name aliases differ"
            );
            names.push(name);
        }
        lists.push(names);
    }
    sims_content::clothing_catalog::Cache::from_retained(sims_content::clothing_catalog::Names {
        bodies: lists.remove(0),
        textures: sims_sim::pose::clothing::texture_names::Cache {
            names: lists.remove(0),
        },
    })
}
