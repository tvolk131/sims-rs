//! Current readonly Global owner, including its existing hash chains, lazy
//! records, resident allocations and stream cursor. No Open or lookup runs.
use super::*;
use crate::original_session::{completed_geometry::bytes, completed_lot::Memory};
use sims_sim::object_catalog::{
    composite::{Composite, Configuration, Filter},
    iff::{Iff, Payload, Record, RetainedHandleEpoch, RetainedParts, RetainedTable},
};
use std::collections::BTreeSet;
fn u(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn w(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes(b[i..i + 2].try_into().unwrap())
}
fn ptr(v: &Value) -> Result<u32> {
    Ok(v.as_u64().context("Global pointer absent")?.try_into()?)
}

/// Available immutable Global BCON configuration, preserving actual resident
/// values and observed override emptiness. This does not materialize native
/// lazy handles or claim an original cache transition.
pub(super) fn constants(
    v: &Value,
    tuning: &Value,
    index: &Index,
    locator: &str,
    program: &mut Program,
) -> Result<Value> {
    let _owner = import(v, index, locator)?;
    let memory = Memory::new(tuning)?;
    let provider = ptr(&tuning["provider"])?;
    let raw = bytes(tuning, "owner_raw")?;
    ensure!(
        tuning["schema"] == 1
            && tuning["tick"] == v["tick"]
            && tuning["native_code_executed"] == false
            && tuning["source_pin"]["observation_sha256"] == v["provenance"]["observation_sha256"]
            && tuning["source_pin"]["physical"]["compressed_sha256"]
                == v["provenance"]["image_sha256"]
            && provider == ptr(&v["provider"]["pointer"])?
            && raw.len() == 0x250
            && raw[..172] == bytes(&v["provider"], "raw")?
            && memory.read(provider, raw.len())? == raw
            && u(&raw, 0) == 0x6672b0,
        "Global tuning owner/source differs"
    );
    let begin = u(&raw, 0x244);
    let end = u(&raw, 0x248);
    let capacity = u(&raw, 0x24c);
    ensure!(
        (begin >= 0x10000 || begin == 0 && end == 0 && capacity == 0)
            && begin == end
            && end <= capacity
            && (capacity - begin) % 8 == 0
            && tuning["otf"] == json!([]),
        "nonempty or invalid original OTF owner requires effective override support"
    );
    let source = format!("native-provider:{provider:08x}");
    ensure!(program.globals == source, "Global constant source differs");
    let module = program
        .modules
        .get(&source)
        .context("Global constant module absent")?;
    let file = index.resources[index.find(locator)?].read()?;
    ensure!(
        sims_fixtures::digest(&file) == module.content_hash,
        "Global constant whole-source version differs"
    );
    let iff = sims_formats::iff::Iff::parse(file)?;
    let mut available = BTreeMap::new();
    for chunk in iff.resource_order(b"BCON")? {
        let payload = iff.data(chunk);
        let values = sims_formats::execution::constants(payload)?;
        ensure!(
            available
                .insert(chunk.id, (payload.to_vec(), values))
                .is_none(),
            "duplicate live Global constant table"
        );
    }
    // The full retained owner above already validates graph/handle aliases.
    // Cache bytes remain authoritative; a differing disk table needs a richer
    // effective profile, never an implicit replacement.
    let handles = v["graph"]["resident_handles"]
        .as_array()
        .context("Global handles absent")?;
    let mut resident = 0;
    for outer in v["graph"]["maps"][0]["nodes"]
        .as_array()
        .context("Global map absent")?
    {
        if outer["type_raw"] != u32::from_be_bytes(*b"BCON") {
            continue;
        }
        for row in outer["inner"]["nodes"]
            .as_array()
            .context("Global constant nodes absent")?
        {
            if row["key_variant"] != 0 || row["handle"] == 0 {
                continue;
            }
            let id = u16::try_from(row["key_id"].as_u64().context("constant ID absent")?)?;
            let handle = handles
                .iter()
                .find(|h| h["pointer"] == row["handle"])
                .context("resident constant handle absent")?;
            ensure!(
                available.get(&id).is_some_and(
                    |(payload, _)| bytes(handle, "payload_raw").is_ok_and(|b| b == *payload)
                ),
                "resident Global constants differ from available source"
            );
            resident += 1;
        }
    }
    let mut staged = BTreeMap::new();
    for (id, (_, values)) in &available {
        if let Some(current) = module.constants.get(id) {
            ensure!(
                current == values,
                "captured Global constants would be replaced"
            );
        } else {
            staged.insert(*id, values.clone());
        }
    }
    let added = staged.len();
    program
        .modules
        .get_mut(&source)
        .unwrap()
        .constants
        .extend(staged);
    Ok(
        json!({"kind":"available_global_constants","observed_tick":v["tick"],
        "source":source,"whole_source_sha256":module_hash(program, &source),
        "tables_available":available.len(),"resident_payloads_preserved":resident,
        "tables_added":added,"OTF":"observed empty vector; no disk inference",
        "native_lazy_handles_materialized":false,"native_cache_order_qualified":false,
        "program_fingerprint":program.fingerprint()}),
    )
}

fn module_hash<'a>(program: &'a Program, source: &str) -> &'a str {
    &program.modules[source].content_hash
}

