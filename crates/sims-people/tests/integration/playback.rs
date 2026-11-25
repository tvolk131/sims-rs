//! The Goth family playing the standing loop and the walk, posed from the
//! simulation's animator state. Needs `SIMS_INSTALL`; skipped when it is
//! unset. Nothing from the game is kept: the tests pin names, numbers and
//! SHA-256 digests only. Set `SIMS_PEOPLE_OUT` to a directory to write
//! the images there.

use crate::common::hex;
use sims_core::F32;
use sims_people::math::Vec3;
use sims_people::raster::{Camera, Canvas};
use sims_people::{Assets, Body, Library, Player, base_clip, placement, residents};
use sims_sim::animator::{Animator, Base, Layer};
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(root) => Some(PathBuf::from(root)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping the playback tests");
            None
        }
    }
}

#[test]
fn the_standing_loop_and_the_walk_load_with_their_samples() {
    let Some(root) = install() else { return };
    let assets = Assets::scan(&root).unwrap();
    let library = Library::scan(&assets).unwrap();
    assert!(library.len() > 5000, "{} clips", library.len());
    let mut seen = Vec::new();
    for name in [
        "a2o-standing-loop",
        "c2o-standing-loop",
        "a2o-walking-loop",
        "c2o-walking-loop",
    ] {
        let clip = library.load(&assets, name).unwrap();
        let a = &clip.animation;
        seen.push(format!(
            "{name} {} frames, {} ms, distance {}, {} motions, {}/{} samples",
            a.frames,
            a.duration_ms,
            a.distance,
            a.motions.len(),
            clip.samples.translations.len(),
            clip.samples.rotations.len()
        ));
    }
    assert_eq!(
        seen,
        [
            "a2o-standing-loop 60 frames, 1999 ms, distance 0, 29 motions, 120/1740 samples",
            "c2o-standing-loop 60 frames, 1999 ms, distance 0, 29 motions, 120/1740 samples",
            "a2o-walking-loop 38 frames, 1266 ms, distance 4.06978, 29 motions, 76/1102 samples",
            "c2o-walking-loop 27 frames, 900 ms, distance 2.42597, 29 motions, 54/783 samples",
        ]
    );
}

/// An animator placed at the origin facing the viewer of the house view
/// (direction 3, three eighths of a turn), with a base layer at `clock`
/// and, for a walk, the walking loop in front at `phase`.
fn animator(clock: i32, walk: Option<(&str, f32)>, library: &Library, assets: &Assets) -> Animator {
    let mut animator = Animator::new(clock);
    animator.position = [0.0, 0.0, 3.0].map(F32::from_f32);
    animator.base = Base::Posture;
    if let Some((clip, phase)) = walk {
        let loaded = library.load(assets, clip).unwrap();
        let mut layer = Layer::start(
            clip,
            F32::from_f32(loaded.animation.duration_ms),
            loaded.animation.frames,
            true,
            0,
            false,
            clock,
        );
        layer.mode = 1;
        layer.phase = F32::from_f32(phase);
        animator.foreground = Some(layer);
    }
    animator
}

#[test]
fn the_goth_family_stands_and_walks() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let people = residents(&root, &house).unwrap();
    let assets = Assets::scan(&root).unwrap();
    let library = Library::scan(&assets).unwrap();
    let mut player = Player::new(&assets, &library);
    let mut digests = Vec::new();
    for resident in &people {
        let body = Body::load(&assets, &resident.skeleton, &resident.outfit).unwrap();
        let base = base_clip(&resident.person, &resident.skeleton);
        let age = &base[..3];
        digests.push(format!("{} base {base}", resident.id));
        let walk = format!("{age}-walking-loop");
        let cases = [
            ("stand", animator(500, None, &library, &assets)),
            (
                "walk",
                animator(500, Some((&walk, 0.25)), &library, &assets),
            ),
            (
                "walk",
                animator(500, Some((&walk, 0.75)), &library, &assets),
            ),
        ];
        let mut images = Vec::new();
        for (i, (kind, animator)) in cases.iter().enumerate() {
            let pose = player.pose(&body.skeleton, animator, None, &base).unwrap();
            let bones = pose.world(&body.skeleton, placement(animator).unwrap());
            let camera = Camera::house_view(60.0, Vec3::new(0.0, 2.8, 0.0));
            let mut canvas = Canvas::new(320, 400, [0, 0, 0, 0]);
            canvas.draw_body(&camera, &body, &bones);
            if let Some(out) = std::env::var_os("SIMS_PEOPLE_OUT") {
                let path = PathBuf::from(out).join(format!("{kind}-{}-{i}.png", resident.id));
                std::fs::write(path, canvas.png()).unwrap();
            }
            assert!(
                canvas.covered() > 5000,
                "person {} barely drawn",
                resident.id
            );
            digests.push(format!("{} {kind} {}", resident.id, hex(&canvas.rgba)));
            images.push(canvas.rgba);
        }
        // The two walk phases are different poses, and differ from standing.
        assert_ne!(images[0], images[1]);
        assert_ne!(images[1], images[2]);
    }
    // The SHA-256 of each image's pixels. The camera and the turn are
    // built from additions, multiplications and square roots, so these
    // hold on every machine.
    assert_eq!(
        digests,
        [
            "16 base c2o-standing-loop",
            "16 stand 2d4de1ff3cb04720af7bdb008ad7cde148d181f45b8402bc563cad845790b623",
            "16 walk 670581ce241ecbae9520041e7c7a795a7faca6828c64c361244194d45fdaf5f8",
            "16 walk ca1213f8b3e37670701430de369e246b733dfb9cef6c48f6067b12d578e7cc75",
            "18 base a2o-standing-loop",
            "18 stand 196e3318d16fb41c441ec1d27b72bdcb463414ee34cef082a751c8ea36d629da",
            "18 walk 7a74b12eff6adb3b0f3d604693397e50cf312a94ad08ac6fecc5db8c818bc557",
            "18 walk 4b925c0726708dfa2ab831ad256d41671daf61010cd6f4aa658bc8b0ca4086df",
            "21 base a2o-standing-loop",
            "21 stand cb802749be0f3d212312d5a2540f8ac11102b36cd688448fae4d10dccc44dc31",
            "21 walk dd8dda7165a993aa088cbf7970fcbc89c6e42f5d3079cfe8cd6142db4e7ad30c",
            "21 walk c142d22ffc41627349475badbb628e67f7ac9390538cb8a0f0753bd0e3bb351b",
        ]
    );
}

