//! Integration tests for Create-a-Family (CAF) catalog enumeration, 3D Sim
//! turntable previews, and 3D Sim portrait rendering (`BMP_` 2002–2007).
//!
//! Live-install tests need `SIMS_INSTALL` and are skipped cleanly when unset.
//! Nothing from the game is stored in the repository.

use sims_people::caf_preview::{
    ADULT_IDLE_CLIPS, Age, BodyType, CHILD_IDLE_CLIPS, FACES_2002_BG_565, FACES_2002_STRIP_SIZE,
    Gender, PREVIEW_SIZE, PortraitKind, REL_2003_STRIP_SIZE, SimConfig, SkinTone, TurntableParams,
    default_hands, enumerate_catalog, render_portrait, render_portrait_transparent,
    render_turntable, render_turntable_clip, turntable_yaw, unpack_rgb565,
};
use sims_people::{Assets, Library};
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(root) => Some(PathBuf::from(root)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping live CAF preview tests");
            None
        }
    }
}

fn alice_config() -> SimConfig {
    SimConfig::from_names(
        Age::Adult,
        Gender::Female,
        SkinTone::Medium,
        "c809fa_beth",
        "c809famed_beth3",
        "b002fafit_01",
        "b002fafitmed_mom02",
    )
}

#[test]
fn caf_catalog_enumerates_all_demographics_and_finds_alice() {
    let Some(root) = install() else { return };
    let assets = Assets::scan(&root).unwrap();

    for age in [Age::Adult, Age::Child] {
        for gender in [Gender::Male, Gender::Female] {
            for tone in SkinTone::ALL {
                let cat = enumerate_catalog(&assets, age, gender, tone);
                assert!(
                    !cat.heads.is_empty(),
                    "expected non-empty heads for ({age:?}, {gender:?}, {tone:?})"
                );
                assert!(
                    !cat.bodies.is_empty(),
                    "expected non-empty bodies for ({age:?}, {gender:?}, {tone:?})"
                );
                for head in &cat.heads {
                    assert!(
                        !head.appearance.starts_with("suit-"),
                        "suit appearance leaked into head catalog: {}",
                        head.appearance
                    );
                    assert!(
                        !head.appearance.as_bytes()[1].eq_ignore_ascii_case(&b't'),
                        "townie/NPC ct* head leaked into head catalog: {}",
                        head.appearance
                    );
                    assert!(head.texture_binding.starts_with("HEAD-HEAD="));
                    assert!(head.body_type.is_none());
                    assert!(
                        assets.contains(&format!("{}.bmp", head.texture_name)),
                        "missing head texture {}.bmp",
                        head.texture_name
                    );
                }
                for body in &cat.bodies {
                    assert!(
                        !body.appearance.starts_with("suit-"),
                        "suit appearance leaked into body catalog: {}",
                        body.appearance
                    );
                    assert!(body.texture_binding.starts_with("BODY="));
                    assert!(body.body_type.is_some());
                    assert!(
                        assets.contains(&format!("{}.bmp", body.texture_name)),
                        "missing body texture {}.bmp",
                        body.texture_name
                    );
                }
            }
        }
    }

    // Verify Alice (Complete Collection CAF oracle User00172.iff) is present.
    let adult_f_med = enumerate_catalog(&assets, Age::Adult, Gender::Female, SkinTone::Medium);
    assert!(
        adult_f_med.heads.iter().any(|h| {
            h.appearance == "c809fa_beth" && h.texture_binding == "HEAD-HEAD=c809famed_beth3"
        }),
        "Alice's head c809fa_beth / c809famed_beth3 missing from CAF head catalog"
    );
    let alice_body = adult_f_med
        .bodies
        .iter()
        .find(|b| b.appearance == "b002fafit_01" && b.texture_binding == "BODY=b002fafitmed_mom02")
        .expect("Alice's body b002fafit_01 / b002fafitmed_mom02 missing from CAF body catalog");
    assert_eq!(alice_body.body_type, Some(BodyType::Fit));

    // Verify first selectable pair for each demographic loads cleanly into a 3D Body.
    for (age, gender, tone) in [
        (Age::Adult, Gender::Female, SkinTone::Medium),
        (Age::Adult, Gender::Male, SkinTone::Light),
        (Age::Child, Gender::Female, SkinTone::Dark),
        (Age::Child, Gender::Male, SkinTone::Medium),
    ] {
        let cat = enumerate_catalog(&assets, age, gender, tone);
        let cfg = SimConfig::from_entries(age, gender, tone, &cat.heads[0], &cat.bodies[0]);
        assert_eq!(
            cfg.hands
                .as_ref()
                .cloned()
                .unwrap_or_else(|| default_hands(age, gender, tone)),
            default_hands(age, gender, tone)
        );
        let body = cfg.load_body(&assets).unwrap();
        assert!(body.parts.len() >= 4, "expected body + 2 hands + head");
    }
}

