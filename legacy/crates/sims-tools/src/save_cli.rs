use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_content::Index;
use sims_formats::{iff::Iff, lot, neighborhood, saved_objects};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
};

pub fn run(index: &Index, resource: Option<&str>, out: &Path, full: bool) -> Result<()> {
    // Reports may contain private original save/asset bytes.
    let cache = std::env::current_dir()?.join("cache");
    fs::create_dir_all(&cache)?;
    let absolute = std::env::current_dir()?.join(out);
    ensure!(
        absolute.starts_with(&cache) && !absolute.components().any(|c| c == Component::ParentDir),
        "original save reports must stay in the private cache directory"
    );
    let existing = absolute
        .parent()
        .unwrap()
        .ancestors()
        .find(|p| p.exists())
        .unwrap()
        .canonicalize()?;
    ensure!(
        existing.starts_with(cache.canonicalize()?),
        "refusing output through a symlink outside cache"
    );
    fs::create_dir_all(absolute.parent().unwrap())?;
    let parent = absolute.parent().unwrap().canonicalize()?;
    ensure!(
        parent.starts_with(cache.canonicalize()?),
        "original save reports must stay in the private cache directory"
    );
    ensure!(!out.is_symlink(), "refusing symlink output");
    let selected: Vec<usize> = if let Some(r) = resource {
        vec![index.find(r)?]
    } else {
        index
            .resources
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                let name = r.locator.to_ascii_lowercase();
                let house = name
                    .rsplit('/')
                    .next()
                    .and_then(|n| n.strip_prefix("house"))
                    .and_then(|n| n.strip_suffix(".iff"))
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
                (name.ends_with("/neighborhood.iff") || house).then_some(i)
            })
            .collect()
    };
    let mut multipart = BTreeMap::<u32, BTreeSet<bool>>::new();
    for resource in &index.resources {
        if !resource.is_iff() {
            continue;
        }
        let Ok(iff) = Iff::parse(resource.read()?) else {
            continue;
        };
        for chunk in iff.all(b"OBJD") {
            let data = iff.data(chunk);
            if data.len() < 32 {
                continue;
            }
            let guid = u32::from_le_bytes(data[28..32].try_into().unwrap());
            let multi = u16::from_le_bytes(data[20..22].try_into().unwrap()) != 0;
            multipart.entry(guid).or_default().insert(multi);
        }
    }
    let mut reports = Vec::new();
    let mut failures = 0;
    let mut parsed = 0;
    for id in selected {
        let resource = &index.resources[id];
        let bytes = resource.read()?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let iff = Iff::parse(bytes)?;
        let types = iff
            .get(b"objt", 0)
            .map(|c| lot::ObjectTypes::parse(iff.data(c)))
            .transpose();
        let mut chunks = Vec::new();
        for chunk in &iff.chunks {
            let bytes = iff.data(chunk);
            let result: sims_formats::Result<Value> = match &chunk.tag {
                b"Arry" => lot::Array::parse(bytes).map(|a| if full { json!(a) } else { json!({"width":a.width,"height":a.height,"stride":a.stride,"nonzero_bytes":a.data.iter().filter(|&&b|b!=0).count()}) }),
                b"SIMI" => lot::Simulator::parse(bytes).map(|v|json!(v)),
                b"HOUS" if chunk.id == 1 => lot::House::parse(bytes).map(|v|json!(v)),
                b"HOUS" if chunk.id == 0 => lot::HouseView::parse(bytes).map(|v|json!(v)),
                b"objt" => lot::ObjectTypes::parse(bytes).map(|v|json!(v)),
                b"ObjM" => {
                    match &types {
                        Ok(Some(t)) => saved_objects::Objects::parse(bytes, t, |guid| multipart.get(&guid).filter(|s|s.len()==1).and_then(|s|s.first().copied())).map(|v| {
                            if full { json!(v) } else {
                                let persons: Vec<_> = v.instances.values().filter_map(|i| i.person.as_ref().map(|p|json!({"id":i.id,"guid":i.guid,"position":i.position,"person":p}))).collect();
                                json!({"version":v.version,"instances":v.instances.len(),"persons":persons,
                                    "with_stacks":v.instances.values().filter(|i|!i.stack.is_empty()).count(),
                                    "multipart":v.instances.values().filter(|i|i.multipart.is_some()).count(),
                                    "tail_bytes":v.tail.len(),
                                    "trailing_record_bits":v.instances.values().map(|i|i.record.len()*8-i.consumed_bits).collect::<BTreeSet<_>>()})
                            }
                        }),
                        _ => Err(sims_formats::Error::new(0,"missing/invalid object type table")),
                    }
                },
                b"WALm" | b"FLRm" => lot::surface_map(bytes, chunk.id, chunk.tag == *b"FLRm").map(|v|json!(v)),
                b"FAMI" => neighborhood::Family::parse(bytes).map(|v|json!(v)),
                b"NBRS" => neighborhood::Neighbors::parse(bytes).map(|v|json!(v)),
                b"NGBH" => neighborhood::Neighborhood::parse(bytes).map(|v|json!(v)),
                _ => continue,
            };
            let data = match result {
                Ok(v) => {
                    parsed += 1;
                    json!({"value":v})
                }
                Err(e) => {
                    failures += 1;
                    json!({"error":e.to_string()})
                }
            };
            chunks
                .push(json!({"tag":chunk.kind(),"id":chunk.id,"label":chunk.label,"result":data}));
        }
        reports.push(json!({"source":resource.locator,"sha256":sha256,"chunks":chunks}));
    }
    let report =
        json!({"schema":1,"files":reports,"parsed_chunks":parsed,"failed_chunks":failures});
    fs::write(out, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{} files · {parsed} decoded chunks · {failures} diagnostics\n{}",
        reports.len(),
        out.display()
    );
    Ok(())
}
