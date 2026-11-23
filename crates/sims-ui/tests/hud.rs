//! The control panel drawn for the Goths' house. Needs the game: set
//! `SIMS_INSTALL` to the install folder.

mod common;

use sha2::{Digest, Sha256};
use sims_ui::balloon::{Help, Texts};
use sims_ui::command::{Command, Panel};
use sims_ui::frame::{Frame, Rect};
use sims_ui::hud::{self, Controls};
use sims_ui::{house, household};

fn digest(frame: &Frame) -> String {
    let mut hash = Sha256::new();
    for pixel in frame.rgb() {
        hash.update(pixel);
    }
    format!("{:x}", hash.finalize())
}

#[test]
fn the_goths_panel_from_the_original_art() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to draw the control panel");
        return;
    };
    let opened = common::open(&root, common::GOTH_HOUSE);
    assert!(opened.art.missing.is_empty(), "{:?}", opened.art.missing);
    let s = &opened.snapshot;
    // Mortimer, Bella and Cassandra, in the family's order, as the
    // original's portraits are; the clock and funds as it showed them
    // after loading.
    let people: Vec<u16> = s.residents.iter().map(|r| r.object).collect();
    assert_eq!(people, [21, 18, 16]);
    assert_eq!(s.selected, Some(21));
    assert_eq!(
        (s.clock().as_str(), s.money().as_str()),
        ("AM  6:30", "$2,542")
    );
    for r in &s.residents {
        let face = opened.faces.face(r.guid).expect("a face");
        assert_eq!((face.width, face.height), (35, 41));
    }
    let open = Controls {
        expanded: true,
        ..Controls::default()
    };
    // The digests pin the drawing; docs/decisions/ says how it compares
    // with the original's screen.
    let cases = [
        ([800, 600], Controls::default(), "closed 800x600"),
        ([800, 600], open, "open 800x600"),
        ([1280, 720], open, "open 1280x720"),
    ];
    let mut digests = Vec::new();
    for (size, controls, name) in cases {
        let mut frame = Frame::new(size[0] as u32, size[1] as u32, [0, 0, 0]);
        let items = hud::layout(size, s, &controls);
        hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
        digests.push((name, digest(&frame).as_str().to_owned()));
        if name == "open 1280x720" {
            // The 7-button gadget stays anchored beside the portrait block at
            // x = 413; the strip's background fills cleanly to the right edge.
            assert_eq!(
                hud::press(&items, [413 + 50, 720 - 100 + 15]),
                Some(Command::OpenPanel(Panel::Interests))
            );
            assert_ne!(frame.pixel(1279, 650), Some([0, 0, 0]));
        }
    }
    let expected = [
        (
            "closed 800x600",
            "b1bc43432952bb7f94824e35de94f403a264990302c44589ac626558689eec71",
        ),
        (
            "open 800x600",
            "8c29854af1d520b860ff97c6df97a67d9e540659586985d35a2d8f563ef083c8",
        ),
        (
            "open 1280x720",
            "ee9167b27c597535da0d2a702ebdf62b4a755f9304280daa6255d874c6412de6",
        ),
    ];
    let digests: Vec<(&str, &str)> = digests.iter().map(|(n, d)| (*n, d.as_str())).collect();
    assert_eq!(digests, expected);
}

#[test]
fn the_goths_personality_panel() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to draw the control panel");
        return;
    };
    let opened = common::open(&root, common::GOTH_HOUSE);
    // Neat, Outgoing, Active, Playful and Nice, from the house file.
    let traits: Vec<[i16; 5]> = opened
        .snapshot
        .residents
        .iter()
        .map(|r| r.personality)
        .collect();
    assert_eq!(
        traits,
        [
            [400, 600, 400, 700, 400],
            [700, 300, 500, 300, 700],
            [700, 200, 500, 700, 400],
        ]
    );
    // Their signs, saved in the house file (person data 70): Taurus,
    // Cancer and Cancer. Worked out from their traits, as the original
    // does where none is saved, they come out the same.
    let signs: Vec<Option<u8>> = opened.snapshot.residents.iter().map(|r| r.zodiac).collect();
    assert_eq!(signs, [Some(2), Some(4), Some(4)]);
    let worked_out: Vec<Option<u8>> = opened
        .snapshot
        .residents
        .iter()
        .map(|r| {
            household::zodiac(|word| {
                household::PERSONALITY
                    .iter()
                    .position(|w| *w == word)
                    .map_or(0, |n| r.personality[n])
            })
        })
        .collect();
    assert_eq!(worked_out, signs);
    let controls = Controls {
        expanded: true,
        panel: Some(Panel::Personality),
        ..Controls::default()
    };
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let items = hud::layout([800, 600], &opened.snapshot, &controls);
    hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
    // Pins the drawing; it has not been compared with the original's
    // screen (see docs/decisions/).
    assert_eq!(
        digest(&frame),
        "71013e6ee23e578bd874a40cb0f18dd023526724999d9e5e9e41a2c26b081d9e"
    );
}

