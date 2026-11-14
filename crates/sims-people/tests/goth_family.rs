//! The Goth family (the base game's House05) drawn from the install's own
//! files. Needs `SIMS_INSTALL`; skipped when it is unset. Nothing from the
//! game is kept: the tests pin names and SHA-256 digests only.

use sha2::{Digest, Sha256};
use sims_people::appearance::load_appearance;
use sims_people::math::{Transform, Vec3};
use sims_people::raster::{Camera, Canvas};
use sims_people::{Assets, Body, Cast, residents, rig};
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
            "16 79b37b6db72878fb410d1b5168abe5a5b5ca247a767e03241137e4d623f4ceaf",
            "18 c90dbc30ea920b25b28a579a9b1279c41510bf778a3260c0e2c2045db07c6742",
            "21 6208df8c45e5ff80bea67cfd0e51af55fd9fd67881b6cefa75c54ed3c0fa12c7",
        ]
    );
}

/// An appearance is found by its own name when its file is named
/// otherwise: the lab technician's uniform `B629MAFitlgt_labtech` is held
/// by `b629ma_labtech.cmx`.
#[test]
fn an_appearance_is_found_by_its_own_name() {
    let Some(root) = install() else { return };
    let assets = Assets::scan(&root).unwrap();
    let appearance = load_appearance(&assets, "b629mafitlgt_labtech").unwrap();
    let meshes: Vec<&str> = appearance
        .bindings
        .iter()
        .map(|b| b.mesh.as_str())
        .collect();
    assert_eq!(appearance.name, "b629mafitlgt_labtech");
    assert_eq!(meshes, ["xskin-B629MAFit_labtech-PELVIS-BODY"]);
    assert!(load_appearance(&assets, "no-such-appearance").is_err());
}

/// A person record with nothing in it, as a script makes one.
fn blank_person() -> sims_formats::saved_objects::Person {
    let idle = sims_formats::saved_objects::Interaction {
        uid: 0,
        caller: 0,
        target: 0,
        icon: 0,
        table_index: -1,
        args: [0; 4],
        priority: 0,
        action: 0,
        attenuation: 0.0,
        flags: 0,
    };
    sims_formats::saved_objects::Person {
        animation_events: 0,
        engaged: 0,
        appearance: Default::default(),
        accessories: Vec::new(),
        animations: Default::default(),
        routing_state: 0,
        floats: [0.0; 9],
        previous_motives: [0.0; 16],
        motives: [0.0; 16],
        data: Vec::new(),
        routing_frames: 0,
        active: idle.clone(),
        last: idle,
        queue: Vec::new(),
        object_uses: Vec::new(),
        motive_deltas: Vec::new(),
    }
}

/// A person a script makes after the load is not among the saved house's
/// residents, and its record names no clothes. Person 259 of the Goth
/// boot is the traveling salesman (type 0x44a86a6c): it is drawn from its
/// type's skeleton and default outfit.
#[test]
fn a_person_made_after_the_load_is_drawn_from_its_type() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    assert!(
        !residents(&root, &house)
            .unwrap()
            .iter()
            .any(|r| r.id == 259)
    );
    let cast = Cast::of_house(&root, &house).unwrap();
    let person = blank_person();
    let found = cast.residents([(259, 0x44a8_6a6c, &person)]).unwrap();
    let [salesman] = found.as_slice() else {
        panic!("{found:?}")
    };
    assert_eq!(salesman.skeleton, "adult");
    let names: Vec<&str> = salesman
        .outfit
        .parts
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "SpellboundMAFat_TravelSales",
            "HfLO",
            "HfRO",
            "CSpellboundMA_TravelSales"
        ]
    );
    let assets = Assets::scan(&root).unwrap();
    let body = Body::load(&assets, &salesman.skeleton, &salesman.outfit).unwrap();
    let world = rig::Pose::rest(&body.skeleton).world(&body.skeleton, Transform::IDENTITY);
    let mut canvas = Canvas::new(320, 400, [0, 0, 0, 0]);
    canvas.draw_body(
        &Camera::house_view(60.0, Vec3::new(0.0, 2.8, 0.0)),
        &body,
        &world,
    );
    assert!(canvas.covered() > 5000);
    // A type that is not a person is refused, not drawn.
    assert!(cast.residents([(1, 0, &person)]).is_err());
}
