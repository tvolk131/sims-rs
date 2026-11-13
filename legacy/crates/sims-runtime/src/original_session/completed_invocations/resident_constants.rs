//! Native resident BCON bytes with observed empty OTF owners. No file fallback.
use super::*;
use serde_json::{Value, json};
fn n(v: &Value) -> Result<u32> {
    Ok(v.as_u64().context("BCON word absent")?.try_into()?)
}
fn rows<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    v[key].as_array().context("BCON inventory absent")
}
pub fn join(
    state: &Value,
    sparse: &Readset<'_>,
    proof: &Value,
    m: &Readset<'_>,
    program: &mut Program,
) -> Result<Value> {
    ensure!(
        proof["schema"] == 1
            && proof["contract"] == "completed-native-resident-bcon-closure-v1"
            && proof["tick"] == state["tick"]
            && proof["source_observation_sha256"]
                == "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299"
            && proof["source_readset_sha256"] == sparse.source_sha256()
            && proof["supplement_readset_sha256"] == m.source_sha256()
            && proof["physical_compressed_sha256"]
                == state["physical_baseline"]["archive"]["compressed_sha256"]
            && proof["physical_decoded_sha256"]
                == state["physical_baseline"]["archive"]["decoded_sha256"]
            && proof["executes_target_code"] == false,
        "BCON source epoch differs"
    );
    for r in m.records() {
        super::super::completed_skin::captured::overlap(sparse, r.address, r.bytes)?;
    }
    let providers = super::resident_sources::providers(state, sparse)?;
    let supplied: BTreeSet<_> = rows(proof, "providers")?
        .iter()
        .map(n)
        .collect::<Result<_>>()?;
    ensure!(
        supplied == providers && rows(proof, "providers")?.len() == providers.len(),
        "BCON provider coverage differs"
    );
    let mut overrides = BTreeSet::new();
    for row in rows(proof, "override_vectors")? {
        let p = n(&row["provider"])?;
        let raw = m.read(at(p, 0x244)?, 12)?;
        let expected = crate::original_session::completed_animations::bytes(
            row["header_raw"].as_str().context("OTF header absent")?,
        )?;
        let (begin, end, cap) = (word(&raw, 0), word(&raw, 4), word(&raw, 8));
        ensure!(
            overrides.insert(p)
                && raw == expected
                && begin == end
                && end <= cap
                && (begin >= 0x10000 || cap == 0)
                && (cap - begin) % 8 == 0
                && row["records_raw"] == "",
            "nonempty or invalid OTF owner requires effective override support"
        );
    }
    ensure!(overrides == providers, "OTF owner coverage differs");
    let mut resources = Resources {
        memory: m,
        providers: BTreeMap::new(),
        components: BTreeMap::new(),
        payloads: BTreeMap::new(),
        traversed: 0,
    };
    let mut expected = BTreeMap::new();
    for provider in &providers {
        for component in resources.provider(*provider)? {
            for ((tree, variant), (record, handle)) in
                resources.type_records(component, 0x42434f4e)?
            {
                if variant == 0 {
                    expected
                        .entry((*provider, tree))
                        .or_insert((component, record, handle));
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut pending = Vec::new();
    let mut existing = 0;
    for row in rows(proof, "tables")? {
        let provider = n(&row["provider"])?;
        let tree: u16 = n(&row["tree"])?.try_into()?;
        let handle = n(&row["handle"])?;
        ensure!(
            handle != 0
                && seen.insert((provider, tree))
                && expected.get(&(provider, tree))
                    == Some(&(n(&row["component"])?, n(&row["record"])?, handle)),
            "BCON precedence/identity differs"
        );
        let header = m.read(handle, 8)?;
        let data = word(&header, 0);
        let size = word(&header, 4) as usize;
        ensure!(
            (2..=1 << 20).contains(&size) && row["data"] == data && row["bytes"] == size,
            "BCON handle differs"
        );
        let bytes = m.read(data, size)?;
        ensure!(
            row["sha256"] == sims_fixtures::digest(&bytes),
            "BCON payload differs"
        );
        let values = sims_formats::execution::constants(&bytes)?;
        let source = name(provider);
        if let Some(old) = program
            .modules
            .get(&source)
            .and_then(|m| m.constants.get(&tree))
        {
            ensure!(*old == values, "native BCON conflicts with Program");
            existing += 1;
        } else {
            pending.push((source, tree, values));
        }
    }
    for row in rows(proof, "nonresident_demands")? {
        let p = n(&row["provider"])?;
        let tree: u16 = n(&row["tree"])?.try_into()?;
        ensure!(
            seen.insert((p, tree))
                && expected.get(&(p, tree))
                    == Some(&(n(&row["component"])?, n(&row["record"])?, 0)),
            "lazy BCON precedence differs"
        );
        ensure!(
            program
                .modules
                .get(&name(p))
                .is_none_or(|m| !m.constants.contains_key(&tree)),
            "nonresident BCON substituted in Program"
        );
    }
    ensure!(
        seen.iter().eq(expected.keys()),
        "BCON resident/lazy coverage differs"
    );
    let added = pending.len();
    for (source, tree, values) in pending {
        program
            .modules
            .entry(source.clone())
            .or_insert_with(|| Module {
                source,
                ..Module::default()
            })
            .constants
            .insert(tree, values);
    }
    Ok(
        json!({"kind":"current_resident_bcon_closure","tables":added+existing,"added":added,"already_bound":existing,"observed_empty_OTF_owners":providers.len(),"lazy_demands":rows(proof,"nonresident_demands")?.len(),"source_readset_sha256":m.source_sha256(),"native_parity_credit":0}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "actual native B resident tuning maps and OTF owner receipts"]
    fn resident_constants_reject_bad_payloads_overrides_and_lazy_substitution() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source=root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-00");
        let output = root.join("fixtures/private/goth-route-physical-20261030-first");
        let encoded = std::fs::read(source.join("observation.json.gz"))?;
        ensure!(
            sims_fixtures::digest(&encoded)
                == "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299",
            "native B differs"
        );
        let state: Value = serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw = std::fs::read(source.join("acknowledged.mem"))?;
        let sparse = Readset::parse(&raw)?;
        let raw = std::fs::read(output.join("resident-bcon-B-first.mem"))?;
        let memory = Readset::parse(&raw)?;
        let raw = std::fs::read(output.join("resident-bcon-B-first.json"))?;
        ensure!(
            sims_fixtures::digest(&raw)
                == "b6c5877142070c1bcd87886b17e8450dab6e5a909d6a9ee78a622175c2b79a17",
            "native BCON proof differs"
        );
        let proof: Value = serde_json::from_slice(&raw)?;
        let mut program = Program::default();
        let result = join(&state, &sparse, &proof, &memory, &mut program)?;
        ensure!(
            result["tables"] == 84
                && result["observed_empty_OTF_owners"] == 146
                && result["lazy_demands"] == 403
                && program.modules["native-provider:0862d328"].constants[&4096].len() == 17,
            "native tuning closure differs"
        );
        let mut candidate = Program::default();
        let before = serde_json::to_vec(&candidate)?;
        let mut bad = proof.clone();
        bad["tables"].as_array_mut().unwrap().last_mut().unwrap()["sha256"] = json!("0".repeat(64));
        ensure!(
            join(&state, &sparse, &bad, &memory, &mut candidate).is_err()
                && serde_json::to_vec(&candidate)? == before,
            "late bad constant receipt mutated Program"
        );
        let mut bad = proof.clone();
        bad["override_vectors"][0]["records_raw"] = json!("0000000000000000");
        ensure!(
            join(&state, &sparse, &bad, &memory, &mut candidate).is_err()
                && serde_json::to_vec(&candidate)? == before,
            "fabricated override accepted"
        );
        let lazy = &proof["nonresident_demands"][0];
        let source = name(n(&lazy["provider"])?);
        let table: u16 = n(&lazy["tree"])?.try_into()?;
        candidate
            .modules
            .entry(source.clone())
            .or_insert_with(|| Module {
                source,
                ..Module::default()
            })
            .constants
            .insert(table, vec![0]);
        let before = serde_json::to_vec(&candidate)?;
        ensure!(
            join(&state, &sparse, &proof, &memory, &mut candidate).is_err()
                && serde_json::to_vec(&candidate)? == before,
            "lazy constant substitution accepted or mutated Program"
        );
        Ok(())
    }
}
