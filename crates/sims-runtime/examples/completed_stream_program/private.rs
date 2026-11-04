use super::*;

/// Compare stopped-memory owners with separately captured current disk files.
/// Range/metadata equality does not authenticate a complete cross-epoch version.
pub fn compare(
    streams: &Value,
    maps: &Value,
    manifest: &Value,
    root: &Path,
) -> Result<(BTreeMap<u64, Iff>, Value)> {
    ensure!(
        maps["stream_provenance"] == streams["provenance"],
        "private map/stream epoch differs"
    );
    let root = root.canonicalize()?;
    let mut files = BTreeMap::new();
    let mut comparisons = vec![];
    for owner in maps["files"].as_array().context("private files absent")? {
        let component = owner["component"].as_u64().context("component absent")?;
        let stream = &streams["components"][component.to_string()];
        ensure!(stream["writable"] == 1, "private owner not writable");
        let logical = owner["logical_name"]
            .as_str()
            .context("private name absent")?
            .replace('\\', "/")
            .to_ascii_lowercase();
        let logical = logical.trim_start_matches("./");
        let candidates: Vec<_> = manifest["files"]
            .as_array()
            .context("captured files absent")?
            .iter()
            .filter(|f| {
                f["logical"].as_str().map(|s| s.to_ascii_lowercase()) == Some(logical.to_owned())
            })
            .collect();
        ensure!(
            candidates.len() == 1,
            "missing/ambiguous current file: {logical}"
        );
        let entry = candidates[0];
        let relative = Path::new(entry["logical"].as_str().context("logical file absent")?);
        let path = root
            .join(relative.file_name().context("file name absent")?)
            .canonicalize()?;
        ensure!(
            path.starts_with(&root),
            "current file escapes captured owner root"
        );
        let bytes = fs::read(path)?;
        ensure!(
            entry["bytes"] == bytes.len()
                && entry["sha256"] == sims_fixtures::digest(&bytes)
                && stream["logical_size"] == bytes.len()
                && stream["base_offset"] == 0,
            "captured current file size/hash/slice differs"
        );
        let backend = &streams["backends"][stream["backend"]
            .as_u64()
            .context("backend absent")?
            .to_string()];
        ensure!(
            backend["buffered_write_bytes"] == 0,
            "pending native writes require overlay"
        );
        let observed = sims_runtime::original_session::completed_animations::bytes(
            backend["read_valid_raw"]
                .as_str()
                .context("buffer bytes absent")?,
        )?;
        let offset = backend["buffered_read_offset"]
            .as_u64()
            .context("buffer offset absent")? as usize;
        ensure!(
            bytes.get(offset..offset + observed.len()) == Some(observed.as_slice()),
            "current/native read buffer differs"
        );
        let iff = Iff::parse(bytes)?;
        let graph = &owner["graph"];
        let handles: BTreeMap<_, _> = graph["resident_handles"]
            .as_array()
            .context("handles absent")?
            .iter()
            .map(|h| Ok((h["pointer"].as_u64().context("handle pointer absent")?, h)))
            .collect::<Result<_>>()?;
        let mut seen = std::collections::BTreeSet::new();
        let mut resident = 0;
        let maps = graph["maps"].as_array().context("resource maps absent")?;
        ensure!(
            maps.len() == 1 && maps[0]["pointer"] == stream["map_owner"],
            "private map owner differs"
        );
        for outer in maps[0]["nodes"].as_array().context("types absent")? {
            ensure!(outer["selected_type"] == true, "private type not captured");
            let tag: u32 = outer["type_raw"]
                .as_u64()
                .context("tag absent")?
                .try_into()?;
            for row in outer["inner"]["nodes"]
                .as_array()
                .context("resource nodes absent")?
            {
                ensure!(
                    row["key_variant"] == 0 && row["key_id"] == row["record_id"],
                    "unsupported private resource key"
                );
                let offset = row["disk_offset_raw"]
                    .as_u64()
                    .context("chunk offset absent")? as usize;
                let ordered = iff.resource_order(&tag.to_be_bytes())?;
                let (ordinal, chunk) = ordered
                    .into_iter()
                    .enumerate()
                    .find(|(_, c)| c.offset == offset)
                    .context("native chunk offset not in current file")?;
                ensure!(
                    chunk.tag == tag.to_be_bytes()
                        && row["key_id"] == chunk.id
                        && row["disk_flags_raw"] == chunk.flags
                        && row["ordinal_raw"] == ordinal + 1
                        && seen.insert(offset),
                    "current/native chunk metadata differs"
                );
                let handle = row["handle"].as_u64().context("handle absent")?;
                if handle != 0 {
                    let payload = sims_runtime::original_session::completed_animations::bytes(
                        handles.get(&handle).context("resident handle missing")?["payload_raw"]
                            .as_str()
                            .context("payload absent")?,
                    )?;
                    ensure!(
                        iff.data(chunk) == payload,
                        "live private payload differs from current disk; overlay required"
                    );
                    resident += 1;
                }
            }
        }
        ensure!(
            seen.len() == iff.chunks.iter().filter(|c| c.flags & 4 == 0).count(),
            "current file has chunks outside retained native map"
        );
        comparisons.push(json!({"component":component,"logical":logical,"owned_locator":entry["logical"],"bytes":iff.bytes.len(),"sha256":entry["sha256"],
            "matched_chunk_headers":seen.len(),"matched_resident_payloads":resident,"lazy_payloads_observed_in_native_RAM":false,
            "native_modified":owner["native_modified"],"complete_cross_epoch_source_version_qualified":false}));
        ensure!(
            files.insert(component, iff).is_none(),
            "duplicate private component"
        );
    }
    Ok((files, json!(comparisons)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs() -> Result<(Value, Value, Value, std::path::PathBuf)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/private/goth-stream-owners-20261028-first");
        Ok((
            read(root.join("streams-fourth.json").to_str().unwrap())?.1,
            read(root.join("private-sources-second.json").to_str().unwrap())?.1,
            read(root.join("current-files/manifest.json").to_str().unwrap())?.1,
            root.join("current-files"),
        ))
    }
    #[test]
    #[ignore = "private original stopped-source evidence"]
    fn original_current_files_match_all_live_headers_and_observed_payloads() -> Result<()> {
        let (streams, maps, manifest, root) = inputs()?;
        let (files, report) = compare(&streams, &maps, &manifest, &root)?;
        ensure!(files.len() == 3, "private source closure differs");
        for row in report.as_array().unwrap() {
            ensure!(
                row["matched_chunk_headers"] == 27 && row["matched_resident_payloads"] == 4,
                "actual source comparison counts differ"
            );
        }
        Ok(())
    }
    #[test]
    #[ignore = "synthetic negative mutations of private original evidence"]
    fn changed_native_payload_or_incomplete_map_is_rejected() -> Result<()> {
        let (streams, maps, manifest, root) = inputs()?;
        let mut changed = maps.clone();
        changed["files"][0]["graph"]["resident_handles"][0]["payload_raw"] = json!("00");
        ensure!(
            compare(&streams, &changed, &manifest, &root).is_err(),
            "changed payload accepted"
        );
        let mut changed = maps.clone();
        changed["files"][0]["graph"]["maps"][0]["nodes"]
            .as_array_mut()
            .unwrap()
            .pop();
        ensure!(
            compare(&streams, &changed, &manifest, &root).is_err(),
            "missing type accepted"
        );
        Ok(())
    }
}