#[test]
fn the_goths_sign_balloon() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to draw the control panel");
        return;
    };
    let opened = common::open(&root, common::GOTH_HOUSE);
    let texts = Texts::load(&opened.install).expect("the balloons' texts");
    // Mortimer is a Taurus: he gets on best with Aries and Libra, worst
    // with Virgo and Cancer.
    let taurus = texts.get(Help::Sign(2)).expect("Taurus");
    assert!(!taurus.title.is_empty());
    let lines: Vec<&str> = taurus.body.lines().collect();
    assert!(lines[lines.len() - 2].ends_with("Aries, Libra"));
    assert!(lines[lines.len() - 1].ends_with("Virgo, Cancer"));

    let mut controls = Controls {
        expanded: true,
        panel: Some(Panel::Personality),
        ..Controls::default()
    };
    let mut items = hud::layout([800, 600], &opened.snapshot, &controls);
    hud::fit(&mut items, &opened.art, &texts);
    // The sign's name, at the panel's bottom left.
    let command = hud::press(&items, [413 + 108 + 10, 600 - 12]);
    assert_eq!(command, Some(Command::Help(Help::Sign(2))));
    controls.apply(&command.unwrap());
    let mut items = hud::layout([800, 600], &opened.snapshot, &controls);
    hud::fit(&mut items, &opened.art, &texts);
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
    // The balloon from 241 to the right edge and up from the strip, as
    // tall as the least a balloon takes. On the original's screen it is
    // there, and every one of its pixels is the same in 16-bit color
    // (docs/decisions/20261116-sign-help-balloon.md).
    assert_eq!(
        items.last().map(|i| i.rect),
        Some(Rect::new(241, 373, 559, 127))
    );
    assert_eq!(
        digest(&frame),
        "595b37c19a714176053a4843977fa72417549215d58106079dbd0919e1382b57"
    );
}

#[test]
fn the_goths_job_panel() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to draw the control panel");
        return;
    };
    let opened = common::open(&root, common::GOTH_HOUSE);
    // Mortimer is a Science "Test Subject" (career 9, level 0), Bella has
    // no career, and Cassandra is at school (-1); nobody has a skill yet,
    // and the next level of Mortimer's career needs none.
    let jobs: Vec<_> = opened
        .snapshot
        .residents
        .iter()
        .map(|r| (r.job, r.skills, r.needs))
        .collect();
    assert_eq!(
        jobs,
        [
            ([9, 0], [0; 6], [0; 6]),
            ([0, 0], [0; 6], [0; 6]),
            ([-1, 4], [0; 6], [0; 6])
        ]
    );
    // The left column: Mortimer's icon (row 9, the flask), title and
    // pay; Bella unemployed; Cassandra, a child, at school, level 0, with
    // no icon (row -1) and no title (school has no `STR#`), her
    // performance her data 57. The family has no friends.
    let shown: Vec<_> = opened
        .snapshot
        .residents
        .iter()
        .map(|r| {
            (
                r.child,
                r.female,
                r.performance,
                r.work.icon,
                r.work.level.as_ref().map(|l| (l.title.as_str(), l.salary)),
                r.work.friends,
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            (false, false, 0, 9, Some(("Test Subject", 155)), 0),
            (false, true, 0, 0, None, 0),
            (true, true, 4, -1, Some(("", 0)), 0),
        ]
    );
    assert_eq!(opened.snapshot.family_friends, 0);
    let controls = Controls {
        expanded: true,
        panel: Some(Panel::Job),
        ..Controls::default()
    };
    let draw = |snapshot| {
        let mut frame = Frame::new(800, 600, [0, 0, 0]);
        let items = hud::layout([800, 600], snapshot, &controls);
        hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
        digest(&frame)
    };
    // Pins the drawing; it has not been compared with the original's
    // screen (see docs/decisions/).
    assert_eq!(
        draw(&opened.snapshot),
        "154e5514ced4c334513df0ea06f03c4e1b60c099263de8e25c1f69fb25fa8acd"
    );
    // A level up the ladder, "Field Researcher" at $320: the next level,
    // "Science Teacher", needs a point of Charisma, three of Logic, which
    // he has two of, and a family friend, which the family lacks.
    let mut snapshot = opened.snapshot.clone();
    snapshot.residents[0].job = [9, 2];
    snapshot.residents[0].skills = [0, 0, 0, 0, 200, 0];
    let snapshot = snapshot.with_careers(&opened.careers);
    assert_eq!(snapshot.residents[0].needs, [0, 0, 100, 0, 300, 0]);
    let work = &snapshot.residents[0].work;
    assert_eq!(
        (
            work.level.as_ref().map(|l| (l.title.as_str(), l.salary)),
            work.friends
        ),
        (Some(("Field Researcher", 320)), 1)
    );
    assert_eq!(
        draw(&snapshot),
        "50cb1d1271096e777ce37692e93d4615b2642099f62dff85e66ffef2bd1ad10c"
    );
    // Titles wider than the column break as the original's do: after a
    // hyphen or a space, and both lines start above the box's bottom.
    let font = opened.art.font(7).expect("the 7-point font");
    assert_eq!(font.line_height, 13);
    assert_eq!(
        font.break_lines("Counter-Intelligence", 95),
        ["Counter-", "Intelligence"]
    );
    assert_eq!(
        font.break_lines("Medical Technician", 95),
        ["Medical ", "Technician"]
    );
    let mut snapshot = opened.snapshot.clone();
    if let Some(level) = &mut snapshot.residents[0].work.level {
        level.title = "Counter-Intelligence".into();
    }
    assert_eq!(
        draw(&snapshot),
        "0a071502ea744252fb5dbb0ed3bb20e8f8c080c05b056d95c041b6895f0005dc"
    );
}

