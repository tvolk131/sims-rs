//! Read-only catalog diagnostic for comparison with an original-game capture.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::{
    Index,
    avatar_installation::{Binding, Installation, complete_collection_asset_bindings},
    object_catalog::Catalog,
};
use std::{fs::File, path::Path, sync::Arc};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: startup_catalog CONTENT_ROOT NEW_PRIVATE_OUTPUT"
    );
    let output = Path::new(&args[1]);
    ensure!(
        output
            .parent()
            .context("output parent")?
            .canonicalize()?
            .starts_with(Path::new("cache").canonicalize()?),
        "private output required"
    );
    let file = File::create_new(output)?;
    let index = Arc::new(Index::scan(Path::new(&args[0]))?);
    let mut catalog = Catalog::household(
        index.clone(),
        "UserData/Neighborhood.iff",
        Default::default(),
    )?;
    let mut entries = vec![];
    for e in &catalog.definitions.entries {
        let word = |i| u16::from_le_bytes(e.data[i..i + 2].try_into().unwrap());
        entries.push(json!({"index":e.index,"guid":u32::from_le_bytes(e.data[28..32].try_into().unwrap()),
            "kind":word(18),"global_flag":word(84),"name":String::from_utf8_lossy(&e.name),
            "path":String::from_utf8_lossy(&catalog.resources.filename(e.key.resource).map_err(anyhow::Error::msg)?),
            "resource":e.key.resource,"chunk":e.key.chunk}));
    }
    let mut bindings = complete_collection_asset_bindings(&index.root);
    bindings.push(Binding {
        directory: b"./UserData/Characters/".to_vec(),
        physical: index.root.join("UserData/Characters"),
    });
    let mut installation = Installation::new(&index, bindings)?;
    installation.initial_mounts(b"./", 255, false)?;
    installation.register(b"./UserData/Characters/", false, true)?;
    let directories = installation.manager.directories().map(|(path,files)| json!({
        "path":String::from_utf8_lossy(path),
        "files":files.iter().map(|(name,_)| String::from_utf8_lossy(name).into_owned()).collect::<Vec<_>>()
    })).collect::<Vec<_>>();
    serde_json::to_writer_pretty(
        file,
        &json!({"entries":entries,"visits":catalog.visits,"directories":directories}),
    )?;
    println!("{} definitions", entries.len());
    Ok(())
}
