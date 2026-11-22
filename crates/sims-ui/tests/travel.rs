use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_ui::art::Art;
use sims_ui::frame::{Frame, Rect};
use sims_ui::neighborhood::ExpansionFlags;
use sims_ui::travel::{
    Destination, LotFilter, TravelBarMode, TravelChrome, TravelCommand, TravelControl,
    TravelDialog, TravelDialogCommand, TravelDialogLayout, TravelDialogState, TravelLayout,
    TravelState, TravelStrings, TravelTransport, VacationLodging, format_status_badge, load_art,
};
use std::path::PathBuf;

fn sims_install() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.width > 0
        && a.height > 0
        && b.width > 0
        && b.height > 0
        && a.x < b.x + b.width
        && b.x < a.x + a.width
        && a.y < b.y + b.height
        && b.y < a.y + a.height
}

#[test]
fn all_four_destinations_have_disjoint_controls_across_resolutions() {
    for dest in Destination::ALL {
        let state = TravelState {
            destination: dest,
            has_importable_lot: true,
            studio_fame_half_stars: 7,
            magic_coins: 250,
            simoleons: 12_345,
            ..TravelState::default()
        };
        for (w, h) in [(1024u32, 768u32), (800, 600), (640, 480), (320, 240)] {
            let layout = TravelLayout::compute(w, h, &state);
            assert!(
                !layout.controls.is_empty(),
                "expected controls for {dest:?} at {w}x{h}"
            );
            for (i, a) in layout.controls.iter().enumerate() {
                assert!(a.rect.x >= 0 && a.rect.y >= 0);
                assert!(a.rect.x + a.rect.width <= w as i32);
                assert!(a.rect.y + a.rect.height <= h as i32);
                assert!(
                    !rects_overlap(a.rect, layout.status_badge_rect),
                    "control {:?} overlapped status badge at {w}x{h}",
                    a.control
                );
                for b in &layout.controls[i + 1..] {
                    assert!(
                        !rects_overlap(a.rect, b.rect),
                        "controls {:?} and {:?} overlapped at {w}x{h}",
                        a.control,
                        b.control
                    );
                }
            }
        }
    }
}

#[test]
fn clicking_controls_emits_expected_commands_and_respects_visit_and_expansion_gates() {
    let mut state = TravelState {
        destination: Destination::VacationIsland,
        mode: TravelBarMode::Browse,
        expansions: ExpansionFlags(ExpansionFlags::HOT_DATE | ExpansionFlags::VACATION),
        has_importable_lot: false,
        ..TravelState::default()
    };
    let layout = TravelLayout::compute(800, 600, &state);
    let mut chrome = TravelChrome::new();

    // ReturnHome works.
    let home = layout
        .controls
        .iter()
        .find(|c| c.control == TravelControl::ReturnHome)
        .expect("ReturnHome control");
    let hx = home.rect.x + home.rect.width / 2;
    let hy = home.rect.y + home.rect.height / 2;
    assert!(chrome.pointer_down(hx, hy, &layout));
    assert_eq!(
        chrome.pointer_up(hx, hy, &layout, &state),
        Some(TravelCommand::ReturnToNeighborhood)
    );

    // Vacation lodging legend toggles Hotel on and off.
    let hotel = layout
        .controls
        .iter()
        .find(|c| c.control == TravelControl::ToggleVacationLodging(VacationLodging::Hotel))
        .expect("Hotel legend control");
    let lx = hotel.rect.x + hotel.rect.width / 2;
    let ly = hotel.rect.y + hotel.rect.height / 2;
    assert!(chrome.pointer_down(lx, ly, &layout));
    assert_eq!(
        chrome.pointer_up(lx, ly, &layout, &state),
        Some(TravelCommand::SetVacationLodging(Some(
            VacationLodging::Hotel
        )))
    );
    state.vacation_lodging_filter = Some(VacationLodging::Hotel);
    assert!(chrome.pointer_down(lx, ly, &layout));
    assert_eq!(
        chrome.pointer_up(lx, ly, &layout, &state),
        Some(TravelCommand::SetVacationLodging(None))
    );

    // Missing expansion (Magic Town) is disabled.
    let magic_tab = layout
        .controls
        .iter()
        .find(|c| c.control == TravelControl::SwitchDestination(Destination::MagicTown))
        .expect("MagicTown tab");
    assert!(!magic_tab.enabled);
    assert!(!chrome.pointer_down(magic_tab.rect.x + 2, magic_tab.rect.y + 2, &layout));

    // Visit mode disables Bulldoze and destination switching.
    state.mode = TravelBarMode::Visit;
    let visit_layout = TravelLayout::compute(800, 600, &state);
    let doze = visit_layout
        .controls
        .iter()
        .find(|c| c.control == TravelControl::Bulldoze)
        .expect("Bulldoze control");
    assert!(!doze.enabled);
}

