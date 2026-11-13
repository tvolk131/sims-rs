//! Read-only payload extraction for the catalog names observed in a frozen map.
//! Provider ambiguity is retained in the report, never resolved by index order.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::OpenOptions, io::Write, path::Path};
fn narrow(v: &Value) -> Result<String> {
    let h = v["used_raw"].as_str().context("native string absent")?;
    ensure!(h.len() % 2 == 0, "bad native string hex");
    let b = (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()?;
    ensure!(
        b.is_ascii() && !b.contains(&0),
        "unsupported native catalog string"
    );
    Ok(String::from_utf8(b)?)
}
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        a.len() == 3,
        "usage: completed_skeleton_catalogs <asset-root> <observed-map.json> <new-output.json>"
    );
    let index = sims_content::Index::scan(Path::new(&a[0]))?;
    let observed: Value = serde_json::from_slice(&std::fs::read(&a[1])?)?;
    let mut catalogs = BTreeMap::new();
    for node in observed["records"]
        .as_array()
        .context("observed map records absent")?
    {
        let catalog = narrow(&node["catalog"])?;
        if catalogs.contains_key(&catalog) {
            continue;
        }
        let mut candidates = vec![];
        for r in &index.resources {
            if r.name
                .rsplit(['\\', '/'])
                .next()
                .is_some_and(|n| n.eq_ignore_ascii_case(&catalog))
            {
                let bytes = r.read()?;
                let parsed = sims_formats::avatar::read_bcf(&bytes)?;
                candidates.push(json!({"locator":r.locator,"name":r.name,"sha256":format!("{:x}",Sha256::digest(&bytes)),"bytes":bytes.len(),"raw":bytes.iter().map(|b|format!("{b:02x}")).collect::<String>(),"skeletons":parsed.skeletons,"appearance_names":parsed.appearances.iter().map(|a|&a.name).collect::<Vec<_>>()}));
            }
        }
        ensure!(
            !candidates.is_empty(),
            "observed catalog {catalog} absent from asset corpus"
        );
        catalogs.insert(catalog, candidates);
    }
    let value = json!({"schema":1,"catalogs":catalogs,"executes_vm":false,"constructs_skeleton_resources":false});
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&a[2])?;
    serde_json::to_writer(&mut f, &value)?;
    f.write_all(b"\n")?;
    println!(
        "{} observed catalog names extracted without constructors or RNG",
        value["catalogs"].as_object().unwrap().len()
    );
    Ok(())
}
