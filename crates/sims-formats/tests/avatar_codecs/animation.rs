use sims_formats::animation::{read_bcf, read_cfp};

fn word(out: &mut Vec<u8>, v: u32) {
    out.extend(v.to_le_bytes());
}
fn string(out: &mut Vec<u8>, v: &str) {
    out.push(v.len() as u8);
    out.extend(v.as_bytes());
}
fn bcf() -> Vec<u8> {
    let mut b = vec![];
    for n in [0, 0, 1] {
        word(&mut b, n);
    }
    string(&mut b, "test");
    string(&mut b, "samples");
    word(&mut b, 1000f32.to_bits());
    word(&mut b, 0);
    word(&mut b, 0);
    word(&mut b, 30);
    word(&mut b, 30);
    word(&mut b, 1);
    string(&mut b, "ROOT");
    word(&mut b, 30);
    word(&mut b, 1000f32.to_bits());
    for n in [1, 1, 0, 0, 0, 1, 2] {
        word(&mut b, n);
    }
    for (time, key, value) in [(500, "xevt", "7"), (0, "footstep", "-1")] {
        word(&mut b, time);
        word(&mut b, 1);
        string(&mut b, key);
        string(&mut b, value);
    }
    b
}
#[test]
fn bcf_preserves_names_samples_and_stable_timed_properties() {
    let a = read_bcf(&bcf()).unwrap().remove(0);
    assert_eq!(a.name, "test");
    assert_eq!(a.skill, "samples");
    assert_eq!(a.frames, 30);
    assert_eq!(a.motions[0].rotation, Some(0));
    assert_eq!(
        a.events.iter().map(|e| e.time_ms).collect::<Vec<_>>(),
        [0, 500]
    );
    assert_eq!(a.events[1].pairs[0], ("xevt".into(), "7".into()));
    assert_eq!(
        a.motions[0].event_lists[0]
            .iter()
            .map(|e| e.time_ms)
            .collect::<Vec<_>>(),
        [500, 0]
    );
}
#[test]
fn bcf_presence_words_and_signed_event_times_follow_native_motion_reader() {
    let mut b = bcf();
    // The fixture's timed-list presence word precedes its count and first time.
    let marker = b
        .windows(8)
        .position(|w| w == [1, 0, 0, 0, 2, 0, 0, 0])
        .unwrap();
    b[marker..marker + 4].copy_from_slice(&7_u32.to_le_bytes());
    b[marker + 8..marker + 12].copy_from_slice(&(-1_i32).to_le_bytes());
    let a = read_bcf(&b).unwrap().remove(0);
    assert_eq!(a.motions[0].event_lists.len(), 1);
    assert_eq!(a.motions[0].event_lists[0][0].time_ms, -1);
    assert_eq!(a.events.len(), 1);
    b[marker + 8..marker + 12].copy_from_slice(&1040_i32.to_le_bytes());
    let a = read_bcf(&b).unwrap().remove(0);
    assert_eq!(a.motions[0].event_lists[0][0].time_ms, 1040);
    assert_eq!(a.events.len(), 2);
}
#[test]
fn bcf_keeps_motions_with_no_frames() {
    let mut b = bcf();
    // The motion's frame count follows its bone name.
    let at = b.windows(5).position(|w| w == b"\x04ROOT").unwrap() + 5;
    assert_eq!(b[at..at + 4], 30_u32.to_le_bytes());
    b[at..at + 4].copy_from_slice(&0_u32.to_le_bytes());
    let a = read_bcf(&b).unwrap().remove(0);
    assert_eq!(a.motions[0].frames, 0);
}
#[test]
fn bcf_rejects_truncations_bad_counts_and_nonfinite_values() {
    let b = bcf();
    for n in 0..b.len() {
        assert!(read_bcf(&b[..n]).is_err(), "prefix {n}");
    }
    let mut bad = b.clone();
    bad[0] = 1;
    assert!(read_bcf(&bad).is_err());
    let mut bad = b.clone();
    bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read_bcf(&bad).is_err());
    let at = 12 + 1 + 4 + 1 + 7;
    let mut bad = b.clone();
    bad[at..at + 4].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
    assert!(read_bcf(&bad).is_err());
    let mut bad = b;
    bad.push(0);
    assert!(read_bcf(&bad).is_err());
}
#[test]
fn cfp_literal_delta_repeat_and_component_reset_preserve_native_coordinates() {
    let mut data = vec![255];
    data.extend(2.0f32.to_le_bytes());
    data.extend([254, 1, 0]); // 3 X values
    data.extend([254, 2, 0]); // Y starts at zero, 3 copies
    data.extend([127, 126, 126]); // Z delta, then unchanged twice
    let s = read_cfp(&data, 3, 0).unwrap();
    assert_eq!(s.translations.len(), 3);
    assert_eq!(s.translations[0][0], 2.0);
    assert_eq!(s.translations[2][0], 2.0);
    assert_eq!(s.translations[0][1], 0.0);
    assert_eq!(s.translations[0][2], 3.9676e-10f32);
    assert_eq!(s.translations[0], s.translations[2]);
    for n in 0..data.len() {
        assert!(read_cfp(&data[..n], 3, 0).is_err());
    }
}
#[test]
fn cfp_rejects_boundary_crossing_nonfinite_reserved_and_oversized_streams() {
    for data in [
        vec![254, 3, 0],
        vec![253],
        vec![255, 0, 0, 128, 127],
        vec![126, 126, 126, 126],
    ] {
        assert!(read_cfp(&data, 1, 0).is_err());
    }
    assert!(read_cfp(&[], u32::MAX, 0).is_err());
    assert!(read_cfp(&[], 0, u32::MAX).is_err());
}
