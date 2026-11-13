//! Authored controls, UNRUN. Real typed/envelope restoration is exercised by
//! sims-app's asset-free managed fixture through Input::read and the real loader.
use super::*;
fn descriptor(count: u64) -> Descriptor {
    Descriptor { bytes: count, regular: true, modified_unix_ns: Some("1".into()),
        device: Some(2), inode: Some(3), mode: Some(0o100644),
        change_seconds: Some(0), change_nanoseconds: Some(1) }
}
#[test]
fn metadata_mismatch_or_unavailable_never_claims_consistency() {
    let before = descriptor(4);
    assert_eq!(descriptor_status(Some(&before), Some(&before), 4), "consistent");
    assert_eq!(descriptor_status(Some(&before), Some(&before), 3), "inconsistent");
    for field in 0..7 {
        let mut after = before.clone();
        match field {
            0 => after.bytes += 1, 1 => after.inode = Some(10),
            2 => after.device = Some(11), 3 => after.mode = Some(0o100600),
            4 => after.modified_unix_ns = Some("2".into()),
            5 => after.change_seconds = Some(1), 6 => after.change_nanoseconds = Some(2),
            _ => unreachable!(),
        }
        assert_eq!(descriptor_status(Some(&before), Some(&after), 4), "inconsistent");
    }
    assert_eq!(descriptor_status(None, Some(&before), 4), "unavailable");
    let mut missing = before;
    missing.inode = None;
    assert_eq!(descriptor_status(Some(&missing), Some(&missing), 4), "unavailable");
}
#[test]
fn malformed_json_bare_saved_and_checkpoint_shapes_are_not_observed_player_inputs() {
    for raw in [b"{".as_slice(), b"null", b"[]", b"{}", br#"{"schema":1,"session":null,"application":{}}"#] {
        assert!(Input::decode(raw.to_vec(), "authored".into(), None, None).is_err());
    }
}
#[test]
fn mutation_of_the_owned_raw_vector_invalidates_size_observation_before_decode() {
    // This component control supplies the same inconsistent descriptor pair
    // the file reader would observe; no fabricated Session/certificate exists.
    let before = descriptor(2);
    let mut raw = b"{}".to_vec();
    raw.push(b' ');
    assert_eq!(descriptor_status(Some(&before), Some(&before), raw.len() as u64), "inconsistent");
    assert!(Input::decode(raw, "authored".into(), Some(before.clone()), Some(before)).is_err());
}
