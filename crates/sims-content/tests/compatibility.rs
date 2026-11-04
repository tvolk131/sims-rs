use sims_content::{Index, compatibility::inventory};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sims-inventory-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::create_dir_all(dir.join("b")).unwrap();
        Self(dir)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn iff(chunks: Vec<([u8; 4], u16, Vec<u8>)>) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
    for (tag, id, data) in chunks {
        bytes.extend(tag);
        bytes.extend((76 + data.len() as u32).to_be_bytes());
        bytes.extend(id.to_be_bytes());
        bytes.extend(0u16.to_be_bytes());
        bytes.extend([0; 64]);
        bytes.extend(data);
    }
    bytes
}
fn bhav(opcodes: &[u16]) -> Vec<u8> {
    let mut bytes = 0x8002u16.to_le_bytes().to_vec();
    bytes.extend((opcodes.len() as u16).to_le_bytes());
    bytes.extend([0; 8]);
    for &opcode in opcodes {
        bytes.extend(opcode.to_le_bytes());
        bytes.extend([254, 255]);
        bytes.extend([0, 0, 1, 0, 255, 5, 8, 7]);
    }
    bytes
}
#[test]
fn inventory_probes_signatures_retains_context_and_exposes_ambiguity() {
    let root = Fixture::new();
    let object = iff(vec![
        (*b"BHAV", 4096, bhav(&[2, 8192, 28])),
        (*b"BHAV", 4097, bhav(&[2])),
        (*b"GLOB", 128, b"\x06shared".to_vec()),
    ]);
    fs::write(root.0.join("a/object.payload"), &object).unwrap();
    let shared = iff(vec![(*b"BHAV", 8192, bhav(&[4097, 8192]))]);
    fs::write(root.0.join("a/shared.iff"), &shared).unwrap();
    fs::write(root.0.join("irrelevant.txt"), b"not a container").unwrap();
    let report = inventory(&Index::scan(&root.0).unwrap());
    assert_eq!(report.probed_resources, 3);
    assert_eq!(report.totals.containers, 2);
    assert_eq!(report.totals.behaviors, 3);
    assert_eq!(report.totals.instructions, 6);
    assert_eq!(report.expression.byte4[&255], 2);
    assert_eq!(report.opcodes[2].operand_variants.len(), 1);
    assert_eq!(report.opcodes[28].support.rust_support, "partial");
    let nested = report
        .dependencies
        .iter()
        .find(|d| d.from_bhav == 8192 && d.to_bhav == 8192)
        .unwrap();
    assert_eq!(nested.candidate_sources, ["a/shared.iff"]);
    let private = report
        .dependencies
        .iter()
        .find(|d| d.from_bhav == 8192 && d.to_bhav == 4097)
        .unwrap();
    assert_eq!(private.candidate_sources, ["a/object.payload"]);
    assert!(report.diagnostics.is_empty());
    assert_eq!(
        serde_json::to_vec(&report).unwrap(),
        serde_json::to_vec(&inventory(&Index::scan(&root.0).unwrap())).unwrap()
    );
    fs::write(root.0.join("b/SHARED.iff"), &shared).unwrap();
    let ambiguous = inventory(&Index::scan(&root.0).unwrap());
    assert_ne!(ambiguous.corpus_sha256, report.corpus_sha256);
    let call = ambiguous
        .dependencies
        .iter()
        .find(|d| d.from_bhav == 4096)
        .unwrap();
    assert_eq!(call.resolution, "ambiguous_physical_sources");
    assert_eq!(call.candidate_sources.len(), 2);
    assert_eq!(fs::read(root.0.join("a/object.payload")).unwrap(), object);
}

#[test]
fn inventory_retains_parse_gaps_duplicate_ids_and_object_callback_roots() {
    let root = Fixture::new();
    let mut functions = vec![0; 8];
    functions.extend(b"fJBO");
    functions.extend(1u32.to_le_bytes());
    functions.extend(4096u16.to_le_bytes());
    functions.extend(4097u16.to_le_bytes());
    let data = iff(vec![
        (*b"BHAV", 4096, bhav(&[2])),
        (*b"BHAV", 4096, bhav(&[0xffff])),
        (*b"BHAV", 4097, vec![2, 128]),
        (*b"OBJf", 7, functions),
    ]);
    fs::write(root.0.join("callbacks.stx"), data).unwrap();
    let report = inventory(&Index::scan(&root.0).unwrap());
    assert_eq!(report.totals.behaviors, 2);
    assert_eq!(report.containers[0].duplicate_bhav_ids, [4096]);
    assert_eq!(
        report.containers[0].function_tables[0].table.entries[0].action,
        4097
    );
    assert_eq!(report.diagnostics.len(), 1);
    assert!(report.diagnostics[0].contains("BHAV#4097"));
    assert_eq!(
        report.dependencies[0].resolution,
        "high_bit_opcode_needs_upstream_classification"
    );
}
