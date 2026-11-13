use anyhow::{Context, Result, ensure};
use serde_json::json;
use sha2::{Digest, Sha256};
use sims_content::{
    Index,
    avatar_installation::{
        Installation, ObjectVisit, ObjectVisitor, complete_collection_asset_bindings,
    },
    object_resources::MountedStream,
};
use sims_sim::{object_catalog::iff::Iff, object_scan::Paths};
use std::collections::BTreeSet;

fn digest(iff: &Iff) -> String {
    let mut hash = Sha256::new();
    for r in iff.records() {
        hash.update(r.tag.to_le_bytes());
        for v in [r.id, r.flags, r.disk_flags, r.ordinal] {
            hash.update(v.to_le_bytes());
        }
        hash.update(r.offset.to_le_bytes());
        hash.update((r.name.len() as u32).to_le_bytes());
        hash.update(&r.name);
        if let Some(p) = &r.payload {
            hash.update((p.bytes.len() as u32).to_le_bytes());
            hash.update(&p.bytes);
        } else {
            hash.update(u32::MAX.to_le_bytes());
        }
    }
    format!("{:x}", hash.finalize())
}

#[test]
#[ignore = "requires extracted Complete Collection files; emits private native probe inputs"]
fn original_mounted_iff_components_and_native_inputs() -> Result<()> {
    #[derive(Default)]
    struct Inventory(Vec<ObjectVisit>);
    impl ObjectVisitor for Inventory {
        fn count(&self) -> usize {
            0
        }
        fn file(&mut self, _: &mut Installation<'_>, visit: ObjectVisit) -> Result<()> {
            self.0.push(visit);
            Ok(())
        }
        fn changed(&mut self) -> Result<()> {
            anyhow::bail!("inventory cannot register definitions")
        }
    }
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&repo.join("game-data/cabinet-extracted"))?;
    let mut installation =
        Installation::new(&index, complete_collection_asset_bindings(&index.root))?;
    let initial = installation.initial_mounts(b"./", 255, false)?;
    let paths = Paths {
        objects: initial[1].clone(),
        user_objects: initial[2].clone(),
        downloads: initial.last().unwrap().clone(),
        expansions: std::array::from_fn(|i| initial[3 + i].clone()),
        shared: initial[11].clone(),
        npcs: initial[12].clone(),
        ..Default::default()
    };
    let mut inventory = Inventory::default();
    installation.scan_objects(&paths, 255, 0, false, &mut inventory)?;
    let directories = installation.directories()?;
    let mut visited = BTreeSet::new();
    let mut rows = vec![];
    let mut record_count = 0;
    let mut directory_resources = vec![];
    for visit in &inventory.0 {
        let stem = &visit.path[..visit.path.len() - 4];
        let directory = [stem, b".dir/"].concat();
        if installation.physical_exists(&directory)? {
            directory_resources.push(directory);
        }
        for extension in [b".iff", b".spf", b".stx"] {
            let path = [stem, extension].concat();
            let mut stream = MountedStream::new(&index, &directories);
            let mut iff = Iff::default();
            let opened = iff
                .open(&path, 0, &mut stream)
                .map_err(anyhow::Error::msg)?;
            if opened == -50 {
                ensure!(!iff.valid);
                continue;
            }
            ensure!(
                opened == 0 && iff.valid,
                "native IFF open {opened}: {}",
                String::from_utf8_lossy(&path)
            );
            let source = stream.source().context("opened component source absent")?;
            if !visited.insert(source) {
                ensure!(iff.close(&mut stream).map_err(anyhow::Error::msg)? == 0);
                continue;
            }
            let resource = &index.resources[source];
            let records = iff.records().count();
            record_count += records;
            let opened_hash = digest(&iff);
            let unused = iff.unused;
            let map_offset = iff.map_offset;
            let tag = u32::from_be_bytes(*b"OBJD");
            let count = iff.count(tag);
            let mut queries = vec![];
            for ordinal in 1..=u32::from(count) {
                let handle = iff
                    .ordinal(tag, ordinal, true, &mut stream)
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    iff.error == 0 && handle != 0,
                    "OBJD ordinal {ordinal}: {}",
                    resource.locator
                );
                let bytes = iff.payload(handle).context("loaded OBJD absent")?;
                let payload_sha256 = format!("{:x}", Sha256::digest(bytes));
                let mut id = 0xaabbccdd;
                iff.id(handle, &mut id);
                ensure!(iff.error == 0);
                let mut name = vec![];
                iff.name(handle, &mut name);
                ensure!(iff.error == 0);
                queries.push(
                    json!({"ordinal":ordinal,"id":id,"name":name,"payload_sha256":payload_sha256}),
                );
            }
            let loaded_hash = digest(&iff);
            ensure!(
                iff.close(&mut stream).map_err(anyhow::Error::msg)? == 0
                    && !iff.valid
                    && stream.source().is_none()
            );
            rows.push(json!({"path":path,"source":resource,"records":records,"map_offset":map_offset,"unused":unused,
                "opened_sha256":opened_hash,"loaded_sha256":loaded_hash,"queries":queries}));
        }
    }
    ensure!(rows.len() >= inventory.0.len());
    std::fs::write(
        repo.join("cache/native-object-iff-original-input.json"),
        serde_json::to_vec(&json!({"files":rows}))?,
    )?;
    std::fs::write(
        repo.join("cache/original-object-iff-input-summary.json"),
        serde_json::to_vec_pretty(&json!({"files":visited.len(),"records":record_count,
            "iff":rows.iter().filter(|r|r["source"]["name"].as_str().unwrap().ends_with(".iff")).count(),
            "stx":rows.iter().filter(|r|r["source"]["name"].as_str().unwrap().ends_with(".stx")).count(),
            "mapped_files":rows.iter().filter(|r|r["map_offset"].as_u64().unwrap()!=0).count(),
            "queries":rows.iter().map(|r|r["queries"].as_array().unwrap().len()).sum::<usize>(),
            "present_directory_resources":directory_resources}))?,
    )?;
    println!(
        "{} source-bound IFF/SPF/STX components, {record_count} records; lazy OBJD queries and read-only close pass; native input written",
        visited.len()
    );
    Ok(())
}
