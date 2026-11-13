//! Reuse the already observed Program additions from the completed World join.
//! Providers without a linked module keep an UNKNOWN content hash; this is not
//! whole-Program file-generation admission or an immutable asset fallback.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use sims_vm::{Program, body_profiles};
pub fn join(mut program: Program, input: &Value) -> Result<(Program, body_profiles::State)> {
    program.neighbor_resource_flags =
        Some(serde_json::from_value(input["resource_flags"].clone())?);
    let mut storage = vec![];
    for d in input["storage_definitions"]
        .as_array()
        .context("native resource-global definitions absent")?
    {
        let source = d["source"].as_str().context("catalog source absent")?;
        storage.push(sims_vm::resource_storage::Definition {
            guid: u32::try_from(d["guid"].as_u64().context("catalog GUID absent")?)?,
            count: i16::try_from(d["count"].as_i64().context("catalog count absent")?)?,
            reset_per_lot: d["reset_per_lot"]
                .as_bool()
                .context("catalog reset flag absent")?,
            source: source.into(),
            object: u16::try_from(d["object"].as_u64().context("catalog chunk absent")?)?,
            content_hash: program
                .modules
                .get(source)
                .map(|m| m.content_hash.clone())
                .unwrap_or_default(),
        });
    }
    program.resource_storage = Some(storage);
    let mut banks = serde_json::Map::new();
    for bank in input["banks"].as_array().context("current banks absent")? {
        let source = bank["source"]
            .as_str()
            .context("current bank source absent")?;
        let id = u16::try_from(bank["bank"].as_u64().context("bank id absent")?)?;
        let raw = super::super::completed_animations::bytes(
            bank["payload"].as_str().context("bank payload absent")?,
        )?;
        let table = sims_formats::strings::StringTable::parse(&raw)?;
        let values: Vec<_> = table
            .entries
            .iter()
            .filter(|e| e.language == 1)
            .map(|e| e.text.clone())
            .collect();
        program
            .modules
            .get_mut(source)
            .context("current bank Program module absent")?
            .strings
            .insert(id, values.clone());
        banks.entry(source).or_insert_with(|| json!({}))[id.to_string()] = json!(values);
    }
    let profiles =
        serde_json::from_value(json!({"definitions":input["definitions"],"banks":banks}))?;
    Ok((program, profiles))
}
