//! The Goth family (the base game's House05) drawn from the install's own
//! files. Needs `SIMS_INSTALL`; skipped when it is unset. Nothing from the
//! game is kept: the tests pin names and SHA-256 digests only.

use crate::common::hex;
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

#[test]
fn mortimers_pie_head_matches_recorded_menu_poses() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let assets = Assets::scan(&root).unwrap();
    let people = residents(&root, &house).unwrap();
    let mortimer = people.iter().find(|r| r.id == 21).unwrap();
    let body = Body::load(&assets, &mortimer.skeleton, &mortimer.outfit).unwrap();

    // Oversized canvases return an error cleanly instead of panicking; zero-sized
    // canvases succeed with an empty buffer.
    assert!(
        sims_people::pie_head::draw_body(
            &body,
            [65_536, 65_536],
            sims_people::pie_head::Look::neutral(),
        )
        .is_err()
    );
    let empty =
        sims_people::pie_head::draw_body(&body, [0, 0], sims_people::pie_head::Look::neutral())
            .unwrap();
    assert_eq!((empty.width, empty.height, empty.rgba.len()), (0, 0, 0));

    let choice2_look =
        sims_people::pie_head::Look::from_menu([200, 198], Some([200, 123]), Some(0), 4);
    let choice2 = sims_people::pie_head::draw_body(&body, [201, 201], choice2_look).unwrap();
    let talk_look =
        sims_people::pie_head::Look::from_menu([400, 274], Some([400, 348]), Some(4), 6);
    let talk = sims_people::pie_head::draw_body(&body, [201, 201], talk_look).unwrap();
    let read = sims_people::pie_head::draw_body(
        &body,
        [201, 201],
        sims_people::pie_head::Look::from_menu([384, 250], Some([384, 324]), Some(2), 3),
    )
    .unwrap();
    let idle_look = sims_people::pie_head::Look::neutral();
    let idle = sims_people::pie_head::draw_body(&body, [201, 201], idle_look).unwrap();

    assert_eq!(
        choice2_look.ambient(),
        sims_people::pie_head::AMBIENT_HIGHLIGHTED
    );
    assert_eq!(
        talk_look.ambient(),
        sims_people::pie_head::AMBIENT_HIGHLIGHTED
    );
    assert_eq!(idle_look.ambient(), sims_people::pie_head::AMBIENT_IDLE);

    let stats = |c: &Canvas| -> (usize, f64, f64) {
        let mut count = 0usize;
        let mut y_sum = 0f64;
        let mut lum_sum = 0f64;
        for (i, p) in c.rgba.as_chunks::<4>().0.iter().enumerate() {
            if p[3] == 255 {
                count += 1;
                y_sum += (i / c.width as usize) as f64;
                lum_sum += f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]);
            }
        }
        (count, y_sum / count as f64, lum_sum / (count as f64 * 3.0))
    };

    let (choice2_n, choice2_y, choice2_lum) = stats(&choice2);
    let (talk_n, talk_y, _) = stats(&talk);
    let (read_n, _, _) = stats(&read);
    let (idle_n, _, idle_lum) = stats(&idle);

    assert_eq!(choice2_n, 1750);
    assert_eq!(talk_n, 1830);
    assert_eq!(read_n, 1830);
    assert_eq!(idle_n, 1940);
    assert_eq!(talk.rgba, read.rgba, "both look straight down at dy=+74");
    assert_ne!(choice2.rgba, talk.rgba);
    // Looking up (`Have A Snack`) shifts the projected head silhouette upward
    // relative to looking down (`Talk...`).
    assert!(
        choice2_y < talk_y,
        "expected choice2 centroid ({choice2_y:.2}) above talk ({talk_y:.2})"
    );
    // Idle lighting (0.8 ambient) is dimmer than highlighted upward pose (1.0 ambient).
    assert!(
        idle_lum < choice2_lum * 0.9,
        "idle_lum={idle_lum:.2}, choice2_lum={choice2_lum:.2}"
    );

    let cases = [
        (
            "choice2-a",
            &choice2,
            "823e32ce6c88d6619f158cb9889c9669487513f9c46cc2b1d2240f37d971321a",
        ),
        (
            "talk-submenu",
            &talk,
            "2a6d0c2c8b4e06ae601b5218e40635bc3c56f7344d0513748bf3cfc7269953e8",
        ),
        (
            "read-queued",
            &read,
            "2a6d0c2c8b4e06ae601b5218e40635bc3c56f7344d0513748bf3cfc7269953e8",
        ),
        (
            "idle",
            &idle,
            "a1669c1efa3e673eb8dec1a3d4907f1c4ab3c4f6600f7343e40458b6ccad7ffa",
        ),
    ];
    for (name, canvas, expected_sha) in cases {
        assert_eq!(
            hex(&canvas.rgba),
            expected_sha,
            "{name} rendered pie head changed"
        );
        if let Some(out) = std::env::var_os("SIMS_PEOPLE_OUT") {
            let path = PathBuf::from(out).join(format!("our-{name}.png"));
            std::fs::write(path, canvas.png()).unwrap();
        }
    }
}

