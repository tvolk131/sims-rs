use anyhow::{Result, ensure};
use sims_content::{Index, saves::House};
use std::path::Path;

#[test]
#[ignore = "private original household files"]
fn original_object_relationship_rows_keep_their_serialized_order() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut rows = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        let program = house.link_program(&index)?;
        let storage = house.prepare_storage(&program)?;
        let mut nonnumeric = 0;
        for (&id, original) in &house.objects.instances {
            let expected: Vec<i16> = original
                .relationships
                .iter()
                .filter(|r| r.present != 0)
                .map(|r| i16::try_from(r.target))
                .collect::<std::result::Result<_, _>>()?;
            let object = &storage.world.objects[&(id as i16)];
            ensure!(
                object
                    .relationship_keys()
                    .map_err(anyhow::Error::msg)?
                    .as_ref()
                    == expected,
                "relationship order differs for {source} object{id}"
            );
            let roundtrip: sims_vm::Object = serde_json::from_slice(&serde_json::to_vec(object)?)?;
            ensure!(roundtrip == *object);
            nonnumeric += usize::from(expected.windows(2).any(|v| v[0] > v[1]));
        }
        rows.push(serde_json::json!({"source":source,"objects":house.objects.instances.len(),"rows_not_in_numeric_order":nonnumeric}));
    }
    let out = root.join(std::env::var("SIMS_RELATIONSHIP_ORDER_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::write(
        out,
        serde_json::to_vec_pretty(
            &serde_json::json!({"houses":rows,"raw_import_and_roundtrip_equal":true}),
        )?,
    )?;
    Ok(())
}
