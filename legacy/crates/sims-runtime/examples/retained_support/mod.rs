//! Exact immutable loader shared by retained-input research examples.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use sims_vm::Program;
use std::{fs, path::PathBuf};

pub(super) fn load_program(
    pins_path: &std::ffi::OsStr,
    expected: &serde_json::Value,
) -> Result<Program> {
    let raw = fs::read(pins_path)?;
    ensure!(
        format!("{:x}", Sha256::digest(&raw))
            == "e357f28584cca76dd0fa2cdb41a7c286e1d1571e18b5fe5beb3bec2eb87aca08",
        "immutable Program loader pins differ"
    );
    let pins: serde_json::Value = serde_json::from_slice(&raw)?;
    let root = PathBuf::from(pins["assets_root"].as_str().context("asset root absent")?);
    for archive in pins["archives"].as_array().context("archive pins absent")? {
        let path = root.join(
            archive["relative"]
                .as_str()
                .context("archive path absent")?,
        );
        ensure!(
            format!("{:x}", Sha256::digest(fs::read(path)?)) == archive["sha256"].as_str().unwrap(),
            "immutable archive differs"
        );
    }
    let index = sims_content::Index::scan(&root)?;
    let global = pins["global"]["locator"]
        .as_str()
        .context("global locator absent")?;
    let mut result: Option<Program> = None;
    for private in pins["private_modules"]
        .as_array()
        .context("private modules absent")?
    {
        let next = sims_content::vm::load_program(
            &index,
            private["locator"]
                .as_str()
                .context("private locator absent")?,
            global,
        )?;
        if let Some(existing) = result.as_mut() {
            for (name, module) in next.modules {
                if let Some(old) = existing.modules.get(&name) {
                    ensure!(
                        serde_json::to_value(old)? == serde_json::to_value(&module)?,
                        "overlapping module differs"
                    );
                } else {
                    existing.modules.insert(name, module);
                }
            }
        } else {
            result = Some(next);
        }
    }
    let mut result = result.context("empty Program loader")?;
    result.resource_storage = serde_json::from_value(expected["resource_storage"].clone())?;
    result.neighbor_resource_flags =
        serde_json::from_value(expected["neighbor_resource_flags"].clone())?;
    // Compare the export's actual JSON encoding. to_value(f32) instead widens
    // to an exact f64, unlike the export's shortest roundtripping f32 text.
    let actual: serde_json::Value = serde_json::from_slice(&serde_json::to_vec(&result)?)?;
    ensure!(
        actual == *expected,
        "real loader Program differs from projected source: {:?}",
        sims_runtime::fixture_replay::first_difference(expected, &actual)
    );
    Ok(result)
}
