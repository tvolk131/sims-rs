use super::*;
use std::sync::Arc;

fn map(size: usize) -> Array {
    Array {
        width: size,
        height: size,
        stride: 1,
        encoding: 0,
        data: (0..size * size)
            .map(|position| (position as u8).wrapping_mul(47))
            .collect(),
    }
}

#[test]
fn repeated_full_inputs_preserve_every_pixel_and_rebind_texture_slots() -> Result<()> {
    for size in [1, 2, 4, 8] {
        let map = map(size);
        let mut cache = Cache::default();
        let (_, original) = cache.bind_input([size, size], Some(&map), 7)?;
        let (ground, repeated) = cache.bind_input([size, size], Some(&map), 42)?;
        let (expected, pixels) = Ground::from_input(Input::new([size, size], Some(&map))?, 42)?;
        assert_eq!(repeated, pixels);
        assert_eq!(ground.texture, expected.texture);
        assert_eq!(ground.texture, 42);
        assert_eq!(ground.size, expected.size);
        assert!(Arc::ptr_eq(&original.rgba, &repeated.rgba));
        assert!(Arc::ptr_eq(&original.depth, &repeated.depth));
        for (column, row, local) in [
            (0, 0, glam::Vec3::ZERO),
            (3, 1, glam::Vec3::new(0.25, 2., 0.75)),
            (-2, 5, glam::Vec3::new(-0.5, 0., 1.25)),
        ] {
            assert_eq!(
                ground.uv(column, row, local),
                expected.uv(column, row, local)
            );
        }
    }
    Ok(())
}

#[test]
fn changed_bytes_dimensions_and_encoding_cannot_reuse_old_pixels() -> Result<()> {
    let mut cache = Cache::default();
    let original_map = map(2);
    let (_, original) = cache.bind_input([2, 2], Some(&original_map), 1)?;
    let mut changed = original_map.clone();
    changed.data[0] ^= 255;
    let (_, updated) = cache.bind_input([2, 2], Some(&changed), 2)?;
    let (_, expected) = Ground::from_input(Input::new([2, 2], Some(&changed))?, 2)?;
    assert_eq!(updated, expected);
    assert_ne!(updated, original);
    assert!(!Arc::ptr_eq(&updated.rgba, &original.rgba));
    changed.encoding = u32::MAX;
    let (_, reencoded) = cache.bind_input([2, 2], Some(&changed), 3)?;
    assert_eq!(reencoded, updated);
    assert!(!Arc::ptr_eq(&reencoded.rgba, &updated.rgba));
    let resized = map(3);
    let (ground, pixels) = cache.bind_input([3, 3], Some(&resized), 4)?;
    let (_, expected) = Ground::from_input(Input::new([3, 3], Some(&resized))?, 4)?;
    assert_eq!(ground.size, 3);
    assert_eq!(pixels, expected);
    let (_, restored) = cache.bind_input([2, 2], Some(&original_map), 5)?;
    assert_eq!(restored, original);
    assert!(!Arc::ptr_eq(&restored.rgba, &original.rgba));
    Ok(())
}

#[test]
fn every_cache_hit_still_validates_presence_shape_stride_and_size() -> Result<()> {
    let mut cache = Cache::default();
    let original_map = map(2);
    let (_, original) = cache.bind_input([2, 2], Some(&original_map), 1)?;
    assert_eq!(
        cache.bind_input([2, 2], None, 2).unwrap_err().to_string(),
        "original lot grass map absent"
    );
    for malformed in [
        Array {
            width: 3,
            ..original_map.clone()
        },
        Array {
            height: 3,
            ..original_map.clone()
        },
        Array {
            stride: 2,
            ..original_map.clone()
        },
        Array {
            data: vec![0; 3],
            ..original_map.clone()
        },
    ] {
        for _ in 0..2 {
            assert_eq!(
                cache
                    .bind_input([2, 2], Some(&malformed), 3)
                    .unwrap_err()
                    .to_string(),
                "invalid original grass map"
            );
        }
    }
    assert_eq!(
        cache
            .bind_input([2, 3], Some(&original_map), 4)
            .unwrap_err()
            .to_string(),
        "invalid original grass map"
    );
    for size in [0, 129] {
        assert_eq!(
            cache
                .bind_input([size, size], Some(&map(size)), 5)
                .unwrap_err()
                .to_string(),
            "ground presentation texture too large"
        );
    }
    let (_, restored) = cache.bind_input([2, 2], Some(&original_map), 6)?;
    assert_eq!(restored, original);
    assert!(Arc::ptr_eq(&restored.rgba, &original.rgba));
    Ok(())
}

#[test]
fn independent_owners_and_payload_writes_preserve_retained_images() -> Result<()> {
    let map = map(2);
    let mut cache = Cache::default();
    let (_, original) = cache.bind_input([2, 2], Some(&map), 1)?;
    let (_, independent) = Cache::default().bind_input([2, 2], Some(&map), 1)?;
    assert_eq!(original, independent);
    assert!(!Arc::ptr_eq(&original.rgba, &independent.rgba));
    let mut changed = original.clone();
    Arc::make_mut(&mut changed.rgba)[0] ^= 255;
    Arc::make_mut(&mut changed.depth)[0] = 0;
    let (_, repeated) = cache.bind_input([2, 2], Some(&map), 2)?;
    assert_eq!(original, repeated);
    assert_eq!(original, independent);
    assert_ne!(original, changed);
    Ok(())
}