/// Ordinary default-score StringSets, not the separate lowercase animation
/// cache. This scoped profile requires the actually observed English locale.
pub(super) fn scores(
    v: &Value,
    banks: &Value,
    index: &Index,
    locator: &str,
    program: &mut Program,
) -> Result<Value> {
    let _owner = import(v, index, locator)?;
    let memory = Memory::new(banks)?;
    let provider = ptr(&banks["provider"])?;
    let source = format!("native-provider:{provider:08x}");
    ensure!(
        banks["schema"] == 1
            && banks["tick"] == v["tick"]
            && banks["native_code_executed"] == false
            && provider == ptr(&v["provider"]["pointer"])?
            && banks["source_pin"]["observation_sha256"] == v["provenance"]["observation_sha256"]
            && banks["source_pin"]["physical"]["compressed_sha256"]
                == v["provenance"]["image_sha256"]
            && program.globals == source
            && memory.read(provider, 172)? == bytes(&v["provider"], "raw")?,
        "score bank source/owner differs"
    );
    let manager = ptr(&banks["manager"])?;
    let raw = memory.read(manager, 0x68)?;
    ensure!(
        u(&memory.read(0x6e48bc, 4)?, 0) == manager
            && raw == bytes(banks, "manager_raw")?
            && raw[0x64] == 1
            && banks["file_language_byte"] == 1
            && u(
                &memory.read(
                    u(&raw, 0x28)
                        .checked_add(0x2c)
                        .context("language owner overflow")?,
                    4
                )?,
                0
            ) == provider,
        "default score profile requires observed file-language1/internal0"
    );
    let component = ptr(&v["component"]["pointer"])?;
    ensure!(
        banks["components"] == json!([component])
            && memory.read(component, 44)? == bytes(&v["component"], "raw")?,
        "score component owner/order differs"
    );
    let graph = &banks["resident_components"][component.to_string()];
    let maps = graph["maps"]
        .as_array()
        .context("score resource maps absent")?;
    ensure!(
        maps.len() == 1 && ptr(&maps[0]["pointer"])? == u(&bytes(&v["component"], "raw")?, 24),
        "score bank map owner differs"
    );
    let mut records = BTreeMap::new();
    for outer in tables(&maps[0], &memory, false)?.into_iter().flatten() {
        let raw = bytes(&outer, "raw")?;
        if u(&raw, 4) != u32::from_be_bytes(*b"STR#") {
            continue;
        }
        ensure!(
            outer["selected_type"] == true
                && ptr(&outer["inner"]["pointer"])? == ptr(&outer["pointer"])? + 8
                && bytes(&outer["inner"], "raw")? == raw[8..],
            "score outer/inner owner alias differs"
        );
        for row in tables(&outer["inner"], &memory, true)?
            .into_iter()
            .flatten()
        {
            let raw = bytes(&row, "raw")?;
            if raw[6] != 0 {
                continue;
            }
            ensure!(
                records.insert(w(&raw, 4), raw).is_none(),
                "duplicate score bank record"
            );
        }
    }
    let module = program
        .modules
        .get(&source)
        .context("score Global module absent")?;
    let file = index.resources[index.find(locator)?].read()?;
    ensure!(
        sims_fixtures::digest(&file) == module.content_hash,
        "score whole-file version differs"
    );
    let iff = sims_formats::iff::Iff::parse(file)?;
    let catalog = iff.resource_order(b"STR#")?;
    let mut staged = BTreeMap::new();
    for id in [501, 503] {
        let chunks: Vec<_> = catalog.iter().filter(|c| c.id == id).collect();
        ensure!(
            chunks.len() == 1,
            "default score STR# absent/ambiguous; CST needs its typed path"
        );
        let chunk = chunks[0];
        let payload = iff.data(chunk);
        let record = records
            .get(&id)
            .context("score availability has no current metadata")?;
        ensure!(
            w(record, 7) == id
                && w(record, 9) == chunk.flags
                && w(record, 11) == chunk.flags
                && u(record, 19) == chunk.offset as u32,
            "score bank metadata/file version differs"
        );
        let handle = u(record, 15);
        if handle != 0 {
            let h = memory.read(handle, 8)?;
            ensure!(
                u(&h, 4) as usize == payload.len()
                    && memory.read(u(&h, 0), payload.len())? == payload,
                "resident score bank differs from immutable source"
            );
        }
        let table = sims_formats::strings::StringTable::parse(payload)?;
        ensure!(
            table.version == -3,
            "ordinary English score decoder profile differs"
        );
        // The parser preserves English and raw-language0 wildcard entries as
        // language1. For requested native0 this is precisely the native vector;
        // there is no per-entry fallback or animation lowercase operation.
        let values: Vec<_> = table
            .entries
            .into_iter()
            .filter(|e| e.language == 1)
            .map(|e| e.text)
            .collect();
        if let Some(old) = module.strings.get(&id) {
            ensure!(old == &values, "captured score StringSet would be replaced");
        } else {
            staged.insert(id, values);
        }
    }
    let added = staged.len();
    program
        .modules
        .get_mut(&source)
        .unwrap()
        .strings
        .extend(staged);
    Ok(
        json!({"kind":"ordinary_default_score_strings","observed_tick":v["tick"],"source":source,
        "file_language":1,"native_internal_language":0,"raw_string_catalog_tables":catalog.len(),
        "ordinary_tables":[501,503],"tables_added":added,"animation_cache_replaced":false,
        "native_lazy_handles_materialized":false,"native_cache_order_qualified":false,
        "program_fingerprint":program.fingerprint()}),
    )
}

