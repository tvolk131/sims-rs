//! Compare B identities and the retained current wall graph to actual C8 inputs.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::draw_parent::{State, WorldComposition, WorldScene};
use sims_vm::scheduler::clock::profiler::Buffer;
use std::{io::{BufRead, BufReader, Write}, path::PathBuf};
type Saved = (State, WorldScene, WorldComposition, sims_sim::Simulation, Buffer);
fn bytes(v: &serde_json::Value) -> Result<Vec<u8>> {
    let s = v.as_str().context("input bytes absent")?;
    ensure!(s.len()%2 == 0, "odd hex input");
    (0..s.len()).step_by(2).map(|i| Ok(u8::from_str_radix(&s[i..i+2],16)?)).collect()
}
fn main() -> Result<()> {
    let root = std::env::var_os("SIMS_DIAG_ROOT").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let old = root.join("fixtures/private/goth-level-geometry-20261101-first");
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(old.join("joined-first.json"))?)?;
    let input: sims_fixtures::BlobRef = serde_json::from_value(report["cold_cuts"].as_array().unwrap().last().unwrap()["blob"].clone())?;
    ensure!(input.sha256 == "3b1b82c2d6e7804eaaa6ea39a9fd70ecf03754513ef9b7b82a8f6043a2ca153d", "input cut differs");
    let (_, scene, _, _, _): Saved = sims_fixtures::Store::open(old.join("cold-first.store"))?.get_value(&input)?;
    scene.validate()?;
    let source = std::fs::read(root.join("fixtures/private/goth-geometry-first-vector-20261101-first/first-vector-source-first.json"))?;
    ensure!(sims_fixtures::digest(&source) == "48bdda26252d0600a22b49fc681cb982a98cff89d1b8279b6aee8731bde22531", "B source differs");
    let source: serde_json::Value = serde_json::from_slice(&source)?;
    let path = std::env::var_os("SIMS_DIAG_NATIVE").map(PathBuf::from).context("native trace absent")?;
    let native = std::fs::read(path)?;
    let mut entry = None;
    for line in BufReader::new(flate2::read::GzDecoder::new(native.as_slice())).lines() {
        let v: serde_json::Value = serde_json::from_str(&line?)?;
        if v["pc"] == 0x54045bu32 { entry = Some(v); break; }
    }
    let entry = entry.context("actual geometry entry absent")?;
    let actual = entry["models"].as_array().context("current models absent")?;
    let original = source["first_vector"]["models"].as_array().context("B models absent")?;
    let indices = &scene.graph.state.floors[0].edges;
    ensure!(actual.len() == original.len() && actual.len() == indices.len() && actual.len() == 28, "first-vector ownership differs");
    let mut changed = Vec::new();
    let mut segments = 0;
    for (i, ((a, b), &index)) in actual.iter().zip(original).zip(indices).enumerate() {
        ensure!(a["pointer"] == b["pointer"] && a["raw"] == b["raw"] && a["segment_begin"] == b["segment_begin"] && a["segment_end"] == b["segment_end"], "model {i} B identities changed");
        let edge = &scene.graph.state.edges[index];
        let observed = bytes(&a["segments"])?;
        let before = bytes(&b["segments"])?;
        let computed: Vec<u8> = edge.segments.iter().flat_map(|s| [s.flags, s.style, s.patterns[0] as u8, (s.patterns[0]>>8) as u8, s.patterns[1] as u8, (s.patterns[1]>>8) as u8]).collect();
        ensure!(computed == observed, "model {i} current scene descriptors differ from actual input");
        ensure!(before.len() == computed.len(), "model {i} segment extent changed");
        for (offset, (&b, &c)) in before.iter().zip(&computed).enumerate() {
            if b != c { changed.push(serde_json::json!({"model":i,"offset":offset,"B":b,"current":c})); }
        }
        segments += edge.segments.len();
    }
    let result = serde_json::json!({"schema":1,"success":true,"input":input,"native_trace_sha256":sims_fixtures::digest(&native),"models":28,"segments":segments,"current_Rust_scene_descriptors_equal_actual_C8":true,"B_to_current_changes":changed,"source_admission":false,"native_parity_credit":0,"scope":"Current retained scene predicts all geometry descriptor inputs. Original results are comparisons, not installed service replies. No complete CPU or completed-tick admission."});
    let output = std::env::var_os("SIMS_DIAG_REPORT").context("report output absent")?;
    std::fs::OpenOptions::new().create_new(true).write(true).open(output)?.write_all(&serde_json::to_vec_pretty(&result)?)?;
    println!("{result}");
    Ok(())
}
