//! Actual between-frame publication, joined to the existing C8/C9 capture.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read, path::Path};

fn bytes(v: &Value) -> Result<Vec<u8>> {
    let s = v.as_str().context("native byte field absent")?;
    ensure!(
        s.is_ascii() && s.len() % 2 == 0,
        "native hex encoding differs"
    );
    (0..s.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&s[i..i + 2], 16)?))
        .collect()
}
fn word(b: &[u8], i: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(i..i + 4)
            .context("native word outside bytes")?
            .try_into()?,
    ))
}
fn decode(path: &Path) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path)?)
        .take((128 << 20) + 1)
        .read_to_end(&mut body)?;
    ensure!(
        body.len() <= 128 << 20,
        "native decoded extent exceeds bound"
    );
    Ok(body)
}
fn packet(row: &Value, address: u64, count: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    for offset in (0..count).step_by(1024) {
        let n = (count - offset).min(1024);
        let mut found = None;
        for r in row["reads"]
            .as_array()
            .context("native read packets absent")?
        {
            if r["address"] == address + offset as u64 && r["bytes"] == n {
                let b = bytes(&r["raw"])?;
                ensure!(
                    b.len() == n && r["sha256"] == sims_fixtures::digest(&b),
                    "native packet receipt differs"
                );
                ensure!(
                    found.as_ref().is_none_or(|old| old == &b),
                    "conflicting same-stop packets"
                );
                found = Some(b);
            }
        }
        body.extend(found.context("native projected field lacks exact packet backing")?);
    }
    Ok(body)
}
fn buffer_map(g: &Value) -> Result<BTreeMap<u32, Vec<u8>>> {
    g["drawables"]
        .as_array()
        .context("native drawable owners absent")?
        .iter()
        .map(|d| {
            Ok((
                d["pointer"]
                    .as_u64()
                    .context("native drawable identity absent")?
                    .try_into()?,
                bytes(&d["vertices_raw"])?,
            ))
        })
        .collect()
}

