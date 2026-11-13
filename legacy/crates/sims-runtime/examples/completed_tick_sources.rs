use anyhow::{Result, ensure};
use serde_json::Value;
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, Session, completed_epoch::sources};
use std::{fs, path::Path};
#[allow(dead_code)]
#[path = "completed_session_join.rs"]
mod linking;
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 8,
        "usage: completed_tick_sources SOURCE_ROOT CONFIG IMMUTABLE STORE REFS FIELD QUALIFICATION NEW_DIR"
    );
    let root = Path::new(&a[0]);
    let config: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let immutable = sims_content::Index::scan(Path::new(&a[2]))?;
    ensure!(
        !immutable.resources.iter().any(|r| r
            .locator
            .split('/')
            .any(|p| p.to_ascii_lowercase().starts_with("userdata"))),
        "source audit requires an immutable-only installation"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&a[4])?)?;
    let saved: Saved =
        Store::open(&a[3])?.get_value(&serde_json::from_value(refs[&a[5]].clone())?)?;
    let original = postcard::to_allocvec(&saved.simulation.state.vm)?;
    let original_full = postcard::to_allocvec(&saved)?;
    let owned = &saved
        .managed
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("owned sources absent"))?
        .owned;
    let (mut program, aliases, _, _) = linking::retained_program(root, &config, owned, &immutable)?;
    if saved.headline_catalog {
        sims_content::vm::load_headline_catalog(&owned.mount(&immutable)?, &mut program)?;
    }
    ensure!(
        saved.view.resource_aliases.as_ref() == Some(&aliases),
        "current native provider map differs from independently linked original descriptors"
    );
    let session = Session::restore_completed_owned_with_program(&immutable, saved, program)?;
    let report = sources::evaluate(&session, &serde_json::from_slice(&fs::read(&a[6])?)?)?;
    ensure!(
        postcard::to_allocvec(&session.save().simulation.state.vm)? == original,
        "source restore/audit changed VM/RNG"
    );
    ensure!(postcard::to_allocvec(&session.save_managed()?)?==original_full,"source restore/audit changed the complete managed checkpoint");
    fs::create_dir(Path::new(&a[7]))?;
    fs::write(
        Path::new(&a[7]).join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "source comparisons {}, differences {}",
        report["comparison_count"], report["difference_count"]
    );
    ensure!(
        report["difference_count"] == 0,
        "native source metadata ownership differs"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    #[derive(Serialize, Deserialize)]
    struct Metadata {
        house: sims_content::saves::House,
        modules: BTreeMap<String, String>,
        immutable: BTreeMap<String, String>,
    }
    #[test]
    #[ignore = "private current-owned Goth metadata and immutable installation"]
    fn actual_source_owners_reject_changed_house_versions_and_catalog() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let config: Value = serde_json::from_slice(&fs::read(
            root.join("fixtures/private/goth-label-owners-20261029-first/session-input.json"),
        )?)?;
        let index = sims_content::Index::scan(
            &root.join("cache/managed-independent-20261022-third/immutable-assets"),
        )?;
        let c = root.join("fixtures/private/goth-label-owners-20261029-first/session-first");
        let refs: Value = serde_json::from_slice(&fs::read(c.join("refs.json"))?)?;
        let saved: Saved = Store::open(c.join("store"))?
            .get_value(&serde_json::from_value(refs["session"].clone())?)?;
        let qualification: Value = serde_json::from_slice(&fs::read(root.join(
            "fixtures/private/goth-kernel-files-20261028-first/qualification-second.json",
        ))?)?;
        let (program, _, _, _) = linking::retained_program(
            &root,
            &config,
            &saved.managed.as_ref().unwrap().owned,
            &index,
        )?;
        let restore = |saved: Saved| {
            Session::restore_completed_owned_with_program(&index, saved, program.clone())
        };
        let session = restore(saved.clone())?;
        let original = postcard::to_allocvec(&session.save().simulation.state.vm)?;
        ensure!(
            sources::evaluate(&session, &qualification)?["difference_count"] == 0,
            "actual current source metadata positive failed"
        );
        let mut bad = saved.clone();
        let content = bad.managed.as_mut().unwrap();
        let mut metadata: Metadata = postcard::from_bytes(&content.house)?;
        metadata.house.number += 1;
        content.house = postcard::to_allocvec(&metadata)?;
        match restore(bad) {
            Err(_) => {}
            Ok(s) => ensure!(
                sources::evaluate(&s, &qualification)?["difference_count"] != 0,
                "coherent wrong historical House metadata admitted"
            ),
        };
        let mut bad = saved.clone();
        let entry = &mut bad.managed.as_mut().unwrap().owned.entries[0];
        entry.bytes[0] ^= 1;
        entry.sha256 = sims_fixtures::digest(&entry.bytes);
        match restore(bad) {
            Err(_) => {}
            Ok(s) => ensure!(
                sources::evaluate(&s, &qualification).is_err(),
                "coherent different current original file version admitted"
            ),
        };
        let mut bad = saved.clone();
        bad.managed.as_mut().unwrap().catalog.text_language = Some(999);
        ensure!(
            restore(bad).is_err(),
            "wrong source catalog language admitted"
        );
        ensure!(
            postcard::to_allocvec(&session.save().simulation.state.vm)? == original,
            "source audit changed VM/RNG"
        );
        Ok(())
    }
}
