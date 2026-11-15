//! The control panel drawn for the Goths' house. Needs the game: set
//! `SIMS_INSTALL` to the install folder.

mod common;

use sha2::{Digest, Sha256};
use sims_ui::command::{Command, Panel};
use sims_ui::frame::Frame;
use sims_ui::household;
use sims_ui::hud::{self, Controls};

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
    // The digests pin the drawing; docs/decisions.md says how it compares
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
            "4053f0e9e8e82794d8237278d08959676d006a0ee7021d53c68bbb959f96b13b",
        ),
        (
            "open 1280x720",
            "607184ac1852a01d42b5f559806a6c07edfbac5edfbe2cd971dbbce708b43fd4",
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
    // screen (see docs/decisions.md).
    assert_eq!(
        digest(&frame),
        "2414e88dac8a620c6185b589d9bfa479073f314686f4c607bc6cbd09379ce669"
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
    // screen (see docs/decisions.md).
    assert_eq!(
        draw(&opened.snapshot),
        "3332eada844f300442b413d9994c5aab8b198314ec41292cd584e9261b6fed64"
    );
    // A level up the ladder, "Field Researcher": the next level, "Science
    // Teacher", needs a point of Charisma and three of Logic, and he has
    // two of Logic.
    let mut snapshot = opened.snapshot.clone();
    snapshot.residents[0].job = [9, 2];
    snapshot.residents[0].skills = [0, 0, 0, 0, 200, 0];
    let snapshot = snapshot.with_careers(&opened.careers);
    assert_eq!(snapshot.residents[0].needs, [0, 0, 100, 0, 300, 0]);
    assert_eq!(
        draw(&snapshot),
        "f31ded2b92e5ae9d45e42de2f78142b1f9ed22423feb4b6d702bb1cce3ba6e01"
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
            )
        })
        .collect();
    assert_eq!(
        relations,
        [
            (33, 100, 0, true, true, false, true),
            (32, 75, 0, true, false, false, true),
        ]
    );
    let controls = Controls {
        expanded: true,
        panel: Panel::Relationships,
        ..Controls::default()
    };
    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    let items = hud::layout([800, 600], &opened.snapshot, &controls);
    hud::draw(&mut frame, &opened.art, &opened.faces, &items, &controls);
    // Pins the drawing; it has not been compared with the original's
    // screen (see docs/decisions.md).
    assert_eq!(
        digest(&frame),
        "4b764150a46580163fb484f70c5eac2db568672649d2e2bf8d05af09c2667c4c"
    );
}
