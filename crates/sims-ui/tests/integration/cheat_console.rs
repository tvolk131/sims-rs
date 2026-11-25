use crate::common::install_dir as install;
use sims_content::Install;
use sims_content::ui_graphics::UiGraphics;
use sims_ui::art::Art;
use sims_ui::cheat_console::{
    CheatCommand, CheatConsole, CheatParseError, FeedbackKind, MAX_CHAIN_COMMANDS, MAX_INPUT_LEN,
    ROSEBUD_AMOUNT, load_art, parse_cheat_line,
};
use sims_ui::dialog::CARET_BLINK_US;
use sims_ui::frame::Frame;

#[test]
fn parses_all_classic_sims1_commands_and_semicolon_repetition() {
    assert_eq!(
        parse_cheat_line("rosebud"),
        Ok(vec![CheatCommand::AddFunds(ROSEBUD_AMOUNT)])
    );
    assert_eq!(
        parse_cheat_line("KLAPAUCIUS"),
        Ok(vec![CheatCommand::AddFunds(ROSEBUD_AMOUNT)])
    );
    assert_eq!(
        parse_cheat_line("rosebud;!;!;!"),
        Ok(vec![CheatCommand::AddFunds(ROSEBUD_AMOUNT); 4])
    );
    assert_eq!(
        parse_cheat_line("move_objects on"),
        Ok(vec![CheatCommand::MoveObjects(true)])
    );
    assert_eq!(
        parse_cheat_line("Move_Objects OFF"),
        Ok(vec![CheatCommand::MoveObjects(false)])
    );
    assert_eq!(
        parse_cheat_line("map_edit on"),
        Ok(vec![CheatCommand::MapEdit(true)])
    );
    assert_eq!(
        parse_cheat_line("map_edit off"),
        Ok(vec![CheatCommand::MapEdit(false)])
    );
    assert_eq!(
        parse_cheat_line("water_tool"),
        Ok(vec![CheatCommand::WaterTool])
    );
    assert_eq!(
        parse_cheat_line("autonomy 0"),
        Ok(vec![CheatCommand::Autonomy(0)])
    );
    assert_eq!(
        parse_cheat_line("autonomy 100"),
        Ok(vec![CheatCommand::Autonomy(100)])
    );
    assert_eq!(
        parse_cheat_line("interests"),
        Ok(vec![CheatCommand::Interests])
    );
    assert_eq!(
        parse_cheat_line("prepare_lot"),
        Ok(vec![CheatCommand::PrepareLot])
    );
    assert_eq!(
        parse_cheat_line("set_hour 0"),
        Ok(vec![CheatCommand::SetHour(0)])
    );
    assert_eq!(
        parse_cheat_line("set_hour 23"),
        Ok(vec![CheatCommand::SetHour(23)])
    );
    assert_eq!(
        parse_cheat_line("sim_speed -1000"),
        Ok(vec![CheatCommand::SimSpeed(-1000)])
    );
    assert_eq!(
        parse_cheat_line("sim_speed 1000"),
        Ok(vec![CheatCommand::SimSpeed(1000)])
    );
    assert_eq!(parse_cheat_line("help"), Ok(vec![CheatCommand::Help]));

    // Mixed semicolon chain with repetition.
    assert_eq!(
        parse_cheat_line("move_objects on; set_hour 18; rosebud; !"),
        Ok(vec![
            CheatCommand::MoveObjects(true),
            CheatCommand::SetHour(18),
            CheatCommand::AddFunds(ROSEBUD_AMOUNT),
            CheatCommand::AddFunds(ROSEBUD_AMOUNT),
        ])
    );
}

#[test]
fn rejects_invalid_arguments_out_of_range_and_unbounded_chains() {
    assert_eq!(parse_cheat_line("   "), Err(CheatParseError::Empty));
    assert_eq!(
        parse_cheat_line("!;!"),
        Err(CheatParseError::NothingToRepeat)
    );
    assert_eq!(
        parse_cheat_line("motherlode"),
        Err(CheatParseError::UnknownCommand("motherlode".into()))
    );
    assert_eq!(
        parse_cheat_line("move_objects"),
        Err(CheatParseError::MissingArgument {
            command: "move_objects",
            expected: "on|off"
        })
    );
    assert_eq!(
        parse_cheat_line("move_objects maybe"),
        Err(CheatParseError::InvalidArgument {
            command: "move_objects",
            expected: "on|off",
            got: "maybe".into()
        })
    );
    assert_eq!(
        parse_cheat_line("autonomy 101"),
        Err(CheatParseError::OutOfRange {
            command: "autonomy",
            min: 0,
            max: 100,
            got: 101
        })
    );
    assert_eq!(
        parse_cheat_line("set_hour 24"),
        Err(CheatParseError::OutOfRange {
            command: "set_hour",
            min: 0,
            max: 23,
            got: 24
        })
    );
    assert_eq!(
        parse_cheat_line("sim_speed 1001"),
        Err(CheatParseError::OutOfRange {
            command: "sim_speed",
            min: -1000,
            max: 1000,
            got: 1001
        })
    );
    assert_eq!(
        parse_cheat_line("water_tool extra"),
        Err(CheatParseError::UnexpectedArguments {
            command: "water_tool"
        })
    );

    let too_many = format!("rosebud{}", ";!".repeat(MAX_CHAIN_COMMANDS));
    assert_eq!(
        parse_cheat_line(&too_many),
        Err(CheatParseError::ChainTooLong {
            max: MAX_CHAIN_COMMANDS
        })
    );
}

