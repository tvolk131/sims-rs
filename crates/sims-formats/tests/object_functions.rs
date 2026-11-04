use sims_formats::execution::ObjectFunctions;

#[test]
fn object_functions_preserve_condition_action_and_reject_bad_framing() {
    let mut bytes = vec![0; 8];
    bytes.extend(b"fJBO");
    bytes.extend(2u32.to_le_bytes());
    for value in [4096u16, 4097, 0, 8192] {
        bytes.extend(value.to_le_bytes());
    }
    let parsed = ObjectFunctions::parse(&bytes).unwrap();
    assert_eq!(parsed.entries[0].condition, 4096);
    assert_eq!(parsed.entries[0].action, 4097);
    assert_eq!(parsed.entries[1].action, 8192);
    for n in 0..bytes.len() {
        assert!(ObjectFunctions::parse(&bytes[..n]).is_err());
    }
    let mut bad = bytes.clone();
    bad[8] = b'x';
    assert!(ObjectFunctions::parse(&bad).is_err());
    let mut bad = bytes.clone();
    bad[4] = 1;
    assert!(ObjectFunctions::parse(&bad).is_err());
    let mut bad = bytes;
    bad[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(ObjectFunctions::parse(&bad).is_err());
}
