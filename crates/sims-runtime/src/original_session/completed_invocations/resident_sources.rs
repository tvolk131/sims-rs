//! Same-B resident routine closure, checked again through the native by-ID map.
//! Future lazy records remain demands; no disk file stands in for a live owner.
use super::*;
use serde_json::{Value, json};

fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("resident BHAV word absent")?
        .try_into()?)
}
fn rows<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    v[key].as_array().context("resident BHAV inventory absent")
}
pub(super) fn providers(state: &Value, sparse: &Readset<'_>) -> Result<BTreeSet<u32>> {
    let mut providers: BTreeSet<_> = state["threads"]["files"]
        .as_array()
        .context("native files absent")?
        .iter()
        .filter(|f| f["kind"] == "aggregate")
        .map(|f| number(&f["pointer"]))
        .collect::<Result<_>>()?;
    for context in state["threads"]["contexts"]
        .as_array()
        .context("native source contexts absent")?
    {
        let raw = crate::original_session::completed_animations::bytes(
            context["storage"]["raw"]
                .as_str()
                .context("native source context bytes absent")?,
        )?;
        ensure!(
            raw.len() == 24
                && word(&raw, 0) == 0x666514
                && sparse.read(number(&context["pointer"])?, 24)? == raw,
            "native source context receipt differs"
        );
        for offset in [4, 8, 12] {
            let pointer = word(&raw, offset);
            if pointer != 0 {
                providers.insert(pointer);
            }
        }
    }
    Ok(providers)
}
pub fn join(
    state: &Value,
    sparse: &Readset<'_>,
    proof: &Value,
    memory: &Readset<'_>,
    program: &mut Program,
) -> Result<Value> {
    ensure!(
        proof["schema"] == 1
            && proof["contract"] == "completed-native-resident-bhav-closure-v1"
            && proof["tick"] == state["tick"]
            && proof["source_observation_sha256"]
                == "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299"
            && proof["source_readset_sha256"] == sparse.source_sha256()
            && proof["supplement_readset_sha256"] == memory.source_sha256()
            && proof["physical_compressed_sha256"]
                == state["physical_baseline"]["archive"]["compressed_sha256"]
            && proof["physical_decoded_sha256"]
                == state["physical_baseline"]["archive"]["decoded_sha256"]
            && proof["executes_target_code"] == false,
        "resident BHAV source epoch differs"
    );
    for r in memory.records() {
        super::super::completed_skin::captured::overlap(sparse, r.address, r.bytes)?;
    }
    let providers = providers(state, sparse)?;
    let supplied: BTreeSet<_> = proof["providers"]
        .as_array()
        .context("resident providers absent")?
        .iter()
        .map(number)
        .collect::<Result<_>>()?;
    ensure!(
        providers == supplied && proof["providers"].as_array().unwrap().len() == providers.len(),
        "resident provider coverage differs"
    );
    let mut resources = Resources {
        memory,
        providers: BTreeMap::new(),
        components: BTreeMap::new(),
        payloads: BTreeMap::new(),
        traversed: 0,
    };
    let mut expected = BTreeMap::new();
    for provider in &providers {
        for component in resources.provider(*provider)? {
            resources.component(component)?;
            for (&(tree, variant), &(record, handle)) in &resources.components[&component] {
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
    for row in rows(proof, "routines")? {
        let provider = number(&row["provider"])?;
        let tree: u16 = number(&row["tree"])?.try_into()?;
        ensure!(seen.insert((provider, tree)), "duplicate resident routine");
        let (payload, receipt) = resources.routine(provider, tree)?;
        ensure!(
            expected.get(&(provider, tree))
                == Some(&(receipt.component, receipt.record, receipt.handle))
                && row["component"] == receipt.component
                && row["record"] == receipt.record
                && row["handle"] == receipt.handle
                && row["data"] == receipt.data
                && row["bytes"] == receipt.bytes
                && row["sha256"] == receipt.sha256,
            "resident routine native precedence or payload receipt differs"
        );
        let source = name(provider);
        let body = sims_formats::behavior::Behavior::parse(&payload)?;
        if let Some(old) = program
            .modules
            .get(&source)
            .and_then(|m| m.routines.get(&tree))
        {
            ensure!(
                serde_json::to_vec(&old.body)? == serde_json::to_vec(&body)?,
                "resident routine conflicts with retained Program"
            );
            existing += 1;
        } else {
            pending.push((
                source,
                tree,
                Routine {
                    name: format!("resident:{tree}"),
                    body,
                },
            ));
        }
    }
    for row in rows(proof, "nonresident_demands")? {
        let provider = number(&row["provider"])?;
        let tree: u16 = number(&row["tree"])?.try_into()?;
        ensure!(
            seen.insert((provider, tree))
                && expected.get(&(provider, tree))
                    == Some(&(number(&row["component"])?, number(&row["record"])?, 0)),
            "lazy routine record/precedence differs"
        );
        ensure!(
            program
                .modules
                .get(&name(provider))
                .is_none_or(|m| !m.routines.contains_key(&tree)),
            "lazy native routine was substituted in Program"
        );
    }
    ensure!(
        seen.iter().eq(expected.keys()),
        "resident/lazy routine coverage differs"
    );
    let added = pending.len();
    for (source, tree, routine) in pending {
        program
            .modules
            .entry(source.clone())
            .or_insert_with(|| Module {
                source,
                ..Module::default()
            })
            .routines
            .insert(tree, routine);
    }
    Ok(
        json!({"kind":"current_resident_bhav_closure","tick":state["tick"],"providers":providers.len(),
        "resident_routines":added+existing,"added":added,"already_bound":existing,"lazy_demands":rows(proof,"nonresident_demands")?.len(),
        "source_readset_sha256":memory.source_sha256(),"native_parity_credit":0}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "actual B frozen physical resource closure and acknowledged readsets"]
    fn resident_closure_is_exact_and_rejects_substitution_atomically() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let output = root.join("fixtures/private/goth-route-physical-20261030-first");
        let encoded=std::fs::read(root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-00/observation.json.gz"))?;
        ensure!(
            sims_fixtures::digest(&encoded)
                == "5ec18620f4412251ffe0f78a29b68c64aff0f609893a998a67d2ba455a359299",
            "B source differs"
        );
        let state: Value = serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw=std::fs::read(root.join("fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence/completed-00/acknowledged.mem"))?;
        let sparse = Readset::parse(&raw)?;
        let data = std::fs::read(output.join("resident-bhav-B-second.mem"))?;
        let memory = Readset::parse(&data)?;
        let data = std::fs::read(output.join("resident-bhav-B-second.json"))?;
        ensure!(
            sims_fixtures::digest(&data)
                == "85c351b101932c16df04317c6e89bf72a352b944c5ab4c91169f111c7a4f84ca",
            "resident source differs"
        );
        let proof: Value = serde_json::from_slice(&data)?;
        let mut program = Program::default();
        let result = join(&state, &sparse, &proof, &memory, &mut program)?;
        ensure!(
            result["resident_routines"] == proof["routines"].as_array().unwrap().len()
                && result["lazy_demands"] == proof["nonresident_demands"].as_array().unwrap().len()
                && program.modules["native-provider:0862d328"]
                    .routines
                    .contains_key(&0x101b),
            "resident closure lacks next-tick routine"
        );
        let initial = Program::default();
        let before = serde_json::to_vec(&initial)?;
        let mut bad = proof.clone();
        bad["routines"].as_array_mut().unwrap().last_mut().unwrap()["sha256"] =
            json!("0".repeat(64));
        let mut candidate = initial.clone();
        ensure!(
            join(&state, &sparse, &bad, &memory, &mut candidate).is_err()
                && serde_json::to_vec(&candidate)? == before,
            "bad late receipt mutated Program"
        );
        let mut bad = proof.clone();
        bad["providers"].as_array_mut().unwrap().pop();
        ensure!(
            join(&state, &sparse, &bad, &memory, &mut candidate).is_err()
                && serde_json::to_vec(&candidate)? == before,
            "incomplete provider coverage accepted"
        );
        let lazy = &proof["nonresident_demands"][0];
        let source = name(number(&lazy["provider"])?);
        let tree: u16 = number(&lazy["tree"])?.try_into()?;
        let routine = program
            .modules
            .values()
            .find_map(|m| m.routines.values().next())
            .unwrap()
            .clone();
        candidate
            .modules
            .entry(source.clone())
            .or_insert_with(|| Module {
                source,
                ..Module::default()
            })
            .routines
            .insert(tree, routine);
        let before = serde_json::to_vec(&candidate)?;
        ensure!(
            join(&state, &sparse, &proof, &memory, &mut candidate).is_err()
                && serde_json::to_vec(&candidate)? == before,
            "lazy routine substitution accepted or changed Program"
        );
        Ok(())
    }
}
