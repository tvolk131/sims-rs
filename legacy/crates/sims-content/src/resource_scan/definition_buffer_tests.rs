use crate::{Index, descriptor_scan_tests::legacy_definitions};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt::Write, path::PathBuf, time::Instant};

struct Fingerprint(Sha256);

impl Write for Fingerprint {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.0.update(value.as_bytes());
        Ok(())
    }
}

fn fingerprint(value: &impl std::fmt::Debug) -> Result<String> {
    let mut output = Fingerprint(Sha256::new());
    write!(&mut output, "{value:?}")?;
    Ok(format!("{:x}", output.0.finalize()))
}

fn checkpoint(variable: &str) -> Result<(Value, String)> {
    let path = PathBuf::from(std::env::var_os(variable).context("checkpoint is required")?);
    ensure!(path.metadata()?.len() <= 64 * 1024 * 1024);
    let bytes = std::fs::read(path)?;
    Ok((
        serde_json::from_slice(&bytes)?,
        format!("{:x}", Sha256::digest(&bytes)),
    ))
}

fn guids(saved: &Value) -> Result<BTreeSet<u32>> {
    saved["session"]["simulation"]["state"]["vm"]["world"]["objects"]
        .as_object()
        .context("saved objects absent")?
        .values()
        .map(|object| {
            u32::try_from(
                object["definition"]["guid"]
                    .as_u64()
                    .context("saved GUID absent")?,
            )
            .map_err(Into::into)
        })
        .collect()
}

fn legacy_link(
    index: &Index,
    program: &mut sims_vm::Program,
    neighborhood: &str,
    requested: &BTreeSet<u32>,
) -> Result<()> {
    for definition in
        legacy_definitions(index, neighborhood, requested, |resource| resource.read())?.values()
    {
        for (name, module) in
            crate::vm::load_program(index, &definition.source, &program.globals)?.modules
        {
            if let Some(old) = program.modules.get(&name) {
                ensure!(
                    old.content_hash == module.content_hash,
                    "inconsistent creation module {name}"
                );
            } else {
                program.modules.insert(name, module);
            }
        }
    }
    Ok(())
}

fn partial_link_failure(
    index: &Index,
    globals: &str,
    neighborhood: &str,
    neighborhood_source: &str,
    requested: &BTreeSet<u32>,
) -> Result<Value> {
    let definitions = crate::saves::definitions(index, neighborhood, requested)?;
    let mut expected_program = crate::vm::load_program(index, globals, globals)?;
    let initial_hash = fingerprint(&expected_program)?;
    let mut actual_program = expected_program.clone();
    let (first_guid, first) = definitions
        .iter()
        .find(|(_, definition)| !expected_program.modules.contains_key(&definition.source))
        .context("first new module absent")?;
    let first_modules = crate::vm::load_program(index, &first.source, globals)?.modules;
    let prefix = format!("{neighborhood}/characters/").to_ascii_lowercase();
    let (later_guid, later) = definitions
        .iter()
        .find(|(guid, definition)| {
            *guid > first_guid
                && definition.source.to_ascii_lowercase().starts_with(&prefix)
                && !expected_program.modules.contains_key(&definition.source)
                && !first_modules.contains_key(&definition.source)
        })
        .context("later independent character module absent")?;
    let mut modified = index.clone();
    let resource_id = modified.find(&later.source)?;
    let resource = &mut modified.resources[resource_id];
    let mut bytes = resource.read()?;
    bytes.extend_from_slice(b"BHAV");
    bytes.extend_from_slice(&77u32.to_be_bytes());
    bytes.extend_from_slice(&65000u16.to_be_bytes());
    bytes.extend_from_slice(&[0; 66]);
    bytes.push(0);
    resource.size = bytes.len() as u64;
    resource.unpacked_size = resource.size;
    resource.owned = Some(bytes.into());
    let selected = [*first_guid, *later_guid].into();
    let expected = legacy_link(&modified, &mut expected_program, neighborhood, &selected)
        .map_err(|error| format!("{error:#}"));
    let actual = crate::saves::link_neighborhood_creation_resources(
        &modified,
        &mut actual_program,
        neighborhood_source,
        &selected,
    )
    .map_err(|error| format!("{error:#}"));
    assert!(expected.is_err());
    assert_eq!(expected, actual);
    assert!(actual_program.modules.contains_key(&first.source));
    assert!(!actual_program.modules.contains_key(&later.source));
    let partial_hash = fingerprint(&expected_program)?;
    assert_ne!(partial_hash, initial_hash);
    assert_eq!(partial_hash, fingerprint(&actual_program)?);
    Ok(json!({
        "stage":"later-BHAV-failure-after-first-module-publication",
        "initial_program_sha256":initial_hash,"partial_program_sha256":partial_hash,
        "result":actual,"selected_guids":selected
    }))
}