/// A clip a script put in the base slot plays from when it started
/// (0x597a88: the layer's clock starts at the animator's, its phase at 0,
/// or 1 backward), not from the clock's zero as the posture's clip does.
#[test]
fn a_scripts_base_clip_plays_from_its_start() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let people = residents(&root, &house).unwrap();
    let assets = Assets::scan(&root).unwrap();
    let library = Library::scan(&assets).unwrap();
    let mut player = Player::new(&assets, &library);
    let resident = &people[0];
    let body = Body::load(&assets, &resident.skeleton, &resident.outfit).unwrap();
    let clip = base_clip(&resident.person, &resident.skeleton);
    let mut pose = |clock: i32, base: Base| {
        let mut animator = Animator::new(clock);
        animator.base = base;
        player.pose(&body.skeleton, &animator, None, &clip).unwrap()
    };
    let script = |reverse: bool, start: i32| Base::Script {
        clip: clip.clone(),
        reverse,
        start,
    };
    // Forward, started at 500, it is at its first frame at 500, as the
    // posture's clip is at 0, and not 500 ms in.
    let posture_start = pose(0, Base::Posture);
    assert_eq!(pose(500, script(false, 500)), posture_start);
    assert_ne!(pose(500, script(false, 0)), posture_start);
    // 250 ms after its start it is where the posture's clip is at 250.
    assert_eq!(pose(750, script(false, 500)), pose(250, Base::Posture));
    // Backward, it starts at its end whenever it started.
    assert_eq!(pose(500, script(true, 500)), pose(0, script(true, 0)));
    assert_eq!(pose(1250, script(true, 1000)), pose(250, script(true, 0)));
}

/// The base layer is the simulation's: a seated person's base slot plays
/// the sit loop at the phase the simulation advanced it to, whatever the
/// saved record's clip and the animator's clock say.
#[test]
fn the_base_layer_is_the_one_the_simulation_keeps() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let people = residents(&root, &house).unwrap();
    let assets = Assets::scan(&root).unwrap();
    let library = Library::scan(&assets).unwrap();
    let mut player = Player::new(&assets, &library);
    let resident = people
        .iter()
        .find(|r| r.skeleton.eq_ignore_ascii_case("adult"))
        .unwrap();
    let body = Body::load(&assets, &resident.skeleton, &resident.outfit).unwrap();
    let standing = base_clip(&resident.person, &resident.skeleton);
    assert_eq!(standing, "a2o-standing-loop");
    let mut animator = Animator::new(1234);
    animator.base = Base::Posture;
    let sit = |phase: f32| {
        let mut layer = Layer::start("a2o-sit-loop", F32::ZERO, 0, true, 0, false, 0);
        layer.mode = 1;
        layer.priority = -10;
        layer.phase = F32::from_f32(phase);
        layer
    };
    let mut pose = |layer: Option<&Layer>| {
        player
            .pose(&body.skeleton, &animator, layer, &standing)
            .unwrap()
    };
    let without = pose(None);
    let seated = pose(Some(&sit(0.25)));
    assert_ne!(seated, without);
    // The clip and the phase are the layer's.
    assert_ne!(pose(Some(&sit(0.75))), seated);
    let mut want = sims_people::rig::Pose::rest(&body.skeleton);
    let clip = player.clip("a2o-sit-loop").unwrap();
    clip.apply(&body.skeleton, &mut want, 0.25, true, 1.0);
    assert_eq!(seated, want);
    // The seated pelvis is lower than the standing one.
    let pelvis = body
        .skeleton
        .bones
        .iter()
        .position(|b| b.name.eq_ignore_ascii_case("PELVIS"))
        .unwrap();
    let mut at = Animator::new(0);
    at.position = [F32::ZERO; 3];
    let root = placement(&at).unwrap();
    let height = |p: &sims_people::rig::Pose| p.world(&body.skeleton, root)[pelvis].translation.y;
    assert!(
        height(&seated) < height(&without) - 0.3,
        "{} {}",
        height(&seated),
        height(&without)
    );
}
