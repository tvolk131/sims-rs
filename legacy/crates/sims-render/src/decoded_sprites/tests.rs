use super::*;
use std::sync::Arc;

mod installed;

fn frame(value: u8) -> Frame {
    Frame {
        width: 1,
        height: 2,
        x: -3,
        y: 4,
        flags: 2,
        rgba: vec![value; 8],
        depth: vec![value, 255],
    }
}

fn legacy(frame: Frame) -> Result<Option<Texture>> {
    if frame.width == 0 || frame.height == 0 {
        return Ok(None);
    }
    ensure!(
        frame.flags & 2 != 0,
        "object sprite needs original depth channel"
    );
    Ok(Some(Texture {
        width: frame.width,
        height: frame.height,
        rgba: frame.rgba.into(),
        depth: frame.depth.into(),
    }))
}

#[test]
fn repeated_frames_preserve_all_texture_entries_and_share_immutable_samples() -> Result<()> {
    let mut frames = Frames::default();
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    let mut calls = 0;
    for (identifier, index, value) in [(4, 0, 7), (9, 0, 13), (4, 0, 7), (4, 1, 19), (9, 0, 13)] {
        expected.push(legacy(frame(value))?.unwrap());
        actual.push(
            frames
                .get((identifier, index), || {
                    calls += 1;
                    Ok(frame(value))
                })?
                .unwrap(),
        );
    }
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 5);
    assert_eq!(calls, 3);
    assert!(Arc::ptr_eq(&actual[0].rgba, &actual[2].rgba));
    assert!(Arc::ptr_eq(&actual[0].depth, &actual[2].depth));
    assert!(!Arc::ptr_eq(&actual[0].rgba, &actual[3].rgba));
    assert!(!Arc::ptr_eq(&actual[0].rgba, &actual[1].rgba));
    Ok(())
}

#[test]
fn empty_frames_skip_depth_validation_and_remain_empty_on_reuse() -> Result<()> {
    for (width, height) in [(0, 2), (1, 0), (0, 0)] {
        let mut frames = Frames::default();
        let mut empty = frame(5);
        empty.width = width;
        empty.height = height;
        empty.flags = 0;
        assert_eq!(frames.get((3, 0), || Ok(empty.clone()))?, legacy(empty)?);
        assert!(
            frames
                .get((3, 0), || panic!("empty frame decoded again"))?
                .is_none()
        );
    }
    Ok(())
}

#[test]
fn parse_and_depth_errors_are_not_cached_or_converted_to_empty_frames() -> Result<()> {
    let mut frames = Frames::default();
    let mut calls = 0;
    for _ in 0..2 {
        let error = frames
            .get((3, 0), || {
                calls += 1;
                anyhow::bail!("original parse error")
            })
            .unwrap_err();
        assert_eq!(error.to_string(), "original parse error");
    }
    assert_eq!(calls, 2);
    let mut invalid = frame(5);
    invalid.flags = 0;
    let expected = legacy(invalid.clone()).unwrap_err().to_string();
    assert_eq!(
        frames.get((3, 0), || Ok(invalid)).unwrap_err().to_string(),
        expected
    );
    assert!(frames.get((3, 0), || Ok(frame(5)))?.is_some());
    Ok(())
}

#[test]
fn new_documents_observe_different_bytes_for_the_same_sprite_identity() -> Result<()> {
    let original = Frames::default().get((3, 0), || Ok(frame(5)))?.unwrap();
    let replacement = Frames::default().get((3, 0), || Ok(frame(17)))?.unwrap();
    assert_ne!(original, replacement);
    assert_eq!(original, legacy(frame(5))?.unwrap());
    assert_eq!(replacement, legacy(frame(17))?.unwrap());
    Ok(())
}

