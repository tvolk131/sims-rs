use super::Texture;
use sims_content::avatar::Texture as AvatarTexture;
use std::sync::Arc;

#[test]
fn avatar_textures_preserve_entries_and_clone_rgba() {
    let mut sources = [
        AvatarTexture {
            width: 2,
            height: 1,
            rgba: vec![3, 7, 11, 0, 13, 17, 19, 255],
        },
        AvatarTexture {
            width: 1,
            height: 2,
            rgba: vec![23, 29, 31, 127, 37, 41, 43, 64],
        },
        AvatarTexture {
            width: 0,
            height: 2,
            rgba: vec![],
        },
    ];
    let expected: Vec<_> = sources
        .iter()
        .map(|source| Texture {
            width: source.width,
            height: source.height,
            rgba: source.rgba.clone().into(),
            depth: vec![255; (source.width * source.height) as usize].into(),
        })
        .collect();
    let mut textures: Vec<_> = sources.iter().map(Texture::from_avatar).collect();
    assert_eq!(textures, expected);
    sources[0].rgba[0] = 89;
    assert_eq!(textures[0], expected[0]);
    Arc::make_mut(&mut textures[0].rgba)[1] = 97;
    assert_eq!(sources[0].rgba[1], 7);
    assert_eq!(textures[1..], expected[1..]);
}

#[test]
fn wall_variants_preserve_entries_share_depth_and_detach_on_write() {
    let covering = Texture {
        width: 2,
        height: 1,
        rgba: vec![3, 7, 11, 255, 13, 17, 19, 127].into(),
        depth: vec![7, 241].into(),
    };
    let mut expected = vec![covering.clone()];
    let mut variants = vec![covering.clone()];
    for rgba in [
        vec![3, 7, 11, 0, 13, 17, 19, 63],
        vec![3, 7, 11, 128, 13, 17, 19, 0],
    ] {
        expected.push(Texture {
            width: covering.width,
            height: covering.height,
            rgba: rgba.clone().into(),
            depth: covering.depth.to_vec().into(),
        });
        variants.push(covering.with_rgba(rgba));
    }
    assert_eq!(variants, expected);
    for variant in &variants[1..] {
        assert!(Arc::ptr_eq(&variant.depth, &covering.depth));
        assert!(!Arc::ptr_eq(&variant.rgba, &covering.rgba));
    }
    let published = variants.clone();
    Arc::make_mut(&mut variants[1].depth)[0] = 99;
    Arc::make_mut(&mut variants[0].depth)[1] = 13;
    Arc::make_mut(&mut variants[2].rgba)[3] = 17;
    assert_eq!(published, expected);
    assert_eq!(variants[2].depth, covering.depth);
    assert!(!Arc::ptr_eq(&variants[1].depth, &covering.depth));
    assert!(!Arc::ptr_eq(&variants[0].depth, &covering.depth));
}
