//! Pack the five authenticated same-cut recovered files as an owned namespace.
//! Does not parse a starting House, construct a VM, or admit a complete Session.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::owned_resources::{Entry, Neighborhood};
use sims_fixtures::{Store, digest};
use std::{collections::BTreeSet, fs, path::Path};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 2,
        "usage: completed_private_files CAPSULE NEW_DIR"
    );
    let capsule = Path::new(&a[0]);
    let report_bytes = fs::read(capsule.join("recovered-third/report.json"))?;
    let report: Value = serde_json::from_slice(&report_bytes)?;
    let qualification: Value =
        serde_json::from_slice(&fs::read(capsule.join("qualification-second.json"))?)?;
    ensure!(
        report["complete_file_recovery"] == true,
        "file recovery incomplete"
    );
    ensure!(
        report["provenance"]["observation_sha256"]
            == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
        "wrong completed cut"
    );
    ensure!(
        qualification["provenance"]["recovery_report_sha256"] == digest(&report_bytes),
        "qualification/recovery differs"
    );
    let readset = fs::read(capsule.join("recovered-third/physical-readset.bin"))?;
    ensure!(
        report["readset"]["sha256"] == digest(&readset),
        "physical readset differs"
    );
    let recovered = report["recovered"]
        .as_array()
        .context("recovered files absent")?;
    ensure!(recovered.len() == 5, "owned file coverage differs");
    let mut entries = vec![];
    for row in qualification["files"]
        .as_array()
        .context("qualified files absent")?
    {
        let locator = row["locator"].as_str().context("locator absent")?;
        let name = locator.rsplit('/').next().context("basename absent")?;
        let matches: Vec<_> = recovered.iter().filter(|r| r["file"] == name).collect();
        ensure!(matches.len() == 1, "recovered file identity ambiguous");
        let source = matches[0];
        let bytes = fs::read(capsule.join("recovered-third").join(name))?;
        let hash = digest(&bytes);
        ensure!(
            source["size"].as_u64() == Some(bytes.len() as u64)
                && source["sha256"] == hash
                && row["sha256"] == hash,
            "recovered file bytes differ"
        );
        // Each file page must independently agree with the owned physical readset.
        let pages = source["pages"].as_array().context("file pages absent")?;
        ensure!(
            pages.len() == bytes.len().div_ceil(4096),
            "file page coverage differs"
        );
        for (i, page) in pages.iter().enumerate() {
            ensure!(
                page["index"].as_u64() == Some(i as u64),
                "file page index differs"
            );
            let owners: Vec<_> = report["readset"]["pages"]
                .as_array()
                .context("readset pages absent")?
                .iter()
                .filter(|p| p["address"] == page["physical"])
                .collect();
            ensure!(owners.len() == 1, "physical page owner ambiguous");
            let offset = owners[0]["offset"].as_u64().context("page offset absent")? as usize;
            let data = readset
                .get(offset..offset + 4096)
                .context("physical page outside readset")?;
            ensure!(
                page["sha256"] == digest(data) && owners[0]["sha256"] == page["sha256"],
                "file physical page differs"
            );
            let start = i * 4096;
            let end = (start + 4096).min(bytes.len());
            ensure!(
                bytes[start..end] == data[..end - start],
                "file is not its recovered page-cache payload"
            );
        }
        entries.push(Entry {
            locator: locator.into(),
            name: name.into(),
            sha256: hash,
            bytes,
        });
    }
    entries.sort_by(|a, b| a.locator.cmp(&b.locator));
    let owned = Neighborhood {
        directory: "UserData".into(),
        entries,
    };
    owned.validate()?;
    let expected: BTreeSet<_> = [
        "UserData/Neighborhood.iff",
        "UserData/Houses/House05.iff",
        "UserData/Characters/User00009.iff",
        "UserData/Characters/User00010.iff",
        "UserData/Characters/User00011.iff",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    ensure!(
        owned.hashes().keys().cloned().collect::<BTreeSet<_>>() == expected,
        "owned current namespace differs"
    );
    let output = Path::new(&a[1]);
    fs::create_dir(output)?;
    let store = Store::create(output.join("store"))?;
    let blob = store.put_value(&owned)?;
    let cold: Neighborhood = store.get_value(&blob)?;
    cold.validate()?;
    ensure!(
        cold.hashes() == owned.hashes(),
        "cold owned namespace differs"
    );
    fs::write(
        output.join("refs.json"),
        serde_json::to_vec_pretty(&json!({"neighborhood":blob}))?,
    )?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"schema":1,"tick":88448,"native_parity_ticks":0,"complete_native_Session_admitted":false,"starting_save_parsed":false,"vm_constructed_or_executed":false,"owned_files":owned.hashes(),"recovery_report_sha256":digest(&report_bytes),"qualification":qualification}),
        )?,
    )?;
    Ok(())
}
