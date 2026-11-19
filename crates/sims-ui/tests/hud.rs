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
        ("AM 6:30", "$2,542")
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
            "dddb9a34934307b37843f418a06a08ac6c072fd0877bbacea83ce3190b5ec338",
        ),
        (
            "open 800x600",
            "bf2b42823851f214c8673a7f05468d22bedf28cf808911288ce8a60dee588fb8",
        ),
        (
            "open 1280x720",
            "ad37fea8d004eda2bcfc8c9f2b28418eae06a22bfbab96d11853bd254dd3dee3",
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
        panel: Panel::Personality,
        ..Controls::default()
    };
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let items = hud::layout([800, 600], &opened.snapshot, &controls);
    hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
    // Pins the drawing; it has not been compared with the original's
    // screen (see docs/decisions/).
    assert_eq!(
        digest(&frame),
        "2414e88dac8a620c6185b589d9bfa479073f314686f4c607bc6cbd09379ce669"
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
        panel: Panel::Personality,
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
        "e76b2e5deafb40d16fae70da6b664f3383fdc6693d8d70a3404506d6729cb0b3"
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
        panel: Panel::Job,
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
        "2cafffacb29e6f322fbe86b6257545d9e4c417eee6afb384b18adfd03b4f3edf"
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
        "d0ad6b0746169b1864e3351448cef65a4d7d7f6b8cfd83263df7d255cf86dc6a"
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
        "49147ac862b847e2f8c06ede08f7f98f45b3fba31a8600ac9489926dc2ff1716"
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
        panel: Panel::Relationships,
        ..Controls::default()
    };
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let items = hud::layout([800, 600], &opened.snapshot, &controls);
    hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
    // Pins the drawing; it has not been compared with the original's
    // screen (see docs/decisions/).
    assert_eq!(
        digest(&frame),
        "24af5346170f6ff5a621af7725e99d1b6c03b15c8b1ae61d5b1cbd65e52c90d9"
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
    // The lot's values are what the original's simulator holds after
    // the load (+0xa0 and +0xa4, track 2's capture of update 1).
    // Furnishings weighs 15,277 over 14,660, 1.042, on the curve.
    assert_eq!(
        (house.values.objects, house.values.architecture),
        (14660, 15277)
    );
    // The house file saved the same two values, but they are worked out
    // from the lot, not read from what it saved.
    assert_eq!(
        (
            opened.world.budget.objects_value,
            opened.world.budget.architecture_value
        ),
        (14660, 15277)
    );
    let mut unsaved = opened.world.clone();
    unsaved.budget.objects_value = 0;
    unsaved.budget.architecture_value = 0;
    assert_eq!(
        house::Values::read(&unsaved, &opened.kinds, &opened.house.prices),
        house.values
    );
    assert_eq!(
        house.ratings,
        [Some(75), Some(58), Some(76), Some(100), None]
    );
    let controls = Controls {
        expanded: true,
        panel: Panel::House,
        ..Controls::default()
    };
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let items = hud::layout([800, 600], &opened.snapshot, &controls);
    hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
    // Pins the drawing; it has not been compared with the original's
    // screen (see docs/decisions/).
    assert_eq!(
        digest(&frame),
        "669ee756a9f2db4a3fdea9cfa89eb3a8a85945b7841f3e6f2d320995c1d35b4e"
    );
}
