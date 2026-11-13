//! Compare the current streaming writer with buffering before hashing/gzip.
//! Authored byte-heavy DTO only; no VM or original-game admission.
use anyhow::Result;
use serde::Serialize;
use sims_fixtures::{Compression, Store};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};
fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args_os().nth(1).expect("fresh output directory"));
    std::fs::create_dir(&root)?;
    let value = BTreeMap::from([(
        "bytes",
        (0..262_161).map(|i| (i % 256) as u8).collect::<Vec<_>>(),
    )]);
    let expected = rmp_serde::to_vec_named(&value)?;
    let mut rows = Vec::new();
    for round in 0..3 {
        for buffered in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let store = Store::create(root.join(format!("r{round}-buffered-{buffered}")))?;
            let begin = Instant::now();
            let blob = if buffered {
                store.put_value(&value)?
            } else {
                store.put_with(Compression::Gzip, |writer| {
                    value.serialize(&mut rmp_serde::Serializer::new(writer).with_struct_map())?;
                    Ok(())
                })?
            };
            let elapsed_ns = begin.elapsed().as_nanos();
            anyhow::ensure!(
                store.read_bytes(&blob, 1 << 20)? == expected,
                "decoded bytes differ"
            );
            anyhow::ensure!(
                blob.sha256 == sims_fixtures::digest(&expected),
                "identity differs"
            );
            rows.push(serde_json::json!({"round":round,"buffered":buffered,"elapsed_ns":elapsed_ns,"blob":blob}));
        }
    }
    let result = serde_json::json!({"schema":1,"scope":"Authored streaming storage microbenchmark; no VM processing or game parity claim. Production buffered put_value versus the preceding unbuffered serialization path.","decoded_bytes":expected.len(),"producer_source_sha256":sims_fixtures::digest(include_bytes!("value_writer_bench.rs")),"every_payload_and_identity_equal":true,"results":rows});
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{result}");
    Ok(())
}
