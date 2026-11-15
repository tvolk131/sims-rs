//! The friendship threshold the interface reads from a global file
//! without the simulation's program, on synthetic files.

use sims_formats::iff::Iff;
use sims_ui::household::friendship_threshold_in;

/// An `FCNS` table (version 0) of `entries`.
fn table(entries: &[(&str, f32)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(0u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(b"SNCF");
    out.extend((entries.len() as u32).to_le_bytes());
    for (name, value) in entries {
        out.extend(name.as_bytes());
        out.push(0);
        if out.len() % 2 != 0 {
            out.push(0);
        }
        out.extend(value.to_le_bytes());
    }
    out
}

/// An IFF file of `chunks` (tag, ID, payload) in this order, with no
/// resource map.
fn iff(chunks: &[(&[u8; 4], u16, Vec<u8>)]) -> Iff {
    let mut out = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOLE 1995".to_vec();
    out.resize(64, 0);
    for (tag, id, payload) in chunks {
        out.extend(*tag);
        out.extend((76 + payload.len() as u32).to_be_bytes());
        out.extend(id.to_be_bytes());
        out.extend(0u16.to_be_bytes());
        out.extend([0; 64]);
        out.extend(payload);
    }
    Iff::parse(out).unwrap()
}

fn read(value: f32) -> i32 {
    friendship_threshold_in(&iff(&[(
        b"FCNS",
        2,
        table(&[("Friendship Threshold", value)]),
    )]))
    .unwrap()
}

#[test]
fn the_last_table_2_in_the_file_wins_and_the_name_is_matched_in_any_case() {
    let global = iff(&[
        (b"FCNS", 2, table(&[("friendship threshold", 30.0)])),
        (b"FCNS", 3, table(&[("friendship threshold", 99.0)])),
        (b"FCNS", 2, table(&[("FRIENDSHIP THRESHOLD", 75.0)])),
    ]);
    assert_eq!(friendship_threshold_in(&global).unwrap(), 75);
}

#[test]
fn a_missing_table_or_entry_gives_25() {
    assert_eq!(friendship_threshold_in(&iff(&[])).unwrap(), 25);
    let global = iff(&[(b"FCNS", 2, table(&[("other", 1.0)]))]);
    assert_eq!(friendship_threshold_in(&global).unwrap(), 25);
}

#[test]
fn any_malformed_tuning_table_is_an_error_as_it_stops_the_programs_load() {
    // Even one a later chunk replaces, and one of another ID.
    for id in [2, 5] {
        let global = iff(&[
            (b"FCNS", id, vec![1, 2, 3]),
            (b"FCNS", 2, table(&[("friendship threshold", 50.0)])),
        ]);
        assert!(friendship_threshold_in(&global).is_err(), "{id}");
    }
}

#[test]
fn the_value_is_truncated_as_ftol_keeps_the_low_word() {
    assert_eq!(read(50.9), 50);
    assert_eq!(read(-3.7), -3);
    // `__ftol` truncates to 64 bits and keeps the low 32.
    assert_eq!(read(4_294_967_808.0), 512);
    // NaN, the infinities and values past 64 bits give the integer
    // indefinite, whose low word is 0.
    assert_eq!(read(f32::NAN), 0);
    assert_eq!(read(f32::INFINITY), 0);
    assert_eq!(read(f32::NEG_INFINITY), 0);
    assert_eq!(read(1e30), 0);
}
