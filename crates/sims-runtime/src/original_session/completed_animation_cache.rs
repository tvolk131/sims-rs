//! Effective Global animation-name vectors, separate from disk STR# decoding.
//! Parsing/diagnostic installation adds no source admission or native tick credit.
use super::completed_lot::Memory;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub mod captured;

const CODE: &[(u32, usize, &str)] = &[
    (
        0x59d860,
        340,
        "e5b81142f2bc8ca948add8e318c63ce3497187651607eb2102fbcdc813b1dd51",
    ),
    (
        0x59db2f,
        259,
        "5d19e68ae0698ce9ad2de72f371d7a6a360a2df19e7a8827bd90fc1bd22c88b1",
    ),
    (
        0x4704fa,
        63,
        "891c15b1fc2a99149d5824094134e3424741207ecf286d6a851672d4b14dee54",
    ),
    (
        0x46fae6,
        28,
        "701b24000696d094fa6fd69036456cfdee9e7dd58b7324dc4c3958afdc857fae",
    ),
    (
        0x401039,
        267,
        "338d2d97fe85cf369d062f4d5752fc436d79d0e179a22e78ab8ad73f744eb72a",
    ),
    (
        0x457b84,
        36,
        "9ec68be2968173df7bc5c6bfe1a6e7b91c800f30240fe8bc3d632ff3f3ce38d4",
    ),
];

fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("animation cache integer absent")?
        .try_into()?)
}
fn word(raw: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap())
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("animation cache bytes absent")?)
}

pub struct GlobalTables {
    pub tick: u64,
    pub source: String,
    tables: BTreeMap<u16, Vec<String>>,
    languages: Vec<u8>,
    canonical_input_sha256: String,
}
impl GlobalTables {
    /// Validate every interpreted value against its actual stopped readset.
    /// Raw language bytes are retained; no disk-language selection occurs.
    /// Non-ASCII/null entries need a richer representation and remain errors.
    pub fn read(input: &Value) -> Result<Self> {
        ensure!(
            input["schema"] == 1
                && input["status"] == "captured"
                && input["executes_target_code"] == false
                && input["native_parity_ticks"] == 0,
            "effective animation cache observation unavailable"
        );
        let memory = Memory::new(input)?;
        for &(address, count, expected) in CODE {
            ensure!(
                sims_fixtures::digest(&memory.read(address, count)?) == expected,
                "effective animation cache code differs"
            );
        }
        let root = number(&input["root"]["pointer"])?;
        ensure!(
            root >= 0x10000 && word(&memory.read(0x6f7698, 4)?, 0) == root,
            "animation cache root differs"
        );
        let manager = number(&input["global_source"]["manager"])?;
        let resources = number(&input["global_source"]["resource_manager"])?;
        let provider = number(&input["global_source"]["provider"])?;
        let source = format!("native-provider:{provider:08x}");
        ensure!(
            manager >= 0x10000
                && resources >= 0x10000
                && provider >= 0x10000
                && word(&memory.read(0x6e48bc, 4)?, 0) == manager
                && word(
                    &memory.read(manager.checked_add(0x28).context("manager overflow")?, 4)?,
                    0
                ) == resources
                && word(
                    &memory.read(
                        resources
                            .checked_add(0x2c)
                            .context("resource owner overflow")?,
                        4
                    )?,
                    0
                ) == provider
                && input["global_source"]["native_source"] == source,
            "animation Global source linkage differs"
        );
        let rows = input["global_tables"]
            .as_array()
            .context("animation tables absent")?;
        ensure!(
            rows.len() == sims_content::vm::NATIVE_ANIMATION_TABLES.len(),
            "animation table closure incomplete"
        );
        let mut tables = BTreeMap::new();
        let mut languages = Vec::new();
        for (ordinal, (&id, row)) in sims_content::vm::NATIVE_ANIMATION_TABLES
            .iter()
            .zip(rows)
            .enumerate()
        {
            let pointer = root
                .checked_add((ordinal as u32) * 0x6c)
                .context("cache overflow")?;
            let raw = memory.read(pointer, 0x6c)?;
            ensure!(
                row["ordinal"] == ordinal
                    && row["selection_table"] == id
                    && number(&row["pointer"])? == pointer
                    && bytes(&row["raw"])? == raw
                    && row["current_language"] == raw[0x5d],
                "animation table owner/order differs"
            );
            let begin = word(&raw, 0x60);
            let end = word(&raw, 0x64);
            let capacity = word(&raw, 0x68);
            ensure!(
                (begin >= 0x10000 || begin == 0 && end == 0 && capacity == 0)
                    && begin <= end
                    && end <= capacity
                    && (end - begin) % 4 == 0
                    && (capacity - begin) % 4 == 0
                    && (capacity - begin) / 4 <= 4096,
                "animation table vector invalid"
            );
            let strings = row["strings"]
                .as_array()
                .context("effective strings absent")?;
            ensure!(
                strings.len() == ((end - begin) / 4) as usize,
                "effective string count differs"
            );
            let mut values = Vec::new();
            for (i, string) in strings.iter().enumerate() {
                let pointer = word(
                    &memory.read(
                        begin
                            .checked_add((i as u32) * 4)
                            .context("string vector overflow")?,
                        4,
                    )?,
                    0,
                );
                ensure!(
                    pointer >= 0x10000
                        && number(&string["pointer"])? == pointer
                        && string["status"] == "captured"
                        && string["terminator_observed"] == true,
                    "null or unavailable effective animation string"
                );
                let mut raw = bytes(&string["raw"])?;
                ensure!(
                    raw.len() <= 4096 && raw.is_ascii() && !raw.contains(&0),
                    "effective animation string requires non-ASCII/null representation"
                );
                raw.push(0);
                ensure!(
                    memory.read(pointer, raw.len())? == raw,
                    "effective animation string readset differs"
                );
                raw.pop();
                values.push(String::from_utf8(raw)?);
            }
            languages.push(raw[0x5d]);
            tables.insert(id, values);
        }
        Ok(Self {
            tick: input["tick"].as_u64().context("animation epoch absent")?,
            source,
            tables,
            languages,
            canonical_input_sha256: sims_fixtures::digest(&serde_json::to_vec(input)?),
        })
    }

