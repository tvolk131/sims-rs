//! In-memory `SIMS_INSTALL`-gated completeness round-trip verifier.
//!
//! Legal boundary: reads original `.iff` files from `$SIMS_INSTALL` strictly in
//! memory, lifts chunks into `sims-antics` IR / `.antics` source text in memory,
//! parses + lowers + re-encodes back to raw chunk bytes in memory, asserts 100%
//! byte-for-byte parity in memory using metadata-only diagnostics (never
//! printing commercial payload bytes), and drops all buffers without ever
//! writing anything derived from commercial assets to disk.

use sims_antics::compile::compile_antics;
use sims_antics::encode::{
    encode_bcon, encode_bhav, encode_objd, encode_objf, encode_slot, encode_string_table,
    encode_ttab,
};
use sims_antics::lift::{
    format_bcon_antics, format_bhav_antics, lift_bcon, lift_bhav, lift_objd, lift_objf, lift_slot,
    lift_string_table, lift_ttab,
};
use sims_content::{ArchivePool, Index, Install};
use sims_formats::behavior::{Behavior, InteractionTable, ObjectDefinition};
use sims_formats::execution::{ObjectFunctions, Slots, constants};
use sims_formats::iff::{DELETED, Iff};
use sims_formats::strings::StringTable;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Compares two byte buffers without ever formatting payload bytes into panic messages.
fn assert_bytes_equal(actual: &[u8], expected: &[u8], context: &str) {
    if actual == expected {
        return;
    }
    let first_diff = actual
        .iter()
        .zip(expected.iter())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| actual.len().min(expected.len()));
    panic!(
        "round-trip byte mismatch in {context}: actual_len={} expected_len={} first_diff_offset={first_diff}",
        actual.len(),
        expected.len()
    );
}

#[derive(Default)]
struct TagStats {
    attempted: usize,
    compared: usize,
    /// Chunks that `sims_formats` itself refuses to parse (e.g. legacy/non-standard
    /// resources the reader rejects), tracked explicitly rather than silently ignored.
    reader_rejected: usize,
    /// Chunks that `sims_formats` parsed but `sims-antics` failed to lift/encode.
    unexpected_skips: usize,
}

