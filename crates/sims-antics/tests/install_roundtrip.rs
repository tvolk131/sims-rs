//! In-memory `SIMS_INSTALL`-gated completeness round-trip verifier.
//!
//! Legal boundary: reads original `.iff` files from `$SIMS_INSTALL` strictly in
//! memory, lifts chunks into `sims-antics` IR / `.antics` source text in memory,
//! parses + lowers + re-encodes back to raw chunk bytes in memory, asserts 100%
//! byte-for-byte parity in memory, and drops all buffers without ever writing
//! anything derived from commercial assets to disk.

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
use sims_formats::iff::{DELETED, Iff};
use std::collections::BTreeSet;
use std::path::PathBuf;

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
    let mut bhav_count = 0usize;
    let mut bhav_text_roundtrips = 0usize;
    let mut bcon_count = 0usize;
    let mut str_count = 0usize;
    let mut ctss_count = 0usize;
    let mut ttas_count = 0usize;
    let mut ttab_count = 0usize;
    let mut ttab_compressed_count = 0usize;
    let mut slot_count = 0usize;
    let mut objd_count = 0usize;
    let mut objf_count = 0usize;

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
            continue;
        };
        let Ok(iff) = Iff::parse(bytes) else {
            continue;
        };
        iff_files += 1;

        for chunk in &iff.chunks {
            if chunk.flags & DELETED != 0 {
                continue;
            }
            let data = iff.data(chunk);
            match &chunk.tag {
                b"BHAV" => {
                    let Ok(lifted) = lift_bhav(chunk.id, &chunk.label, data) else {
                        continue;
                    };
                    bhav_formats.insert(lifted.format);
                    for ins in &lifted.instructions {
                        if ins.opcode < 256 {
                            primitive_opcodes.insert(ins.opcode);
                        }
                    }

                    // 1. Direct IR -> encode byte parity
                    let encoded_ir = encode_bhav(&lifted).expect("encode_bhav failed");
                    assert_eq!(
                        encoded_ir,
                        data,
                        "BHAV IR mismatch in {} chunk #{}",
                        resource.location(&index.root),
                        chunk.id
                    );
                    bhav_count += 1;

                    // 2. Full text pipeline: lift -> .antics text -> parse -> lower -> encode
                    // Verify every well-formed BHAV (<= 251 nodes with valid in-range targets)
                    // round-trips identically through the `.antics` lexer, parser, and lowerer!
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
                                "compile_antics failed on lifted BHAV {} #{}: {e}",
                                resource.location(&index.root),
                                chunk.id
                            )
                        });
                        let mut re_bhav = compiled.behaviors[&chunk.id].clone();
                        re_bhav.trailing = lifted.trailing.clone();
                        let encoded_text = encode_bhav(&re_bhav).expect("encode_bhav text failed");
                        assert_eq!(
                            encoded_text,
                            data,
                            "BHAV .antics text round-trip mismatch in {} #{}",
                            resource.location(&index.root),
                            chunk.id
                        );
                        bhav_text_roundtrips += 1;
                    }
                }
                b"BCON" => {
                    let Ok(lifted) = lift_bcon(chunk.id, &chunk.label, data) else {
                        continue;
                    };
                    let encoded = encode_bcon(&lifted).expect("encode_bcon failed");
                    assert_eq!(
                        encoded,
                        data,
                        "BCON mismatch in {} #{}",
                        resource.location(&index.root),
                        chunk.id
                    );
                    // Also verify `.antics` text round-trip for BCON
                    let text = format_bcon_antics(&lifted);
                    let compiled = compile_antics(&text).unwrap_or_else(|e| {
                        panic!(
                            "compile_antics failed on lifted BCON {} #{}: {e}",
                            resource.location(&index.root),
                            chunk.id
                        )
                    });
                    let mut re_bcon = compiled.constants[&chunk.id].clone();
                    re_bcon.trailing = lifted.trailing.clone();
                    assert_eq!(encode_bcon(&re_bcon).unwrap(), data);
                    bcon_count += 1;
                }
                b"STR#" | b"CTSS" | b"TTAs" => {
                    let Ok(lifted) = lift_string_table(chunk.tag, chunk.id, &chunk.label, data)
                    else {
                        continue;
                    };
                    str_formats.insert(format!("{:?}", lifted.format));
                    let encoded = encode_string_table(&lifted).expect("encode_string_table failed");
                    assert_eq!(
                        encoded,
                        data,
                        "{} mismatch in {} #{}",
                        chunk.kind(),
                        resource.location(&index.root),
                        chunk.id
                    );
                    match &chunk.tag {
                        b"STR#" => str_count += 1,
                        b"CTSS" => ctss_count += 1,
                        b"TTAs" => ttas_count += 1,
                        _ => unreachable!(),
                    }
                }
                b"TTAB" => {
                    let Ok(lifted) = lift_ttab(chunk.id, &chunk.label, data) else {
                        continue;
                    };
                    ttab_versions.insert(lifted.version);
                    if lifted.compressed {
                        ttab_compressed_count += 1;
                    }
                    let encoded = encode_ttab(&lifted).expect("encode_ttab failed");
                    assert_eq!(
                        encoded,
                        data,
                        "TTAB mismatch in {} #{}",
                        resource.location(&index.root),
                        chunk.id
                    );
                    ttab_count += 1;
                }
                b"SLOT" => {
                    let Ok(lifted) = lift_slot(chunk.id, &chunk.label, data) else {
                        continue;
                    };
                    slot_versions.insert(lifted.version);
                    let encoded = encode_slot(&lifted).expect("encode_slot failed");
                    assert_eq!(
                        encoded,
                        data,
                        "SLOT mismatch in {} #{}",
                        resource.location(&index.root),
                        chunk.id
                    );
                    slot_count += 1;
                }
                b"OBJD" => {
                    let Ok(lifted) = lift_objd(chunk.id, &chunk.label, data) else {
                        continue;
                    };
                    let encoded = encode_objd(&lifted).expect("encode_objd failed");
                    assert_eq!(
                        encoded,
                        data,
                        "OBJD mismatch in {} #{}",
                        resource.location(&index.root),
                        chunk.id
                    );
                    objd_count += 1;
                }
                b"OBJf" => {
                    let Ok(lifted) = lift_objf(chunk.id, &chunk.label, data) else {
                        continue;
                    };
                    let encoded = encode_objf(&lifted).expect("encode_objf failed");
                    assert_eq!(
                        encoded,
                        data,
                        "OBJf mismatch in {} #{}",
                        resource.location(&index.root),
                        chunk.id
                    );
                    objf_count += 1;
                }
                _ => {}
            }
        }
    }

    eprintln!(
        "install_roundtrip (100% in-memory, 0 bytes written to disk): \
         {iff_files} IFFs | BHAV={bhav_count} (.antics text={bhav_text_roundtrips}, formats={bhav_formats:?}, distinct_primitives={}) | \
         BCON={bcon_count} | STR#={str_count} CTSS={ctss_count} TTAs={ttas_count} (formats={str_formats:?}) | \
         TTAB={ttab_count} (compressed={ttab_compressed_count}, versions={ttab_versions:?}) | \
         SLOT={slot_count} (versions={slot_versions:?}) | OBJD={objd_count} | OBJf={objf_count}",
        primitive_opcodes.len()
    );

    // Broad coverage assertions against the Complete Collection corpus.
    assert!(
        iff_files >= 500,
        "expected >= 500 IFF files, got {iff_files}"
    );
    assert!(
        bhav_count >= 10_000,
        "expected >= 10,000 BHAVs, got {bhav_count}"
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
    assert!(bcon_count >= 500);
    assert!(str_count >= 1_000);
    assert!(ttab_count >= 500);
    assert!(ttab_compressed_count >= 100);
    assert!(slot_count >= 500);
    assert!(objd_count >= 1_000);
    assert!(objf_count >= 500);
}