#[test]
fn console_editing_history_caret_blink_and_repeat_across_submits() {
    let mut console = CheatConsole::new();
    assert!(!console.is_open());
    assert!(console.toggle());
    assert!(console.is_open());

    // Type and edit mid-string.
    assert!(console.paste_text("rosebd"));
    assert!(console.move_left());
    assert!(console.insert_char('u'));
    assert_eq!(console.input(), "rosebud");
    assert!(console.move_end());
    assert_eq!(console.cursor(), 7);

    // Submit rosebud -> clears input, records history, updates feedback.
    let cmds = console.submit().expect("rosebud succeeds");
    assert_eq!(cmds, vec![CheatCommand::AddFunds(ROSEBUD_AMOUNT)]);
    assert_eq!(console.input(), "");
    assert_eq!(console.history(), &["rosebud".to_owned()]);
    assert_eq!(console.feedback().map(|f| f.kind), Some(FeedbackKind::Ok));

    // Standalone `!;!` repeats previous executed command (`rosebud`).
    assert!(console.paste_text("!;!"));
    let repeated = console.submit().expect("repeat succeeds");
    assert_eq!(repeated, vec![CheatCommand::AddFunds(ROSEBUD_AMOUNT); 2]);

    // Invalid command preserves input for editing and sets Error feedback.
    assert!(console.paste_text("bad_cmd"));
    assert!(console.submit().is_err());
    assert_eq!(console.input(), "bad_cmd");
    assert_eq!(
        console.feedback().map(|f| f.kind),
        Some(FeedbackKind::Error)
    );

    // History Up/Down navigation preserves draft and restores it at the bottom.
    assert!(console.history_up());
    assert_eq!(console.input(), "bad_cmd");
    assert!(console.history_up());
    assert_eq!(console.input(), "!;!");
    assert!(console.history_up());
    assert_eq!(console.input(), "rosebud");
    assert!(!console.history_up());
    assert!(console.history_down());
    assert_eq!(console.input(), "!;!");
    assert!(console.history_down());
    assert_eq!(console.input(), "bad_cmd");
    assert!(console.history_down());
    assert_eq!(console.input(), "bad_cmd");

    // Caret blinks on 500 ms interval and input length is bounded.
    assert!(!console.sync_blink(0));
    assert!(console.sync_blink(CARET_BLINK_US));
    assert!(console.paste_text(&"a".repeat(MAX_INPUT_LEN + 50)));
    assert_eq!(console.input().chars().count(), MAX_INPUT_LEN);

    // Closing the console blocks submit even if a valid draft remains in the buffer.
    console.close();
    assert_eq!(console.submit(), Err(CheatParseError::Closed));
}

#[test]
fn closed_console_refuses_submit_without_emitting_cheats() {
    let mut console = CheatConsole::new();
    console.open();
    assert!(console.paste_text("rosebud"));
    console.close();
    assert_eq!(console.submit(), Err(CheatParseError::Closed));
    assert!(console.history().is_empty());
}

#[test]
fn console_layout_and_rendering_across_viewports_and_live_install() {
    let mut console = CheatConsole::new();
    console.open();
    console.paste_text("help");
    assert_eq!(console.submit(), Ok(vec![CheatCommand::Help]));
    console.paste_text("rosebud;!;!");

    let art = Art::default();
    for size in [
        [320, 240],
        [640, 480],
        [800, 600],
        [1280, 720],
        [1920, 1080],
    ] {
        let layout = console.layout(size);
        assert!(layout.bar.x >= 0 && layout.bar.y >= 0);
        assert!(layout.bar.x + layout.bar.width <= size[0] as i32);
        assert!(layout.feedback_line.is_some());

        let mut frame = Frame::new(size[0], size[1], [0, 0, 0]);
        console.draw(&mut frame, &art);
        assert!(frame.rgb().iter().any(|&px| px != [0, 0, 0]));
    }

    let Some(root) = install() else {
        return;
    };
    let install = Install::detect(&root).expect("detect install");
    let ui = UiGraphics::open(&install).expect("open UIGraphics");
    let live_art = load_art(&ui, 1).expect("load cheat console art");
    assert!(
        live_art.missing.is_empty(),
        "missing native 1.91 cheat console resources: {:?}",
        live_art.missing
    );

    let mut frame = Frame::new(800, 600, [0, 0, 0]);
    console.draw(&mut frame, &live_art);
    let non_black = frame.rgb().iter().filter(|&&px| px != [0, 0, 0]).count();
    assert!(
        non_black > 8_000,
        "expected rich 1.91 cheat console rendering, got {non_black} non-black pixels"
    );

    // Verify `help` wrapped lines all fit inside feedback_line with shipped bitmap fonts.
    let font9 = live_art.font(9).expect("9pt shipped font");
    let help_msg = &console.feedback().expect("help feedback").message;
    for size in [[320, 240], [800, 600], [1920, 1080]] {
        let layout = console.layout_with_art(size, Some(&live_art));
        let fb_rect = layout.feedback_line.expect("feedback rect");
        let lines = font9.wrap(help_msg, fb_rect.width);
        let needed_h = lines.len() as i32 * font9.line_height.max(1);
        assert!(
            fb_rect.height >= needed_h,
            "help text ({} lines, {needed_h}px) did not fit in feedback_line ({}px) at {size:?}",
            lines.len(),
            fb_rect.height
        );
    }
}