#[test]
fn complete_collection_in_memory_chunk_roundtrip_parity() {
    let Ok(install_root) = std::env::var("SIMS_INSTALL") else {
        eprintln!("install_roundtrip: skipped (SIMS_INSTALL unset)");
        return;
    };
    if install_root.trim().is_empty() {
        eprintln!("install_roundtrip: skipped (SIMS_INSTALL empty)");
        return;
    }

    let install = Install::detect(&PathBuf::from(&install_root))
        .expect("SIMS_INSTALL did not resolve to a valid Sims installation");
    let index = Index::build(&install).expect("failed to index SIMS_INSTALL");
    let pool = ArchivePool::new();

    let mut iff_files = 0usize;
    let mut iff_read_or_parse_failed = 0usize;
    let mut bhav_text_roundtrips = 0usize;
    let mut bhav_text_skipped_invalid_targets = 0usize;
    let mut ttab_compressed_count = 0usize;

    let mut stats: BTreeMap<&'static str, TagStats> = BTreeMap::new();
    let mut unexpected_notes: Vec<String> = Vec::new();

    let mut bhav_formats = BTreeSet::new();
    let mut primitive_opcodes = BTreeSet::new();
    let mut ttab_versions = BTreeSet::new();
    let mut slot_versions = BTreeSet::new();
    let mut str_formats = BTreeSet::new();

    for resource in &index.resources {
        let name = resource.name();
        if name.len() < 4 || !name[name.len() - 4..].eq_ignore_ascii_case(".iff") {
            continue;
        }
        let Ok(bytes) = resource.read_with(&pool) else {
            iff_read_or_parse_failed += 1;
            continue;
        };
        let Ok(iff) = Iff::parse(bytes) else {
            iff_read_or_parse_failed += 1;
            continue;
        };
        iff_files += 1;
        let loc = resource.location(&index.root);

        for chunk in &iff.chunks {
            if chunk.flags & DELETED != 0 {
                continue;
            }
            let data = iff.data(chunk);
            match &chunk.tag {
                b"BHAV" => {
                    let s = stats.entry("BHAV").or_default();
                    s.attempted += 1;
                    if Behavior::parse(data).is_err() {
                        s.reader_rejected += 1;
                        continue;
                    }
                    let lifted = match lift_bhav(chunk.id, &chunk.label, data) {
                        Ok(v) => v,
                        Err(e) => {
                            s.unexpected_skips += 1;
                            unexpected_notes.push(format!("{loc} BHAV #{}: {e}", chunk.id));
                            continue;
                        }
                    };
                    bhav_formats.insert(lifted.format);
                    for ins in &lifted.instructions {
                        if ins.opcode < 256 {
                            primitive_opcodes.insert(ins.opcode);
                        }
                    }

                    let encoded_ir = encode_bhav(&lifted).expect("encode_bhav failed");
                    assert_bytes_equal(
                        &encoded_ir,
                        data,
                        &format!("{loc} BHAV #{} (IR)", chunk.id),
                    );
                    s.compared += 1;

                    let valid_targets = lifted.instructions.len() <= 251
                        && lifted.instructions.iter().all(|ins| {
                            let ok = |t: u8| {
                                matches!(t, 0xFD..=0xFF)
                                    || usize::from(t) < lifted.instructions.len()
                            };
                            ok(ins.on_true) && ok(ins.on_false)
                        });
                    if valid_targets {
                        let source_text = format_bhav_antics(&lifted);
                        let compiled = compile_antics(&source_text).unwrap_or_else(|e| {
                            panic!(
                                "compile_antics failed on lifted BHAV {loc} #{}: {e}",
                                chunk.id
                            )
                        });
                        let mut re_bhav = compiled.behaviors[&chunk.id].clone();
                        re_bhav.trailing = lifted.trailing.clone();
                        let encoded_text = encode_bhav(&re_bhav).expect("encode_bhav text failed");
                        assert_bytes_equal(
                            &encoded_text,
                            data,
                            &format!("{loc} BHAV #{} (.antics text)", chunk.id),
                        );
                        bhav_text_roundtrips += 1;
                    } else {
                        bhav_text_skipped_invalid_targets += 1;
                    }
                }
                b"BCON" => {
                    let s = stats.entry("BCON").or_default();
                    s.attempted += 1;
                    if constants(data).is_err() {
                        s.reader_rejected += 1;
                        continue;
                    }
                    let lifted = match lift_bcon(chunk.id, &chunk.label, data) {
                        Ok(v) => v,
                        Err(e) => {
                            s.unexpected_skips += 1;
                            unexpected_notes.push(format!("{loc} BCON #{}: {e}", chunk.id));
                            continue;
                        }
                    };
                    let encoded = encode_bcon(&lifted).expect("encode_bcon failed");
                    assert_bytes_equal(&encoded, data, &format!("{loc} BCON #{} (IR)", chunk.id));
                    let text = format_bcon_antics(&lifted);
                    let compiled = compile_antics(&text).unwrap_or_else(|e| {
                        panic!(
                            "compile_antics failed on lifted BCON {loc} #{}: {e}",
                            chunk.id
                        )
                    });
                    let mut re_bcon = compiled.constants[&chunk.id].clone();
                    re_bcon.trailing = lifted.trailing.clone();
                    let encoded_text = encode_bcon(&re_bcon).expect("encode_bcon text failed");
                    assert_bytes_equal(
                        &encoded_text,
                        data,
                        &format!("{loc} BCON #{} (.antics text)", chunk.id),
                    );
                    s.compared += 1;
                }
                b"STR#" | b"CTSS" | b"TTAs" => {
                    let tag_key = match &chunk.tag {
                        b"STR#" => "STR#",
                        b"CTSS" => "CTSS",
                        b"TTAs" => "TTAs",
                        _ => unreachable!(),
                    };
                    let s = stats.entry(tag_key).or_default();
                    s.attempted += 1;
                    if StringTable::parse(data).is_err() {
                        s.reader_rejected += 1;
                        continue;
                    }
                    let lifted = match lift_string_table(chunk.tag, chunk.id, &chunk.label, data) {
                        Ok(v) => v,
                        Err(e) => {
                            s.unexpected_skips += 1;
                            unexpected_notes.push(format!("{loc} {tag_key} #{}: {e}", chunk.id));
                            continue;
                        }
                    };
                    str_formats.insert(format!("{:?}", lifted.format));
                    let encoded = encode_string_table(&lifted).expect("encode_string_table failed");
                    assert_bytes_equal(&encoded, data, &format!("{loc} {tag_key} #{}", chunk.id));
                    s.compared += 1;
                }
                b"TTAB" => {
                    let s = stats.entry("TTAB").or_default();
                    s.attempted += 1;
                    if InteractionTable::parse(data).is_err() {
                        s.reader_rejected += 1;
                        continue;
                    }
                    let lifted = match lift_ttab(chunk.id, &chunk.label, data) {
                        Ok(v) => v,
                        Err(e) => {
                            s.unexpected_skips += 1;
                            unexpected_notes.push(format!("{loc} TTAB #{}: {e}", chunk.id));
                            continue;
                        }
                    };
                    ttab_versions.insert(lifted.version);
                    if lifted.compressed {
                        ttab_compressed_count += 1;
                    }
                    let encoded = encode_ttab(&lifted).expect("encode_ttab failed");
                    assert_bytes_equal(&encoded, data, &format!("{loc} TTAB #{}", chunk.id));
                    s.compared += 1;
                }
                b"SLOT" => {
                    let s = stats.entry("SLOT").or_default();
                    s.attempted += 1;
                    if Slots::parse(data).is_err() {
                        s.reader_rejected += 1;
                        continue;
                    }
                    let lifted = match lift_slot(chunk.id, &chunk.label, data) {
                        Ok(v) => v,
                        Err(e) => {
                            s.unexpected_skips += 1;
                            unexpected_notes.push(format!("{loc} SLOT #{}: {e}", chunk.id));
                            continue;
                        }
                    };
                    slot_versions.insert(lifted.version);
                    let encoded = encode_slot(&lifted).expect("encode_slot failed");
                    assert_bytes_equal(&encoded, data, &format!("{loc} SLOT #{}", chunk.id));
                    s.compared += 1;
                }
                b"OBJD" => {
                    let s = stats.entry("OBJD").or_default();
                    s.attempted += 1;
                    if ObjectDefinition::parse(data).is_err() {
                        s.reader_rejected += 1;
                        continue;
                    }
                    let lifted = match lift_objd(chunk.id, &chunk.label, data) {
                        Ok(v) => v,
                        Err(e) => {
                            s.unexpected_skips += 1;
                            unexpected_notes.push(format!("{loc} OBJD #{}: {e}", chunk.id));
                            continue;
                        }
                    };
                    let encoded = encode_objd(&lifted).expect("encode_objd failed");
                    assert_bytes_equal(&encoded, data, &format!("{loc} OBJD #{}", chunk.id));
                    s.compared += 1;
                }
                b"OBJf" => {
                    let s = stats.entry("OBJf").or_default();
                    s.attempted += 1;
                    if ObjectFunctions::parse(data).is_err() {
                        s.reader_rejected += 1;
                        continue;
                    }
                    let lifted = match lift_objf(chunk.id, &chunk.label, data) {
                        Ok(v) => v,
                        Err(e) => {
                            s.unexpected_skips += 1;
                            unexpected_notes.push(format!("{loc} OBJf #{}: {e}", chunk.id));
                            continue;
                        }
                    };
                    let encoded = encode_objf(&lifted).expect("encode_objf failed");
                    assert_bytes_equal(&encoded, data, &format!("{loc} OBJf #{}", chunk.id));
                    s.compared += 1;
                }
                _ => {}
            }
        }
    }

    let get = |tag: &str| stats.get(tag).map_or(0, |s| s.compared);
    let summary: Vec<String> = stats
        .iter()
        .map(|(tag, s)| {
            format!(
                "{tag}={}/{} (reader_rej={}, unexp={})",
                s.compared, s.attempted, s.reader_rejected, s.unexpected_skips
            )
        })
        .collect();
    eprintln!(
        "install_roundtrip (100% in-memory, 0 bytes written to disk): \
         {iff_files} IFFs (read/parse_err={iff_read_or_parse_failed}) | {} | \
         BHAV .antics text={bhav_text_roundtrips} (nonstandard_targets={bhav_text_skipped_invalid_targets}, \
         formats={bhav_formats:?}, distinct_primitives={}) | \
         STR formats={str_formats:?} | TTAB compressed={ttab_compressed_count} versions={ttab_versions:?} | \
         SLOT versions={slot_versions:?}",
        summary.join(" | "),
        primitive_opcodes.len()
    );

    assert!(
        unexpected_notes.is_empty(),
        "unexpected lift/encode skips on sims_formats-valid chunks ({} total), first few: {:?}",
        unexpected_notes.len(),
        &unexpected_notes[..unexpected_notes.len().min(10)]
    );
    assert!(
        iff_read_or_parse_failed == 0,
        "expected 0 IFF read/parse failures, got {iff_read_or_parse_failed}"
    );
    assert!(
        iff_files >= 500,
        "expected >= 500 IFF files, got {iff_files}"
    );
    assert!(
        get("BHAV") >= 10_000,
        "expected >= 10,000 BHAVs, got {}",
        get("BHAV")
    );
    assert!(
        bhav_text_roundtrips >= 10_000,
        "expected >= 10,000 BHAV .antics text round-trips, got {bhav_text_roundtrips}"
    );
    assert!(
        bhav_formats.contains(&0x8000)
            && bhav_formats.contains(&0x8001)
            && bhav_formats.contains(&0x8002),
        "expected all 3 Sims 1 BHAV formats (0x8000, 0x8001, 0x8002), saw {bhav_formats:?}"
    );
    assert!(
        primitive_opcodes.len() >= 40,
        "expected >= 40 distinct SimAntics primitive opcodes, saw {}",
        primitive_opcodes.len()
    );
    assert!(get("BCON") >= 500);
    assert!(get("STR#") >= 1_000);
    assert!(get("TTAB") >= 500);
    assert!(ttab_compressed_count >= 100);
    assert!(get("SLOT") >= 500);
    assert!(get("OBJD") >= 1_000);
    assert!(get("OBJf") >= 500);
}