#[test]
fn status_badges_format_magicoins_fame_and_vacancy() {
    let magic = TravelState {
        destination: Destination::MagicTown,
        magic_coins: 125,
        simoleons: 4_500,
        ..TravelState::default()
    };
    assert_eq!(format_status_badge(&magic), "MC 125 | $4500");

    let studio = TravelState {
        destination: Destination::StudioTown,
        studio_fame_half_stars: 7,
        ..TravelState::default()
    };
    assert_eq!(format_status_badge(&studio), "Fame 3.5");

    let vacation_full = TravelState {
        destination: Destination::VacationIsland,
        vacation_has_no_vacancy: true,
        simoleons: 900,
        ..TravelState::default()
    };
    assert_eq!(format_status_badge(&vacation_full), "Full | $900");
}

#[test]
fn travel_confirm_dialog_enforces_fares_and_handles_taxi_shuttle_magic_hole() {
    let mut dialog = TravelDialog::new();

    // Taxi to Downtown: $50 fare, insufficient funds blocks confirm.
    let mut taxi = TravelDialogState::for_destination(Destination::Downtown, vec!["Sim A".into()]);
    assert_eq!(taxi.transport, TravelTransport::Taxi);
    assert_eq!(taxi.fare_simoleons, 50);
    taxi.household_simoleons = 25;
    assert!(!taxi.can_confirm());
    assert_eq!(dialog.key_press("Enter", &taxi), None);
    assert_eq!(
        dialog.key_press("Escape", &taxi),
        Some(TravelDialogCommand::Cancel)
    );

    // Sufficient funds enables confirm by click and Enter.
    taxi.household_simoleons = 200;
    assert!(taxi.can_confirm());
    let layout = TravelDialogLayout::compute(800, 600);
    assert!(!rects_overlap(layout.confirm_rect, layout.cancel_rect));
    let cx = layout.confirm_rect.x + layout.confirm_rect.width / 2;
    let cy = layout.confirm_rect.y + layout.confirm_rect.height / 2;
    assert!(dialog.pointer_down(cx, cy, &layout, &taxi));
    assert_eq!(
        dialog.pointer_up(cx, cy, &layout, &taxi),
        Some(TravelDialogCommand::Confirm {
            destination: Destination::Downtown,
            transport: TravelTransport::Taxi,
        })
    );

    // Magic Hole to Magic Town with MagiCoin fare.
    let mut hole = TravelDialogState::for_destination(Destination::MagicTown, vec!["Sim B".into()]);
    assert_eq!(hole.transport, TravelTransport::MagicHole);
    hole.fare_magic_coins = 30;
    hole.household_magic_coins = 30;
    assert_eq!(
        dialog.key_press("Enter", &hole),
        Some(TravelDialogCommand::Confirm {
            destination: Destination::MagicTown,
            transport: TravelTransport::MagicHole,
        })
    );

    // Empty traveler list blocks confirmation even if fare is 0.
    let empty_shuttle = TravelDialogState::for_destination(Destination::StudioTown, Vec::new());
    assert!(!empty_shuttle.can_confirm());

    // Drawing dialog with fallback art does not panic at compact size.
    let art = Art::default();
    let strings = TravelStrings::default();
    let mut frame = Frame::new(320, 240, [0, 0, 0]);
    let compact_layout = dialog.draw(&mut frame, &hole, &art, &strings);
    assert!(compact_layout.panel_rect.width > 0);
}

#[test]
fn complete_collection_loads_travel_art_and_localized_tooltips() {
    let Some(root) = sims_install() else {
        return;
    };
    let install = Install::detect(&root).expect("open Complete Collection install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let art = load_art(&ui, US_ENGLISH).expect("load travel art");
    let strings = TravelStrings::load(&install, US_ENGLISH).expect("load UIText.iff strings");

    assert!(
        art.missing.is_empty(),
        "unexpected missing travel art IDs: {:?}",
        art.missing
    );
    assert!(!strings.downtown_tips.is_empty());
    assert!(!strings.vacation_tips.is_empty());
    assert!(!strings.studio_town_tips.is_empty());
    assert!(!strings.magic_town_tips.is_empty());
    assert!(strings.filter_tips.len() >= 13);
    assert!(!strings.go_downtown_dialog.is_empty());
    assert!(!strings.go_vacation_dialog.is_empty());
    assert!(!strings.visit_magic_town.is_empty());

    let chrome = TravelChrome::new();
    let dialog = TravelDialog::new();
    for dest in Destination::ALL {
        assert!(
            strings.tooltip(dest, TravelControl::ReturnHome).is_some(),
            "missing return tooltip for {dest:?}"
        );
        let state = TravelState {
            destination: dest,
            active_filter: Some(dest.filters()[0]),
            studio_fame_half_stars: 9,
            magic_coins: 500,
            simoleons: 8_000,
            ..TravelState::default()
        };
        let mut frame = Frame::new(800, 600, [0, 0, 0]);
        let layout = chrome.draw(&mut frame, &state, &art, &strings);
        assert!(layout.controls.len() >= 8);

        let dstate = TravelDialogState::for_destination(dest, vec!["Sim".into()]);
        dialog.draw(&mut frame, &dstate, &art, &strings);
    }

    assert!(
        strings
            .tooltip(
                Destination::StudioTown,
                TravelControl::ToggleFilter(LotFilter::Music)
            )
            .is_some()
    );
}
