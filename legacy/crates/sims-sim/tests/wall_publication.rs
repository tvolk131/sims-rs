#![cfg(feature = "private-fixtures")]

//! Saved callback boundaries recovered from4a814d/4a817e/4b1459.
//! These focused assertions are not a substitute for the native corpus.
use sims_sim::lot_presenter::lifecycle::{Cursor, Reply, Request, State, roof};

fn state() -> State {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/view-lifecycle.json")).unwrap();
    let mut state: State =
        serde_json::from_value(fixture["cases"][1]["input"]["state"].clone()).unwrap();
    state.all_walls = 0;
    state.visibility.enabled = 1;
    state.visibility.dirty = 17;
    state
}

fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap()
}

#[test]
fn walls_store_before_clear_and_invalidate_after_clear_returns() {
    let mut state = state();
    let history = state.history.clone();
    let mut cursor = Cursor::walls_down(&mut state, 1).unwrap();
    assert_eq!(state.all_walls, 1);
    assert_eq!(state.visibility.enabled, 1);
    assert_eq!(state.visibility.dirty, 17);
    assert_eq!(cursor.request(), Some(Request::ClearMask));
    (state, cursor) = roundtrip(&(state, cursor));
    let before = (state.clone(), cursor.clone());
    assert!(
        cursor
            .acknowledge(&mut state, &Request::InvalidateView, Reply::Done)
            .is_err()
    );
    assert_eq!((state.clone(), cursor.clone()), before);
    // A supplied clear callback can change fields; only the two native
    // stores following its return should overwrite that callback's writes.
    state.all_walls = 128;
    state.visibility.enabled = 255;
    state.visibility.dirty = 99;
    cursor
        .acknowledge(&mut state, &Request::ClearMask, Reply::Done)
        .unwrap();
    assert_eq!(state.all_walls, 128);
    assert_eq!(state.visibility.enabled, 0);
    assert_eq!(state.visibility.dirty, 1);
    assert_eq!(cursor.request(), Some(Request::InvalidateView));
    (state, cursor) = roundtrip(&(state, cursor));
    state.visibility.dirty = 77;
    cursor
        .acknowledge(&mut state, &Request::InvalidateView, Reply::Done)
        .unwrap();
    assert!(cursor.complete());
    assert_eq!(state.visibility.dirty, 77);
    assert_eq!(state.history, history);
}

#[test]
fn walls_same_value_returns_even_with_active_mask_and_disabled_mask_skips_clear() {
    let mut state = state();
    let before = state.clone();
    assert!(Cursor::walls_down(&mut state, 0).unwrap().complete());
    assert_eq!(state, before);
    state.visibility.enabled = 0;
    let cursor = Cursor::walls_down(&mut state, 0x82).unwrap();
    assert_eq!(cursor.request(), Some(Request::InvalidateView));
    assert_eq!(state.all_walls, 0x82);
    assert_eq!(state.visibility.dirty, 1);
}

#[test]
fn roof_low_bytes_force_live_controls_and_post_invalidation_stores() {
    for (value, force, changed) in [
        (0x100, 0x100, false),
        (0x100, 0x101, true),
        (0x101, 0, true),
    ] {
        let (mut roof, mut dirty, mut flags) = (0, 17, 0x11223344u32);
        let mut cursor = roof::Cursor::new(value, force);
        let first = cursor.advance(&mut roof, &mut dirty, true).unwrap();
        if !changed {
            assert_eq!(first, None);
            assert!(cursor.complete());
            assert_eq!((roof, dirty), (0, 17));
            continue;
        }
        assert_eq!(first, Some(roof::Request::InvalidateView));
        assert_eq!((roof, dirty), (value as u8, 17));
        cursor = roundtrip(&cursor);
        let before = cursor.clone();
        assert!(cursor.acknowledge(roof::Request::UiFlags(0x80)).is_err());
        assert_eq!(cursor, before);
        assert_eq!(cursor.advance(&mut roof, &mut dirty, false).unwrap(), first);
        // Callback writes persist except for the following dirty-byte store.
        roof = 255;
        dirty = 99;
        cursor.acknowledge(roof::Request::InvalidateView).unwrap();
        cursor = roundtrip(&cursor);
        let request = cursor.advance(&mut roof, &mut dirty, force != 0).unwrap();
        assert_eq!((roof, dirty), (255, 1));
        if force != 0 {
            assert_eq!(request, Some(roof::Request::UiFlags(0x80)));
            cursor = roundtrip(&cursor);
            assert!(!cursor.complete());
            flags |= 0x80;
            cursor.acknowledge(roof::Request::UiFlags(0x80)).unwrap();
        } else {
            assert_eq!(request, None);
        }
        assert!(cursor.complete());
        assert_eq!(flags, if force != 0 { 0x112233c4 } else { 0x11223344 });
        assert!(cursor.acknowledge(roof::Request::InvalidateView).is_err());
    }
}
