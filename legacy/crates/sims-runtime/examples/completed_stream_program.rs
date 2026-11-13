//! Compare immutable FAR slices with the actual captured resident BHAV bytes.
//! Mutable native files remain demands; this does not admit a complete Session.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_formats::iff::Iff;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
#[path = "completed_stream_program/private.rs"]
mod private;

fn read(path: &str) -> Result<(Vec<u8>, Value)> {
    let bytes = fs::read(path)?;
    let value = serde_json::from_slice(&bytes)?;
    Ok((bytes, value))
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 5 || args.len() == 8,
        "usage: completed_stream_program REENTRY STREAMS SLICES IMMUTABLE_ROOT NEW_OUTPUT [PRIVATE_MAPS CURRENT_FILES_MANIFEST CURRENT_ROOT]"
    );
    let (_, reentry) = read(&args[0])?;
    let (stream_bytes, streams) = read(&args[1])?;
    let (_, slices) = read(&args[2])?;
    ensure!(
        slices["snapshot_sha256"] == sims_fixtures::digest(&stream_bytes)
            && streams["provenance"]["image_sha256"] == reentry["provenance"]["image_sha256"]
            && streams["provenance"]["observation_sha256"]
                == reentry["provenance"]["observation_sha256"],
        "stream/slice/Program source epoch differs"
    );
    let root = Path::new(&args[3]).canonicalize()?;
    let mut files = BTreeMap::new();
    let mut locators = BTreeMap::new();
    for entry in slices["matched_slices"]
        .as_array()
        .context("slice list absent")?
    {
        let locator = entry["locator"].as_str().context("slice locator absent")?;
        let (archive, _) = locator.split_once("::").context("slice archive absent")?;
        let path = root.join(archive).canonicalize()?;
        ensure!(
            path.starts_with(&root) && !archive.to_ascii_lowercase().starts_with("userdata"),
            "slice escapes immutable assets"
        );
        let mut file = fs::File::open(path)?;
        file.seek(SeekFrom::Start(
            entry["offset"].as_u64().context("slice offset absent")?,
        ))?;
        let count = entry["bytes"].as_u64().context("slice size absent")?;
        ensure!(count <= 128 << 20, "slice size bound");
        let mut bytes = vec![0; count as usize];
        file.read_exact(&mut bytes)?;
        ensure!(
            entry["sha256"] == sims_fixtures::digest(&bytes),
            "current immutable slice hash differs"
        );
        let component = entry["component"]
            .as_u64()
            .context("slice component absent")?;
        locators.insert(component, locator.to_owned());
        ensure!(
            files.insert(component, Iff::parse(bytes)?).is_none(),
            "duplicate component slice"
        );
    }
    let immutable_count = files.len();
    let private_report = if args.len() == 8 {
        let (_, maps) = read(&args[5])?;
        let (_, manifest) = read(&args[6])?;
        let (private_files, report) =
            private::compare(&streams, &maps, &manifest, Path::new(&args[7]))?;
        for (component, file) in private_files {
            ensure!(
                files.insert(component, file).is_none(),
                "private/immutable component alias"
            );
        }
        for entry in report.as_array().context("private comparisons absent")? {
            locators.insert(
                entry["component"]
                    .as_u64()
                    .context("private component absent")?,
                entry["owned_locator"]
                    .as_str()
                    .context("private locator absent")?
                    .to_owned(),
            );
        }
        report
    } else {
        json!([])
    };
    let mut matched = vec![];
    let mut unresolved = vec![];
    let mut bindings = vec![];
    for (pointer, provider) in streams["providers"]
        .as_object()
        .context("providers absent")?
    {
        let components = provider["components"]
            .as_array()
            .context("provider components absent")?;
        let pointer: u32 = pointer.parse()?;
        let resolved: Vec<_> = components
            .iter()
            .map(|c| c.as_u64().and_then(|c| locators.get(&c)).cloned())
            .collect();
        let primary = components
            .first()
            .and_then(Value::as_u64)
            .context("empty provider")?;
        bindings.push(json!({"native_source":format!("native-provider:{pointer:08x}"),
            "ordered_component_locators":resolved,"primary_content_sha256":files.get(&primary).map(|f|sims_fixtures::digest(&f.bytes)),
            "complete_native_source_version_qualified":false}));
    }
    for module in reentry["stacks"]["modules"]
        .as_array()
        .context("current modules absent")?
    {
        let source = module["source"].as_str().context("module source absent")?;
        let provider = u64::from_str_radix(
            source
                .strip_prefix("native-provider:")
                .context("non-native provider")?,
            16,
        )?;
        let components = streams["providers"][provider.to_string()]["components"]
            .as_array()
            .context("module provider closure absent")?;
        for (id, routine) in module["routines"]
            .as_object()
            .context("module routines absent")?
        {
            let id: u16 = id.parse()?;
            let receipt = &routine["receipt"];
            ensure!(
                receipt["provider"] == provider,
                "routine provider alias differs"
            );
            let component = receipt["component"]
                .as_u64()
                .context("routine component absent")?;
            ensure!(
                components.iter().any(|c| c.as_u64() == Some(component)),
                "routine component not in provider"
            );
            let payload = sims_runtime::original_session::completed_animations::bytes(
                routine["payload"]
                    .as_str()
                    .context("routine payload absent")?,
            )?;
            ensure!(
                receipt["payload_sha256"] == sims_fixtures::digest(&payload),
                "captured routine hash differs"
            );
            let Some(iff) = files.get(&component) else {
                unresolved.push(json!({"source":source,"routine":id,"component":component,"reason":"current mutable native provider version required"}));
                continue;
            };
            let chunks: Vec<_> = iff
                .resource_order(b"BHAV")?
                .into_iter()
                .filter(|c| c.id == id)
                .collect();
            ensure!(
                chunks.len() == 1 && iff.data(chunks[0]) == payload,
                "current resource/captured BHAV mismatch: {source}/{id} component {component}"
            );
            let mut previous_unknown = false;
            let mut first = None;
            for c in components {
                let pointer = c.as_u64().context("component pointer absent")?;
                if let Some(candidate) = files.get(&pointer) {
                    if candidate
                        .resource_order(b"BHAV")?
                        .iter()
                        .any(|chunk| chunk.id == id)
                    {
                        first = Some(pointer);
                        break;
                    }
                } else {
                    previous_unknown = true;
                }
            }
            ensure!(
                first == Some(component),
                "native by-ID component priority differs"
            );
            matched.push(json!({"source":source,"routine":id,"component":component,
                "payload_sha256":sims_fixtures::digest(&payload),"by_id_priority_qualified":!previous_unknown}));
        }
    }
    let report = json!({"schema":1,"immutable_IFF_slices":immutable_count,"current_private_file_comparisons":private_report,"candidate_module_bindings":bindings,"matched_resident_BHAVs":matched,
        "mutable_resident_BHAV_demands":unresolved,"native_parity_ticks":0,
        "complete_native_Session_qualified":false,
        "scope":"captured active resident BHAV payloads, ordered component lookup and candidate current-file metadata correspondence; not complete native file version/source admission"});
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[4])?;
    use std::io::Write;
    output.write_all(&serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"immutable_slices":immutable_count,"current_files":files.len() - immutable_count,"matched_BHAVs":report["matched_resident_BHAVs"].as_array().unwrap().len(),
        "mutable_demands":report["mutable_resident_BHAV_demands"].as_array().unwrap().len(),"native_parity_ticks":0})
    );
    Ok(())
}