#[test]
#[ignore = "requires installed assets and private before/after definition-growth checkpoints"]
fn installed_mounted_definition_buffers_preserve_definitions_order_and_program() -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("SIMS_DEFINITION_TEST_ASSET_ROOT").context("asset root is required")?,
    );
    let index = Index::scan(&root)?;
    let (before, before_hash) = checkpoint("SIMS_DEFINITION_TEST_BEFORE_SAVE")?;
    let (after, after_hash) = checkpoint("SIMS_DEFINITION_TEST_AFTER_SAVE")?;
    let before_guids = guids(&before)?;
    let after_guids = guids(&after)?;
    ensure!(before_guids.is_subset(&after_guids));
    ensure!(before_guids.len() < after_guids.len());
    let growth: BTreeSet<_> = after_guids.difference(&before_guids).copied().collect();
    let mut rows = Vec::new();
    for (profile, saved) in [("before", &before), ("after", &after)] {
        let state_hash = fingerprint(saved)?;
        let managed = &saved["session"]["managed"];
        ensure!(managed["schema"] == 1, "requires historical owned schema1");
        let owned: crate::owned_resources::Neighborhood =
            serde_json::from_value(managed["owned"].clone())?;
        let mounted = owned.mount(&index)?;
        let neighborhood_source = saved["session"]["assets"]["neighborhood_source"]
            .as_str()
            .context("saved neighborhood source absent")?;
        let (neighborhood, _) = neighborhood_source
            .rsplit_once('/')
            .context("saved neighborhood directory absent")?;
        let globals = managed["catalog"]["globals"]
            .as_str()
            .context("saved global source absent")?;
        let mut expected_program = crate::vm::load_program(&mounted, globals, globals)?;
        let mut actual_program = expected_program.clone();
        for (stage, requested) in [
            ("first", before_guids.clone()),
            ("growth", growth.clone()),
            ("already-owned", after_guids.clone()),
            ("empty", BTreeSet::new()),
            ("missing", [u32::MAX].into()),
        ] {
            let mut expected_order = Vec::new();
            let started = Instant::now();
            let expected = legacy_definitions(&mounted, neighborhood, &requested, |resource| {
                expected_order.push(resource.locator.clone());
                resource.read()
            })
            .map(|definitions| serde_json::to_value(definitions).unwrap())
            .map_err(|error| format!("{error:#}"));
            let legacy_microseconds = started.elapsed().as_micros();
            let mut actual_order = Vec::new();
            let mut input_bytes = 0u64;
            let mut capacity_growths = 0;
            let mut peak_capacity = 0;
            let mut scan = crate::resource_scan::Scan::default();
            let started = Instant::now();
            let actual = crate::saves::definitions_read(
                &mounted,
                neighborhood,
                &requested,
                |resource, bytes| {
                    actual_order.push(resource.locator.clone());
                    let prior_capacity = bytes.capacity();
                    scan.read_into(resource, bytes)?;
                    input_bytes += bytes.len() as u64;
                    capacity_growths += usize::from(bytes.capacity() > prior_capacity);
                    peak_capacity = peak_capacity.max(bytes.capacity());
                    Ok(())
                },
            )
            .map(|definitions| serde_json::to_value(definitions).unwrap())
            .map_err(|error| format!("{error:#}"));
            let reused_microseconds = started.elapsed().as_micros();
            assert_eq!(expected_order, actual_order);
            assert_eq!(expected, actual);
            let expected_link =
                legacy_link(&mounted, &mut expected_program, neighborhood, &requested)
                    .map_err(|error| format!("{error:#}"));
            let actual_link = crate::saves::link_neighborhood_creation_resources(
                &mounted,
                &mut actual_program,
                neighborhood_source,
                &requested,
            )
            .map_err(|error| format!("{error:#}"));
            assert_eq!(expected_link, actual_link);
            let program_hash = fingerprint(&expected_program)?;
            assert_eq!(program_hash, fingerprint(&actual_program)?);
            if stage == "missing" {
                assert!(actual.is_err());
                assert!(actual_link.is_err());
            } else {
                assert!(actual.is_ok());
                assert!(actual_link.is_ok());
            }
            rows.push(json!({
                "profile":profile,"stage":stage,"requested":requested.len(),
                "resources_read":actual_order.len(),"input_bytes":input_bytes,
                "buffer_capacity_growths":capacity_growths,"peak_capacity":peak_capacity,
                "definitions_sha256":fingerprint(&actual)?,"program_sha256":program_hash,
                "legacy_microseconds":legacy_microseconds,"reused_microseconds":reused_microseconds
            }));
        }
        let mut partial = partial_link_failure(
            &mounted,
            globals,
            neighborhood,
            neighborhood_source,
            &after_guids,
        )?;
        partial["profile"] = profile.into();
        rows.push(partial);
        assert_eq!(state_hash, fingerprint(saved)?);
    }
    println!(
        "{}",
        json!({
            "scope":"Mounted definition/Program differential; no gameplay or native qualification; ordered diagnostic timings only",
            "before_sha256":before_hash,"after_sha256":after_hash,
            "before_guids":before_guids.len(),"after_guids":after_guids.len(),"passes":rows
        })
    );
    Ok(())
}