#[test]
fn the_goths_relationship_panel() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to draw the control panel");
        return;
    };
    let opened = common::open(&root, common::GOTH_HOUSE);
    // Mortimer, selected, knows his wife and daughter: friends both ways,
    // the family first by the day's feeling, and Bella with the heart.
    let relations: Vec<_> = opened
        .snapshot
        .relations
        .iter()
        .map(|r| {
            (
                r.id,
                r.daily,
                r.lifetime,
                r.friends,
                r.pink_heart,
                r.red_heart,
                r.family,
                r.star,
            )
        })
        .collect();
    assert_eq!(
        relations,
        [
            (33, 100, 0, true, true, false, true, false),
            (32, 75, 0, true, false, false, true, false),
        ]
    );
    // The install has the Superstar pack, so its button can be pressed
    // (0x51a7ea).
    assert!(opened.snapshot.superstar);
    let controls = Controls {
        expanded: true,
        panel: Some(Panel::Relationships),
        ..Controls::default()
    };
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let items = hud::layout([800, 600], &opened.snapshot, &controls);
    hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
    // Pins the drawing; it has not been compared with the original's
    // screen (see docs/decisions/).
    assert_eq!(
        digest(&frame),
        "8e50d3bb46331ffd62e455a3704fbd804c4fa0a6c52475561ef02c98a575d584"
    );
}

#[test]
fn the_goths_house_panel() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to draw the control panel");
        return;
    };
    let opened = common::open(&root, common::GOTH_HOUSE);
    // The global file's curves (STR# 505) and yard multiplier (tuning
    // table 4), as the original's capture holds the multiplier.
    assert_eq!(opened.house.size.points.len(), 5);
    assert_eq!(opened.house.furnishings.points.len(), 6);
    assert_eq!(opened.house.yard, sims_core::F32::from_f32(0.33));
    // After the load: 1,836 square feet, two bedrooms and a bathroom on
    // a large lot. Size: 612 square feet for each of three members;
    // Yard: the outdoor rooms' score times 0.33. Layout is not ported.
    let house = &opened.snapshot.house;
    assert_eq!(
        (
            house.square_feet,
            house.bedrooms,
            house.bathrooms,
            house.lot
        ),
        (1836, 2, 1, 2)
    );
    // Upkeep: 73 of the lot's 253 object types count, and nothing is
    // dirty or broken after the load, so it is 100 whatever the
    // penalties, which the install's table sets to 3 and 6.
    let counted = opened.kinds.counted.values().filter(|c| **c).count();
    assert_eq!((counted, opened.kinds.counted.len()), (73, 253));
    // A person type Make New Character made counts as its template does.
    for template_counts in [true, false] {
        let (&template, _) = opened
            .kinds
            .counted
            .iter()
            .find(|&(_, &c)| c == template_counts)
            .unwrap();
        let mut world = opened.world.clone();
        let made = 0x1234_0042;
        world.characters.made.insert(
            made,
            sims_sim::characters::Character {
                template,
                file: 0,
                strings: Default::default(),
            },
        );
        world.objects.values_mut().next().unwrap().guid = made;
        let mut kinds = house::Kinds::default();
        kinds.learn(&world, &opened.index, &opened.catalog);
        assert_eq!(kinds.counted[&made], template_counts);
    }
    assert_eq!((opened.house.dirty, opened.house.broken), (3, 6));
    // The panel weighs the lot's values as the simulation keeps them:
    // the house file saved 14,660 and 15,277, what the original's
    // simulator holds after the load (+0xa0 and +0xa4, track 2's capture
    // of update 1). Furnishings weighs 15,277 over 14,660, 1.042, on the
    // curve.
    assert_eq!(
        (house.values.objects, house.values.architecture),
        (14660, 15277)
    );
    // The lot statistics' sums, worked out from the lot with the saved
    // values cleared, come to the same two.
    let mut unsaved = opened.world.clone();
    unsaved.budget.objects_value = 0;
    unsaved.budget.architecture_value = 0;
    assert_eq!(
        house::Values::read(&unsaved, &opened.kinds, &opened.prices),
        house.values
    );
    assert_eq!(
        house.ratings,
        [Some(75), Some(58), Some(76), Some(100), None]
    );
    let controls = Controls {
        expanded: true,
        panel: Some(Panel::House),
        ..Controls::default()
    };
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let items = hud::layout([800, 600], &opened.snapshot, &controls);
    hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
    // Pins the drawing; it has not been compared with the original's
    // screen (see docs/decisions/).
    assert_eq!(
        digest(&frame),
        "414dd3f98cdf74e01e7b8363dae9f59a9ac17dcb1be9736721a8d7ae8a50a77c"
    );
}