#[test]
#[ignore = "private original draw witness, retained C8 assets and immutable installed data"]
fn native_between_frame_publication_matches_c8_c9_and_production_vertices() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let witness = root.join("fixtures/private/goth-draw-witness-20261031-sixth");
    let result_raw = std::fs::read(witness.join("draw-result.json"))?;
    ensure!(
        sims_fixtures::digest(&result_raw)
            == "6648d23cbdf6e6b5d3385b60f565cd7340053aec8ae872bf0c331fbec8665100",
        "draw result generation differs"
    );
    let result: Value = serde_json::from_slice(&result_raw)?;
    ensure!(
        result["success"] == true
            && result["completed_returns"] == 10
            && result["guest_input"] == false
            && result["guest_RPC"] == false,
        "native draw witness incomplete"
    );
    let encoded = std::fs::read(witness.join("draw-trace.jsonl.gz"))?;
    ensure!(
        sims_fixtures::digest(&encoded)
            == "59bd84e71302a7ba7ac5689ad2ca767b4b79496b0b97ebe5927dbdffd30212d6",
        "draw trace generation differs"
    );
    let rows: Vec<Value> = decode(&witness.join("draw-trace.jsonl.gz"))?
        .split(|b| *b == b'\n')
        .filter(|b| !b.is_empty())
        .map(serde_json::from_slice)
        .collect::<std::result::Result<_, _>>()?;
    let mut stops = BTreeMap::new();
    for r in &rows {
        if let Some(index) = r["stop_index"].as_u64() {
            let cpu = bytes(&r["raw_g"])?;
            for (field, offset) in [("eax", 0), ("esp", 16), ("eip", 32)] {
                ensure!(
                    r["registers"][field] == word(&cpu, offset)?,
                    "native register projection differs"
                );
            }
            let pc = r["registers"]["eip"].as_u64().context("native PC absent")?;
            ensure!(
                packet(r, pc, 16)? == bytes(&r["code"]["raw"])?,
                "native source/read differs"
            );
            let teb = r["teb"].as_u64().context("native TIB absent")?;
            let tib = packet(r, teb, 64)?;
            ensure!(
                word(&tib, 0x18)? as u64 == teb
                    && word(&tib, 0x20)? == 236
                    && word(&tib, 0x24)? == 240,
                "native stop is outside the captured main thread"
            );
            ensure!(
                stops.insert(index, r).is_none(),
                "duplicate actual stopped epoch"
            );
        }
    }
    ensure!(
        stops.len() == 44 && stops.keys().copied().eq(1..=44),
        "native stopped sequence differs"
    );
    for (i, kind, tick) in [
        (25, "completed_boundary", 89081),
        (27, "application_after_clock", 89081),
        (28, "application_outer_return", 89081),
        (41, "application_entry", 89081),
        (42, "clock_entry", 89081),
        (44, "completed_boundary", 89082),
    ] {
        ensure!(
            stops[&i]["kind"] == kind && stops[&i]["tick"] == tick,
            "native frame order differs"
        );
    }
    let mut frame_inputs = Vec::new();
    for (index, now) in [(27, 584466), (28, 584466), (41, 585326), (42, 585326)] {
        let stop = stops[&index];
        let observed = packet(stop, 0x6e54e0, 4)?;
        ensure!(
            observed == bytes(&stop["global_now"]["raw"])? && word(&observed, 0)? == now,
            "supplemental capture's actual global input differs"
        );
        frame_inputs.push(json!({"trace_stop":index,"global_milliseconds":now}));
    }
    let native = root.join(
        "fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence",
    );
    let seal_raw = std::fs::read(native.join("capture-result.json"))?;
    ensure!(
        sims_fixtures::digest(&seal_raw)
            == "4139107d1965925bf3ee5f44fa524d5d4b1fae22bfdb617edfffb69d33bcd393",
        "original capture seal differs"
    );
    let seal: Value = serde_json::from_slice(&seal_raw)?;
    let mut originals = BTreeMap::new();
    for i in [8, 9] {
        let path = native.join(format!("completed-{i:02}/observation.json.gz"));
        ensure!(
            seal["observations"][i]["sha256"] == sims_fixtures::digest(&std::fs::read(&path)?),
            "original component generation differs"
        );
        originals.insert(i, serde_json::from_slice::<Value>(&decode(&path)?)?);
    }
    let mut models = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for (&i, state) in &originals {
        let mut buffers = BTreeMap::new();
        for g in [
            &state["domains"]["geometry"],
            &state["domains"]["ordinary"]["geometry"],
        ] {
            buffers.extend(buffer_map(g)?);
            for c in g["characters"]
                .as_array()
                .context("native model owners absent")?
            {
                let model = c["model"].as_u64().context("native model absent")?;
                let id = c["object"].as_i64().context("native actor absent")?;
                ensure!(
                    models.insert(model, id).is_none_or(|old| old == id),
                    "native model alias changes owner"
                );
            }
        }
        expected.insert(i, buffers);
    }
    let mut pending = None;
    let mut order = Vec::new();
    let mut publications = Vec::new();
    for r in rows
        .iter()
        .filter(|r| r["kind"] == "same_stop_draw_geometry_readback")
    {
        let index = r["trace_stop"]
            .as_u64()
            .context("draw readback epoch absent")?;
        let stop = stops[&index];
        ensure!(
            r["raw_g"] == stop["raw_g"]
                && r["registers"] == stop["registers"]
                && r["tick"] == 89081
                && (29..=40).contains(&index),
            "draw readback changed stopped epoch"
        );
        ensure!(
            stop["hardware"]
                .as_str()
                .is_some_and(|s| s.contains("FCW=003f ")),
            "native draw precision differs"
        );
        let esp = r["registers"]["esp"]
            .as_u64()
            .context("native draw ESP absent")?;
        let frame = bytes(&r["call_frame"]["raw"])?;
        ensure!(
            r["call_frame"]["address"] == esp && frame == packet(r, esp, 64)?,
            "native call frame lacks packet backing"
        );
        let caller = word(&frame, 0)? as u64;
        let code = bytes(&r["caller_source"]["raw"])?;
        ensure!(
            r["caller_source"]["address"] == caller - 5
                && code == packet(r, caller - 5, 24)?
                && code[0] == 0xe8
                && (caller as i64 + i32::from_le_bytes(code[1..5].try_into()?) as i64) == 0x5a1092,
            "actual caller is not the original direct publication call"
        );
        let helper = packet(r, 0x5a1092, 113)?;
        ensure!(
            helper == bytes(&r["publication_helper_source"]["raw"])?
                && sims_fixtures::digest(&helper)
                    == "49aa08146bc4462780aac3820cb2b7a8aafc1b536fbfa3af9ca0d0f8b4873798",
            "native publication helper bytes lack packet backing"
        );
        let model = r["character"]["model"]
            .as_u64()
            .context("draw model absent")?;
        let id = *models
            .get(&model)
            .context("draw receiver lacks original model identity")?;
        let entering = stop["kind"] == "model_cpu_publication_entry";
        let target = if entering { 8 } else { 9 };
        if entering {
            ensure!(
                pending.is_none() && stop["registers"]["ecx"] == model,
                "native publication receiver/call nesting differs"
            );
            pending = Some((model, esp, caller, stop));
            order.push(id);
        } else {
            let (owner, stack, return_pc, entry) = pending
                .take()
                .context("native publication return lacks entry")?;
            ensure!(
                stop["kind"] == "model_cpu_publication_return"
                    && (owner, stack, return_pc) == (model, esp, caller),
                "native publication return frame differs"
            );
            for key in ["global_rng", "lot_rng", "crt_rng"] {
                ensure!(
                    entry[key] == stop[key],
                    "native draw changed a captured RNG owner"
                );
            }
        }
        let mut vertices = 0;
        let mut boundary_word_differences = 0;
        // The indicator has two observed passes. Its first return is an
        // intermediate render value, not a completed C8/C9 boundary. Keep and
        // count that raw evidence; only its second return is the final buffer.
        let intermediate_indicator_pass = id == 383 && caller == 0x585f12 && !entering;
        for d in r["drawables"].as_array().context("draw buffers absent")? {
            let pointer = d["pointer"]
                .as_u64()
                .context("draw buffer identity absent")?;
            let header = packet(r, pointer, 0x118)?;
            ensure!(
                word(&header, 0)? == 0x6735d8 && header[0xc0] == 0,
                "native notification requires another drawable/conversion profile"
            );
            let address = word(&header, 0xc8)? as u64;
            let data = bytes(&d["vertices_raw"])?;
            ensure!(
                d["vertices_pointer"] == address && data == packet(r, address, data.len())?,
                "native publication lacks exact packet backing"
            );
            let boundary = expected[&target]
                .get(&(pointer as u32))
                .context("native boundary drawable absent")?;
            ensure!(
                boundary.len() == data.len(),
                "native drawable extent changed"
            );
            boundary_word_differences += data
                .chunks_exact(4)
                .zip(boundary.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            vertices += data.len() / 32;
        }
        ensure!(
            boundary_word_differences == if intermediate_indicator_pass { 318 } else { 0 },
            "native publication differs from its observed boundary/intermediate profile"
        );
        publications.push(json!({"actor":id,"model":model,"entering":entering,"caller":caller,"vertices":vertices,"trace_stop":index,
            "intermediate_indicator_pass":intermediate_indicator_pass,"boundary_word_differences":boundary_word_differences}));
    }
    ensure!(
        pending.is_none() && publications.len() == 12 && order == [21, 383, 383, 18, 16, 259],
        "native actor publication coverage differs"
    );

    // Calculate from the already-validated Rust C8 pose using the production
    // binding updater. No target output is installed in replacement state.
    let report_path =
        root.join("fixtures/private/goth-heading-20261031-first/sequence-heading-full-third.json");
    let report: Value = serde_json::from_slice(&std::fs::read(&report_path)?)?;
    let reference = serde_json::from_value(report["boundary_sequence"][7]["retained_cut"].clone())?;
    let mut cut: sims_runtime::original_session::completed_assets::checkpoints::Retained =
        sims_fixtures::Store::open(report_path.with_extension("store"))?.get_value(&reference)?;
    ensure!(
        cut.simulation.state.vm.tick == 89081,
        "Rust publication input is not C8"
    );
    let before = cut.simulation.clone();
    let assets = sims_content::Index::scan(
        &root.join("cache/managed-independent-20261022-third/immutable-assets"),
    )?;
    let g = &originals[&9]["domains"]["geometry"];
    let mut calculated = Vec::new();
    for (&id, a) in &mut cut.characters {
        let avatar = sims_content::saved_avatar::BoundAvatar::restore(
            &assets,
            &a.skeleton,
            id as u64,
            &a.binding,
        )?;
        let resident = cut
            .simulation
            .state
            .characters
            .get_mut(&id)
            .context("Rust resident absent")?;
        let control = resident.character.skin.animator.fp_control();
        ensure!(control == 0x003f, "Rust captured draw control differs");
        let uploads = a.model.with_bound(resident, |bound| {
            a.geometry
                .update_loaded_vertices_with_control(&avatar, bound, false, control)
        })?;
        for &binding in &uploads {
            a.geometry
                .vertices
                .get_mut(&binding)
                .context("published buffer missing")?
                .acknowledge_null_bounds_upload(false)?;
        }
        let c = g["characters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["object"] == id)
            .context("native actor absent")?;
        let mut expected_words = Vec::new();
        let mut expected_flags = Vec::new();
        for appearance in c["ordered_appearances"].as_array().unwrap() {
            let appearance = g["appearances"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["pointer"] == *appearance)
                .context("native appearance absent")?;
            for record in appearance["records"].as_array().unwrap() {
                let binding = g["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|b| b["pointer"] == record["binding"])
                    .context("native binding absent")?;
                let data = &expected[&9][&(binding["drawable"].as_u64().unwrap() as u32)];
                let drawable = g["drawables"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|d| d["pointer"] == binding["drawable"])
                    .context("native published buffer absent")?;
                expected_flags.push(word(&bytes(&drawable["raw"])?, 0xc4)?);
                expected_words.push(
                    data.chunks_exact(32)
                        .flat_map(|v| {
                            v[..24]
                                .chunks_exact(4)
                                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                        })
                        .collect::<Vec<_>>(),
                );
            }
        }
        ensure!(
            a.geometry.vertices.len() == expected_words.len(),
            "Rust/native binding domain differs"
        );
        let mut words = 0;
        for (i, expected) in expected_words.iter().enumerate() {
            ensure!(
                a.geometry.vertices[&i].buffer_flags == expected_flags[i],
                "native renderer notification mismatch for actor {id}, binding {i}"
            );
            let actual: Vec<_> = a.geometry.vertices[&i]
                .vertices
                .iter()
                .flat_map(|v| v.position.into_iter().chain(v.normal).map(f32::to_bits))
                .collect();
            ensure!(
                &actual == expected,
                "production vertex publication differs for actor {id}, binding {i}"
            );
            words += actual.len();
        }
        calculated.push(json!({"actor":id,"words":words,"uploads":uploads}));
    }
    ensure!(
        cut.simulation == before,
        "CPU publication changed simulation/VM/RNG/continuation state"
    );
    let output = json!({"schema":1,"kind":"native_between_frame_cpu_publication","native_parity_credit":0,
        "supplemental_frame_inputs":frame_inputs,
        "native_actor_order":order,"native_publications":publications,"production_calculations":calculated,
        "complete_simulation_unchanged":true,"qualification":"Actual helper/caller and before/after buffers at C8, joined to the prior C8/C9 capture. This supplemental capture sampled a different next-frame time; it does not replace the 32-tick case's recorded inputs. Production character calculations checked. Complete frontend caller, ordinary draw execution, all33-boundary re-execution and normal-runner admission remain separate."});
    let mut output = output;
    output["renderer_notifications_acknowledged"] = true.into();
    if let Ok(path) = std::env::var("SIMS_RETAINED_DRAW_DIAG_OUTPUT") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&serde_json::to_vec_pretty(&output)?)?;
    }
    eprintln!(
        "native six-call publication order {order:?}; four production character calculations match"
    );
    Ok(())
}