#[test]
fn caf_turntable_renders_alice_child_and_idle_clips() {
    let Some(root) = install() else { return };
    let assets = Assets::scan(&root).unwrap();
    let library = Library::scan(&assets).unwrap();
    let body = alice_config().load_body(&assets).unwrap();

    // Every advertised adult & child idle clip must load from the install.
    for &clip_name in ADULT_IDLE_CLIPS.iter().chain(CHILD_IDLE_CLIPS.iter()) {
        library
            .load(&assets, clip_name)
            .unwrap_or_else(|e| panic!("CAF idle clip {clip_name:?} failed to load: {e}"));
    }

    let idle = library.load(&assets, ADULT_IDLE_CLIPS[0]).unwrap();
    let center =
        render_turntable_clip(&body, Some(&idle), 0.0, true, &TurntableParams::default()).unwrap();
    assert_eq!([center.width, center.height], PREVIEW_SIZE);
    assert!(
        center.covered() > 3500,
        "Alice turntable should cover >3500 px in 100x220 viewport, got {}",
        center.covered()
    );

    // Measure vertical and horizontal bounding box in the native 100x220 CAF viewport
    // and verify it matches Complete Collection's CAF preview proportions (~189-197 px tall).
    let mut min_y = center.height;
    let mut max_y = 0u32;
    let mut min_x = center.width;
    let mut max_x = 0u32;
    for y in 0..center.height {
        for x in 0..center.width {
            let p = ((y * center.width + x) * 4) as usize;
            if center.rgba[p + 3] > 0 {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    let height_px = max_y - min_y + 1;
    let width_px = max_x - min_x + 1;
    assert!(
        (180..=205).contains(&height_px),
        "expected adult Sim height ~189-195 px in 100x220 CAF viewport, got {height_px} (y={min_y}..={max_y})"
    );
    assert!(
        (40..=75).contains(&width_px),
        "expected adult Sim width ~56 px in 100x220 CAF viewport, got {width_px} (x={min_x}..={max_x})"
    );

    // Turntable yaw oscillation produces distinct views at 2500ms (+45°) and 7500ms (-45°).
    let left_turn = render_turntable(&body, None, &TurntableParams::at_time(2500)).unwrap();
    let right_turn = render_turntable(&body, None, &TurntableParams::at_time(7500)).unwrap();
    assert_ne!(left_turn.rgba, right_turn.rgba);
    assert!(turntable_yaw(2500) > turntable_yaw(7500));

    // Pose changes between rest and the CAF hands-on-hips/arms-crossed transition clip.
    let transition = library.load(&assets, ADULT_IDLE_CLIPS[1]).unwrap();
    let posed_mid = render_turntable_clip(
        &body,
        Some(&transition),
        0.5,
        false,
        &TurntableParams::default(),
    )
    .unwrap();
    assert_ne!(center.rgba, posed_mid.rgba);

    // Verify a child Sim also renders cleanly and shorter than an adult Sim.
    let child_cat = enumerate_catalog(&assets, Age::Child, Gender::Female, SkinTone::Medium);
    let child_body = SimConfig::from_entries(
        Age::Child,
        Gender::Female,
        SkinTone::Medium,
        &child_cat.heads[0],
        &child_cat.bodies[0],
    )
    .load_body(&assets)
    .unwrap();
    let child_clip = library.load(&assets, CHILD_IDLE_CLIPS[0]).unwrap();
    let child_shot = render_turntable_clip(
        &child_body,
        Some(&child_clip),
        0.0,
        true,
        &TurntableParams::default(),
    )
    .unwrap();
    assert!(
        child_shot.covered() > 2000 && child_shot.covered() < center.covered(),
        "child Sim should cover fewer pixels than adult Sim ({} vs {})",
        child_shot.covered(),
        center.covered()
    );
}

#[test]
fn caf_portraits_render_all_bmp_variants_and_transparent_alpha() {
    let Some(root) = install() else { return };
    let assets = Assets::scan(&root).unwrap();
    let body = alice_config().load_body(&assets).unwrap();

    let bmp2002 = render_portrait(&body, PortraitKind::Faces2002).unwrap();
    assert_eq!([bmp2002.width, bmp2002.height], FACES_2002_STRIP_SIZE);
    let bg2002 = unpack_rgb565(FACES_2002_BG_565);
    assert_eq!(&bmp2002.rgba[0..4], &[bg2002[0], bg2002[1], bg2002[2], 255]);

    // Verify the three frames (neutral, happy, sad) are all distinct.
    let frame_slice = |i: usize| -> Vec<u8> {
        let mut out = Vec::with_capacity(35 * 41 * 4);
        for y in 0..41 {
            let start = (y * 105 + i * 35) * 4;
            out.extend_from_slice(&bmp2002.rgba[start..start + 35 * 4]);
        }
        out
    };
    let f0 = frame_slice(0);
    let f1 = frame_slice(1);
    let f2 = frame_slice(2);
    assert_ne!(f0, f1, "neutral and happy BMP_ 2002 frames must differ");
    assert_ne!(f0, f2, "neutral and sad BMP_ 2002 frames must differ");
    assert_ne!(f1, f2, "happy and sad BMP_ 2002 frames must differ");

    let bmp2003 = render_portrait(&body, PortraitKind::Relationship2003).unwrap();
    assert_eq!([bmp2003.width, bmp2003.height], REL_2003_STRIP_SIZE);
    // Verify the 5 distinct relationship background colours across the 5 frames.
    let top_left_colors: Vec<[u8; 3]> = (0..5)
        .map(|i| {
            let p = i * 40 * 4;
            [bmp2003.rgba[p], bmp2003.rgba[p + 1], bmp2003.rgba[p + 2]]
        })
        .collect();
    for i in 0..5 {
        for j in (i + 1)..5 {
            assert_ne!(top_left_colors[i], top_left_colors[j]);
        }
    }

    let bmp2004 = render_portrait(&body, PortraitKind::NewImage2004).unwrap();
    assert_eq!([bmp2004.width, bmp2004.height], [25, 25]);
    assert_eq!(&bmp2004.rgba[0..4], &[0, 0, 248, 255]);
    assert_eq!(
        &bmp2004.rgba[(25 + 1) * 4..(25 + 1) * 4 + 4],
        &[0, 0, 0, 255]
    );

    let bmp2005 = render_portrait(&body, PortraitKind::SpeechMed2005).unwrap();
    assert_eq!([bmp2005.width, bmp2005.height], [16, 16]);

    let bmp2006 = render_portrait(&body, PortraitKind::SpeechLarge2006).unwrap();
    assert_eq!([bmp2006.width, bmp2006.height], [32, 32]);

    let bmp2007 = render_portrait(&body, PortraitKind::WebImage2007).unwrap();
    assert_eq!([bmp2007.width, bmp2007.height], [45, 45]);

    // Transparent alpha portrait has clean 0-alpha corners and opaque head core.
    let alpha = render_portrait_transparent(&body, [35, 41], 0.0, 0.0).unwrap();
    assert_eq!([alpha.width, alpha.height], [35, 41]);
    assert_eq!(&alpha.rgba[0..4], &[0, 0, 0, 0]);
    let opaque_px = alpha
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] == 255)
        .count();
    let transparent_px = alpha
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] == 0)
        .count();
    assert!(
        opaque_px > 400,
        "expected >400 opaque head pixels, got {opaque_px}"
    );
    assert!(
        transparent_px > 100,
        "expected >100 transparent background pixels, got {transparent_px}"
    );
}
