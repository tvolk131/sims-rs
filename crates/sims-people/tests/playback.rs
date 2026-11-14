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
            "16 stand 21459cb8d5fb060da247bb799f7cf17543500f626bfb1c3cd2c400bb5d9353a6",
            "16 walk d19c99d690d8cfd7c86d19c905a1dfa70564bb80f061f17a847067440c98b87e",
            "16 walk c43237929c36dfb7dd667d20ef50e3df7fa06a7791e99ecebf2bb3e95083a0fc",
            "18 base a2o-standing-loop",
            "18 stand 6af7dd63535a45f76e9572ee0be93fdc198afba469b2d6775a851b093d59ed32",
            "18 walk 396076bc31ff965a53e72d9547828c50d492584a3460891ea036ce48b4f05d08",
            "18 walk 2792eb544b2c450d5ced6f00c8f431b54c00269fb65ab5f124a3e608e6541bd3",
            "21 base a2o-standing-loop",
            "21 stand 565e5e1a8411e6b4eaa2ca855c41c51722265882c38c29e1efaefb1c54508aeb",
            "21 walk 9aa3541af8066271b193c872a8f99e606c6f1999751402997a18544dd1b11ea4",
            "21 walk bc24b05e528829eefd01027f46efacc54291a2f2126fc5341e58f3110fb56d19",
        ]
    );
}