pub(super) fn routines(
    v: &Value,
    index: &Index,
    locator: &str,
    program: &mut Program,
) -> Result<usize> {
    // This checks every raw table/node/handle alias and stream provenance.
    let _owner = import(v, index, locator)?;
    let source = format!("native-provider:{:08x}", ptr(&v["provider"]["pointer"])?);
    ensure!(
        program.globals == source,
        "resident Global Program owner differs"
    );
    let module = program
        .modules
        .get(&source)
        .context("resident Global module absent")?;
    let handles = v["graph"]["resident_handles"]
        .as_array()
        .context("Global resident handles absent")?;
    let mut staged = BTreeMap::new();
    for outer in v["graph"]["maps"][0]["nodes"]
        .as_array()
        .context("Global resource types absent")?
    {
        if outer["type_raw"] != u32::from_be_bytes(*b"BHAV") {
            continue;
        }
        for record in outer["inner"]["nodes"]
            .as_array()
            .context("Global BHAV records absent")?
        {
            if record["key_variant"] != 0 || record["handle"] == 0 {
                continue;
            }
            let id = u16::try_from(
                record["key_id"]
                    .as_u64()
                    .context("Global routine id absent")?,
            )?;
            let handle = handles
                .iter()
                .find(|h| h["pointer"] == record["handle"])
                .context("resident Global routine allocation absent")?;
            let payload = super::super::completed_animations::bytes(
                handle["payload_raw"]
                    .as_str()
                    .context("resident Global routine payload absent")?,
            )?;
            ensure!(
                payload.len() >= 12,
                "resident Global routine header incomplete"
            );
            let body = sims_formats::behavior::Behavior::parse(&payload)?;
            if let Some(old) = module.routines.get(&id) {
                ensure!(
                    serde_json::to_vec(&old.body)? == serde_json::to_vec(&body)?,
                    "active/resident Global routine bodies disagree"
                );
            }
            let version = u(&payload, 8);
            if let Some(old) = module.routine_versions.get(&id) {
                ensure!(*old == version, "active/resident Global revisions disagree");
            }
            ensure!(
                staged.insert(id, (body, version)).is_none(),
                "duplicate resident Global routine"
            );
        }
    }
    let module = program.modules.get_mut(&source).unwrap();
    let mut added = 0;
    for (id, (body, version)) in staged {
        if !module.routines.contains_key(&id) {
            module.routines.insert(
                id,
                sims_vm::Routine {
                    name: format!("resident:{id}"),
                    body,
                },
            );
            added += 1;
        }
        module.routine_versions.insert(id, version);
    }
    Ok(added)
}
fn tables(v: &Value, m: &Memory, inner: bool) -> Result<Vec<Vec<Value>>> {
    let raw = bytes(v, "raw")?;
    let p = ptr(&v["pointer"])?;
    ensure!(
        raw.len() == if inner { 20 } else { 24 },
        "Global table header extent differs"
    );
    ensure!(
        m.read(p, raw.len())? == raw && v["declared_entry_count_raw"] == u(&raw, 16),
        "Global table owner differs"
    );
    let head = bytes(&v["buckets"], "heads_raw")?;
    ensure!(
        u(&raw, 8).checked_sub(u(&raw, 4)) == Some(head.len().try_into()?)
            && head.len() % 4 == 0
            && !head.is_empty()
            && m.read(u(&raw, 4), head.len())? == head,
        "Global bucket allocation differs"
    );
    let nodes = v["nodes"].as_array().context("Global nodes absent")?;
    let mut used = BTreeSet::new();
    let mut result = vec![];
    for b in head.chunks_exact(4) {
        let mut p = u(b, 0);
        let mut bucket = vec![];
        while p != 0 {
            ensure!(used.insert(p), "Global bucket cycle/shared node");
            let rows: Vec<_> = nodes.iter().filter(|n| n["pointer"] == p).collect();
            ensure!(rows.len() == 1, "Global bucket node unavailable/duplicated");
            let row = rows[0];
            let raw = bytes(row, "raw")?;
            ensure!(
                raw.len() == if inner { 100 } else { 28 },
                "Global node extent differs"
            );
            ensure!(
                m.read(p, raw.len())? == raw && row["next"] == u(&raw, 0),
                "Global linked node differs"
            );
            bucket.push(row.clone());
            p = u(&raw, 0);
        }
        result.push(bucket);
    }
    ensure!(
        used.len() == nodes.len() && used.len() == u(&raw, 16) as usize,
        "Global table coverage differs"
    );
    Ok(result)
}
pub(in crate::original_session) fn import(
    v: &Value,
    index: &Index,
    locator: &str,
) -> Result<sims_content::autonomy_resources::Provider> {
    ensure!(
        v["schema"] == 1
            && v["tick"] == 88448
            && v["executes_target_code"] == false
            && v["loads_resources"] == false
            && v["provenance"]["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && v["provenance"]["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
        "Global source profile differs"
    );
    let m = Memory::new(v)?;
    let p = &v["provider"];
    let raw = bytes(p, "raw")?;
    ensure!(
        raw.len() == 172 && u(&raw, 0) == 0x6672b0 && m.read(ptr(&p["pointer"])?, 172)? == raw,
        "Global composite header differs"
    );
    let configs = bytes(v, "configuration_raw")?;
    ensure!(
        configs.len() == 384 && m.read(ptr(&p["pointer"])? + 0xac, 384)? == configs,
        "Global configurations absent"
    );
    let mut configurations = vec![];
    for (i, row) in configs.chunks_exact(24).enumerate() {
        let kind = u(row, 0);
        if kind == 0 {
            break;
        }
        let length = usize::from(w(row, 12));
        ensure!(
            (1..=3).contains(&kind)
                && u(row, 8) == ptr(&p["pointer"])? + 0xac + (i * 24 + 16) as u32
                && w(row, 14) == 8
                && length < 8
                && row[16 + length] == 0,
            "Global configuration storage differs"
        );
        configurations.push(Configuration {
            kind: kind as u8,
            extension: row[16..16 + length].to_vec(),
        });
    }
    let c = &v["component"];
    let component = ptr(&c["pointer"])?;
    let cr = bytes(c, "raw")?;
    ensure!(
        cr.len() == 44
            && m.read(component, 44)? == cr
            && u(&cr, 0) == 0x666944
            && cr[36] == 0
            && c["writable"] == 0
            && c["modified"] == 0
            && u(&cr, 16) != 0,
        "Global current readonly IFF binding differs"
    );
    let mut slots = [None; 8];
    let mut filters = vec![];
    for i in 0..8 {
        let child = u(&raw, 12 + i * 20);
        if child != 0 {
            ensure!(i == 0 && child == component, "unknown Global component");
            slots[i] = Some(1);
        }
        let begin = u(&raw, 16 + i * 20);
        let end = u(&raw, 20 + i * 20);
        let source = &v["filters"][i];
        let tags = bytes(source, "raw")?;
        ensure!(
            end.checked_sub(begin) == Some(tags.len().try_into()?)
                && tags.len() % 4 == 0
                && source["exclude"] == w(&raw, 28 + i * 20),
            "Global component filter differs"
        );
        if !tags.is_empty() {
            ensure!(
                m.read(begin, tags.len())? == tags,
                "Global filter data differs"
            );
        }
        filters.push(Filter {
            tags: tags.chunks_exact(4).map(|b| u(b, 0)).collect(),
            exclude: w(&raw, 28 + i * 20),
        });
    }
    let aggregate = Composite {
        configurations,
        slots,
        filters: filters.try_into().unwrap(),
        error: u(&raw, 8) as i32,
    };
    let maps = v["graph"]["maps"]
        .as_array()
        .context("Global map owner absent")?;
    ensure!(
        maps.len() == 1 && maps[0]["pointer"] == u(&cr, 24),
        "Global IFF/hash table alias differs"
    );
    let graph = &v["graph"];
    let source_handles = graph["resident_handles"]
        .as_array()
        .context("Global residents absent")?;
    let mut storage = vec![];
    let mut handles = BTreeMap::new();
    let mut native_handles = BTreeMap::new();
    let mut outer = vec![];
    for bucket in tables(&maps[0], &m, false)? {
        let mut out = vec![];
        for node in bucket {
            let nr = bytes(&node, "raw")?;
            let tag = u(&nr, 4);
            ensure!(
                node["type_raw"] == tag && node["selected_type"] == true,
                "unobserved Global resource type"
            );
            let mut inner = vec![];
            ensure!(
                node["inner"]["pointer"] == ptr(&node["pointer"])? + 8
                    && bytes(&node["inner"], "raw")? == nr[8..],
                "Global outer/inner hash owner alias differs"
            );
            for bucket in tables(&node["inner"], &m, true)? {
                let mut entries = vec![];
                for node in bucket {
                    let nr = bytes(&node, "raw")?;
                    let key = u32::from(w(&nr, 4)) | u32::from(nr[6]) << 16;
                    let name = &node["name"];
                    let name_bytes = super::super::completed_animations::bytes(
                        name["bytes_raw"]
                            .as_str()
                            .context("Global record narrow name unavailable")?,
                    )?;
                    ensure!(
                        u(&nr, 23) == 0x66651c
                            && usize::from(w(&nr, 31)) == name_bytes.len()
                            && w(&nr, 33) == 64
                            && name_bytes.len() < 64
                            && m.read(u(&nr, 27), name_bytes.len() + 1)?
                                == [name_bytes.clone(), vec![0]].concat(),
                        "Global record name owner differs"
                    );
                    let native = u(&nr, 15);
                    let payload = if native == 0 {
                        None
                    } else {
                        let matches: Vec<_> = source_handles
                            .iter()
                            .filter(|h| h["pointer"] == native)
                            .collect();
                        ensure!(
                            matches.len() == 1,
                            "Global resident allocation absent/duplicated"
                        );
                        let h = matches[0];
                        let hr = bytes(h, "raw")?;
                        let bytes = bytes(h, "payload_raw")?;
                        ensure!(
                            hr.len() == 8
                                && m.read(native, 8)? == hr
                                && u(&hr, 4) as usize == bytes.len()
                                && m.read(u(&hr, 0), bytes.len())? == bytes,
                            "Global resident payload owner differs"
                        );
                        let next = native_handles.len() as u32 + 1;
                        let local = *native_handles.entry(native).or_insert(next);
                        handles.insert(local, local);
                        Some(Payload {
                            handle: local,
                            bytes,
                        })
                    };
                    let record = Record {
                        tag,
                        id: w(&nr, 7),
                        flags: w(&nr, 9),
                        disk_flags: w(&nr, 11),
                        ordinal: w(&nr, 13),
                        offset: u(&nr, 19),
                        name: name_bytes,
                        payload,
                    };
                    let id = storage.len();
                    storage.push(record);
                    entries.push((key, id));
                }
                inner.push(entries);
            }
            out.push((
                tag,
                RetainedTable {
                    len: inner.iter().map(Vec::len).sum(),
                    buckets: inner,
                    short: true,
                },
            ));
        }
        outer.push(out);
    }
    ensure!(
        native_handles.len() == source_handles.len(),
        "unclaimed Global resident handle"
    );
    let root = bytes(&maps[0], "raw")?;
    let iff = Iff::from_retained_parts(RetainedParts {
        valid: true,
        error: u(&cr, 8) as i32,
        map_offset: u(&cr, 32),
        unused: u(&root, 20),
        write_cursor: u(&cr, 28),
        modified: cr[36] != 0,
        table: RetainedTable {
            len: outer.iter().map(Vec::len).sum(),
            buckets: outer,
            short: false,
        },
        storage,
        handle_epoch: RetainedHandleEpoch::NewUnpublished {
            next: handles.len() as u32 + 1,
        },
    })
    .map_err(anyhow::Error::msg)?;
    let wrapper = bytes(c, "wrapper_raw")?;
    ensure!(
        wrapper.len() == 36
            && m.read(u(&cr, 16), 36)? == wrapper
            && c["cursor"] == u(&wrapper, 24)
            && c["logical_size"] == u(&wrapper, 16)
            && c["base_offset"] == u(&wrapper, 20),
        "Global current stream differs"
    );
    let retained = sims_content::autonomy_resources::Retained {
        aggregate,
        iff,
        stream_position: u(&wrapper, 24),
        stream_open: true,
        stream_writable: false,
        next_handle: handles.len() as u32 + 1,
        handles,
        buffers: BTreeMap::new(),
    };
    sims_content::autonomy_resources::Provider::from_retained(index, locator, retained)
}
