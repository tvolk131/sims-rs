//! Keep the previous full-scan successor as an independent behavioral control.
use super::*;
use crate::pose::clothing::{catalog_mount::Metadata, name_map::hash};

fn previous_next(search: &mut MountedSearch, manager: &Manager) -> Result<Option<Entry>, String> {
    let Some(current) = &search.current else {
        return Ok(None);
    };
    let files = manager
        .directory(&search.directory)?
        .ok_or("object iterator directory disappeared")?;
    let mut entries = files.iter();
    if !entries.any(|(name, _)| name == current) {
        return Err("object iterator current node disappeared".into());
    }
    let next = entries.next().map(|(name, metadata)| Entry {
        directory: search.directory.clone(),
        name: name.to_vec(),
        directory_flag: metadata.directory,
        writable: metadata.writable,
    });
    search.current = next.as_ref().map(|e| e.name.clone());
    Ok(next)
}
fn metadata(n: usize) -> Metadata {
    Metadata {
        archive: n as u32,
        offset: 0,
        reserved: 0,
        size: 0,
        archived: 0,
        writable: n as u8,
        directory: (n >> 8) as u8,
    }
}
fn insert(manager: &mut Manager, n: usize) {
    manager
        .directory_mut(b"./Objects/")
        .unwrap()
        .insert(format!("MiXeD-{n:05}.IFF").as_bytes(), metadata(n))
        .unwrap();
}
fn cursors(manager: &Manager) -> (MountedSearch, MountedSearch) {
    let (actual, _) = MountedSearch::first(manager, b"./OBJECTS/*").unwrap();
    let reference = MountedSearch {
        directory: actual.directory.clone(),
        current: actual.current.clone(),
    };
    (actual, reference)
}
fn compare_next(
    actual: &mut MountedSearch,
    reference: &mut MountedSearch,
    manager: &Manager,
) -> Result<Option<Entry>, String> {
    let expected = previous_next(reference, manager);
    let result = actual.next(manager);
    assert_eq!(result, expected);
    assert_eq!(actual.current, reference.current);
    assert_eq!(actual.directory, reference.directory);
    result
}

#[test]
fn successor_matches_full_scan_across_growth_and_collisions() {
    for count in [0, 1, 2, 192, 193, 194, 389, 390, 769, 770, 1544] {
        let mut manager = Manager::default();
        manager.directory_mut(b"./Objects/").unwrap();
        for n in 0..count {
            insert(&mut manager, n);
        }
        let (mut actual, mut reference) = cursors(&manager);
        let mut visited = usize::from(actual.current.is_some());
        while compare_next(&mut actual, &mut reference, &manager)
            .unwrap()
            .is_some()
        {
            visited += 1;
        }
        assert_eq!(visited, count);
        assert!(
            compare_next(&mut actual, &mut reference, &manager)
                .unwrap()
                .is_none()
        );
    }
    // Force a long same-bucket chain; its insertion order is not lexical order.
    let mut manager = Manager::default();
    let files = manager.directory_mut(b"./Objects/").unwrap();
    for (n, name) in (0..100_000)
        .map(|n| format!("collision-{n}.iff").into_bytes())
        .filter(|name| hash(name) % 193 == 57)
        .take(40)
        .enumerate()
    {
        files.insert(&name, metadata(n)).unwrap();
    }
    let (mut actual, mut reference) = cursors(&manager);
    while compare_next(&mut actual, &mut reference, &manager)
        .unwrap()
        .is_some()
    {}
}

#[test]
fn successor_observes_live_metadata_insertion_and_rehash() {
    for initial in [1, 192, 193, 389] {
        let mut manager = Manager::default();
        for n in 0..initial {
            insert(&mut manager, n);
        }
        let (mut actual, mut reference) = cursors(&manager);
        for step in 0..50 {
            if step < 12 {
                // This crosses table-growth boundaries while retaining current.
                for n in 0..30 {
                    insert(&mut manager, initial + step * 30 + n);
                }
                let files = manager.directory_mut(b"./Objects/").unwrap();
                let names: Vec<_> = files.iter().map(|(k, _)| k.to_vec()).collect();
                for name in names {
                    let value = files.get_mut(&name).unwrap().unwrap();
                    value.writable = step as u8;
                    value.directory = 255 - step as u8;
                    // Duplicate insertion must retain the existing payload.
                    assert!(!files.insert(&name, metadata(0)).unwrap());
                }
            }
            if compare_next(&mut actual, &mut reference, &manager)
                .unwrap()
                .is_none()
            {
                break;
            }
        }
        while compare_next(&mut actual, &mut reference, &manager)
            .unwrap()
            .is_some()
        {}
        insert(&mut manager, 999_999);
        assert!(
            compare_next(&mut actual, &mut reference, &manager)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn successor_preserves_missing_directory_node_and_counted_key_behavior() {
    let mut manager = Manager::default();
    for n in 0..5 {
        insert(&mut manager, n);
    }
    let (mut actual, mut reference) = cursors(&manager);
    let current = actual.current.clone();
    assert_eq!(
        compare_next(&mut actual, &mut reference, &Manager::default()).unwrap_err(),
        "object iterator directory disappeared"
    );
    assert_eq!(actual.current, current);
    let mut replacement = Manager::default();
    replacement.directory_mut(b"./Objects/").unwrap();
    assert_eq!(
        compare_next(&mut actual, &mut reference, &replacement).unwrap_err(),
        "object iterator current node disappeared"
    );
    assert_eq!(actual.current, current);
    let files = replacement.directory_mut(b"./Objects/").unwrap();
    for key in [
        b"".as_slice(),
        b"A.iff",
        b"a/IFF",
        b"a:iff",
        &[0xff, b'.', b'i', b'f', b'f'],
    ] {
        files.insert(key, metadata(2)).unwrap();
    }
    let (mut actual, mut reference) = cursors(&replacement);
    while compare_next(&mut actual, &mut reference, &replacement)
        .unwrap()
        .is_some()
    {}
}
