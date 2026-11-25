//! The neighborhood save against a file the original wrote.
//!
//! `SIMS_UD2_AFTER` names a recorded neighborhood folder whose
//! `Neighborhood.iff` the original's save (0x4252cc) wrote last. The file
//! is local and never committed; without it these tests do nothing.

use sims_formats::iff::Iff;
use sims_formats::neighborhood::{Family, Neighborhood, Neighbors};
use sims_formats::neighborhood_file::{NeighborhoodFile, parse_family_name};
use sims_formats::tatt::Tatt;
use std::path::PathBuf;

fn recorded() -> Option<Vec<u8>> {
    let Some(folder) = std::env::var_os("SIMS_UD2_AFTER") else {
        eprintln!("SIMS_UD2_AFTER not set; skipping");
        return None;
    };
    Some(std::fs::read(PathBuf::from(folder).join("Neighborhood.iff")).expect("read the save"))
}

fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y)
}

/// Each chunk the save wrote is what the matching writer gives for what
/// the reader read from it.
#[test]
fn each_recorded_chunk_is_its_reading_written_again() {
    let Some(bytes) = recorded() else { return };
    let iff = Iff::parse(bytes).unwrap();
    let mut checked = 0;
    for chunk in &iff.chunks {
        let data = iff.data(chunk);
        let written = match &chunk.tag {
            b"NGBH" => Neighborhood::parse(data).unwrap().to_bytes().unwrap(),
            b"NBRS" => {
                let (neighbors, padding) = Neighbors::parse_with_padding(data).unwrap();
                neighbors.to_bytes_with_padding(&padding).unwrap()
            }
            b"FAMI" => Family::parse_chunk(data, chunk.id)
                .unwrap()
                .to_chunk(chunk.id)
                .unwrap(),
            b"FAMs" => {
                let (name, stray) = parse_family_name(data).unwrap();
                sims_formats::neighborhood_file::family_name(&name, stray).unwrap()
            }
            b"TATT" => Tatt::parse(data).unwrap().to_bytes().unwrap(),
            _ => continue,
        };
        assert!(
            written == data,
            "{} #{} differs from byte {:?}: {} bytes written, {} recorded",
            chunk.kind(),
            chunk.id,
            first_difference(&written, data),
            written.len(),
            data.len()
        );
        checked += 1;
    }
    assert!(checked > 4, "only {checked} chunks checked");
}

/// The whole file: the same resources, in the same order, with the same
/// resource map.
#[test]
fn the_recorded_save_is_written_again_byte_for_byte() {
    let Some(bytes) = recorded() else { return };
    let (file, uninitialized) = NeighborhoodFile::parse(&bytes).unwrap();
    let written = file.to_bytes(&uninitialized).unwrap();
    assert_eq!(written.len(), bytes.len());
    assert!(
        written == bytes,
        "the written file differs from byte {:?}",
        first_difference(&written, &bytes)
    );
}

/// The save writes the family vector in its order, which the original
/// keeps sorted by number, then ID, both descending (0x425209).
#[test]
fn the_recorded_families_are_in_the_vectors_sorted_order() {
    let Some(bytes) = recorded() else { return };
    let (file, _) = NeighborhoodFile::parse(&bytes).unwrap();
    let keys: Vec<_> = file
        .families
        .iter()
        .map(|record| (record.family.number, record.id))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(keys, sorted);
}
