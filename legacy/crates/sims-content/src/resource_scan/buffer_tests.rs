use super::{Scan, tests::Directory, tests::resource};
use std::{fs, sync::Arc};

#[test]
fn one_buffer_reuses_its_allocation_for_current_full_ranges_and_owned_inputs() {
    let directory = Directory::new();
    let path = directory.path("archive");
    let initial: Vec<_> = (0..8192).map(|offset| (offset % 251) as u8).collect();
    fs::write(&path, &initial).unwrap();
    let mut scan = Scan::default();
    let mut bytes = Vec::new();
    scan.read_into(&resource(&path, 0, 8192), &mut bytes)
        .unwrap();
    let allocation = bytes.as_ptr();
    let capacity = bytes.capacity();
    for (offset, size, archived) in [
        (8190, 2, true),
        (0, 0, true),
        (0, 8192, false),
        (1, 8191, true),
        (0, 8192, true),
    ] {
        let mut entry = resource(&path, offset, size);
        entry.archived = archived;
        scan.read_into(&entry, &mut bytes).unwrap();
        assert_eq!(bytes, entry.read().unwrap());
        assert_eq!(bytes.as_ptr(), allocation);
        assert_eq!(bytes.capacity(), capacity);
    }
    let mut owned = resource(&directory.path("must-not-open"), u64::MAX, 8192);
    owned.owned = Some(Arc::from(vec![73; 8192]));
    scan.read_into(&owned, &mut bytes).unwrap();
    assert_eq!(bytes, vec![73; 8192]);
    assert_eq!(bytes.as_ptr(), allocation);
    assert_eq!(bytes.capacity(), capacity);
    let retained = bytes.clone();
    fs::write(&path, vec![94; 8192]).unwrap();
    scan.read_into(&resource(&path, 0, 8192), &mut bytes)
        .unwrap();
    assert_eq!(bytes, vec![94; 8192]);
    assert_eq!(retained, vec![73; 8192]);
    assert_eq!(bytes.as_ptr(), allocation);
}

#[test]
fn failed_reused_reads_preserve_ordinary_errors_and_recover_from_current_bytes() {
    let directory = Directory::new();
    let path = directory.path("archive");
    fs::write(&path, b"original").unwrap();
    let mut scan = Scan::default();
    let mut bytes = vec![0xff; 8192];
    let mut entries = vec![
        resource(&path, u64::MAX, 2),
        resource(&path, 4, 5),
        resource(&directory.path("missing"), 0, 8),
        resource(&directory.0, 0, 1),
        resource(&path, 0, crate::MAX_RESOURCE + 1),
    ];
    let mut compressed = resource(&path, 0, 8);
    compressed.unpacked_size = 9;
    entries.push(compressed);
    let mut invalid_owned = resource(&path, 0, 8);
    invalid_owned.owned = Some(Arc::from(&b"short"[..]));
    entries.push(invalid_owned);
    for entry in entries {
        let expected = entry.read().unwrap_err();
        let actual = scan.read_into(&entry, &mut bytes).unwrap_err();
        assert_eq!(format!("{actual:#}"), format!("{expected:#}"));
        scan.read_into(&resource(&path, 0, 8), &mut bytes).unwrap();
        assert_eq!(bytes, b"original");
    }
    fs::write(&path, b"x").unwrap();
    let entry = resource(&path, 0, 8);
    assert_eq!(
        format!("{:#}", scan.read_into(&entry, &mut bytes).unwrap_err()),
        format!("{:#}", entry.read().unwrap_err())
    );
    fs::write(&path, b"new-data").unwrap();
    scan.read_into(&entry, &mut bytes).unwrap();
    assert_eq!(bytes, b"new-data");
}
