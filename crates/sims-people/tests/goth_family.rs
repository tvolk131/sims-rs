//! The Goth family (the base game's House05) drawn from the install's own
//! files. Needs `SIMS_INSTALL`; skipped when it is unset. Nothing from the
//! game is kept: the tests pin names and SHA-256 digests only.

use sha2::{Digest, Sha256};
use sims_people::appearance::load_appearance;
use sims_people::math::{Transform, Vec3};
use sims_people::raster::{Camera, Canvas};
use sims_people::{Assets, Body, Cast, Outfit, residents, rig, suit};
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
            "16 b7c08e82d976d973e7fde8cc8c172b85dee21356d7bf6c888efc3d80d489d04f",
            "18 89e983dfb481a63b04661d3e69af79b6f514477bef53e47ffb7b471dcc07bd2b",
            "21 2c571514845a277552ce7f5880c2f8123649fe739d039d324065c1b2fb6d7923",
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
        join: 0,
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

#[test]
fn an_appearance_in_two_catalogs_comes_from_the_first_folder_searched() {
    let Some(root) = install() else { return };
    let assets = Assets::scan(&root).unwrap();
    // `b200mafit_ctb` is in GameData/Skins as text and in Deluxe's archive
    // as binary, bound differently. The original searches GameData/Skins
    // first and keeps the first it loads.
    let text = assets.sources("b200mafit_ctb.cmx");
    let binary = assets.sources("b200mafit_ctb.cmx.bcf");
    assert_eq!(text.len(), 1);
    assert_eq!(binary.len(), 1);
    assert_eq!(assets.folder(&text[0]), "gamedata/skins");
    assert_eq!(assets.folder(&binary[0]), "deluxe/skins");
    let read = |bytes: Vec<u8>, binary: bool| {
        let catalog = if binary {
            sims_formats::avatar::read_bcf(&bytes).unwrap()
        } else {
            sims_formats::avatar::read_cmx(&bytes).unwrap()
        };
        catalog
            .appearances
            .into_iter()
            .find(|a| a.name.eq_ignore_ascii_case("b200mafit_ctb"))
            .unwrap()
    };
    let from_text = read(text[0].read().unwrap(), false);
    let from_binary = read(binary[0].read().unwrap(), true);
    // Bindings have no equality of their own; their printed forms do.
    let bindings = |a: &sims_formats::avatar::Appearance| format!("{:?}", a.bindings);
    assert_ne!(bindings(&from_text), bindings(&from_binary));
    let chosen = load_appearance(&assets, "B200MAFit_CTB").unwrap();
    assert_eq!(bindings(&chosen), bindings(&from_text));
}

/// Mortimer Goth dressed for work (suit 3, set when he changes for his
/// job) wears his career level's clothes: the science track's first
/// level names `Science$g$b_01` with the texture `Science$g$c_01`, which
/// for a man of fit build and light skin match these files.
#[test]
fn mortimer_in_his_work_clothes() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let cast = Cast::of_house(&root, &house).unwrap();
    let assets = Assets::scan(&root).unwrap();
    let people = residents(&root, &house).unwrap();
    let mortimer = people.iter().find(|r| r.id == 21).unwrap();
    let mut data = mortimer.person.data.clone();
    assert_eq!(
        [data[suit::JOB], data[suit::JOB_LEVEL]],
        [9, 0],
        "Mortimer's career"
    );
    data[suit::SUIT] = 3;
    let rows = cast.body_strings(mortimer.guid).unwrap();
    let worn = suit::Clothes::of(&mortimer.person, &rows);
    let clothes = suit::dress(
        &worn,
        &data,
        &rows,
        cast.careers(),
        &suit::Wardrobe::of(&assets),
    )
    .unwrap();
    assert_eq!(
        clothes.body,
        ("sciencemfit_01".into(), "BODY=sciencemlgt_01".into())
    );
    assert_eq!(clothes.hands, worn.hands);
    let mut person = mortimer.person.clone();
    person.data = data;
    let outfit = clothes.wear(&person, &rows);
    assert_eq!(outfit.parts[0], clothes.body);
    assert_eq!(
        outfit.parts.len(),
        Outfit::from_person(&mortimer.person).parts.len()
    );
    let body = Body::load(&assets, &mortimer.skeleton, &outfit).unwrap();
    assert!(
        body.parts
            .iter()
            .any(|p| p.texture_name.as_deref() == Some("sciencemlgt_01")),
        "the coat is drawn with its texture"
    );
}

/// A person a script made has a record that names no clothes; their suit
/// still dresses them, starting from their type's default outfit.
#[test]
fn a_person_with_no_chosen_clothes_still_wears_their_suit() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let cast = Cast::of_house(&root, &house).unwrap();
    let assets = Assets::scan(&root).unwrap();
    let people = residents(&root, &house).unwrap();
    let mortimer = people.iter().find(|r| r.id == 21).unwrap();
    let rows = cast.body_strings(mortimer.guid).unwrap();
    let mut person = mortimer.person.clone();
    person.appearance = Default::default();
    person.accessories.clear();
    person.data[suit::SUIT] = 3;
    let worn = suit::Clothes::of(&person, &rows);
    let defaults = Outfit::from_body_strings(&rows);
    assert_eq!(worn.body, defaults.parts[0]);
    let clothes = suit::dress(
        &worn,
        &person.data,
        &rows,
        cast.careers(),
        &suit::Wardrobe::of(&assets),
    )
    .unwrap();
    assert_eq!(clothes.body.0, "sciencemfit_01");
    let outfit = clothes.wear(&person, &rows);
    // The coat, the type's two hands and its head.
    assert_eq!(outfit.parts[0], clothes.body);
    assert_eq!(&outfit.parts[1..], &defaults.parts[1..]);
}
