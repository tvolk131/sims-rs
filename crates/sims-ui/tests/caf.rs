//! Unit and live-install tests for the Create-a-Family (`cWinEditFamily`) and
//! Create-a-Sim (`cWinEditPerson`) UI screens.

mod common;

use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_ui::art::Art;
use sims_ui::caf::{
    Age, BIO_MAX, CafCommand, CafController, CafLayout, CafScreen, CafTexts, ConfirmKind,
    FamilyDraft, FocusField, Gender, MAX_MEMBERS, MemberKind, NAME_MAX, PERSONALITY_BUDGET,
    Personality, RESOURCES, SimDraft, SkinTone, Trait,
};
use sims_ui::dialog::CARET_BLINK_US;
use sims_ui::frame::Frame;

fn center(r: sims_ui::frame::Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

#[test]
fn personality_enforces_25_point_budget_and_computes_oracle_zodiac() {
    let mut p = Personality::default();
    assert_eq!(p.spent(), 0);
    assert_eq!(p.remaining(), PERSONALITY_BUDGET);
    // Initial zero-point Sim shows no zodiac sign yet.
    assert_eq!(p.zodiac(), None);

    // Oracle Sim from caf-create-family-20261121: Neat=7, Outgoing=5, Active=3,
    // Playful=4, Nice=6 (sum=25 -> Cancer = 4).
    assert_eq!(p.set(Trait::Neat, 7), 7);
    assert_eq!(p.set(Trait::Outgoing, 5), 5);
    assert_eq!(p.set(Trait::Active, 3), 3);
    assert_eq!(p.set(Trait::Playful, 4), 4);
    assert_eq!(p.set(Trait::Nice, 9), 6); // clamped by remaining budget (6)
    assert_eq!(p.spent(), 25);
    assert_eq!(p.remaining(), 0);
    assert_eq!(p.zodiac(), Some(4)); // Cancer

    // Lowering one trait returns points to the pool and allows raising another.
    assert_eq!(p.set(Trait::Neat, 4), 4);
    assert_eq!(p.remaining(), 3);
    assert_eq!(p.set(Trait::Nice, 10), 9);
    assert_eq!(p.spent(), 25);
    assert_eq!(p.remaining(), 0);

    // from_raw clamps traits above 10 and totals above 25.
    let clamped = Personality::from_raw([12, 10, 10, 10, 10]);
    assert_eq!(clamped.values(), [10, 10, 5, 0, 0]);
    assert_eq!(clamped.spent(), 25);
}

#[test]
fn full_create_family_and_create_sim_flow() {
    let mut ctrl = CafController::new(CafTexts::default());
    let size = [800, 600];
    let layout = CafLayout::compute(size[0], size[1]);

    // Initially on Family screen, Done is disabled (no name, no members).
    assert_eq!(ctrl.screen(), CafScreen::Family);
    assert!(!ctrl.family().can_finish());

    // Type family last name.
    for ch in "Oracle".chars() {
        assert!(ctrl.key_char(ch));
    }
    assert_eq!(ctrl.family().last_name, "Oracle");

    // Tab to Family Bio and type a line.
    ctrl.key_tab();
    assert_eq!(ctrl.focus(), FocusField::FamilyBio);
    for ch in "Test bio".chars() {
        assert!(ctrl.key_char(ch));
    }
    assert_eq!(ctrl.family().bio, "Test bio");

    // Click + Add Sim -> switches to EditSim screen.
    let (ax, ay) = center(layout.family_add);
    assert_eq!(
        ctrl.pointer_down(ax, ay, size),
        Some(CafCommand::OpenSimEditor { index: None })
    );
    ctrl.pointer_up();
    assert_eq!(ctrl.screen(), CafScreen::EditSim);
    ctrl.set_skin_catalog_counts(4, 6);

    // Type First Name "Alice".
    for ch in "Alice".chars() {
        assert!(ctrl.key_char(ch));
    }
    assert_eq!(ctrl.editing_sim().unwrap().first_name, "Alice");

    // Toggle Female & Medium skin, cycle head & body, rotate turntable.
    let (fx, fy) = center(layout.sim_female);
    assert!(matches!(
        ctrl.pointer_down(fx, fy, size),
        Some(CafCommand::AppearanceChanged(a)) if a.gender == Gender::Female
    ));
    let (mx, my) = center(layout.sim_medium);
    assert!(matches!(
        ctrl.pointer_down(mx, my, size),
        Some(CafCommand::AppearanceChanged(a)) if a.skin_tone == SkinTone::Medium
    ));
    let (hx, hy) = center(layout.head_next);
    assert!(matches!(
        ctrl.pointer_down(hx, hy, size),
        Some(CafCommand::AppearanceChanged(a)) if a.head_index == 1
    ));
    let (bx, by) = center(layout.body_prev);
    assert!(matches!(
        ctrl.pointer_down(bx, by, size),
        Some(CafCommand::AppearanceChanged(a)) if a.body_index == 5
    ));
    let (rx, ry) = center(layout.rotate_right);
    assert!(matches!(
        ctrl.pointer_down(rx, ry, size),
        Some(CafCommand::RotatePreview(yaw)) if (yaw - 30.0).abs() < 0.01
    ));
    ctrl.pointer_up();

    // Click Done with 0 personality points spent -> opens MorePersonality dialog.
    let (dx, dy) = center(layout.sim_done);
    assert_eq!(ctrl.pointer_down(dx, dy, size), None);
    assert_eq!(ctrl.confirm(), Some(ConfirmKind::MorePersonality));

    // Dismiss dialog via Escape, then spend all 25 points via slider clicks.
    assert_eq!(ctrl.key_escape(), None);
    assert_eq!(ctrl.confirm(), None);

    // Drag/click sliders to fill Neat=10, Outgoing=10, Active=5.
    for (trait_, frac) in [
        (Trait::Neat, 0.99),
        (Trait::Outgoing, 0.99),
        (Trait::Active, 0.45),
    ] {
        let r = layout.trait_sliders[trait_.index()];
        let sx = r.x + (f32::from(r.width as i16) * frac) as i32;
        ctrl.pointer_down(sx, r.y + r.height / 2, size);
        ctrl.pointer_up();
    }
    assert_eq!(ctrl.editing_sim().unwrap().personality.spent(), 25);

    // Now Done saves immediately and returns to Family screen.
    assert_eq!(
        ctrl.pointer_down(dx, dy, size),
        Some(CafCommand::SimSaved {
            index: 0,
            is_new: true
        })
    );
    ctrl.pointer_up();
    assert_eq!(ctrl.screen(), CafScreen::Family);
    assert_eq!(ctrl.family().members.len(), 1);
    assert!(ctrl.family().can_finish());

    // Clicking Family Done opens FinishFamily confirm; accepting emits CommitFamily.
    let (fdx, fdy) = center(layout.family_done);
    assert_eq!(ctrl.pointer_down(fdx, fdy, size), None);
    assert_eq!(ctrl.confirm(), Some(ConfirmKind::FinishFamily));
    let (px, py) = center(layout.dialog_primary);
    let committed = ctrl.pointer_down(px, py, size);
    assert!(matches!(committed, Some(CafCommand::CommitFamily(f)) if f.last_name == "Oracle"));
}

#[test]
fn edit_delete_cancel_and_eight_member_cap() {
    let mut family = FamilyDraft {
        last_name: "Test".into(),
        bio: String::new(),
        members: Vec::new(),
    };
    for i in 0..MAX_MEMBERS {
        family.members.push(SimDraft {
            first_name: format!("Sim{i}"),
            kind: if i == MAX_MEMBERS - 1 {
                MemberKind::Dog
            } else {
                MemberKind::Sim
            },
            ..SimDraft::default()
        });
    }
    let mut ctrl = CafController::with_family(CafTexts::default(), family);
    let size = [800, 600];
    let layout = CafLayout::compute(size[0], size[1]);

    // At 8 members, Add is blocked.
    assert!(!ctrl.family().can_add_member());
    let (ax, ay) = center(layout.family_add);
    assert_eq!(ctrl.pointer_down(ax, ay, size), None);
    assert_eq!(ctrl.screen(), CafScreen::Family);

    // Select slot 7 (Dog): Edit does not open human Sim editor, Delete asks confirmation.
    let (s7x, s7y) = center(layout.family_slots[7]);
    ctrl.pointer_down(s7x, s7y, size);
    assert_eq!(ctrl.selected_member(), Some(7));
    let (ex, ey) = center(layout.family_edit);
    assert_eq!(ctrl.pointer_down(ex, ey, size), None);

    let (delx, dely) = center(layout.family_delete);
    assert_eq!(ctrl.pointer_down(delx, dely, size), None);
    assert_eq!(ctrl.confirm(), Some(ConfirmKind::DeleteSim(7)));
    assert_eq!(ctrl.key_enter(), Some(CafCommand::SimDeleted(7)));
    assert_eq!(ctrl.family().members.len(), 7);

    // Family with only a Child cannot finish.
    let child_only = FamilyDraft {
        last_name: "Kids".into(),
        bio: String::new(),
        members: vec![SimDraft {
            first_name: "Tim".into(),
            appearance: sims_ui::caf::SimAppearance {
                age: Age::Child,
                ..Default::default()
            },
            ..Default::default()
        }],
    };
    assert!(!child_only.can_finish());

    // Cancel on non-empty family asks confirmation; confirming emits CancelFamily.
    let (cx, cy) = center(layout.family_cancel);
    assert_eq!(ctrl.pointer_down(cx, cy, size), None);
    assert_eq!(ctrl.confirm(), Some(ConfirmKind::CancelFamily));
    assert_eq!(ctrl.key_enter(), Some(CafCommand::CancelFamily));
}

#[test]
fn text_editing_bounds_caret_blink_and_compact_reflow() {
    let mut ctrl = CafController::new(CafTexts::default());
    for _ in 0..(NAME_MAX + 5) {
        ctrl.key_char('A');
    }
    assert_eq!(ctrl.family().last_name.chars().count(), NAME_MAX);
    assert!(ctrl.key_backspace());
    assert_eq!(ctrl.family().last_name.chars().count(), NAME_MAX - 1);

    ctrl.key_tab();
    for _ in 0..(BIO_MAX + 5) {
        ctrl.key_char('B');
    }
    assert_eq!(ctrl.family().bio.chars().count(), BIO_MAX);

    // Caret blink toggles every CARET_BLINK_US (500 ms).
    assert!(!ctrl.tick(CARET_BLINK_US - 1));
    assert!(ctrl.tick(CARET_BLINK_US));

    // Compact reflow below 800x600 draws both screens without clipping or panic.
    let art = Art::default();
    let mut small = Frame::new(640, 480, [0, 0, 0]);
    let compact_layout = CafLayout::compute(640, 480);
    assert!(compact_layout.compact);
    assert!(compact_layout.preview_rect.x + compact_layout.preview_rect.width <= 640);
    assert!(compact_layout.sim_cancel.y + compact_layout.sim_cancel.height <= 480);

    ctrl.draw(&mut small, &art, None);
    ctrl.begin_add_sim();
    ctrl.draw(&mut small, &art, None);
}

#[test]
fn live_install_loads_all_caf_resources_strings_and_draws() {
    let Some(root) = common::install_root() else {
        return;
    };
    let install = Install::detect(&root).expect("detect install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = Art::load(&ui, US_ENGLISH, &RESOURCES).expect("load CAF art");
    assert!(
        art.missing.is_empty(),
        "missing CAF art resources: {:?}",
        art.missing
    );

    let texts = CafTexts::load(&install, US_ENGLISH).expect("load CAF UIText strings");
    assert!(!texts.family_title.is_empty());
    assert!(!texts.sim_title.is_empty());
    assert!(!texts.confirm_finish_body.is_empty());
    assert!(!texts.confirm_personality_body.is_empty());
    assert_eq!(texts.zodiac_names.len(), 12);
    assert!(texts.zodiac_names.iter().all(|s| !s.is_empty()));
    assert!(texts.trait_names.iter().all(|s| !s.is_empty()));

    let mut ctrl = CafController::new(texts);
    let mut fam_frame = Frame::new(800, 600, [0, 0, 0]);
    ctrl.draw(&mut fam_frame, &art, None);
    assert!(fam_frame.rgb().iter().any(|&p| p != [0, 0, 0]));

    // Enter Create-a-Sim, set Alice's oracle traits, and render at 800x600 and 640x480.
    ctrl.begin_add_sim();
    {
        let sim = ctrl.editing_sim_mut().unwrap();
        sim.first_name = "Alice".into();
        sim.personality = Personality::from_raw([7, 5, 3, 4, 6]);
        assert_eq!(sim.personality.zodiac(), Some(4));
    }
    let mut sim_frame = Frame::new(800, 600, [0, 0, 0]);
    ctrl.draw(&mut sim_frame, &art, None);
    assert_ne!(sim_frame.rgb(), fam_frame.rgb());

    let mut compact_frame = Frame::new(640, 480, [0, 0, 0]);
    ctrl.draw(&mut compact_frame, &art, None);
    assert!(compact_frame.rgb().iter().any(|&p| p != [0, 0, 0]));
}