#[test]
fn report_card_superstar_and_needs_pet_balloons_from_install() {
    use sims_ui::balloon::Picture;
    use sims_ui::thumbnail::Thumbnails;

    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to test live subpanels and balloons");
        return;
    };
    let opened = common::open(&root, common::GOTH_HOUSE);
    let texts = Texts::load(&opened.install).expect("balloon texts");

    // Complete Collection loads all optional panel & balloon artwork + font 18.
    assert!(opened.art.font(18).is_some());
    assert!(opened.art.image(hud::id::SOCIAL_POPUP).is_some());
    assert!(opened.art.image(hud::id::JOB_POPUP).is_some());
    assert!(opened.art.image(hud::id::FAME_BAR_FULL).is_some());
    assert!(opened.art.image(hud::id::FAME_BAR_EMPTY).is_some());
    assert!(opened.art.image(hud::id::FAME_FRIENDS).is_some());
    assert!(opened.art.image(hud::id::FAME_RUNWAY).is_some());
    assert!(opened.art.image(hud::id::FAME_MOVIE_SET).is_some());
    assert!(opened.art.image(hud::id::FAME_MUSIC_VIDEO).is_some());

    // 1. Cassandra (`residents[2]`, object 16, school_age = true, grade 4 = "B")
    //    routes Job tab to Child Report Card and opens the Report Card balloon.
    assert_eq!(opened.snapshot.residents[2].school_grade, "B");
    let mut cass_snap = opened.snapshot.clone();
    cass_snap.selected = Some(cass_snap.residents[2].object);
    cass_snap.selected_guid = Some(cass_snap.residents[2].guid);
    let mut controls = Controls {
        expanded: true,
        panel: Some(Panel::Job),
        ..Controls::default()
    };
    let mut items = hud::layout([800, 600], &cass_snap, &controls);
    hud::fit(&mut items, &opened.art, &texts);
    assert!(items.iter().any(|i| matches!(
        &i.kind,
        hud::Kind::ReportCard { grade } if grade == "B"
    )));
    let grade_btn = items
        .iter()
        .find(|i| matches!(i.kind, hud::Kind::ReportCardButton { .. }))
        .expect("Report Card grade button")
        .rect;
    assert_eq!(
        hud::press(&items, [grade_btn.x + 20, grade_btn.y + 5]),
        Some(Command::Help(Help::ReportCard))
    );
    controls.apply(&Command::Help(Help::ReportCard));
    let mut items = hud::layout([800, 600], &cass_snap, &controls);
    hud::fit(&mut items, &opened.art, &texts);
    let rc_balloon = items
        .iter()
        .find_map(|i| match &i.kind {
            hud::Kind::Balloon(b) => Some(b),
            _ => None,
        })
        .expect("Report Card balloon");
    assert_eq!(rc_balloon.title, "Grades");
    assert!(rc_balloon.lines.iter().any(|l| l.contains("school")));
    let mut frame_no_balloon = Frame::new(800, 600, [0, 0, 0]);
    let mut plain_controls = controls;
    plain_controls.help = None;
    let plain_items = hud::layout([800, 600], &cass_snap, &plain_controls);
    hud::draw(
        &mut frame_no_balloon,
        &opened.art,
        &opened.faces,
        &plain_items,
        &plain_controls,
    );
    let mut frame_rc = Frame::new(800, 600, [0, 0, 0]);
    hud::draw(&mut frame_rc, &opened.art, &opened.faces, &items, &controls);
    assert_ne!(frame_rc.rgb(), frame_no_balloon.rgb());

    // 2. Human Needs (`STR# 130`), Pet Needs (`STR# 180`), and Dog/Cat Pet
    //    Skills (`STR# 172`) balloons load real strings and pictures.
    let hunger = texts
        .get(Help::Need {
            pet: false,
            index: 0,
        })
        .unwrap();
    let pet_hunger = texts
        .get(Help::Need {
            pet: true,
            index: 0,
        })
        .unwrap();
    assert_eq!(hunger.title, "Hunger");
    assert_eq!(pet_hunger.title, "Hunger");
    assert_ne!(hunger.body, pet_hunger.body);
    assert!(pet_hunger.body.contains("pet"));
    let dog0 = texts
        .get(Help::PetSkill {
            cat: false,
            index: 0,
        })
        .unwrap();
    let cat0 = texts
        .get(Help::PetSkill {
            cat: true,
            index: 0,
        })
        .unwrap();
    assert_eq!(dog0.title, "House Breaking Skill");
    assert_eq!(cat0.title, "Hunting Skill");

    // Draw Hunger balloon with preloaded native isometric object sprites (`0x4e5306 -> 0x4384c8`),
    // including Hunger's right-slot zoom 2 override (`0xff9f_18e0`, `0x6f3994`), and verify
    // representative multi-tile and Superstar object GUIDs render non-empty sprites.
    let mut thumbs = Thumbnails::default();
    for guid in [
        0xb947_55df, // Hunger left (fridge)
        0xff9f_18e0, // Hunger right (zoom 2 override)
        0x82e0_4c5b, // Comfort (chair/sofa)
        0x8eec_19f7, // Pet Hunger left
        0xffc0_ea58, // Pet Hunger right (zoom 2 override)
        0x4450_e4e0, // Superstar Fame level object
        0x4b61_d53e, // Superstar level 10 object
    ] {
        thumbs
            .read_balloon(&opened.index, &opened.catalog, guid)
            .expect("isometric balloon picture loads");
        let pic = thumbs
            .balloon_picture(guid, None, 168, 127)
            .unwrap_or_else(|| panic!("isometric sprite for 0x{guid:08X}"));
        assert!(pic.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
    }
    let left_pic = thumbs
        .balloon_picture(0xb947_55df, None, 168, 127)
        .expect("fridge isometric sprite");
    let right_pic = thumbs
        .balloon_picture(0xff9f_18e0, Some(2), 168, 127)
        .expect("food right-slot zoom-2 isometric sprite");
    assert_ne!([left_pic.width, left_pic.height], [45, 45]);
    assert!(left_pic.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
    assert!(right_pic.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));

    let need_controls = Controls {
        expanded: true,
        panel: Some(Panel::Needs),
        help: Some(Help::Need {
            pet: false,
            index: 0,
        }),
        ..Controls::default()
    };
    let mut need_items = hud::layout([800, 600], &opened.snapshot, &need_controls);
    hud::fit(&mut need_items, &opened.art, &texts);
    let mut frame_need_empty = Frame::new(800, 600, [0, 0, 0]);
    hud::draw_with_thumbnails(
        &mut frame_need_empty,
        &opened.art,
        &opened.faces,
        None,
        &need_items,
        &need_controls,
    );
    let mut frame_need = Frame::new(800, 600, [0, 0, 0]);
    hud::draw_with_thumbnails(
        &mut frame_need,
        &opened.art,
        &opened.faces,
        Some(&thumbs),
        &need_items,
        &need_controls,
    );
    assert_ne!(frame_need.rgb(), frame_need_empty.rgb());
    assert!(need_items.iter().any(|i| matches!(
        &i.kind,
        hud::Kind::Balloon(b) if b.title == "Hunger" && b.text_left == 168
    )));

    // 3. Superstar / Fame panel loaded from `GameData/fame.iff`.
    assert_eq!(opened.fame.title, "Fame");
    assert_eq!(opened.fame.level_title(0), "Nobody");
    assert_eq!(opened.fame.level_title(10), "Superstar");
    assert_eq!(
        texts.get(Help::FameLevel(7)).unwrap().pictures,
        [Some(Picture::Image(hud::id::FAME_RUNWAY)), None]
    );
    assert_eq!(
        texts.get(Help::FameLevel(8)).unwrap().pictures,
        [Some(Picture::Image(hud::id::FAME_MUSIC_VIDEO)), None]
    );
    assert_eq!(
        texts.get(Help::FameLevel(9)).unwrap().pictures,
        [Some(Picture::Image(hud::id::FAME_MOVIE_SET)), None]
    );
    assert_eq!(
        texts.get(Help::FameFriends).unwrap().title,
        "Friends Total Starpower"
    );

    let mut star_snap = opened.snapshot.clone();
    // Bella (`residents[1]`) has no career: give her fame score > 0 and level 4 (2.0 stars).
    star_snap.selected = Some(star_snap.residents[1].object);
    star_snap.selected_guid = Some(star_snap.residents[1].guid);
    star_snap.residents[1].fame_score = 50;
    star_snap.residents[1].fame_level = 4;
    star_snap.residents[1].fame_title = opened.fame.level_title(4).to_owned();
    star_snap.residents[1].fame_needs = opened.fame.skill_needs(4, &star_snap.residents[1].skills);
    star_snap.residents[1].fame_friends_text = opened.fame.format_friend_star_power(4, 3);
    assert_eq!(star_snap.residents[1].fame_title, "Studio Fly");
    assert_eq!(star_snap.residents[1].fame_needs, [0, 0, 4, 3, 0, 4]);
    assert_eq!(star_snap.residents[1].fame_friends_text, "1.5 (Need 2)");

    let fame_controls = Controls {
        expanded: true,
        panel: Some(Panel::Job),
        help: Some(Help::FameLevel(4)),
        ..Controls::default()
    };
    let mut fame_items = hud::layout([800, 600], &star_snap, &fame_controls);
    hud::fit(&mut fame_items, &opened.art, &texts);
    assert!(fame_items.iter().any(|i| matches!(
        &i.kind,
        hud::Kind::Fame {
            fame_level: 4,
            fame_title,
            ..
        } if fame_title == "Studio Fly"
    )));
    let mut fame_frame = Frame::new(800, 600, [0, 0, 0]);
    hud::draw_with_thumbnails(
        &mut fame_frame,
        &opened.art,
        &opened.faces,
        Some(&thumbs),
        &fame_items,
        &fame_controls,
    );
    assert!(fame_items.iter().any(|i| matches!(
        &i.kind,
        hud::Kind::Balloon(b) if b.title.starts_with("Studio Fly")
    )));
}

