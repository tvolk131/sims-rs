//! Asset-backed tests for the Options control panel (`cWinOptions`).
//! Needs the game: set `SIMS_INSTALL` to the install folder.

mod common;

use sims_content::Install;
use sims_content::ui_graphics::{US_ENGLISH, UiGraphics};
use sims_ui::art::Art;
use sims_ui::command::{
    Command, DetailLevel, Mode, OptionsAction, OptionsHelp, OptionsRadio, OptionsTab,
    OptionsToggle, VolumeChannel,
};
use sims_ui::frame::Frame;
use sims_ui::household::{Faces, Snapshot};
use sims_ui::hud::{self, Controls, Kind};
use sims_ui::options::{self, LotVisibility, OptionsTexts};

#[test]
fn options_art_and_texts_load_and_lay_out_all_three_subpanels() {
    let Some(root) = common::install_root() else {
        eprintln!("skipping: set SIMS_INSTALL to test Options panel assets");
        return;
    };
    let install = Install::detect(&root).unwrap();
    let ui = UiGraphics::open(&install).unwrap();
    let ids: Vec<i32> = hud::RESOURCES
        .iter()
        .chain(&options::RESOURCES)
        .copied()
        .collect();
    let mut art = Art::load(&ui, US_ENGLISH, &ids).unwrap();
    assert!(art.missing.is_empty(), "{:?}", art.missing);
    for id in options::HELP_RESOURCES {
        art.load_optional(&ui, US_ENGLISH, id);
    }
    assert!(
        art.image(options::help_bitmap(OptionsHelp::Shadows))
            .is_some()
    );

    let texts = OptionsTexts::load(&install).unwrap();
    assert_eq!(texts.nbhd_pos, [88, 6]);
    assert_eq!(texts.save_pos, [29, 6]);
    assert_eq!(texts.quit_pos, [159, 6]);
    assert_eq!(texts.graphics_pos, [29, 58]);
    assert_eq!(texts.sound_pos, [89, 58]);
    assert_eq!(texts.play_pos, [151, 58]);
    assert_eq!(texts.slider_width, 130);
    assert_eq!(texts.sound_labels, ["F/X", "Music", "Vox"]);
    let (shadows_title, shadows_body) = texts.help_text(OptionsHelp::Shadows);
    assert!(!shadows_title.is_empty() && !shadows_body.is_empty());
    assert!(!texts.action_tooltips[1].is_empty());

    let snapshot = Snapshot::read(&sims_sim::World::empty(), &[]);
    let mut controls = Controls {
        mode: Mode::Options,
        expanded: true,
        ..Controls::default()
    };

    // Initial state has no subpanel open (ShowSubpanel(0) at 0x514086).
    let items = hud::layout_with_options([800, 600], &snapshot, &controls, &texts, Some(&art));
    assert_eq!(
        hud::press(&items, [220 + 29 + 5, 500 + 6 + 5]),
        Some(Command::OptionsAction(OptionsAction::Save))
    );
    assert_eq!(
        hud::press(&items, [220 + 88 + 5, 500 + 6 + 5]),
        Some(Command::OptionsAction(OptionsAction::Neighborhood))
    );
    assert_eq!(
        hud::press(&items, [220 + 159 + 5, 500 + 6 + 5]),
        Some(Command::OptionsAction(OptionsAction::Quit))
    );
    assert_eq!(
        hud::press(&items, [220 + 29 + 5, 500 + 58 + 5]),
        Some(Command::OpenOptionsTab(OptionsTab::Graphics))
    );
    assert_eq!(
        hud::press(&items, [220 + 89 + 5, 500 + 58 + 5]),
        Some(Command::OpenOptionsTab(OptionsTab::Sound))
    );
    assert_eq!(
        hud::press(&items, [220 + 151 + 5, 500 + 58 + 5]),
        Some(Command::OpenOptionsTab(OptionsTab::Play))
    );

    // Graphics subpanel: Shadows/Lighting/Interface Effects checkboxes & 2 detail radio groups.
    controls.apply(&Command::OpenOptionsTab(OptionsTab::Graphics));
    let items = hud::layout_with_options([800, 600], &snapshot, &controls, &texts, Some(&art));
    assert_eq!(
        hud::press(&items, [220 + 245 + 5, 500 + 27 + 5]),
        Some(Command::ToggleOption(OptionsToggle::Shadows))
    );
    assert_eq!(
        hud::press(&items, [220 + 485 + 5, 500 + 30 + 5]),
        Some(Command::SetDetail {
            radio: OptionsRadio::TerrainDetail,
            level: DetailLevel::Low,
        })
    );
    assert_eq!(
        hud::press(&items, [220 + 515 + 5, 500 + 56 + 5]),
        Some(Command::SetDetail {
            radio: OptionsRadio::CharacterDetail,
            level: DetailLevel::Med,
        })
    );
    let mut frame = Frame::new(800, 600, [0; 3]);
    hud::draw(&mut frame, &art, &Faces::default(), &items, &controls);

    // Sound subpanel: F/X, Music, Vox 11-step sliders.
    controls.apply(&Command::OpenOptionsTab(OptionsTab::Sound));
    let items = hud::layout_with_options([800, 600], &snapshot, &controls, &texts, Some(&art));
    for channel in [VolumeChannel::Fx, VolumeChannel::Music, VolumeChannel::Vox] {
        let rect = hud::slider_rect(&items, channel).expect("sound slider laid out");
        assert_eq!(rect.width, 130);
        assert_eq!(rect.height, 14);
        assert_eq!(
            hud::press(&items, [rect.x + 1, rect.y + 7]),
            Some(Command::SetVolume { channel, step: 0 })
        );
        assert_eq!(
            hud::press(&items, [rect.x + rect.width - 1, rect.y + 7]),
            Some(Command::SetVolume { channel, step: 10 })
        );
    }
    hud::draw(&mut frame, &art, &Faces::default(), &items, &controls);

    // Play subpanel: 8 checkboxes + Reset Tutorial on residential lot;
    // right column sits 2 px below left column with English 17 px 9 pt font
    // (`0x51584c`, `0x515b44`, `0x515e14`).
    controls.apply(&Command::OpenOptionsTab(OptionsTab::Play));
    let items = hud::layout_with_options([800, 600], &snapshot, &controls, &texts, Some(&art));
    let auto_center_cb = items
        .iter()
        .find(|i| i.command == Some(Command::ToggleOption(OptionsToggle::AutoCentering)))
        .unwrap();
    let quick_tips_cb = items
        .iter()
        .find(|i| i.command == Some(Command::ToggleOption(OptionsToggle::QuickTips)))
        .unwrap();
    assert_eq!(quick_tips_cb.rect.y - auto_center_cb.rect.y, 2);
    let auto_center_lbl = items
        .iter()
        .find(|i| i.command == Some(Command::OptionsHelp(OptionsHelp::AutoCentering)))
        .unwrap();
    let quick_tips_lbl = items
        .iter()
        .find(|i| i.command == Some(Command::OptionsHelp(OptionsHelp::QuickTips)))
        .unwrap();
    assert_eq!(quick_tips_lbl.rect.y - auto_center_lbl.rect.y, 2);
    assert!(
        items
            .iter()
            .any(|i| i.command == Some(Command::ToggleOption(OptionsToggle::FreeWill)))
    );
    assert!(
        items
            .iter()
            .any(|i| i.command == Some(Command::ToggleOption(OptionsToggle::SimInBackground)))
    );
    assert!(
        items
            .iter()
            .any(|i| i.command == Some(Command::ToggleOption(OptionsToggle::ExportHtml)))
    );
    assert!(
        items
            .iter()
            .any(|i| i.command == Some(Command::OptionsAction(OptionsAction::TutorialReset)))
    );
    hud::draw(&mut frame, &art, &Faces::default(), &items, &controls);

    // Clicking an Options label opens its help balloon; Quick Tips enables hover tooltips.
    controls.apply(&Command::OptionsHelp(OptionsHelp::FreeWill));
    controls.apply(&Command::ToggleOption(OptionsToggle::QuickTips));
    controls.pointer = Some([220 + 29 + 5, 500 + 6 + 5]);
    let items = hud::layout_with_options([800, 600], &snapshot, &controls, &texts, Some(&art));
    assert!(
        items
            .iter()
            .any(|i| matches!(i.kind, Kind::OptionsBalloon(_)))
    );
    assert!(
        items
            .iter()
            .any(|i| matches!(i.kind, Kind::OptionsTooltip(_)))
    );
    hud::draw(&mut frame, &art, &Faces::default(), &items, &controls);

    // Per-lot visibility rules (`0x4f2490`, `0x5177aa..0x517833`):
    // Downtown hides Save (visit), Free Will, and Reset Tutorial.
    let downtown = LotVisibility::for_lot(25, Some(2), true, false);
    assert!(!downtown.can_save && !downtown.show_free_will && !downtown.show_reset_tutorial);
    let items = hud::layout_with_options_lot(
        [800, 600],
        &snapshot,
        &controls,
        &texts,
        downtown,
        Some(&art),
    );
    assert!(
        !items
            .iter()
            .any(|i| i.command == Some(Command::OptionsAction(OptionsAction::Save)))
    );
    assert!(
        !items
            .iter()
            .any(|i| i.command == Some(Command::ToggleOption(OptionsToggle::FreeWill)))
    );
    assert!(
        !items
            .iter()
            .any(|i| i.command == Some(Command::OptionsHelp(OptionsHelp::FreeWill)))
    );
    assert!(
        !items
            .iter()
            .any(|i| i.command == Some(Command::OptionsAction(OptionsAction::TutorialReset)))
    );

    // Community lot (40..=49) hides Reset Tutorial & Save (visit) but shows Free Will.
    let community = LotVisibility::for_lot(42, Some(2), true, false);
    assert!(!community.can_save && community.show_free_will && !community.show_reset_tutorial);
    // Edit mode on Magic Town (93..=98) enables Save while still hiding Free Will.
    let magic_edit = LotVisibility::for_lot(95, Some(2), false, true);
    assert!(magic_edit.can_save && !magic_edit.show_free_will && !magic_edit.show_reset_tutorial);

    // Installed `UserData/Options.iff` (`Optn` 128, `0x4e6505..0x4e685e`) loads
    // sound volume steps `[8, 8, 8]`, preserves exact native byte mapping
    // (`[15]` anti-alias, `[27]` interface_effects), and round-trips byte-for-byte.
    let opt_iff_bytes = std::fs::read(install.file("UserData/Options.iff").unwrap()).unwrap();
    let opt_iff = sims_formats::iff::Iff::parse(opt_iff_bytes).unwrap();
    let raw_optn = opt_iff
        .data(opt_iff.get(b"Optn", options::OPTN_CHUNK_ID).unwrap())
        .to_vec();
    let loaded = options::OptionsState::load_install(&install).unwrap();
    assert_eq!(loaded.to_optn(), raw_optn);
    assert_eq!(loaded.sound.steps(), [8, 8, 8]);
    assert!(loaded.graphics.shadows);
    assert!(loaded.graphics.lighting);
    assert!(loaded.graphics.interface_effects);
    assert!(loaded.play.free_will);
    assert!(loaded.play.edge_scrolling);
    assert!(!loaded.play.sim_in_background);
    let mut modified = loaded;
    modified.sound.set(VolumeChannel::Fx, 3);
    modified.sound.set(VolumeChannel::Music, 7);
    modified.sound.set(VolumeChannel::Vox, 2);
    modified.graphics.anti_alias = false;
    modified.graphics.interface_effects = false;
    modified.graphics.shadows = false;
    modified.graphics.terrain_detail = DetailLevel::Low;
    modified.play.sim_in_background = true;
    modified.play.quick_tips = false;
    let encoded = modified.to_optn();
    assert_eq!(encoded[15], 0); // anti_alias at byte 15 (`0x4e6556..0x4e6579`)
    assert_eq!(encoded[27], 0); // interface_effects at byte 27 (`0x4e6753..0x4e676d`)
    let temp_dir =
        std::env::temp_dir().join(format!("sims-ui-options-optn-{}", std::process::id()));
    let temp_path = temp_dir.join("Options.iff");
    modified.save_iff(&temp_path).unwrap();
    let reloaded = options::OptionsState::load_iff(&temp_path).unwrap();
    assert_eq!(reloaded.sound.steps(), [3, 7, 2]);
    assert!(!reloaded.graphics.anti_alias);
    assert!(!reloaded.graphics.interface_effects);
    assert!(!reloaded.graphics.shadows);
    assert_eq!(reloaded.graphics.terrain_detail, DetailLevel::Low);
    assert!(reloaded.play.sim_in_background);
    assert!(!reloaded.play.quick_tips);
    let _ = std::fs::remove_dir_all(&temp_dir);

    // Older `Optn` versions (1..=3) default missing v3/v4 flags to `true`
    // (`0x4e650f..0x4e6513`, `0x4e6814..0x4e681a`, `0x4e6842`).
    for ver in [1u16, 2, 3] {
        let mut legacy = raw_optn.clone();
        legacy[12..14].copy_from_slice(&ver.to_le_bytes());
        let len = match ver {
            1 => 28,
            2 => 32,
            _ => 35,
        };
        if ver == 3 {
            legacy[32] = 0;
            legacy[33] = 0;
            legacy[34] = 0;
        }
        let parsed = options::OptionsState::from_optn(&legacy[..len]).unwrap();
        if ver <= 2 {
            assert!(parsed.play.quick_tips && parsed.play.auto_snapshot && parsed.play.live_pip);
        } else {
            assert!(!parsed.play.quick_tips && !parsed.play.auto_snapshot && !parsed.play.live_pip);
        }
        assert!(parsed.play.export_html);
    }

    // Narrow windows (500x480, 600x480, 640x480) compactly lay out all three
    // Options subpanels and help balloons so no interactive control, header, or
    // help balloon clips past the window edge, no same-row controls overlap,
    // every rendered label fits inside its own bounding rect, and help balloons
    // draw above the cluster (`0x514008..0x51401b`). Wide windows (1024x768)
    // anchor the help balloon to the right edge (`0x5100b2..0x5100c7`: x = 465).
    for size in [
        [500, 480],
        [600, 480],
        [640, 480],
        [764, 480],
        [790, 480],
        [799, 480],
        [800, 600],
        [1024, 768],
    ] {
        let mut narrow = controls;
        narrow.help = None;
        for tab in [OptionsTab::Graphics, OptionsTab::Sound, OptionsTab::Play] {
            narrow.apply(&Command::OpenOptionsTab(tab));
            narrow.options.help = None;
            let items = hud::layout_with_options(size, &snapshot, &narrow, &texts, Some(&art));
            let interactive: Vec<_> = items.iter().filter(|i| i.command.is_some()).collect();
            for item in &items {
                if let Kind::Text { text, size: pt, .. } = &item.kind
                    && let Some(font) = art.font(*pt)
                    && item.rect.x >= 220
                {
                    assert!(
                        font.width(text) <= item.rect.width,
                        "text {text:?} (w={}) overflows rect {:?} at {size:?} in {tab:?}",
                        font.width(text),
                        item.rect
                    );
                }
            }
            for item in &interactive {
                assert!(
                    item.rect.x >= 0 && item.rect.x + item.rect.width <= size[0],
                    "control {:?} clipped at {size:?} in {tab:?}: {:?}",
                    item.command,
                    item.rect
                );
            }
            // Clicking the center of every interactive control dispatches that
            // exact control, and no two controls on the same row overlap
            // horizontally (native 21 px checkboxes step by 20 px vertically).
            for item in &interactive {
                let cx = item.rect.x + item.rect.width / 2;
                let cy = item.rect.y + item.rect.height / 2;
                let got = hud::press(&items, [cx, cy]);
                if let Some(Command::SetVolume { channel, .. }) = item.command {
                    assert!(
                        matches!(got, Some(Command::SetVolume { channel: c, .. }) if c == channel),
                        "center of slider {channel:?} ({:?}) hit {got:?} at {size:?}",
                        item.rect
                    );
                } else {
                    assert_eq!(
                        got, item.command,
                        "center of {:?} ({:?}) hit wrong control at {size:?} in {tab:?}",
                        item.command, item.rect
                    );
                }
            }
            for (i, a) in interactive.iter().enumerate() {
                for b in &interactive[i + 1..] {
                    if a.rect.y.abs_diff(b.rect.y) < 10 {
                        let overlap_x = a.rect.x < b.rect.x + b.rect.width
                            && b.rect.x < a.rect.x + a.rect.width;
                        assert!(
                            !overlap_x,
                            "same-row controls {:?} ({:?}) and {:?} ({:?}) overlap horizontally at {size:?} in {tab:?}",
                            a.command, a.rect, b.command, b.rect
                        );
                    }
                }
            }
            // Help balloon sits AFTER the cluster in draw order (`0x514008..0x51401b`),
            // fits completely inside [0..size[0]], right-anchors at `size[0] - width`
            // (`0x5100b2..0x5100c7`), and wraps every line inside its width.
            narrow.options.help = Some(OptionsHelp::FreeWill);
            let with_help = hud::layout_with_options(size, &snapshot, &narrow, &texts, Some(&art));
            let cluster_idx = with_help
                .iter()
                .position(|i| i.kind == Kind::Image(hud::id::CLUSTER))
                .expect("cluster present");
            let (balloon_idx, balloon) = with_help
                .iter()
                .enumerate()
                .find_map(|(idx, i)| match &i.kind {
                    Kind::OptionsBalloon(b) => Some((idx, b)),
                    _ => None,
                })
                .expect("OptionsBalloon present");
            assert!(
                balloon_idx > cluster_idx,
                "OptionsBalloon must draw after cluster at {size:?}"
            );
            assert!(
                balloon.rect.x >= 0 && balloon.rect.x + balloon.rect.width <= size[0],
                "balloon {:?} clipped at {size:?}",
                balloon.rect
            );
            if size[0] == 1024 {
                assert_eq!(balloon.rect.x, 465);
            }
            let body_font = art.font(9).unwrap();
            for line in &balloon.lines {
                assert!(
                    balloon.rect.x + balloon.text_left + 10 + body_font.width(line)
                        <= balloon.rect.x + balloon.rect.width,
                    "balloon line {line:?} overflows {:?} at {size:?}",
                    balloon.rect
                );
            }
            if tab == OptionsTab::Graphics {
                let radios = items
                    .iter()
                    .filter(|i| matches!(i.command, Some(Command::SetDetail { .. })))
                    .count();
                assert_eq!(radios, 6);
                let help_labels = items
                    .iter()
                    .filter(|i| matches!(i.command, Some(Command::OptionsHelp(_))))
                    .count();
                assert_eq!(
                    help_labels, 5,
                    "all 5 Graphics setting labels must remain visible at {size:?}"
                );
                if size[0] >= 800 {
                    // `0x514efa..0x514f0a` + `0x537b3c..0x537b96`: 93 px span
                    // divided into thirds gives `Med` `[728, 759)`.
                    let med = items
                        .iter()
                        .find(|i| matches!(&i.kind, Kind::Text { text, .. } if text == "Med"))
                        .unwrap();
                    assert_eq!((med.rect.x, med.rect.x + med.rect.width), (728, 759));
                }
            } else if tab == OptionsTab::Sound {
                let sliders = items
                    .iter()
                    .filter(|i| matches!(i.kind, Kind::Slider { .. }))
                    .count();
                assert_eq!(sliders, 3);
            } else if tab == OptionsTab::Play {
                let checkboxes = items
                    .iter()
                    .filter(|i| matches!(i.command, Some(Command::ToggleOption(_))))
                    .count();
                assert_eq!(checkboxes, 8);
                let help_labels = items
                    .iter()
                    .filter(|i| matches!(i.command, Some(Command::OptionsHelp(_))))
                    .count();
                assert_eq!(
                    help_labels, 8,
                    "all 8 Play setting labels must remain visible at {size:?}"
                );
            }
            if size[0] < 800 {
                // Zero pairwise overlap (even 1-px vertical) between any two
                // Options strip controls (`x >= 220`) in narrow compact mode.
                let strip_controls: Vec<_> = interactive
                    .iter()
                    .copied()
                    .filter(|i| i.rect.x >= 220)
                    .collect();
                for (i, a) in strip_controls.iter().enumerate() {
                    for b in &strip_controls[i + 1..] {
                        let overlap_x = a.rect.x < b.rect.x + b.rect.width
                            && b.rect.x < a.rect.x + a.rect.width;
                        let overlap_y = a.rect.y < b.rect.y + b.rect.height
                            && b.rect.y < a.rect.y + a.rect.height;
                        assert!(
                            !(overlap_x && overlap_y),
                            "controls {:?} ({:?}) and {:?} ({:?}) overlap at {size:?} in {tab:?}",
                            a.command,
                            a.rect,
                            b.command,
                            b.rect
                        );
                    }
                }
            }
        }
    }
}
