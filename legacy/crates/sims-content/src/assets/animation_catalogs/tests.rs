use super::*;

fn clips() -> Arc<[Animation]> {
    Vec::new().into()
}

#[test]
fn complete_bytes_and_parser_mode_are_required_for_reuse() -> Result<()> {
    let mut catalogs = AnimationCatalogs::default();
    let first = catalogs.parse_with("source", vec![1, 2], true, |_| Ok(clips()))?;
    let repeated = catalogs.parse_with("source", vec![1, 2], true, |_| {
        panic!("identical catalog decoded again")
    })?;
    assert!(Arc::ptr_eq(&first, &repeated));
    let ordinary = catalogs.parse_with("source", vec![1, 2], false, |_| Ok(clips()))?;
    assert!(!Arc::ptr_eq(&first, &ordinary));
    let changed = catalogs.parse_with("source", vec![1, 3], false, |_| Ok(clips()))?;
    assert!(!Arc::ptr_eq(&ordinary, &changed));
    assert_eq!(catalogs.retained_bytes, 2);
    assert_eq!(catalogs.entries.len(), 1);
    Ok(())
}

#[test]
fn changed_failures_invalidate_old_certification_and_are_not_cached() -> Result<()> {
    let mut catalogs = AnimationCatalogs::default();
    let original = catalogs.parse_with("source", vec![1], true, |_| Ok(clips()))?;
    let mut calls = 0;
    for _ in 0..2 {
        let error = catalogs
            .parse_with("source", vec![2], true, |_| {
                calls += 1;
                bail!("original catalog failure")
            })
            .unwrap_err();
        assert_eq!(error.to_string(), "original catalog failure");
        assert_eq!(catalogs.retained_bytes, 0);
        assert!(catalogs.entries.is_empty());
    }
    assert_eq!(calls, 2);
    let restored = catalogs.parse_with("source", vec![1], true, |_| Ok(clips()))?;
    assert!(!Arc::ptr_eq(&original, &restored));
    Ok(())
}

#[test]
fn separate_sources_and_forked_owners_do_not_replace_each_other() -> Result<()> {
    let mut catalogs = AnimationCatalogs::default();
    let first = catalogs.parse_with("first", vec![1], true, |_| Ok(clips()))?;
    let other = catalogs.parse_with("second", vec![1], true, |_| Ok(clips()))?;
    assert!(!Arc::ptr_eq(&first, &other));
    let mut forked = catalogs.clone();
    catalogs.parse_with("first", vec![2], true, |_| Ok(clips()))?;
    let retained = forked.parse_with("first", vec![1], true, |_| panic!("fork lost its input"))?;
    assert!(Arc::ptr_eq(&first, &retained));
    assert_eq!(catalogs.retained_bytes, 2);
    assert_eq!(forked.retained_bytes, 2);
    Ok(())
}

#[test]
fn byte_limit_falls_back_to_decoding_without_caching_success_or_failure() -> Result<()> {
    let mut catalogs = AnimationCatalogs {
        limit: 1,
        ..Default::default()
    };
    let mut calls = 0;
    for _ in 0..2 {
        catalogs.parse_with("large", vec![1, 2], true, |_| {
            calls += 1;
            Ok(clips())
        })?;
    }
    assert_eq!(calls, 2);
    assert_eq!(catalogs.retained_bytes, 0);
    assert!(catalogs.entries.is_empty());
    catalogs.parse_with("small", vec![1], true, |_| Ok(clips()))?;
    assert_eq!(catalogs.retained_bytes, 1);
    catalogs
        .parse_with("small", vec![2, 3], true, |_| bail!("failure"))
        .unwrap_err();
    assert_eq!(catalogs.retained_bytes, 0);
    assert!(catalogs.entries.is_empty());
    Ok(())
}