    /// Explicit configuration experiment after the accepted comparison prefix.
    /// All writes are staged; captured effective tables are never overwritten.
    /// Private unmaterialized caches remain unresolved, not declared absent.
    pub fn install_diagnostic(&self, program: &mut sims_vm::Program) -> Result<Value> {
        ensure!(
            program.globals == self.source,
            "effective Global provider differs"
        );
        let module = program
            .modules
            .get_mut(&self.source)
            .context("Global module absent")?;
        ensure!(
            module.content_hash.len() == 64
                && module.content_hash.bytes().all(|c| c.is_ascii_hexdigit()),
            "Global file version unbound"
        );
        ensure!(
            self.tables
                .keys()
                .all(|id| !module.strings.contains_key(id)),
            "effective animation input would replace captured tables"
        );
        module.strings.extend(self.tables.clone());
        Ok(
            json!({"kind":"effective_global_animation_tables","observed_tick":self.tick,
            "source":self.source,"canonical_input_sha256":self.canonical_input_sha256,
            "current_languages":self.languages,"tables":self.tables.keys().collect::<Vec<_>>(),
            "private_cache_absence_inferred":false,"native_cache_order_qualified":false,
            "program_fingerprint":program.fingerprint()}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private same-cut original effective animation vectors"]
    fn stored_cache_preserves_effective_values_and_rejects_contradictions() -> Result<()> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/private/goth-animation-cache-20261029-first/capture-first.json");
        let input: Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let profile = GlobalTables::read(&input)?;
        assert_eq!(profile.tick, 88448);
        assert_eq!(profile.tables[&150][20], "a2o-walking-loop");
        assert_eq!(profile.tables[&152][20], "");
        assert_eq!(profile.tables[&158][20], "a2o-pool-swim-loop");
        assert!(profile.languages.iter().all(|l| *l == 0));
        let mut program = sims_vm::Program {
            globals: profile.source.clone(),
            ..Default::default()
        };
        program.modules.insert(
            profile.source.clone(),
            sims_vm::Module {
                source: profile.source.clone(),
                content_hash: "a".repeat(64),
                ..Default::default()
            },
        );
        let before = program.fingerprint();
        profile.install_diagnostic(&mut program)?;
        assert_ne!(before, program.fingerprint());
        let after = program.fingerprint();
        assert!(profile.install_diagnostic(&mut program).is_err());
        assert_eq!(
            program.fingerprint(),
            after,
            "failed install partially changed code inputs"
        );
        for corrupt in 0..5 {
            let mut changed = input.clone();
            match corrupt {
                0 => changed["global_tables"][0]["strings"][20]["raw"] = json!("626164"),
                1 => changed["global_tables"][0]["current_language"] = json!(1),
                2 => changed["global_tables"]
                    .as_array_mut()
                    .unwrap()
                    .pop()
                    .map(|_| ())
                    .unwrap(),
                3 => changed["global_source"]["provider"] = json!(0x12345),
                _ => changed["global_tables"][1]["selection_table"] = json!(150),
            }
            assert!(
                GlobalTables::read(&changed).is_err(),
                "corrupt cache accepted: {corrupt}"
            );
        }
        Ok(())
    }
}