/// Stock oversized headwear (`cspellboundfafit_flamingo` from Makin' Magic)
/// overflows the 201×201 face bitmap. Rendering directly in house-viewport
/// coordinates (`draw_body_in_viewport`, matching native `0x462621` →
/// `0x462985`) preserves the full uncropped headwear up to the house viewport
/// boundary while still clipping cleanly at the viewport edge.
#[test]
fn flamingo_pie_head_is_not_cropped_by_the_face_bitmap() {
    let Some(root) = install() else { return };
    let house = root.join("UserData/Houses/House05.iff");
    let assets = Assets::scan(&root).unwrap();
    let people = residents(&root, &house).unwrap();
    let bella = people.iter().find(|r| r.id == 18).unwrap();
    let mut outfit = bella.outfit.clone();
    // Replace the head appearance with the stock Flamingo head.
    let head_part = outfit
        .parts
        .iter_mut()
        .find(|(mesh, _)| mesh.to_ascii_lowercase().starts_with('c'))
        .expect("Bella has a head part");
    head_part.0 = "cspellboundfafit_flamingo".into();
    head_part.1 = String::new();
    let body = Body::load(&assets, &bella.skeleton, &outfit).unwrap();

    let look = sims_people::pie_head::Look::neutral();
    let cropped = sims_people::pie_head::draw_body(&body, [201, 201], look).unwrap();
    let full = sims_people::pie_head::draw_body_in_viewport(
        &body,
        [800, 500],
        [400, 250],
        sims_render::Zoom::Far,
        look,
    )
    .unwrap();
    let count_opaque = |rgba: &[u8]| {
        rgba.as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 255)
            .count()
    };
    let cropped_opaque = count_opaque(&cropped.rgba);
    let full_opaque = count_opaque(&full.canvas.rgba);

    assert_eq!(cropped_opaque, 11_628);
    assert_eq!(full_opaque, 14_014);
    assert!(
        full.canvas.width > 201 || full.canvas.height > 201,
        "expected flamingo bounds {}x{} to exceed 201x201",
        full.canvas.width,
        full.canvas.height
    );

    // Aiming the tall Flamingo hat straight down pitches its tip past the
    // native Direct3D 6 near plane (`0x49b506`, `0x462414`) at far zoom,
    // clipping pixels that remain visible at near zoom where the near plane is
    // 16× farther away in HEAD-bone units.
    let down = sims_people::pie_head::Look::from_menu([400, 250], Some([400, 324]), Some(4), 6);
    let far_down = sims_people::pie_head::draw_body_in_viewport(
        &body,
        [800, 500],
        [400, 250],
        sims_render::Zoom::Far,
        down,
    )
    .unwrap();
    let near_down = sims_people::pie_head::draw_body_in_viewport(
        &body,
        [800, 500],
        [400, 250],
        sims_render::Zoom::Near,
        down,
    )
    .unwrap();
    let far_down_opaque = count_opaque(&far_down.canvas.rgba);
    let near_down_opaque = count_opaque(&near_down.canvas.rgba);
    assert_eq!(near_down_opaque, 9_381);
    assert_eq!(far_down_opaque, 8_909);
    assert_eq!(near_down_opaque - far_down_opaque, 472);

    // Near the bottom of an 800×500 viewport, the viewport boundary clips the
    // mesh cleanly instead of overflowing.
    let edge = sims_people::pie_head::draw_body_in_viewport(
        &body,
        [800, 500],
        [400, 487],
        sims_render::Zoom::Far,
        look,
    )
    .unwrap();
    assert!(edge.origin[1] + edge.canvas.height as i32 <= 500);
    let edge_opaque = count_opaque(&edge.canvas.rgba);
    assert!(edge_opaque > 0 && edge_opaque < full_opaque);
}
