use super::*;
use std::sync::Arc;

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
fn repeated_frames_retain_complete_texture_sequence_and_share_immutable_payloads() -> Result<()> {
    let mut frames = Frames::default();
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    let mut decoded = 0;
    for (identifier, index, value) in [(4, 0, 7), (9, 0, 13), (4, 0, 7), (4, 1, 19), (9, 0, 13)] {
        expected.push(legacy(frame(value))?.unwrap());
        actual.push(
            frames
                .get((identifier, index), || {
                    decoded += 1;
                    Ok(frame(value))
                })?
                .unwrap(),
        );
    }
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 5);
    assert_eq!(decoded, 3);
    assert!(Arc::ptr_eq(&actual[0].rgba, &actual[2].rgba));
    assert!(Arc::ptr_eq(&actual[1].depth, &actual[4].depth));
    assert!(!Arc::ptr_eq(&actual[0].rgba, &actual[3].rgba));
    Ok(())
}

#[test]
fn empty_frames_skip_depth_validation_and_remain_empty_on_reuse() -> Result<()> {
    for (width, height) in [(0, 0), (0, 2), (1, 0)] {
        let mut frames = Frames::default();
        let mut empty = frame(5);
        empty.width = width;
        empty.height = height;
        empty.flags = 0;
        let expected = legacy(empty.clone())?;
        assert_eq!(frames.get((1, 0), || Ok(empty))?, expected);
        assert_eq!(
            frames.get((1, 0), || panic!("empty frame was decoded twice"))?,
            expected
        );
    }
    Ok(())
}

#[test]
fn errors_are_not_cached_and_keep_the_legacy_message() -> Result<()> {
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
fn new_document_observes_different_bytes_for_the_same_sprite_identity() -> Result<()> {
    let original = Frames::default().get((3, 0), || Ok(frame(5)))?.unwrap();
    let replacement = Frames::default().get((3, 0), || Ok(frame(17)))?.unwrap();
    assert_ne!(original, replacement);
    assert_eq!(original, legacy(frame(5))?.unwrap());
    assert_eq!(replacement, legacy(frame(17))?.unwrap());
    Ok(())
}

#[test]
fn payload_writes_cannot_change_another_retained_texture() -> Result<()> {
    let mut frames = Frames::default();
    let mut first = frames.get((3, 0), || Ok(frame(5)))?.unwrap();
    let retained = frames.get((3, 0), || panic!("repeat decode"))?.unwrap();
    Arc::make_mut(&mut first.rgba)[0] = 99;
    assert_eq!(retained, legacy(frame(5))?.unwrap());
    assert_ne!(first, retained);
    Ok(())
}
