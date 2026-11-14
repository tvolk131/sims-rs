//! The Goth family (the base game's House05) drawn from the install's own
//! files. Needs `SIMS_INSTALL`; skipped when it is unset. Nothing from the
//! game is kept: the tests pin names and SHA-256 digests only.

use sha2::{Digest, Sha256};
use sims_people::math::{Transform, Vec3};
use sims_people::raster::{Camera, Canvas};
use sims_people::{Assets, Body, residents, rig};
use std::path::PathBuf;

fn install() -> Option<PathBuf> {
    match std::env::var_os("SIMS_INSTALL") {
        Some(root) => Some(PathBuf::from(root)),
        None => {
            eprintln!("SIMS_INSTALL is not set; skipping the Goth family tests");
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
fn the_goth_family_wears_its_saved_appearances() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let people = residents(&root, &house).unwrap();
    let assets = Assets::scan(&root).unwrap();
    let mut seen = Vec::new();
    for resident in &people {
        let body = Body::load(&assets, &resident.skeleton, &resident.outfit).unwrap();
        let parts: Vec<String> = body
            .parts
            .iter()
            .map(|p| {
                format!(
                    "{}:{}",
                    p.mesh.name,
                    p.texture_name.as_deref().unwrap_or("-")
                )
            })
            .collect();
        seen.push(format!(
            "{} {} {}",
            resident.id,
            resident.skeleton,
            parts.join(" ")
        ));
        // Every file the family is drawn from has one source, or copies
        // with the same bytes, so the search order does not matter here.
        for name in body.parts.iter().flat_map(|p| {
            [
                format!("{}.bmf", p.mesh.name),
                format!("{}.bmp", p.texture_name.clone().unwrap_or_default()),
            ]
        }) {
            assert!(
                !assets.conflicts(&name).unwrap(),
                "{name} has differing copies"
            );
        }
    }
    assert_eq!(
        seen,
        [
            "16 child xskin-b009fcchd_01-PELVIS-BODY:b009fcchdmed_nerd \
             xskin-hulo-L_HAND-HANDCL:huaomed xskin-huro-R_HAND-HANDCR:huaomed \
             xskin-c006fc_nerd-HEAD-HEAD:c006fcmed_nerd xskin-c006fc_nerd-HEAD-NERDSPECS:nerdspecs",
            "18 adult xskin-b004fafit_01-PELVIS-BODY:b004fafitmed_fatale01 \
             xskin-hflo-L_HAND-HANDFL:huaomed xskin-hfro-R_HAND-HANDFR:huaomed \
             xskin-c003fa_longhair-HEAD-HEAD:c003famed_asian01",
            "21 adult xskin-b004mafit_01-PELVIS-BODY:b004mafitlgt_rat \
             xskin-hmlo-L_HAND-HANDML:huaolgt xskin-hmro-R_HAND-HANDMR:huaolgt \
             xskin-c013ma_pompa-HEAD-HEAD:c013malgt_pompa",
        ]
    );
}

#[test]
fn the_goth_family_draws_in_its_rest_pose() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let people = residents(&root, &house).unwrap();
    let assets = Assets::scan(&root).unwrap();
    let mut digests = Vec::new();
    for resident in &people {
        let body = Body::load(&assets, &resident.skeleton, &resident.outfit).unwrap();
        let world = rig::Pose::rest(&body.skeleton).world(&body.skeleton, Transform::IDENTITY);
        let camera = Camera::house_view(60.0, Vec3::new(0.0, 2.8, 0.0));
        let mut canvas = Canvas::new(320, 400, [0, 0, 0, 0]);
        canvas.draw_body(&camera, &body, &world);
        if let Some(out) = std::env::var_os("SIMS_PEOPLE_OUT") {
            let path = PathBuf::from(out).join(format!("rest-{}.png", resident.id));
            std::fs::write(path, canvas.png()).unwrap();
        }
        assert!(
            canvas.covered() > 5000,
            "person {} barely drawn",
            resident.id
        );
        digests.push(format!("{} {}", resident.id, hex(&canvas.rgba)));
    }
    // The SHA-256 of each image's pixels. The camera is built from square
    // roots, so these hold on every machine.
    assert_eq!(
        digests,
        [
            "16 88619b856df8c39dff6da51ceb8cca8f866fcdd05b6acc29bf37322ee0d5ad22",
            "18 e1f677b7ea82d687fb1e5ce8c649b56b5757b5fec857f4ae15f8742134bd8493",
            "21 97b69425658b9a63c4e1fdcb0d3b320fbb3aa91ba6ca5fe0f001321cc7abc7db",
        ]
    );
}
