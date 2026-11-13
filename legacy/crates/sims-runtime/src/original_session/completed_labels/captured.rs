//! Actual persistent labels at a current completed cut. The caller must admit
//! and pin the supplemental physical observation; sparse holes are not filled.
use super::*;
use crate::original_session::{completed_animations::bytes, completed_skin};
use sims_fixtures::readset::Readset;

pub fn import(
    source: &Value,
    boundary: &completed_skin::Input,
    state: &Value,
    memory: &Readset<'_>,
) -> Result<BTreeMap<ObjectId, Vec<u8>>> {
    ensure!(
        source["schema"] == 2
            && source["contract"] == "completed-native-person-labels-v2"
            && source["tick"] == state["tick"]
            && source["tick"] == boundary.tick
            && source["source_observation_sha256"] == boundary.observation_sha256
            && source["source_readset_sha256"] == memory.source_sha256()
            && source["physical_compressed_sha256"] == boundary.image_sha256
            && source["original_guest_executed"] == false,
        "current label source epoch differs"
    );
    let code = &source["source_code"];
    let raw = bytes(code["raw"].as_str().context("current label code absent")?)?;
    ensure!(
        code["address"] == 0x45ce82
            && code["bytes"] == 0x55
            && raw.len() == 0x55
            && code["sha256"] == "28309fbacaf293333b7a816cebe69c0c8f9c33707db917f569ffec53a3ba76db"
            && code["sha256"] == sims_fixtures::digest(&raw),
        "current label source code differs"
    );
    completed_skin::captured::overlap(memory, 0x45ce82, &raw)?;
    let mut owners = BTreeMap::new();
    for c in state["characters"]["characters"]
        .as_array()
        .context("current label people absent")?
    {
        let id: ObjectId = serde_json::from_value(c["object"].clone())?;
        let p: u32 = c["person"]
            .as_u64()
            .context("current label person absent")?
            .try_into()?;
        ensure!(
            id > 0 && p != 0 && owners.insert(id, p).is_none(),
            "duplicate current label owner"
        );
    }
    let mut result = BTreeMap::new();
    for row in source["objects"]
        .as_array()
        .context("current label rows absent")?
    {
        let id: ObjectId = serde_json::from_value(row["object"].clone())?;
        let person = *owners.get(&id).context("current label owner absent")?;
        let address = person
            .checked_add(0x18c)
            .context("current label address overflow")?;
        let raw = bytes(row["raw"].as_str().context("current label raw absent")?)?;
        ensure!(
            row["person"] == person
                && row["address"] == address
                && row["bytes"] == raw.len()
                && (1..=1024).contains(&raw.len())
                && raw.last() == Some(&0)
                && !raw[..raw.len() - 1].contains(&0)
                && row["sha256"] == sims_fixtures::digest(&raw),
            "current label storage differs"
        );
        completed_skin::captured::overlap(memory, address, &raw)?;
        ensure!(
            result.insert(id, raw[..raw.len() - 1].to_vec()).is_none(),
            "duplicate current label"
        );
    }
    ensure!(
        !result.is_empty() && result.keys().eq(owners.keys()),
        "current label coverage differs"
    );
    Ok(result)
}
