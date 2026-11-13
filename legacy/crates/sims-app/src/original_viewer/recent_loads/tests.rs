//! Bounded state controls only; adapter controls supply actual Load evidence.
//! ALL UNRUN, including compilation. Parent filter: recent_loads_
use super::*;

#[test]
fn recent_loads_state_exact_spelling_dedupes_orders_and_evicts() {
    let mut history = History::default();
    for name in ["a//b.json", "a/b.json", "./a/b.json", "家族 e\u{301}.json"] {
        history.record(Path::new(name));
    }
    assert_eq!(history.paths().len(), 4, "component equality would erase spelling");
    history.record(Path::new("a/b.json"));
    assert_eq!(history.paths()[0].as_os_str(), "a/b.json");
    assert_eq!(history.paths()[1].as_os_str(), "家族 e\u{301}.json");
    for i in 0..10 { history.record(Path::new(&format!("{i}.json"))); }
    assert_eq!(history.paths().len(), MAX_ENTRIES);
    assert_eq!(history.paths()[0].as_os_str(), "9.json");
    assert_eq!(history.paths()[7].as_os_str(), "2.json");
    assert!(history.retained_bytes() <= MAX_TOTAL_BYTES);
}

#[test]
fn recent_loads_state_limits_and_snapshot_never_invent_or_retarget_paths() {
    let mut history = History::default();
    history.record(Path::new("kept.json"));
    let snapshot = history.snapshot();
    for invalid in [String::new(), "nul\0.json".into(), "x".repeat(MAX_PATH_BYTES + 1)] {
        history.record(Path::new(&invalid));
    }
    assert_eq!(history.paths(), snapshot.paths());
    for i in 0..9 {
        let value = format!("{i}{}", "x".repeat(MAX_PATH_BYTES - 1));
        history.record(Path::new(&value));
    }
    assert_eq!(history.retained_bytes(), MAX_TOTAL_BYTES);
    assert_eq!(history.paths().len(), MAX_ENTRIES);
    assert_eq!(snapshot.paths()[0].as_os_str(), "kept.json");
}

#[cfg(unix)]
#[test]
fn recent_loads_state_non_utf8_is_skipped_without_lossy_alias() {
    use std::os::unix::ffi::OsStringExt;
    let mut history = History::default();
    history.record(Path::new("valid.json"));
    history.record(Path::new(&std::ffi::OsString::from_vec(b"bad-\xff.json".to_vec())));
    assert_eq!(history.paths().len(), 1);
    assert_eq!(history.paths()[0].as_os_str(), "valid.json");
}

#[cfg(windows)]
#[test]
fn recent_loads_state_nonrepresentable_wide_path_is_skipped() {
    use std::os::windows::ffi::OsStringExt;
    let mut history = History::default();
    history.record(Path::new(&std::ffi::OsString::from_wide(&[0xd800])));
    assert!(history.paths().is_empty());
}
