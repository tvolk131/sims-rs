//! The Goth family playing the standing loop and the walk, posed from the
//! simulation's animator state. Needs `SIMS_INSTALL`; skipped when it is
//! unset. Nothing from the game is kept: the tests pin names, numbers and
//! SHA-256 digests only. Set `SIMS_PEOPLE_OUT` to a directory to write
//! the images there.

use sha2::{Digest, Sha256};
use sims_core::F32;
use sims_people::math::Vec3;
use sims_people::raster::{Camera, Canvas};
use sims_people::{Assets, Body, Library, Player, base_clip, placement, residents};
use sims_sim::animator::{Animator, Layer};
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

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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
/// (direction 7, seven eighths of a turn), with a base layer at `clock`
/// and, for a walk, the walking loop in front at `phase`.
fn animator(clock: i32, walk: Option<(&str, f32)>, library: &Library, assets: &Assets) -> Animator {
    let mut animator = Animator::new(clock);
    animator.position = [0.0, 0.0, 7.0].map(F32::from_f32);
    animator.base = true;
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
            let pose = player.pose(&body.skeleton, animator, &base).unwrap();
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
            "16 stand 12849b69f44ac1bb51b826d18c77d74a2be6e464081cbad895d48ecb7070605d",
            "16 walk d465a811a54a9cfd18d3c388fd4a167d9001b03b2b91cb622187b15ccebdce96",
            "16 walk dc6f1842c3471c5c4946593df69efe3f89c25b825fc2fa0e4e9e0e21fdd55a1f",
            "18 base a2o-standing-loop",
            "18 stand 76aced914fc2cacea9564e3f70b43fc4993795e3030159e488d9a25616265b54",
            "18 walk 14f10ac08f52082013774d12c160df6a5d534ed994ca9f1b9fb4421b371f65b6",
            "18 walk 0192bc04cd028c09f82d106695de8eb61754f60ea4901a9e356a431c417a7e59",
            "21 base a2o-standing-loop",
            "21 stand bce43b91328161b4ce89fcf78df9c407c00b7992d367349d4c9eb666cc208915",
            "21 walk 1278e08b1292f77ebbd93c7d68062f990da284f9c12fc76a0d44f584b50f6c6c",
            "21 walk 7159f751d3cde3c91ca39c202f85e818b90690a082d4acd30460451706628720",
        ]
    );
}