#[test]
fn payload_writes_do_not_mutate_another_retained_texture() -> Result<()> {
    let mut frames = Frames::default();
    let mut changed = frames.get((3, 0), || Ok(frame(5)))?.unwrap();
    let retained = frames.get((3, 0), || panic!("repeat decode"))?.unwrap();
    Arc::make_mut(&mut changed.rgba)[0] = 99;
    Arc::make_mut(&mut changed.depth)[0] = 3;
    assert_eq!(retained, legacy(frame(5))?.unwrap());
    assert_ne!(changed, retained);
    Ok(())
}

#[test]
fn repeated_identical_document_bytes_reuse_only_successful_samples() -> Result<()> {
    let mut documents = Documents::default();
    let bytes = vec![1, 2, 3];
    let first = documents
        .for_document("source", &bytes)
        .get((3, 0), || Ok(frame(5)))?
        .unwrap();
    documents.retain_document("source", bytes.clone());
    let independently_read = bytes.clone();
    let repeated = documents
        .for_document("source", &independently_read)
        .get((3, 0), || panic!("identical document decoded twice"))?
        .unwrap();
    assert_eq!(first, repeated);
    assert!(Arc::ptr_eq(&first.rgba, &repeated.rgba));
    assert!(Arc::ptr_eq(&first.depth, &repeated.depth));
    Ok(())
}

#[test]
fn any_changed_document_byte_resets_all_frames_without_mutating_retained_samples() -> Result<()> {
    let original_bytes = vec![1, 2, 3];
    for offset in 0..original_bytes.len() {
        let mut documents = Documents::default();
        let retained = documents
            .for_document("source", &original_bytes)
            .get((3, 0), || Ok(frame(5)))?
            .unwrap();
        documents.retain_document("source", original_bytes.clone());
        let mut changed_bytes = original_bytes.clone();
        changed_bytes[offset] += 1;
        let changed = documents
            .for_document("source", &changed_bytes)
            .get((3, 0), || Ok(frame(17)))?
            .unwrap();
        documents.retain_document("source", changed_bytes);
        assert_eq!(changed, legacy(frame(17))?.unwrap());
        assert_eq!(retained, legacy(frame(5))?.unwrap());
        assert!(!Arc::ptr_eq(&retained.rgba, &changed.rgba));
        let returned = documents
            .for_document("source", &original_bytes)
            .get((3, 0), || Ok(frame(5)))?
            .unwrap();
        assert_eq!(returned, retained);
        assert!(!Arc::ptr_eq(&returned.rgba, &retained.rgba));
    }
    Ok(())
}

#[test]
fn independent_documents_and_render_owners_do_not_replace_each_others_frames() -> Result<()> {
    let mut documents = Documents::default();
    let first = documents
        .for_document("first", &[1])
        .get((3, 0), || Ok(frame(5)))?
        .unwrap();
    documents.retain_document("first", vec![1]);
    documents
        .for_document("second", &[2])
        .get((3, 0), || Ok(frame(17)))?;
    documents.retain_document("second", vec![2]);
    let repeated = documents
        .for_document("first", &[1])
        .get((3, 0), || panic!("unchanged source was decoded again"))?
        .unwrap();
    assert_eq!(first, repeated);
    assert!(Arc::ptr_eq(&first.rgba, &repeated.rgba));
    let independent = Documents::default()
        .for_document("first", &[1])
        .get((3, 0), || Ok(frame(5)))?
        .unwrap();
    assert_eq!(independent, first);
    assert!(!Arc::ptr_eq(&independent.rgba, &first.rgba));
    Ok(())
}

#[test]
fn failed_changed_document_cannot_certify_partial_frames_for_the_old_bytes() -> Result<()> {
    let mut documents = Documents::default();
    documents
        .for_document("source", &[1])
        .get((3, 0), || Ok(frame(5)))?;
    documents.retain_document("source", vec![1]);
    documents
        .for_document("source", &[2])
        .get((3, 0), || Ok(frame(17)))?;
    let restored = documents
        .for_document("source", &[1])
        .get((3, 0), || Ok(frame(5)))?
        .unwrap();
    assert_eq!(restored, legacy(frame(5))?.unwrap());
    Ok(())
}