#[test]
fn expanded_hud_at_800x600_and_1024x768_preserves_native_pixels_outside_backpatch() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to verify 800x600 and 1024x768 HUD preservation");
        return;
    };
    let opened = common::open(&root, common::GOTH_HOUSE);
    let open = Controls {
        expanded: true,
        ..Controls::default()
    };
    for [w, h] in [[800, 600], [1024, 768]] {
        let items = hud::layout([w, h], &opened.snapshot, &open);
        // Gadget stays at native x = 413 on both 800x600 and 1024x768.
        let gauge = items
            .iter()
            .find(|i| matches!(i.kind, hud::Kind::Image(hud::id::GAUGE)))
            .expect("gauge present");
        assert_eq!(gauge.rect, Rect::new(413, h - hud::STRIP, 108, hud::STRIP));

        let mut with_patch = Frame::new(w as u32, h as u32, [0, 0, 0]);
        hud::draw(&mut with_patch, &opened.art, &opened.faces, &items, &open);

        let without_patch_items: Vec<_> = items
            .iter()
            .filter(|i| !matches!(i.kind, hud::Kind::Image(hud::id::BACK_PATCH)))
            .cloned()
            .collect();
        let mut without_patch = Frame::new(w as u32, h as u32, [0, 0, 0]);
        hud::draw(
            &mut without_patch,
            &opened.art,
            &opened.faces,
            &without_patch_items,
            &open,
        );

        let patch_rect = Rect::new(168, h - hud::STRIP, 52, hud::STRIP);
        let mut patch_diff = 0usize;
        for y in 0..h {
            for x in 0..w {
                let p_with = with_patch.pixel(x, y);
                let p_without = without_patch.pixel(x, y);
                if patch_rect.contains(x, y) {
                    if p_with != p_without {
                        patch_diff += 1;
                    }
                } else {
                    assert_eq!(
                        p_with, p_without,
                        "unexpected pixel change outside BackPatch at ({x}, {y}) on {w}x{h}"
                    );
                }
            }
        }
        assert!(
            patch_diff > 0,
            "BackPatch must actually fill transparent cluster wedge at {w}x{h}"
        );
        if [w, h] == [800, 600] {
            // Pin 100.00% of pixels outside BackPatch directly against the
            // exact pre-branch `open 800x600` SHA-256 baseline.
            assert_eq!(
                digest(&without_patch),
                "3829280fd7136e5abc9d561e63fdec9aa38a28dec6ae0b3a72cbd3434b898f1a"
            );
        }
    }
}

