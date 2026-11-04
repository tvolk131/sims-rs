use anyhow::{Result, ensure};
use sims_content::character_geometry::PublishedVertices;

#[cfg(feature = "private-fixtures")]
#[test]
fn null_bounds_acknowledgement_matches_original_and_rejects_conversion() -> Result<()> {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-drawable-ack.json"))?;
    ensure!(
        oracle["source_function_sha256"]
            == "0969ea2f63a115600a29736f66fffbd4e73baaa4b48ba9d23ccb2c5f0ef7a9b7"
            && oracle["null_bounds"] == true
            && oracle["native_c0"] == 0,
        "native notification fixture differs"
    );
    let rows = oracle["cases"].as_array().unwrap();
    ensure!(rows.len() == 42, "native notification cases absent");
    for row in rows {
        let mut buffer = PublishedVertices {
            vertices: vec![sims_sim::pose::vertices::Vertex {
                position: [1.25, -2.5, 3.75],
                normal: [0., 1., 0.],
                uv: [0.5, 0.25],
            }],
            buffer_flags: row["before_flags"].as_u64().unwrap().try_into()?,
            range: [-17, 31],
        };
        let before = buffer.clone();
        ensure!(
            buffer.acknowledge_null_bounds_upload(true).is_err() && buffer == before,
            "unsupported conversion changed a buffer"
        );
        buffer.acknowledge_null_bounds_upload(false)?;
        ensure!(
            row["all_other_drawable_bytes_unchanged"] == true
                && row["eax"] == 0
                && row["after_flags"] == buffer.buffer_flags
                && buffer.range == before.range
                && buffer.vertices == before.vertices,
            "native notification mismatch"
        );
    }
    Ok(())
}
