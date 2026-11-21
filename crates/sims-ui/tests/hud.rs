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
            // The needs keep to the right edge; the strip's background
            // fills to it.
            assert_eq!(
                hud::press(&items, [1280 - 387 + 50, 720 - 100 + 15]),
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
            "3829280fd7136e5abc9d561e63fdec9aa38a28dec6ae0b3a72cbd3434b898f1a",
        ),
        (
            "open 1280x720",
            "88a916a28ceae6f35248f44bc949d13006c5f7721e6a810c21a0754791e9c09d",
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
        "ddc5cef05deafc9f9d7b56a0ea0dc55519f7197ef960c60840c3888d69fd000f"
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
        "ab457a389a9d7411856cd4aa7c3a3f924f4bbc43d88c1a6bb40985a85afbfad8"
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
        "2933c5ccdbddd5c2e14db77125169b9ea324cc01e1cb8ad675aa50877f79413e"
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
        "95900f89b384d95110bddbed3482325f87c0f68580660f1c03c42e34ce8f41c2"
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
        "d8835a991f6c3d51fab6da35e8f4619a93b0731c272d06567516ce8595a5a4f9"
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
        "f33c3a0c53302ab56f0818d272dd97fa596a5b97054337edd1173fe238c0aea1"
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
        "2cd4d2e1ac37fb40b11fbc8971d8ed8bf7f9154c68f02ac5402ab3e454b3ed5a"
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

    // Draw Hunger balloon with preloaded object catalog thumbnails.
    let mut thumbs = Thumbnails::default();
    for guid in [0xb947_55df, 0xff9f_18e0] {
        let _ = thumbs.read(&opened.index, &opened.catalog, guid);
    }
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
    let mut frame_need = Frame::new(800, 600, [0, 0, 0]);
    hud::draw_with_thumbnails(
        &mut frame_need,
        &opened.art,
        &opened.faces,
        Some(&thumbs),
        &need_items,
        &need_controls,
    );
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