#[test]
fn interests_and_gifts_panels_render_native_art_and_paginate() {
    let Some(root) = common::install_root() else {
        return;
    };
    let opened = common::open(&root, common::GOTH_HOUSE);
    // Verify all 19 Interest*.bmp (1600..=1618), InterestBars (1619),
    // InterestBarsBackground (1620), and Inventory{Gift,Magic,Other}Sort (4112..=4114)
    // loaded cleanly via hud::OPTIONAL_RESOURCES.
    for id in hud::id::INTEREST_ICONS {
        assert!(
            opened.art.image(id).is_some(),
            "missing native Interest bitmap {id}"
        );
    }
    assert!(opened.art.image(hud::id::INTEREST_BARS).is_some());
    assert!(opened.art.image(hud::id::INTEREST_BARS_BG).is_some());
    assert!(opened.art.image(hud::id::INVENTORY_GIFT_SORT).is_some());
    assert!(opened.art.image(hud::id::INVENTORY_MAGIC_SORT).is_some());
    assert!(opened.art.image(hud::id::INVENTORY_OTHER_SORT).is_some());

    // 1. Panel::Interests on adult Sim (Mortimer): 15 adult interests paginated
    //    across pages with non-blank native icons & bars.
    let mut snap = opened.snapshot.clone();
    if let Some(r) = snap.residents.first_mut() {
        r.interests = [
            700, 500, 800, 300, 600, 900, 400, 200, 700, 500, 800, 300, 1000, 600, 500, 400, 800,
            700, 900,
        ];
    }
    let mut ctrl = Controls {
        expanded: true,
        panel: Some(Panel::Interests),
        ..Controls::default()
    };
    let page0_items = hud::layout([800, 600], &snap, &ctrl);
    let interest_count_p0 = page0_items
        .iter()
        .filter(|i| matches!(i.kind, hud::Kind::Interest { .. }))
        .count();
    assert_eq!(interest_count_p0, 6, "2 cols x 3 rows on 800x600 page 0");
    // Native `0x519251`: 1024x768 shows 3 columns x 3 rows = 9 adult interests on page 0,
    // while intermediate width 880x600 keeps 2 columns (6 items) strictly left of the next arrow.
    let page0_880 = hud::layout([880, 600], &snap, &ctrl);
    let interest_880: Vec<Rect> = page0_880
        .iter()
        .filter(|i| matches!(i.kind, hud::Kind::Interest { .. }))
        .map(|i| i.rect)
        .collect();
    let next_880 = page0_880
        .iter()
        .find(|i| i.command == Some(Command::InterestsPage(1)))
        .expect("next arrow at 880x600")
        .rect;
    assert_eq!(interest_880.len(), 6, "2 cols x 3 rows at 880x600");
    assert!(
        interest_880.iter().all(|r| r.x + r.width <= next_880.x),
        "880x600 interest cells must stay strictly left of next arrow"
    );
    let page0_1024 = hud::layout([1024, 768], &snap, &ctrl);
    let interest_count_1024 = page0_1024
        .iter()
        .filter(|i| matches!(i.kind, hud::Kind::Interest { .. }))
        .count();
    assert_eq!(interest_count_1024, 9, "3 cols x 3 rows on 1024x768 page 0");
    assert!(
        page0_items
            .iter()
            .any(|i| i.command == Some(Command::InterestsPage(1))),
        "adult 15 interests must show next-page arrow at 800x600"
    );

    // Draw Panel::Interests and compare subpanel strip pixels against an empty strip.
    let mut blank_ctrl = ctrl;
    blank_ctrl.panel = None;
    let blank_items = hud::layout([800, 600], &snap, &blank_ctrl);
    let mut blank_frame = Frame::new(800, 600, [0, 0, 0]);
    hud::draw(
        &mut blank_frame,
        &opened.art,
        &opened.faces,
        &blank_items,
        &blank_ctrl,
    );

    let mut interests_frame = Frame::new(800, 600, [0, 0, 0]);
    hud::draw(
        &mut interests_frame,
        &opened.art,
        &opened.faces,
        &page0_items,
        &ctrl,
    );
    if let Some(dir) = std::env::var_os("SIMS_WAVE1_SHOTS_DIR").map(std::path::PathBuf::from) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join("interests_adult_800x600.png"),
            interests_frame.png(),
        );
    }
    let mut diff_px = 0usize;
    for y in 500..600 {
        for x in 521..800 {
            if interests_frame.pixel(x, y) != blank_frame.pixel(x, y) {
                diff_px += 1;
            }
        }
    }
    assert!(
        diff_px > 4_000,
        "Panel::Interests must draw native icons/bars (diff_px={diff_px})"
    );

    // Page forward -> PAGE_BACK appears and distinct icons render.
    ctrl.apply(&Command::InterestsPage(1));
    let page1_items = hud::layout([800, 600], &snap, &ctrl);
    assert!(
        page1_items
            .iter()
            .any(|i| i.command == Some(Command::InterestsPage(0)))
    );

    // Child Sim shows child-specific interest icons (Toys 1608, Aliens 1609, Pets 1610, School 1611).
    let mut child_snap = snap.clone();
    if let Some(r) = child_snap.residents.first_mut() {
        r.school_age = true;
    }
    ctrl.interests_page = 0;
    let child_items = hud::layout([800, 600], &child_snap, &ctrl);
    let child_icons: Vec<i32> = child_items
        .iter()
        .filter_map(|i| match i.kind {
            hud::Kind::Interest { icon_id, .. } => Some(icon_id),
            _ => None,
        })
        .collect();
    assert!(child_icons.contains(&1608) && child_icons.contains(&1611));
    if let Some(dir) = std::env::var_os("SIMS_WAVE1_SHOTS_DIR").map(std::path::PathBuf::from) {
        let mut child_frame = Frame::new(800, 600, [0, 0, 0]);
        hud::draw(
            &mut child_frame,
            &opened.art,
            &opened.faces,
            &child_items,
            &ctrl,
        );
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("interests_child_800x600.png"), child_frame.png());
    }

    // 2. Panel::Gifts / Inventory with Gifts (kind 0), Magic (kind 7), and Other (kind 8) tokens.
    snap.inventory = vec![
        sims_ui::household::InventoryEntry {
            kind: 0,
            guid: 0x26bf_bb29,
            count: 2,
            name: "Red Roses".into(),
        },
        sims_ui::household::InventoryEntry {
            kind: 7,
            guid: 0x1234_5678,
            count: 5,
            name: "Toad Sweat".into(),
        },
        sims_ui::household::InventoryEntry {
            kind: 8,
            guid: 0x8765_4321,
            count: 1,
            name: "Pet Treat".into(),
        },
    ];
    let mut gifts_ctrl = Controls {
        expanded: true,
        panel: Some(Panel::Gifts),
        ..Controls::default()
    };
    let gift_items = hud::layout([800, 600], &snap, &gifts_ctrl);
    assert!(gift_items.iter().any(|i| matches!(
        &i.kind,
        hud::Kind::InventoryItem { entry, .. } if entry.name == "Red Roses"
    )));
    assert!(gift_items.iter().any(|i| i.command
        == Some(Command::ShowInventory(
            sims_ui::command::InventoryTab::Magic
        ))));

    let mut thumbs = sims_ui::thumbnail::Thumbnails::default();
    let _ = thumbs.read(&opened.index, &opened.catalog, 0x26bf_bb29);
    let mut gifts_frame = Frame::new(800, 600, [0, 0, 0]);
    hud::draw_with_thumbnails(
        &mut gifts_frame,
        &opened.art,
        &opened.faces,
        Some(&thumbs),
        &gift_items,
        &gifts_ctrl,
    );
    if let Some(dir) = std::env::var_os("SIMS_WAVE1_SHOTS_DIR").map(std::path::PathBuf::from) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("gifts_800x600.png"), gifts_frame.png());
    }
    let mut gift_diff_px = 0usize;
    for y in 500..600 {
        for x in 521..800 {
            if gifts_frame.pixel(x, y) != blank_frame.pixel(x, y) {
                gift_diff_px += 1;
            }
        }
    }
    assert!(
        gift_diff_px > 2_000,
        "Panel::Gifts must draw native sort tabs and token slots (gift_diff_px={gift_diff_px})"
    );

    // Switch to Magic tab -> shows Toad Sweat, hides Red Roses.
    gifts_ctrl.apply(&Command::ShowInventory(
        sims_ui::command::InventoryTab::Magic,
    ));
    let magic_items = hud::layout([800, 600], &snap, &gifts_ctrl);
    assert!(magic_items.iter().any(|i| matches!(
        &i.kind,
        hud::Kind::InventoryItem { entry, .. } if entry.name == "Toad Sweat"
    )));
    assert!(!magic_items.iter().any(|i| matches!(
        &i.kind,
        hud::Kind::InventoryItem { entry, .. } if entry.name == "Red Roses"
    )));

    // Native `0x51e402`: Other tab enumerates ONLY kind 8 tokens, excluding
    // internal bookkeeping tokens (kinds 1 and 3) populated via `Snapshot::with_inventory`.
    let mut mixed_world = opened.world.clone();
    let sel_guid = snap.selected_guid.unwrap();
    let nb_id = mixed_world
        .neighbors
        .slots
        .iter()
        .flatten()
        .find(|n| n.guid == sel_guid)
        .expect("selected Sim has neighbor record")
        .id;
    mixed_world
        .neighbors
        .inventories
        .retain(|inv| inv.neighbor != nb_id);
    let mut raw_items = vec![
        sims_sim::neighbors::Item {
            kind: 1,
            guid: 0xAAAA_0001,
            count: 1,
        },
        sims_sim::neighbors::Item {
            kind: 3,
            guid: 0xAAAA_0003,
            count: 2,
        },
        sims_sim::neighbors::Item {
            kind: 8,
            guid: 0xAAAA_0008,
            count: 1,
        },
    ];
    for i in 0..10u32 {
        raw_items.push(sims_sim::neighbors::Item {
            kind: 0,
            guid: 0xBBBB_0000 + i,
            count: 1,
        });
    }
    mixed_world
        .neighbors
        .inventories
        .push(sims_sim::neighbors::Inventory {
            neighbor: nb_id,
            items: raw_items,
        });
    let mixed_snap = snap
        .clone()
        .with_inventory(&mixed_world, |g| format!("Item {g:X}"));
    gifts_ctrl.apply(&Command::ShowInventory(
        sims_ui::command::InventoryTab::Other,
    ));
    let other_items = hud::layout([800, 600], &mixed_snap, &gifts_ctrl);
    let other_guids: Vec<u32> = other_items
        .iter()
        .filter_map(|i| match &i.kind {
            hud::Kind::InventoryItem { entry, .. } => Some(entry.guid),
            _ => None,
        })
        .collect();
    assert_eq!(
        other_guids,
        vec![0xAAAA_0008],
        "Other tab must show only kind 8 and exclude internal kinds 1 and 3"
    );

    // Native `0x51927e` / `0x519262` / `0x51e355`: Gifts shows 5 items/page at
    // 800x600 and 9 items/page at 1024x768.
    gifts_ctrl.apply(&Command::ShowInventory(
        sims_ui::command::InventoryTab::Gifts,
    ));
    let gifts_800 = hud::layout([800, 600], &mixed_snap, &gifts_ctrl)
        .iter()
        .filter(|i| matches!(i.kind, hud::Kind::InventoryItem { .. }))
        .count();
    let gifts_1024 = hud::layout([1024, 768], &mixed_snap, &gifts_ctrl)
        .iter()
        .filter(|i| matches!(i.kind, hud::Kind::InventoryItem { .. }))
        .count();
    assert_eq!(gifts_800, 5, "Gifts must fit 5 items/page at 800x600");
    assert_eq!(gifts_1024, 9, "Gifts must fit 9 items/page at 1024x768");
}

/// 0x403d64 takes the titles it shows from each career's second string
/// table, `STR#` ID + 1000, which spells some differently from the first,
/// and leaves the Livin' Large careers' woman's titles empty.
#[test]
fn career_titles_come_from_the_second_string_table() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to read work.iff's career titles");
        return;
    };
    let install = sims_content::Install::detect(&root).unwrap();
    let index = sims_content::Index::build(&install).unwrap();
    let careers = household::Careers::load(&index).unwrap();
    // Musician (12) level 6, Slacker (13) level 7, Paranormal (14) level 5,
    // Journalism (15) level 3.
    for (career, level, title) in [
        (12, 6, "Back-up Musician"),
        (13, 7, "Free Lance Photographer"),
        (14, 5, "Douser"),
        (15, 3, "Papparazzi"),
    ] {
        assert_eq!(careers.title(career, level, false), Some(title));
        // The second table's woman's title is empty: the man's shows.
        assert_eq!(careers.title(career, level, true), Some(title));
    }
    // Where both tables agree, nothing changes.
    assert_eq!(careers.title(2, 0, false), Some("Waiter"));
    assert_eq!(careers.title(2, 0, true), Some("Waitress"));
}
